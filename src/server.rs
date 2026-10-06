use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(test)]
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

use serde::Deserialize;
use serde_json::json;
#[cfg(test)]
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::deck::{self, Diagnostic};
use crate::html;
#[cfg(test)]
use crate::presentation_plan::PlannedScreenState;
use crate::presentation_plan::{PresentationPlan, ScreenRouteInventory};
use crate::presentation_protocol::{
    MAX_CONTROL_BODY, SseEventV1 as SseEvent, decode_control_v1, error_payload_v1,
    project_snapshot_v1, session_event_v1 as project_session_event_v1, snapshot_json_v1,
};
#[cfg(test)]
use crate::presentation_session::SessionCommand;
use crate::presentation_session::{PresentationSession, SessionConflict};
use crate::theme;

#[derive(Debug, Clone)]
pub struct ServeOptions {
    pub source: PathBuf,
    pub theme_search_paths: Vec<PathBuf>,
    pub fallback_theme: Option<String>,
    pub fallback_theme_params: BTreeMap<String, String>,
    pub cli_theme_override: Option<String>,
    pub cli_theme_params: BTreeMap<String, String>,
    pub port: u16,
    pub speaker_port: Option<u16>,
    pub speaker_enabled: bool,
    pub open: bool,
    pub debounce: Duration,
    pub theme_switcher: bool,
}

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("failed to bind live server on 127.0.0.1:{port}: {source}")]
    Bind { port: u16, source: io::Error },
    #[error("live server error: {0}")]
    Io(#[from] io::Error),
}

pub fn serve(options: ServeOptions) -> Result<(), ServerError> {
    serve_with_browser_opener(options, open_browser)
}

fn serve_with_browser_opener(
    options: ServeOptions,
    browser_opener: impl Fn(&str) -> io::Result<()>,
) -> Result<(), ServerError> {
    serve_with_runtime(options, browser_opener, watch_live_source)
}

fn serve_with_runtime(
    options: ServeOptions,
    browser_opener: impl Fn(&str) -> io::Result<()>,
    runtime: impl FnOnce(&ServeOptions, &Arc<Mutex<ServerState>>) -> Result<(), ServerError>,
) -> Result<(), ServerError> {
    let host = start_live_host(options.clone())?;
    let bound_port = host.presentation_port();
    if options.port != 0 && bound_port != options.port {
        println!(
            "Presentation port {} is busy; using next available port {}.",
            options.port, bound_port
        );
    }
    if let (Some(requested), Some(actual)) = (host.requested_speaker_port, host.speaker_port())
        && requested != 0
        && requested != actual
    {
        println!("Speaker port {requested} is busy; using next available port {actual}.");
    }

    for line in startup_messages(&options, &host) {
        println!("{line}");
    }

    if options.open {
        for warning in open_active_views(&host, browser_opener) {
            eprintln!("{warning}");
        }
    }

    runtime(&options, &host.state)
}

fn watch_live_source(
    options: &ServeOptions,
    state: &Arc<Mutex<ServerState>>,
) -> Result<(), ServerError> {
    let mut snapshot = live_watch_snapshot(options, state);
    loop {
        thread::sleep(options.debounce);
        let next_snapshot = live_watch_snapshot(options, state);
        if next_snapshot != snapshot {
            snapshot = next_snapshot;
            // Let editors finish atomic writes before parsing the source file.
            thread::sleep(options.debounce);
            rebuild(options, state, false);
        }
    }
}

fn startup_messages(options: &ServeOptions, host: &LiveServerHost) -> Vec<String> {
    let mut messages = vec![
        format!("Serving {}", options.source.display()),
        format!("Presentation: {}", host.presentation_url()),
    ];
    if let Some(speaker_url) = host.speaker_url() {
        messages.push(format!("Speaker: {speaker_url}"));
    }
    messages.push("Press Ctrl-C to stop.".to_string());
    messages
}

fn open_active_views(
    host: &LiveServerHost,
    browser_opener: impl Fn(&str) -> io::Result<()>,
) -> Vec<String> {
    std::iter::once(host.presentation_url())
        .chain(host.speaker_url())
        .filter_map(|url| {
            browser_opener(url)
                .err()
                .map(|error| format!("warning: could not open {url}: {error}"))
        })
        .collect()
}

struct LiveServerHost {
    presentation_url: String,
    speaker_url: Option<String>,
    requested_speaker_port: Option<u16>,
    state: Arc<Mutex<ServerState>>,
    rebuild_coordinator: Arc<Mutex<RebuildCoordinator>>,
    shutdown: Arc<AtomicBool>,
    listener_handles: Vec<JoinHandle<()>>,
    connection_handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
    active_connections: Arc<ActiveConnections>,
}

#[derive(Default)]
struct ActiveConnections {
    next_id: AtomicU64,
    streams: Mutex<BTreeMap<u64, TcpStream>>,
    #[cfg(test)]
    fail_registration: AtomicBool,
}

impl ActiveConnections {
    fn register(&self, stream: &TcpStream) -> io::Result<u64> {
        #[cfg(test)]
        if self.fail_registration.load(Ordering::Acquire) {
            return Err(io::Error::other("injected connection registration failure"));
        }
        let tracked = stream.try_clone()?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.streams
            .lock()
            .expect("active streams poisoned")
            .insert(id, tracked);
        Ok(id)
    }

    fn finished(&self, id: u64) {
        self.streams
            .lock()
            .expect("active streams poisoned")
            .remove(&id);
    }

    fn shutdown_all(&self) {
        for (_, stream) in
            std::mem::take(&mut *self.streams.lock().expect("active streams poisoned"))
        {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

impl LiveServerHost {
    fn presentation_url(&self) -> &str {
        &self.presentation_url
    }

    fn speaker_url(&self) -> Option<&str> {
        self.speaker_url.as_deref()
    }

    fn presentation_port(&self) -> u16 {
        self.presentation_url
            .rsplit_once(':')
            .and_then(|(_, port)| port.parse().ok())
            .expect("live URL contains its bound port")
    }

    fn speaker_port(&self) -> Option<u16> {
        self.speaker_url.as_ref().map(|url| {
            url.rsplit_once(':')
                .and_then(|(_, port)| port.parse().ok())
                .expect("live URL contains its bound port")
        })
    }

    fn shutdown(&mut self) {
        if self.shutdown.swap(true, Ordering::SeqCst) {
            return;
        }
        let closed_hook = {
            let mut coordinator = self
                .rebuild_coordinator
                .lock()
                .expect("rebuild coordinator poisoned");
            coordinator.closed = true;
            #[cfg(test)]
            let hook = coordinator.closed_hook.clone();
            #[cfg(not(test))]
            let hook = ();
            hook
        };
        #[cfg(test)]
        if let Some(hook) = closed_hook {
            let _ = hook.send(());
        }
        #[cfg(not(test))]
        let _ = closed_hook;
        for handle in self.listener_handles.drain(..) {
            let _ = handle.join();
        }
        let mailboxes = {
            let state = self.state.lock().expect("server state poisoned");
            state.clients.values().cloned().collect::<Vec<_>>()
        };
        for mailbox in mailboxes {
            mailbox.close();
        }
        self.active_connections.shutdown_all();
        self.state
            .lock()
            .expect("server state poisoned")
            .clients
            .clear();
        let handles = std::mem::take(
            &mut *self
                .connection_handles
                .lock()
                .expect("connection handles poisoned"),
        );
        for handle in handles {
            let _ = handle.join();
        }
    }
}

impl Drop for LiveServerHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn start_live_host(options: ServeOptions) -> Result<LiveServerHost, ServerError> {
    start_live_host_with_listener_config(options, |listener, _| listener.set_nonblocking(true))
}

fn start_live_host_with_listener_config(
    options: ServeOptions,
    mut configure_listener: impl FnMut(&TcpListener, ViewRole) -> io::Result<()>,
) -> Result<LiveServerHost, ServerError> {
    let presentation_listener = bind_live_listener(options.port)?;
    let presentation_address = presentation_listener.local_addr()?;
    let bound_port = presentation_address.port();

    let (speaker_listener, requested_speaker_port) = if options.speaker_enabled {
        let requested = options.speaker_port.unwrap_or_else(|| {
            if options.port == 0 || bound_port == u16::MAX {
                0
            } else {
                bound_port + 1
            }
        });
        let listener = bind_live_listener_avoiding(requested, bound_port)?;
        (Some(listener), Some(requested))
    } else {
        (None, None)
    };
    let speaker_address = speaker_listener
        .as_ref()
        .map(TcpListener::local_addr)
        .transpose()?;
    let presentation_url = format!("http://{presentation_address}");
    let speaker_url = speaker_address.map(|address| format!("http://{address}"));

    // Complete every fallible listener operation before any listener thread is
    // started. If either role cannot be configured, both bound sockets are
    // dropped by this stack frame and no detached server remains behind.
    configure_listener(&presentation_listener, ViewRole::Presentation)?;
    if let Some(listener) = speaker_listener.as_ref() {
        configure_listener(listener, ViewRole::Speaker)?;
    }

    let deck_root = options
        .source
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let state = Arc::new(Mutex::new(ServerState::new_for_options(
        &options,
        deck_root,
        presentation_url.clone(),
        speaker_url.clone(),
    )));
    let rebuild_coordinator = state
        .lock()
        .expect("server state poisoned")
        .rebuild_coordinator
        .clone();
    rebuild(&options, &state, true);

    let server_options = Arc::new(options.clone());
    let shutdown = Arc::new(AtomicBool::new(false));
    let connection_handles = Arc::new(Mutex::new(Vec::new()));
    let active_connections = Arc::new(ActiveConnections::default());
    let mut listener_handles = Vec::new();
    listener_handles.push(spawn_listener(
        presentation_listener,
        ViewRole::Presentation,
        Arc::clone(&server_options),
        Arc::clone(&state),
        Arc::clone(&shutdown),
        Arc::clone(&connection_handles),
        Arc::clone(&active_connections),
    ));
    if let Some(listener) = speaker_listener {
        listener_handles.push(spawn_listener(
            listener,
            ViewRole::Speaker,
            Arc::clone(&server_options),
            Arc::clone(&state),
            Arc::clone(&shutdown),
            Arc::clone(&connection_handles),
            Arc::clone(&active_connections),
        ));
    }
    Ok(LiveServerHost {
        presentation_url,
        speaker_url,
        requested_speaker_port,
        state,
        rebuild_coordinator,
        shutdown,
        listener_handles,
        connection_handles,
        active_connections,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewRole {
    Presentation,
    Speaker,
}

fn spawn_listener(
    listener: TcpListener,
    role: ViewRole,
    options: Arc<ServeOptions>,
    state: Arc<Mutex<ServerState>>,
    shutdown: Arc<AtomicBool>,
    connection_handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
    active_connections: Arc<ActiveConnections>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while !shutdown.load(Ordering::Acquire) {
            let stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(_) => continue,
            };
            // The listeners use nonblocking mode so their accept loops can
            // observe shutdown. Accepted sockets are handled by blocking HTTP
            // parsers on worker threads, and some platforms inherit the
            // listener's nonblocking mode. Normalize every accepted socket
            // before registering it or spawning a worker.
            if stream.set_nonblocking(false).is_err() {
                continue;
            }
            reap_finished_connection_handles(&connection_handles);
            let connection_id = match active_connections.register(&stream) {
                Ok(connection_id) => connection_id,
                Err(_) => continue,
            };
            let state = Arc::clone(&state);
            let options = Arc::clone(&options);
            let active_connections = Arc::clone(&active_connections);
            let handle = thread::spawn(move || {
                let _ = handle_connection(stream, role, options, state);
                active_connections.finished(connection_id);
            });
            connection_handles
                .lock()
                .expect("connection handles poisoned")
                .push(handle);
        }
    })
}

fn reap_finished_connection_handles(connection_handles: &Mutex<Vec<JoinHandle<()>>>) {
    let finished = {
        let mut handles = connection_handles
            .lock()
            .expect("connection handles poisoned");
        let mut finished = Vec::new();
        let mut index = 0;
        while index < handles.len() {
            if handles[index].is_finished() {
                finished.push(handles.swap_remove(index));
            } else {
                index += 1;
            }
        }
        finished
    };
    for handle in finished {
        let _ = handle.join();
    }
}

fn open_browser(url: &str) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    let status = Command::new("open").arg(url).status()?;
    #[cfg(target_os = "linux")]
    let status = Command::new("xdg-open").arg(url).status()?;
    #[cfg(target_os = "windows")]
    let status = Command::new("cmd")
        .args(["/C", "start", "", url])
        .status()?;
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    return Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "opening a browser is not supported on this platform",
    ));
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "browser opener exited with status {status}"
        )))
    }
}

fn bind_live_listener(requested_port: u16) -> Result<TcpListener, ServerError> {
    if requested_port == 0 {
        return TcpListener::bind(("127.0.0.1", 0)).map_err(|source| ServerError::Bind {
            port: requested_port,
            source,
        });
    }

    let mut last_error = None;
    for port in requested_port..=u16::MAX {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => return Ok(listener),
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                last_error = Some(error);
            }
            Err(source) => return Err(ServerError::Bind { port, source }),
        }
    }

    Err(ServerError::Bind {
        port: requested_port,
        source: last_error
            .unwrap_or_else(|| io::Error::new(io::ErrorKind::AddrInUse, "no available port found")),
    })
}

fn bind_live_listener_avoiding(
    requested_port: u16,
    reserved_port: u16,
) -> Result<TcpListener, ServerError> {
    if requested_port == 0 {
        return bind_live_listener(0);
    }
    let mut port = requested_port;
    loop {
        if port != reserved_port {
            match TcpListener::bind(("127.0.0.1", port)) {
                Ok(listener) => return Ok(listener),
                Err(error) if error.kind() == io::ErrorKind::AddrInUse => {}
                Err(source) => return Err(ServerError::Bind { port, source }),
            }
        }
        if port == u16::MAX {
            break;
        }
        port += 1;
    }
    Err(ServerError::Bind {
        port: requested_port,
        source: io::Error::new(io::ErrorKind::AddrInUse, "no available port found"),
    })
}

#[derive(Debug)]
struct ServerState {
    last_good_deck: Option<deck::Deck>,
    last_good_rendered_theme: Option<theme::RenderedTheme>,
    last_good_base_html: String,
    last_good_preview_html: String,
    last_good_html: String,
    last_good_theme: Option<String>,
    last_good_theme_params: BTreeMap<String, String>,
    last_good_palette_parameter: String,
    last_good_theme_css: String,
    last_good_theme_dependency_root: Option<PathBuf>,
    last_good_theme_dependencies: Vec<String>,
    last_good_speaker_notes: crate::speaker_notes::SpeakerNotesCatalog,
    diagnostics: Vec<Diagnostic>,
    clients: BTreeMap<SubscriberId, Arc<SubscriberMailbox>>,
    next_subscriber_id: SubscriberId,
    sse_settings: SseSettings,
    #[cfg(test)]
    sse_metrics: Arc<SseMetrics>,
    #[cfg(test)]
    request_paths: Vec<String>,
    deck_root: PathBuf,
    theme_switcher_enabled: bool,
    theme_switcher_catalog: Vec<ThemeSwitcherTheme>,
    rebuild_coordinator: Arc<Mutex<RebuildCoordinator>>,
    // This is the accepted Last-good Theme override. Presenter intent for an
    // in-flight or invalid attempt lives separately in RebuildCoordinator.
    live_theme_override: Option<LiveThemeOverride>,
    presentation_url: String,
    speaker_url: Option<String>,
    session_clock_origin: Instant,
    session: PresentationSession,
}

impl ServerState {
    #[cfg(test)]
    fn new(deck_root: PathBuf) -> Self {
        Self {
            last_good_deck: None,
            last_good_rendered_theme: None,
            last_good_base_html: initial_html(),
            last_good_preview_html: initial_html(),
            last_good_html: resolve_live_generation(
                &decorate_live_html(&initial_html(), false, "http://127.0.0.1:3000"),
                0,
            ),
            last_good_theme: None,
            last_good_theme_params: BTreeMap::new(),
            last_good_palette_parameter: "variant".to_string(),
            last_good_theme_css: String::new(),
            last_good_theme_dependency_root: None,
            last_good_theme_dependencies: Vec::new(),
            last_good_speaker_notes: crate::speaker_notes::SpeakerNotesCatalog::default(),
            diagnostics: Vec::new(),
            clients: BTreeMap::new(),
            next_subscriber_id: 1,
            sse_settings: SseSettings::default(),
            #[cfg(test)]
            sse_metrics: Arc::new(SseMetrics::default()),
            #[cfg(test)]
            request_paths: Vec::new(),
            theme_switcher_enabled: false,
            theme_switcher_catalog: Vec::new(),
            rebuild_coordinator: Arc::new(Mutex::new(RebuildCoordinator::default())),
            live_theme_override: None,
            deck_root,
            presentation_url: "http://127.0.0.1:3000".to_string(),
            speaker_url: None,
            session_clock_origin: Instant::now(),
            session: PresentationSession::default(),
        }
    }

    fn new_for_options(
        options: &ServeOptions,
        deck_root: PathBuf,
        presentation_url: String,
        speaker_url: Option<String>,
    ) -> Self {
        Self {
            last_good_deck: None,
            last_good_rendered_theme: None,
            last_good_base_html: initial_html(),
            last_good_preview_html: initial_html(),
            last_good_html: resolve_live_generation(
                &decorate_live_html(&initial_html(), options.theme_switcher, &presentation_url),
                0,
            ),
            last_good_theme: None,
            last_good_theme_params: BTreeMap::new(),
            last_good_palette_parameter: "variant".to_string(),
            last_good_theme_css: String::new(),
            last_good_theme_dependency_root: None,
            last_good_theme_dependencies: Vec::new(),
            last_good_speaker_notes: crate::speaker_notes::SpeakerNotesCatalog::default(),
            diagnostics: Vec::new(),
            clients: BTreeMap::new(),
            next_subscriber_id: 1,
            sse_settings: SseSettings::default(),
            #[cfg(test)]
            sse_metrics: Arc::new(SseMetrics::default()),
            #[cfg(test)]
            request_paths: Vec::new(),
            theme_switcher_enabled: options.theme_switcher,
            theme_switcher_catalog: discover_theme_switcher_catalog(
                &deck_root,
                &options.theme_search_paths,
            ),
            rebuild_coordinator: Arc::new(Mutex::new(RebuildCoordinator::default())),
            live_theme_override: None,
            deck_root,
            presentation_url,
            speaker_url,
            session_clock_origin: Instant::now(),
            session: PresentationSession::default(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct ThemeSwitcherTheme {
    name: String,
    palette_parameter: String,
    variants: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveThemeOverride {
    theme: String,
    variant: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SseDeliveryClass {
    DiagnosticsSnapshot,
    SessionSnapshot,
    RenderGeneration,
}

fn sse_delivery_class(event: &SseEvent) -> SseDeliveryClass {
    match event {
        SseEvent::Diagnostics { .. } => SseDeliveryClass::DiagnosticsSnapshot,
        SseEvent::Session { .. } => SseDeliveryClass::SessionSnapshot,
        SseEvent::Reload { .. }
        | SseEvent::ThemeCss { .. }
        | SseEvent::Assets { .. }
        | SseEvent::RenderState { .. } => SseDeliveryClass::RenderGeneration,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RenderSeverity {
    ThemeCss,
    Assets,
    Reload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingRender {
    generation: u64,
    severity: RenderSeverity,
}

impl PendingRender {
    fn from_event(event: &SseEvent) -> Option<Self> {
        match event {
            SseEvent::Reload { generation } => Some(Self {
                generation: *generation,
                severity: RenderSeverity::Reload,
            }),
            SseEvent::Assets { generation, .. } => Some(Self {
                generation: *generation,
                severity: RenderSeverity::Assets,
            }),
            SseEvent::ThemeCss { generation, .. } => Some(Self {
                generation: *generation,
                severity: RenderSeverity::ThemeCss,
            }),
            SseEvent::Diagnostics { .. }
            | SseEvent::Session { .. }
            | SseEvent::RenderState { .. } => None,
        }
    }

    fn merge(self, incoming: Self) -> Self {
        if incoming.generation < self.generation {
            return self;
        }
        Self {
            generation: incoming.generation,
            severity: self.severity.max(incoming.severity),
        }
    }

    fn into_event(self) -> SseEvent {
        match self.severity {
            RenderSeverity::Reload => SseEvent::Reload {
                generation: self.generation,
            },
            RenderSeverity::Assets => SseEvent::Assets {
                generation: self.generation,
                version: self.generation,
                theme_css_href: theme_css_href(self.generation),
            },
            RenderSeverity::ThemeCss => SseEvent::ThemeCss {
                generation: self.generation,
                href: theme_css_href(self.generation),
            },
        }
    }
}

const SSE_MAILBOX_CAPACITY: usize = 3;
const SSE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const SSE_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy)]
struct SseSettings {
    // Loopback clients should never need fifteen seconds to observe liveness,
    // while this interval avoids needless traffic during a normal talk.
    heartbeat_interval: Duration,
    // Five seconds is generous for three current-state loopback writes and
    // finite enough to reap a suspended browser before it retains resources.
    write_timeout: Duration,
}

impl Default for SseSettings {
    fn default() -> Self {
        Self {
            heartbeat_interval: SSE_HEARTBEAT_INTERVAL,
            write_timeout: SSE_WRITE_TIMEOUT,
        }
    }
}

#[derive(Default)]
struct MailboxState {
    diagnostics: Option<SseEvent>,
    session: Option<SseEvent>,
    render: Option<PendingRender>,
    closed: bool,
    #[cfg(test)]
    max_depth: usize,
    #[cfg(test)]
    after_drain_hook: Option<Arc<dyn Fn() + Send + Sync>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublishResult {
    Published,
    Closed,
}

enum MailboxWake {
    Events(Vec<SseEvent>),
    Heartbeat,
    Closed,
}

#[derive(Default)]
struct SubscriberMailbox {
    state: Mutex<MailboxState>,
    changed: Condvar,
}

impl std::fmt::Debug for SubscriberMailbox {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.lock().map_err(|_| std::fmt::Error)?;
        formatter
            .debug_struct("SubscriberMailbox")
            .field("depth", &mailbox_depth(&state))
            .field("closed", &state.closed)
            .finish()
    }
}

impl SubscriberMailbox {
    fn publish_batch(&self, events: &[SseEvent]) -> PublishResult {
        let mut state = self.state.lock().expect("SSE mailbox poisoned");
        if state.closed {
            return PublishResult::Closed;
        }
        for event in events {
            match sse_delivery_class(event) {
                SseDeliveryClass::DiagnosticsSnapshot => {
                    state.diagnostics = Some(event.clone());
                }
                SseDeliveryClass::SessionSnapshot => {
                    let SseEvent::Session {
                        generation,
                        revision,
                        ..
                    } = event
                    else {
                        unreachable!("Session delivery class is exhaustive")
                    };
                    let replace = match state.session.as_ref() {
                        Some(SseEvent::Session {
                            generation: current_generation,
                            revision: current_revision,
                            ..
                        }) => (*generation, *revision) >= (*current_generation, *current_revision),
                        _ => true,
                    };
                    if replace {
                        state.session = Some(event.clone());
                    }
                }
                SseDeliveryClass::RenderGeneration => {
                    if matches!(event, SseEvent::RenderState { .. }) {
                        debug_assert!(false, "RenderState is bootstrap-only");
                        continue;
                    }
                    let incoming = PendingRender::from_event(event)
                        .expect("incremental render events have render metadata");
                    state.render = Some(
                        state
                            .render
                            .map_or(incoming, |current| current.merge(incoming)),
                    );
                }
            }
        }
        #[cfg(test)]
        {
            state.max_depth = state.max_depth.max(mailbox_depth(&state));
        }
        debug_assert!(mailbox_depth(&state) <= SSE_MAILBOX_CAPACITY);
        self.changed.notify_one();
        PublishResult::Published
    }

    fn wait_and_drain(&self, heartbeat_interval: Duration) -> MailboxWake {
        let mut state = self.state.lock().expect("SSE mailbox poisoned");
        if !state.closed && mailbox_depth(&state) == 0 {
            let (next, timeout) = self
                .changed
                .wait_timeout_while(state, heartbeat_interval, |state| {
                    !state.closed && mailbox_depth(state) == 0
                })
                .expect("SSE mailbox poisoned while waiting");
            state = next;
            if timeout.timed_out() && !state.closed && mailbox_depth(&state) == 0 {
                return MailboxWake::Heartbeat;
            }
        }
        if state.closed {
            return MailboxWake::Closed;
        }
        let mut events = Vec::with_capacity(SSE_MAILBOX_CAPACITY);
        if let Some(event) = state.diagnostics.take() {
            events.push(event);
        }
        if let Some(event) = state.session.take() {
            events.push(event);
        }
        if let Some(render) = state.render.take() {
            events.push(render.into_event());
        }
        #[cfg(test)]
        let hook = state.after_drain_hook.clone();
        drop(state);
        #[cfg(test)]
        if let Some(hook) = hook {
            hook();
        }
        MailboxWake::Events(events)
    }

    fn close(&self) {
        let mut state = self.state.lock().expect("SSE mailbox poisoned");
        state.closed = true;
        self.changed.notify_all();
    }

    #[cfg(test)]
    fn is_closed(&self) -> bool {
        self.state.lock().expect("SSE mailbox poisoned").closed
    }
}

fn mailbox_depth(state: &MailboxState) -> usize {
    usize::from(state.diagnostics.is_some())
        + usize::from(state.session.is_some())
        + usize::from(state.render.is_some())
}

type SubscriberId = u64;

#[cfg(test)]
#[derive(Debug, Default)]
struct SseMetrics {
    active_writers: AtomicUsize,
    registrations: AtomicU64,
    deregistrations: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveUpdate {
    Reload,
    ThemeCss,
    Assets,
}

#[derive(Clone)]
struct SourceSnapshot {
    text: String,
    #[cfg(test)]
    fingerprint: String,
}

#[derive(Clone)]
struct RebuildAttempt {
    ticket: u64,
    source: Result<SourceSnapshot, Vec<Diagnostic>>,
    source_path: PathBuf,
    requested_theme_override: Option<LiveThemeOverride>,
    initial: bool,
    theme_switcher_enabled: bool,
    presentation_url: String,
}

struct PreparedLiveGeneration {
    ticket: u64,
    #[cfg(test)]
    fingerprint: Option<String>,
    requested_theme_override: Option<LiveThemeOverride>,
    initial: bool,
    deck: deck::Deck,
    rendered_theme: theme::RenderedTheme,
    base_html: String,
    preview_html: String,
    html: String,
    theme: Option<String>,
    theme_params: BTreeMap<String, String>,
    palette_parameter: String,
    theme_css: String,
    theme_dependency_root: Option<PathBuf>,
    theme_dependencies: Vec<String>,
    routes: ScreenRouteInventory,
    speaker_notes: crate::speaker_notes::SpeakerNotesCatalog,
    diagnostics: Vec<Diagnostic>,
    section_count: usize,
}

struct PreparedInvalidGeneration {
    ticket: u64,
    #[cfg(test)]
    fingerprint: Option<String>,
    #[cfg(test)]
    requested_theme_override: Option<LiveThemeOverride>,
    diagnostics: Vec<Diagnostic>,
    log_message: String,
}

enum PreparedRebuild {
    Valid(PreparedLiveGeneration),
    Invalid(PreparedInvalidGeneration),
}

impl PreparedRebuild {
    fn ticket(&self) -> u64 {
        match self {
            Self::Valid(prepared) => prepared.ticket,
            Self::Invalid(prepared) => prepared.ticket,
        }
    }

    #[cfg(test)]
    fn fingerprint(&self) -> Option<&str> {
        match self {
            Self::Valid(prepared) => prepared.fingerprint.as_deref(),
            Self::Invalid(prepared) => prepared.fingerprint.as_deref(),
        }
    }

    #[cfg(test)]
    fn is_valid(&self) -> bool {
        matches!(self, Self::Valid(_))
    }
}

#[derive(Debug, Clone)]
enum CommitOutcome {
    AcceptedValid,
    AcceptedInvalid { diagnostics: Vec<Diagnostic> },
    Superseded,
    ShuttingDown,
}

#[cfg(test)]
#[derive(Clone)]
struct RebuildHookEvent {
    phase: RebuildHookPhase,
    ticket: u64,
    fingerprint: Option<String>,
    requested_theme_override: Option<LiveThemeOverride>,
    valid: Option<bool>,
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum RebuildHookPhase {
    AfterAttemptCaptured,
    BeforeCommit,
    AfterStateInstalledBeforeBroadcast,
}

#[cfg(test)]
type RebuildTestHook = Arc<dyn Fn(RebuildHookEvent) + Send + Sync>;

struct RebuildCoordinator {
    next_ticket: u64,
    newest_requested: Option<u64>,
    requested_theme_override: Option<LiveThemeOverride>,
    closed: bool,
    #[cfg(test)]
    test_hook: Option<RebuildTestHook>,
    #[cfg(test)]
    closed_hook: Option<Sender<()>>,
    #[cfg(test)]
    control_before_state_lock_hook: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Default for RebuildCoordinator {
    fn default() -> Self {
        Self {
            next_ticket: 1,
            newest_requested: None,
            requested_theme_override: None,
            closed: false,
            #[cfg(test)]
            test_hook: None,
            #[cfg(test)]
            closed_hook: None,
            #[cfg(test)]
            control_before_state_lock_hook: None,
        }
    }
}

impl std::fmt::Debug for RebuildCoordinator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebuildCoordinator")
            .field("next_ticket", &self.next_ticket)
            .field("newest_requested", &self.newest_requested)
            .field("requested_theme_override", &self.requested_theme_override)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

fn rebuild(
    options: &ServeOptions,
    state: &Arc<Mutex<ServerState>>,
    initial: bool,
) -> CommitOutcome {
    rebuild_with_requested_theme(options, state, initial, None)
}

fn rebuild_with_requested_theme(
    options: &ServeOptions,
    state: &Arc<Mutex<ServerState>>,
    initial: bool,
    requested_theme_override: Option<LiveThemeOverride>,
) -> CommitOutcome {
    let attempt = match begin_rebuild_attempt(options, state, initial, requested_theme_override) {
        Ok(attempt) => attempt,
        Err(outcome) => return outcome,
    };
    let prepared = prepare_live_generation(attempt, options);
    #[cfg(test)]
    invoke_rebuild_hook(state, &prepared, RebuildHookPhase::BeforeCommit);
    commit_prepared(state, prepared)
}

fn begin_rebuild_attempt(
    options: &ServeOptions,
    state: &Arc<Mutex<ServerState>>,
    initial: bool,
    requested_theme_override: Option<LiveThemeOverride>,
) -> Result<RebuildAttempt, CommitOutcome> {
    let (coordinator, accepted_theme_override, theme_switcher_enabled, presentation_url) = {
        let state = state.lock().expect("server state poisoned");
        (
            state.rebuild_coordinator.clone(),
            state.live_theme_override.clone(),
            state.theme_switcher_enabled,
            state.presentation_url.clone(),
        )
    };
    let (attempt, hook) = {
        // Attempt order is freshness order, while session generation counts
        // only accepted valid Decks. Source capture stays under this mutex so
        // an old delayed read cannot receive a ticket after a newer request.
        // Accepted presentation state is never locked during file I/O.
        let mut coordinator = coordinator.lock().expect("rebuild coordinator poisoned");
        if coordinator.closed {
            return Err(CommitOutcome::ShuttingDown);
        }
        if let Some(requested) = requested_theme_override {
            coordinator.requested_theme_override = Some(requested);
        }
        let captured_override = coordinator
            .requested_theme_override
            .clone()
            .or(accepted_theme_override);
        let source = match fs::read_to_string(&options.source) {
            Ok(text) => {
                #[cfg(test)]
                let fingerprint = format!("{:x}", Sha256::digest(text.as_bytes()));
                Ok(SourceSnapshot {
                    text,
                    #[cfg(test)]
                    fingerprint,
                })
            }
            Err(error) => Err(vec![Diagnostic::error(
                None,
                format!(
                    "failed to read source file {}: {error}",
                    options.source.display()
                ),
            )]),
        };
        let ticket = coordinator.next_ticket;
        coordinator.next_ticket = coordinator
            .next_ticket
            .checked_add(1)
            .expect("live rebuild ticket overflow");
        coordinator.newest_requested = Some(ticket);
        let attempt = RebuildAttempt {
            ticket,
            source,
            source_path: options.source.clone(),
            requested_theme_override: captured_override,
            initial,
            theme_switcher_enabled,
            presentation_url,
        };
        #[cfg(test)]
        let hook = coordinator.test_hook.clone();
        #[cfg(not(test))]
        let hook = ();
        (attempt, hook)
    };
    #[cfg(test)]
    if let Some(hook) = hook {
        hook(RebuildHookEvent {
            phase: RebuildHookPhase::AfterAttemptCaptured,
            ticket: attempt.ticket,
            fingerprint: attempt
                .source
                .as_ref()
                .ok()
                .map(|snapshot| snapshot.fingerprint.clone()),
            requested_theme_override: attempt.requested_theme_override.clone(),
            valid: None,
        });
    }
    #[cfg(not(test))]
    let _ = hook;
    Ok(attempt)
}

fn prepare_live_generation(attempt: RebuildAttempt, options: &ServeOptions) -> PreparedRebuild {
    let snapshot = match attempt.source {
        Ok(snapshot) => snapshot,
        Err(diagnostics) => {
            return PreparedRebuild::Invalid(PreparedInvalidGeneration {
                ticket: attempt.ticket,
                #[cfg(test)]
                fingerprint: None,
                #[cfg(test)]
                requested_theme_override: attempt.requested_theme_override,
                log_message: diagnostics
                    .first()
                    .map(|diagnostic| diagnostic.message.clone())
                    .unwrap_or_else(|| "could not read Source file".to_string()),
                diagnostics,
            });
        }
    };
    #[cfg(test)]
    let fingerprint = Some(snapshot.fingerprint.clone());
    let mut deck = match deck::parse_source_text(&snapshot.text, Some(attempt.source_path)) {
        Ok(deck) => deck,
        Err(error) => {
            return PreparedRebuild::Invalid(PreparedInvalidGeneration {
                ticket: attempt.ticket,
                #[cfg(test)]
                fingerprint,
                #[cfg(test)]
                requested_theme_override: attempt.requested_theme_override,
                diagnostics: vec![Diagnostic::error(None, error.to_string())],
                log_message: error.to_string(),
            });
        }
    };
    let rendered_theme = match resolve_live_theme(
        &mut deck,
        options,
        attempt.requested_theme_override.as_ref(),
    ) {
        Ok(rendered_theme) => rendered_theme,
        Err(error) => {
            return PreparedRebuild::Invalid(PreparedInvalidGeneration {
                ticket: attempt.ticket,
                #[cfg(test)]
                fingerprint,
                #[cfg(test)]
                requested_theme_override: attempt.requested_theme_override,
                diagnostics: vec![Diagnostic::error(None, error.to_string())],
                log_message: format!("warning: could not resolve live Theme: {error}"),
            });
        }
    };
    if deck.diagnostics.iter().any(Diagnostic::is_fatal) {
        let fatal_count = deck
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.is_fatal())
            .count();
        return PreparedRebuild::Invalid(PreparedInvalidGeneration {
            ticket: attempt.ticket,
            #[cfg(test)]
            fingerprint,
            #[cfg(test)]
            requested_theme_override: attempt.requested_theme_override,
            diagnostics: deck.diagnostics,
            log_message: format!(
                "Current Source has {fatal_count} fatal diagnostic(s); keeping Last good Deck."
            ),
        });
    }

    let routes = PresentationPlan::for_theme_api_v1(&deck).into_screen_routes();
    let speaker_notes = crate::speaker_notes::SpeakerNotesCatalog::from_deck(&deck);
    let base_html = html::render_debug_html(&deck, &rendered_theme);
    let preview_html = html::render_preview_html(&deck, &rendered_theme);
    let decorated_html = decorate_live_html(
        &base_html,
        attempt.theme_switcher_enabled,
        &attempt.presentation_url,
    );
    let theme_dependency_root = rendered_theme.manifest.path.parent().map(Path::to_path_buf);
    let theme_dependencies = theme::theme_dependency_paths(&rendered_theme.manifest)
        .map(str::to_string)
        .collect();
    let theme_css = rendered_theme.screen_css.clone();
    let diagnostics = deck.diagnostics.clone();
    let section_count = deck.sections.len();
    let theme = deck.metadata.theme.clone();
    let theme_params = rendered_theme.params.clone();
    let palette_parameter = rendered_theme.manifest.palette_parameter.clone();
    PreparedRebuild::Valid(PreparedLiveGeneration {
        ticket: attempt.ticket,
        #[cfg(test)]
        fingerprint,
        requested_theme_override: attempt.requested_theme_override,
        initial: attempt.initial,
        deck,
        rendered_theme,
        base_html,
        preview_html,
        html: decorated_html,
        theme,
        theme_params,
        palette_parameter,
        theme_css,
        theme_dependency_root,
        theme_dependencies,
        routes,
        speaker_notes,
        diagnostics,
        section_count,
    })
}

#[cfg(test)]
fn invoke_rebuild_hook(
    state: &Arc<Mutex<ServerState>>,
    prepared: &PreparedRebuild,
    phase: RebuildHookPhase,
) {
    let coordinator = state
        .lock()
        .expect("server state poisoned")
        .rebuild_coordinator
        .clone();
    let hook = coordinator
        .lock()
        .expect("rebuild coordinator poisoned")
        .test_hook
        .clone();
    if let Some(hook) = hook {
        let requested_theme_override = match prepared {
            PreparedRebuild::Valid(prepared) => prepared.requested_theme_override.clone(),
            PreparedRebuild::Invalid(prepared) => prepared.requested_theme_override.clone(),
        };
        hook(RebuildHookEvent {
            phase,
            ticket: prepared.ticket(),
            fingerprint: prepared.fingerprint().map(str::to_string),
            requested_theme_override,
            valid: Some(prepared.is_valid()),
        });
    }
}

fn commit_prepared(state: &Arc<Mutex<ServerState>>, prepared: PreparedRebuild) -> CommitOutcome {
    let coordinator = state
        .lock()
        .expect("server state poisoned")
        .rebuild_coordinator
        .clone();
    let coordinator = coordinator.lock().expect("rebuild coordinator poisoned");
    if coordinator.closed {
        return CommitOutcome::ShuttingDown;
    }
    if coordinator.newest_requested != Some(prepared.ticket()) {
        return CommitOutcome::Superseded;
    }

    match prepared {
        PreparedRebuild::Valid(prepared) => {
            let (section_count, diagnostics) = {
                let mut state = state.lock().expect("server state poisoned");
                let html_changed = state.last_good_base_html != prepared.base_html;
                let theme_css_changed = state.last_good_theme_css != prepared.theme_css;
                let live_update = if prepared.initial {
                    None
                } else if html_changed {
                    Some(LiveUpdate::Reload)
                } else if theme_css_changed {
                    Some(LiveUpdate::ThemeCss)
                } else {
                    Some(LiveUpdate::Assets)
                };
                state.last_good_deck = Some(prepared.deck);
                state.last_good_rendered_theme = Some(prepared.rendered_theme);
                state.last_good_base_html = prepared.base_html;
                state.last_good_preview_html = prepared.preview_html;
                state.last_good_theme = prepared.theme;
                state.last_good_theme_params = prepared.theme_params;
                state.last_good_palette_parameter = prepared.palette_parameter;
                state.last_good_theme_css = prepared.theme_css;
                state.last_good_theme_dependency_root = prepared.theme_dependency_root;
                state.last_good_theme_dependencies = prepared.theme_dependencies;
                state.last_good_speaker_notes = prepared.speaker_notes;
                state.diagnostics = prepared.diagnostics.clone();
                state.live_theme_override = prepared.requested_theme_override.clone();
                state.session.install_routes(prepared.routes);
                let generation = state.session.generation();
                state.last_good_html = resolve_live_generation(&prepared.html, generation);
                let mut events = vec![diagnostics_event(&state.diagnostics), session_event(&state)];
                if let Some(update) = live_update {
                    events.push(live_update_event(update, generation));
                }
                #[cfg(test)]
                if let Some(hook) = coordinator.test_hook.clone() {
                    hook(RebuildHookEvent {
                        phase: RebuildHookPhase::AfterStateInstalledBeforeBroadcast,
                        ticket: prepared.ticket,
                        fingerprint: prepared.fingerprint.clone(),
                        requested_theme_override: prepared.requested_theme_override.clone(),
                        valid: Some(true),
                    });
                }
                broadcast_event_batch_locked(&mut state, &events);
                (prepared.section_count, state.diagnostics.clone())
            };
            print_diagnostics(&diagnostics);
            println!("Rebuilt {section_count} section(s).");
            CommitOutcome::AcceptedValid
        }
        PreparedRebuild::Invalid(prepared) => {
            {
                let mut state = state.lock().expect("server state poisoned");
                state.diagnostics = prepared.diagnostics.clone();
                let events = vec![diagnostics_event(&state.diagnostics)];
                #[cfg(test)]
                if let Some(hook) = coordinator.test_hook.clone() {
                    hook(RebuildHookEvent {
                        phase: RebuildHookPhase::AfterStateInstalledBeforeBroadcast,
                        ticket: prepared.ticket,
                        fingerprint: prepared.fingerprint.clone(),
                        requested_theme_override: prepared.requested_theme_override.clone(),
                        valid: Some(false),
                    });
                }
                broadcast_event_batch_locked(&mut state, &events);
            }
            print_diagnostics(&prepared.diagnostics);
            eprintln!("{}", prepared.log_message);
            CommitOutcome::AcceptedInvalid {
                diagnostics: prepared.diagnostics,
            }
        }
    }
}

fn resolve_live_theme(
    deck: &mut deck::Deck,
    options: &ServeOptions,
    live_theme_override: Option<&LiveThemeOverride>,
) -> Result<theme::RenderedTheme, theme::ThemeError> {
    let parsed_theme = deck.metadata.theme.clone();
    if let Some(override_theme) = live_theme_override {
        deck.metadata.theme = Some(override_theme.theme.clone());
    } else if let Some(theme) = &options.cli_theme_override {
        deck.metadata.theme = Some(theme.clone());
    } else if deck.metadata.theme.is_none() {
        deck.metadata.theme = options.fallback_theme.clone();
    }

    let mut supplied = if let Some(override_theme) = live_theme_override {
        if parsed_theme.as_deref() == Some(override_theme.theme.as_str()) {
            deck.metadata.theme_params.clone()
        } else {
            BTreeMap::new()
        }
    } else if let Some(theme) = &options.cli_theme_override {
        if parsed_theme.as_deref() == Some(theme.as_str()) {
            deck.metadata.theme_params.clone()
        } else {
            BTreeMap::new()
        }
    } else if deck.metadata.theme == options.fallback_theme && deck.metadata.theme_params.is_empty()
    {
        options.fallback_theme_params.clone()
    } else {
        deck.metadata.theme_params.clone()
    };
    let Some(theme_name) = deck.metadata.theme.as_deref() else {
        supplied.extend(options.cli_theme_params.clone());
        deck.metadata.theme_params = supplied;
        let deck_root = deck.deck_root().unwrap_or_else(|| Path::new("."));
        let rendered_theme = theme::render_deck_theme(
            None,
            deck_root,
            &options.theme_search_paths,
            &deck.metadata.theme_params,
        )?;
        theme::prepare_deck_for_theme(deck, &rendered_theme.manifest)?;
        return Ok(rendered_theme);
    };
    let deck_root = deck.deck_root().unwrap_or_else(|| Path::new("."));
    let manifest = theme::load_named_theme(theme_name, deck_root, &options.theme_search_paths)?;
    supplied.extend(options.cli_theme_params.clone());
    if let Some(override_theme) = live_theme_override
        && override_theme.theme == theme_name
        && let Some(variant) = &override_theme.variant
    {
        supplied.insert(manifest.palette_parameter.clone(), variant.clone());
    }
    let (theme_params, warnings) =
        theme::validate_theme_params_best_effort_with_warnings(&manifest, &supplied);
    deck.metadata.theme_params = theme_params;
    deck.diagnostics.extend(
        warnings
            .into_iter()
            .map(|warning| Diagnostic::warning(None, warning)),
    );
    let rendered_theme = theme::render_theme(&manifest, &deck.metadata.theme_params)?;
    theme::prepare_deck_for_theme(deck, &rendered_theme.manifest)?;
    Ok(rendered_theme)
}

fn print_diagnostics(diagnostics: &[Diagnostic]) {
    for diagnostic in diagnostics {
        let severity = format!("{:?}", diagnostic.severity);
        let location = diagnostic.span.as_ref().map_or_else(
            || "<unknown>".to_string(),
            |span| {
                let path = span
                    .source_path
                    .as_ref()
                    .map_or_else(|| "<source>".to_string(), |path| path.display().to_string());
                format!("{}:{}:{}", path, span.line, span.column)
            },
        );
        eprintln!("{location}: {severity}: {}", diagnostic.message);
    }
}

fn handle_connection(
    stream: TcpStream,
    role: ViewRole,
    options: Arc<ServeOptions>,
    state: Arc<Mutex<ServerState>>,
) -> io::Result<()> {
    handle_connection_with_timeout(stream, role, options, state, Duration::from_secs(5))
}

fn handle_connection_with_timeout(
    mut stream: TcpStream,
    role: ViewRole,
    options: Arc<ServeOptions>,
    state: Arc<Mutex<ServerState>>,
    read_timeout: Duration,
) -> io::Result<()> {
    stream.set_read_timeout(Some(read_timeout))?;
    let request = match read_http_request(&stream) {
        Ok(request) => request,
        Err(error) if !error.respond => return Ok(()),
        Err(error) => return write_json_error(&mut stream, error.status, &error.message),
    };
    let uri = request.target.trim_start_matches('/');
    let (encoded_path, query) = uri.split_once('?').unwrap_or((uri, ""));
    let Some(path) = percent_decode_path(encoded_path) else {
        return write_bad_request(
            &mut stream,
            "request path is not valid percent-encoded UTF-8",
        );
    };
    #[cfg(test)]
    state
        .lock()
        .expect("server state poisoned")
        .request_paths
        .push(request.target.clone());

    match path.as_str() {
        "" | "index.html" => {
            if request.method != "GET" {
                return write_method_not_allowed(&mut stream, "GET");
            }
            let html = {
                let state = state.lock().expect("server state poisoned");
                match role {
                    ViewRole::Presentation => state.last_good_html.clone(),
                    ViewRole::Speaker => {
                        crate::speaker_view::render(&crate::speaker_view::SpeakerViewConfig {
                            presentation_url: state.presentation_url.clone(),
                            rendered_generation: state.session.generation(),
                        })
                    }
                }
            };
            write_response(&mut stream, "text/html; charset=utf-8", html.as_bytes())
        }
        "presentation.html" => {
            if request.method != "GET" {
                return write_method_not_allowed(&mut stream, "GET");
            }
            let preview = match parse_preview_query(query) {
                Ok(preview) => preview,
                Err(message) => return write_bad_request(&mut stream, message),
            };
            let (html, route_render, valid_bootstrap) = {
                let state = state.lock().expect("server state poisoned");
                let valid_bootstrap = preview.route.is_none_or(|route| {
                    preview.generation == Some(state.session.generation())
                        && state
                            .session
                            .routes()
                            .route_index(route.section, route.detail, route.step)
                            .is_some()
                });
                if preview.enabled {
                    let route_render = preview.route.and_then(|route| {
                        Some((
                            state.last_good_deck.clone()?,
                            state.last_good_rendered_theme.clone()?,
                            html::PreviewInitialState {
                                section: route.section,
                                detail: route.detail,
                                step: route.step,
                            },
                        ))
                    });
                    (
                        state.last_good_preview_html.clone(),
                        route_render,
                        valid_bootstrap,
                    )
                } else {
                    (state.last_good_html.clone(), None, valid_bootstrap)
                }
            };
            if !valid_bootstrap {
                return write_bad_request(
                    &mut stream,
                    "preview route or generation is not available in the current Deck",
                );
            }
            let html = route_render.map_or(html, |(deck, theme, initial)| {
                html::render_preview_html_at(&deck, &theme, initial)
            });
            if preview.enabled {
                write_preview_response(&mut stream, html.as_bytes())
            } else {
                write_response(&mut stream, "text/html; charset=utf-8", html.as_bytes())
            }
        }
        "zpres/session.json" => {
            if request.method != "GET" {
                return write_method_not_allowed(&mut stream, "GET");
            }
            let body = {
                let state = state.lock().expect("server state poisoned");
                snapshot_json_v1(&projected_session_v1(&state)).to_string()
            };
            write_response(
                &mut stream,
                "application/json; charset=utf-8",
                body.as_bytes(),
            )
        }
        "zpres/control" => {
            if request.method != "POST" {
                return write_method_not_allowed(&mut stream, "POST");
            }
            if let Err(error) = validate_state_change_request(&request, &state) {
                return write_state_change_rejection(&mut stream, error);
            }
            handle_control(&mut stream, &state, &request.body)
        }
        "zpres/theme-switcher.json" => {
            if request.method != "GET" {
                return write_method_not_allowed(&mut stream, "GET");
            }
            let body = theme_switcher_state_json(&state);
            write_response(
                &mut stream,
                "application/json; charset=utf-8",
                body.as_bytes(),
            )
        }
        "zpres/theme-switch" => {
            if request.method != "POST" {
                return write_method_not_allowed(&mut stream, "POST");
            }
            if let Err(error) = validate_state_change_request(&request, &state) {
                return write_state_change_rejection(&mut stream, error);
            }
            handle_theme_switch(&mut stream, &options, &state, &request.body)
        }
        "events" => {
            if request.method != "GET" {
                return write_method_not_allowed(&mut stream, "GET");
            }
            handle_events(stream, state)
        }
        asset_path => {
            if request.method != "GET" {
                return write_method_not_allowed(&mut stream, "GET");
            }
            if let Some((content_type, body)) = html::preview_runtime_asset(asset_path) {
                write_response(&mut stream, content_type, body.as_bytes())
            } else if let Some((content_type, body)) =
                html::core_runtime_asset(asset_path, theme::ThemeApiVersion::V1)
            {
                write_response(&mut stream, content_type, body.as_bytes())
            } else if asset_path == "assets/theme.css" {
                let body = {
                    let state = state.lock().expect("server state poisoned");
                    state.last_good_theme_css.clone()
                };
                write_response(&mut stream, "text/css; charset=utf-8", body.as_bytes())
            } else if let Some((content_type, body)) = read_theme_asset(asset_path, &state) {
                write_response(&mut stream, content_type, &body)
            } else if let Some((content_type, body)) = read_deck_asset(asset_path, &state) {
                write_response(&mut stream, content_type, &body)
            } else {
                write_not_found(&mut stream)
            }
        }
    }
}

const MAX_HEADER_BYTES: usize = 32 * 1024;

#[derive(Debug)]
struct HttpRequest {
    method: String,
    target: String,
    host: Option<String>,
    origin: Option<String>,
    content_type: Option<String>,
    sec_fetch_site: Option<String>,
    body: Vec<u8>,
}

#[derive(Debug)]
struct HttpRequestError {
    status: &'static str,
    message: String,
    respond: bool,
}

fn request_error(status: &'static str, message: impl Into<String>) -> HttpRequestError {
    HttpRequestError {
        status,
        message: message.into(),
        respond: true,
    }
}

fn silent_request_error(message: impl Into<String>) -> HttpRequestError {
    HttpRequestError {
        status: "400 Bad Request",
        message: message.into(),
        respond: false,
    }
}

fn read_http_request(stream: &TcpStream) -> Result<HttpRequest, HttpRequestError> {
    let clone = stream
        .try_clone()
        .map_err(|error| request_error("400 Bad Request", error.to_string()))?;
    let mut reader = BufReader::new(clone);
    let mut request_line = String::new();
    match reader.read_line(&mut request_line) {
        Ok(0) => return Err(silent_request_error("connection closed before a request")),
        Ok(_) => {}
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            return Err(silent_request_error(error.to_string()));
        }
        Err(error) => return Err(request_error("400 Bad Request", error.to_string())),
    }
    if request_line.len() > 8 * 1024 {
        return Err(request_error(
            "413 Content Too Large",
            "request line is too large",
        ));
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or_default();
    if method.is_empty()
        || target.is_empty()
        || !version.starts_with("HTTP/1.")
        || parts.next().is_some()
    {
        return Err(request_error(
            "400 Bad Request",
            "malformed HTTP request line",
        ));
    }

    let mut header_bytes = request_line.len();
    let mut content_length = None;
    let mut host = None;
    let mut origin = None;
    let mut content_type = None;
    let mut sec_fetch_site = None;
    let mut line = String::new();
    loop {
        line.clear();
        reader
            .read_line(&mut line)
            .map_err(|error| request_error("400 Bad Request", error.to_string()))?;
        if line.is_empty() {
            return Err(request_error(
                "400 Bad Request",
                "request headers ended unexpectedly",
            ));
        }
        header_bytes += line.len();
        if header_bytes > MAX_HEADER_BYTES {
            return Err(request_error(
                "413 Content Too Large",
                "request headers are too large",
            ));
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        let Some((name, value)) = line.trim_end_matches(['\r', '\n']).split_once(':') else {
            return Err(request_error("400 Bad Request", "malformed HTTP header"));
        };
        if name.eq_ignore_ascii_case("transfer-encoding") && !value.trim().is_empty() {
            return Err(request_error(
                "400 Bad Request",
                "transfer encoding is not supported",
            ));
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(request_error(
                    "400 Bad Request",
                    "duplicate Content-Length header",
                ));
            }
            let length = value
                .trim()
                .parse::<usize>()
                .map_err(|_| request_error("400 Bad Request", "invalid Content-Length header"))?;
            if length > MAX_CONTROL_BODY {
                return Err(request_error(
                    "413 Content Too Large",
                    "request body is too large",
                ));
            }
            content_length = Some(length);
        } else if name.eq_ignore_ascii_case("host") {
            retain_sensitive_header(&mut host, "Host", value)?;
        } else if name.eq_ignore_ascii_case("origin") {
            retain_sensitive_header(&mut origin, "Origin", value)?;
        } else if name.eq_ignore_ascii_case("content-type") {
            retain_sensitive_header(&mut content_type, "Content-Type", value)?;
        } else if name.eq_ignore_ascii_case("sec-fetch-site") {
            retain_sensitive_header(&mut sec_fetch_site, "Sec-Fetch-Site", value)?;
        }
    }
    let mut body = vec![0; content_length.unwrap_or(0)];
    std::io::Read::read_exact(&mut reader, &mut body)
        .map_err(|_| request_error("400 Bad Request", "request body ended unexpectedly"))?;
    Ok(HttpRequest {
        method: method.to_string(),
        target: target.to_string(),
        host,
        origin,
        content_type,
        sec_fetch_site,
        body,
    })
}

fn retain_sensitive_header(
    slot: &mut Option<String>,
    name: &'static str,
    value: &str,
) -> Result<(), HttpRequestError> {
    if slot.is_some() {
        return Err(request_error(
            "400 Bad Request",
            format!("duplicate {name} header"),
        ));
    }
    *slot = Some(value.trim().to_string());
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HttpOrigin {
    port: u16,
}

fn parse_http_origin(value: &str) -> Option<HttpOrigin> {
    let (scheme, authority) = value.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http")
        || authority.is_empty()
        || authority.contains(['/', '?', '#', '@'])
        || authority.chars().any(char::is_whitespace)
    {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, port.parse::<u16>().ok()?),
        None => (authority, 80),
    };
    if !host.eq_ignore_ascii_case("localhost") && host != "127.0.0.1" {
        return None;
    }
    Some(HttpOrigin { port })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StateChangeRequestError {
    Forbidden,
    UnsupportedMediaType,
}

fn validate_state_change_request(
    request: &HttpRequest,
    state: &Arc<Mutex<ServerState>>,
) -> Result<(), StateChangeRequestError> {
    // Host is retained as part of the parsed security context even though the
    // active-origin allowlist, not a caller-controlled Host value, authorizes
    // browser requests.
    let _host = request.host.as_deref();
    match (&request.origin, &request.sec_fetch_site) {
        (None, None) => {}
        (None, Some(_)) => return Err(StateChangeRequestError::Forbidden),
        (Some(origin), fetch_site) => {
            let Some(origin) = parse_http_origin(origin) else {
                return Err(StateChangeRequestError::Forbidden);
            };
            let allowed = {
                let state = state.lock().expect("server state poisoned");
                std::iter::once(state.presentation_url.as_str())
                    .chain(state.speaker_url.as_deref())
                    .filter_map(parse_http_origin)
                    .any(|allowed| allowed == origin)
            };
            if !allowed {
                return Err(StateChangeRequestError::Forbidden);
            }
            if let Some(fetch_site) = fetch_site
                && !matches!(
                    fetch_site.to_ascii_lowercase().as_str(),
                    "same-origin" | "same-site" | "none"
                )
            {
                return Err(StateChangeRequestError::Forbidden);
            }
        }
    }
    let is_json = request.content_type.as_deref().is_some_and(|content_type| {
        content_type
            .split(';')
            .next()
            .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case("application/json"))
    });
    if !is_json {
        return Err(StateChangeRequestError::UnsupportedMediaType);
    }
    Ok(())
}

fn write_state_change_rejection(
    stream: &mut TcpStream,
    error: StateChangeRequestError,
) -> io::Result<()> {
    match error {
        StateChangeRequestError::Forbidden => {
            write_json_error(stream, "403 Forbidden", "request origin is not allowed")
        }
        StateChangeRequestError::UnsupportedMediaType => write_json_error(
            stream,
            "415 Unsupported Media Type",
            "state-changing requests require Content-Type: application/json",
        ),
    }
}

fn handle_control(
    stream: &mut TcpStream,
    state: &Arc<Mutex<ServerState>>,
    body: &[u8],
) -> io::Result<()> {
    let command = match decode_control_v1(body) {
        Ok(command) => command,
        Err(error) => return write_json_error(stream, "400 Bad Request", &error.to_string()),
    };
    #[cfg(test)]
    invoke_control_before_state_lock_hook(state);
    let (outcome, snapshot) = {
        let mut state = state.lock().expect("server state poisoned");
        let now = state.session_clock_origin.elapsed();
        let outcome = state.session.apply(command, now);
        let domain_snapshot = outcome.as_ref().err().map_or_else(
            || state.session.snapshot(now),
            |rejected| rejected.snapshot.clone(),
        );
        let snapshot = project_snapshot_v1(domain_snapshot, &state.last_good_speaker_notes);
        if outcome.is_ok() {
            let event = session_event_from_projection(&snapshot);
            broadcast_event_batch_locked(&mut state, &[event]);
        }
        (outcome, snapshot)
    };
    if let Err(rejected) = outcome {
        let message = match rejected.reason {
            SessionConflict::StaleGeneration => {
                "the Deck generation changed; refresh navigation state"
            }
            SessionConflict::InvalidRoute => "the requested presentation route is not valid",
        };
        return write_json_response_status(
            stream,
            "409 Conflict",
            &error_payload_v1(message, Some(&snapshot)),
        );
    }
    let snapshot = snapshot_json_v1(&snapshot).to_string();
    write_response(
        stream,
        "application/json; charset=utf-8",
        snapshot.as_bytes(),
    )
}

#[cfg(test)]
fn invoke_control_before_state_lock_hook(state: &Arc<Mutex<ServerState>>) {
    let coordinator = state
        .lock()
        .expect("server state poisoned")
        .rebuild_coordinator
        .clone();
    let hook = coordinator
        .lock()
        .expect("rebuild coordinator poisoned")
        .control_before_state_lock_hook
        .clone();
    if let Some(hook) = hook {
        hook();
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeSwitchRequest {
    theme: String,
    #[serde(default)]
    variant: Option<String>,
}

fn handle_theme_switch(
    stream: &mut TcpStream,
    options: &ServeOptions,
    state: &Arc<Mutex<ServerState>>,
    body: &[u8],
) -> io::Result<()> {
    let enabled = {
        let state = state.lock().expect("server state poisoned");
        state.theme_switcher_enabled
    };
    if !enabled {
        return write_bad_request(stream, "theme switcher is not enabled");
    }
    let request = match serde_json::from_slice::<ThemeSwitchRequest>(body) {
        Ok(request) => request,
        Err(error) => return write_bad_request(stream, &error.to_string()),
    };
    let theme_name = request.theme.trim();
    if theme_name.is_empty() {
        return write_bad_request(stream, "missing theme");
    }
    if let Err(error) = theme::validate_theme_name(theme_name) {
        return write_bad_request(stream, &error.to_string());
    }
    let variant = request
        .variant
        .filter(|value| value != "defaults" && !value.is_empty());
    let deck_root = {
        let state = state.lock().expect("server state poisoned");
        state.deck_root.clone()
    };
    let manifest =
        match theme::load_named_theme(theme_name, &deck_root, &options.theme_search_paths) {
            Ok(manifest) => manifest,
            Err(error) => return write_bad_request(stream, &error.to_string()),
        };
    if let Some(variant) = &variant
        && !manifest.color_variants.contains_key(variant)
    {
        return write_bad_request(
            stream,
            &format!("theme '{}' has no variant '{}'", manifest.name, variant),
        );
    }
    let requested = LiveThemeOverride {
        theme: manifest.name,
        variant,
    };
    match rebuild_with_requested_theme(options, state, false, Some(requested)) {
        CommitOutcome::AcceptedValid => {
            let body = theme_switcher_state_json(state);
            write_response(stream, "application/json; charset=utf-8", body.as_bytes())
        }
        CommitOutcome::AcceptedInvalid { diagnostics } => {
            let state = state.lock().expect("server state poisoned");
            let session = snapshot_json_v1(&projected_session_v1(&state));
            write_json_response_status(
                stream,
                "422 Unprocessable Content",
                &json!({
                    "outcome": "invalid",
                    "error": "the requested Theme could not build the current Source",
                    "session": session,
                    "diagnostics": diagnostics
                        .iter()
                        .map(|diagnostic| diagnostic.message.clone())
                        .collect::<Vec<_>>(),
                }),
            )
        }
        CommitOutcome::Superseded => {
            let (session, theme) = {
                let state = state.lock().expect("server state poisoned");
                (
                    snapshot_json_v1(&projected_session_v1(&state)),
                    theme_switcher_state_value(&state),
                )
            };
            write_json_response_status(
                stream,
                "409 Conflict",
                &json!({
                    "outcome": "superseded",
                    "error": "theme switch was superseded by a newer rebuild request",
                    "session": session,
                    "theme": theme,
                }),
            )
        }
        CommitOutcome::ShuttingDown => write_json_response_status(
            stream,
            "503 Service Unavailable",
            &json!({
                "outcome": "shutting-down",
                "error": "the live presentation is shutting down",
            }),
        ),
    }
}

fn read_deck_asset(
    asset_path: &str,
    state: &Arc<Mutex<ServerState>>,
) -> Option<(&'static str, Vec<u8>)> {
    if !is_safe_local_asset_path(asset_path) {
        return None;
    }
    let deck_root = {
        let state = state.lock().expect("server state poisoned");
        state.deck_root.clone()
    };
    let canonical_root = fs::canonicalize(deck_root).ok()?;
    let path = fs::canonicalize(canonical_root.join(asset_path)).ok()?;
    if !path.starts_with(&canonical_root) || !path.is_file() {
        return None;
    }
    Some((content_type_for_path(&path), fs::read(path).ok()?))
}

fn read_theme_asset(
    asset_path: &str,
    state: &Arc<Mutex<ServerState>>,
) -> Option<(&'static str, Vec<u8>)> {
    let relative = asset_path.strip_prefix("assets/")?;
    if !is_safe_local_asset_path(relative) {
        return None;
    }
    let (root, dependencies) = {
        let state = state.lock().expect("server state poisoned");
        (
            state.last_good_theme_dependency_root.clone()?,
            state.last_good_theme_dependencies.clone(),
        )
    };
    if !dependencies.iter().any(|dependency| dependency == relative) {
        return None;
    }
    let canonical_root = fs::canonicalize(root).ok()?;
    let path = fs::canonicalize(canonical_root.join(relative)).ok()?;
    if !path.starts_with(&canonical_root) || !path.is_file() {
        return None;
    }
    Some((content_type_for_path(&path), fs::read(path).ok()?))
}

struct SubscriberGuard {
    id: SubscriberId,
    state: Arc<Mutex<ServerState>>,
    mailbox: Arc<SubscriberMailbox>,
    #[cfg(test)]
    metrics: Arc<SseMetrics>,
}

impl Drop for SubscriberGuard {
    fn drop(&mut self) {
        self.mailbox.close();
        self.state
            .lock()
            .expect("server state poisoned")
            .clients
            .remove(&self.id);
        #[cfg(test)]
        {
            self.metrics.active_writers.fetch_sub(1, Ordering::SeqCst);
            self.metrics.deregistrations.fetch_add(1, Ordering::SeqCst);
        }
    }
}

fn handle_events(mut stream: TcpStream, state: Arc<Mutex<ServerState>>) -> io::Result<()> {
    let settings = state.lock().expect("server state poisoned").sse_settings;
    stream.set_write_timeout(Some(settings.write_timeout))?;
    stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n",
    )?;
    stream.write_all(b"retry: 500\n\n")?;

    let mailbox = Arc::new(SubscriberMailbox::default());
    let (id, bootstrap, metrics) = {
        let mut state = state.lock().expect("server state poisoned");
        let id = state.next_subscriber_id;
        state.next_subscriber_id = state
            .next_subscriber_id
            .checked_add(1)
            .expect("SSE subscriber ID overflow");
        state.clients.insert(id, Arc::clone(&mailbox));
        #[cfg(test)]
        let metrics = {
            state
                .sse_metrics
                .registrations
                .fetch_add(1, Ordering::SeqCst);
            state
                .sse_metrics
                .active_writers
                .fetch_add(1, Ordering::SeqCst);
            Some(Arc::clone(&state.sse_metrics))
        };
        #[cfg(not(test))]
        let metrics: Option<()> = None;
        (
            id,
            vec![
                diagnostics_event(&state.diagnostics),
                session_event(&state),
                SseEvent::RenderState {
                    generation: state.session.generation(),
                },
            ],
            metrics,
        )
    };
    let _guard = SubscriberGuard {
        id,
        state,
        mailbox: Arc::clone(&mailbox),
        #[cfg(test)]
        metrics: metrics.expect("test metrics are present"),
    };
    #[cfg(not(test))]
    let _ = metrics;
    for event in &bootstrap {
        write_sse_event(&mut stream, event)?;
    }

    loop {
        match mailbox.wait_and_drain(settings.heartbeat_interval) {
            MailboxWake::Events(events) => {
                for event in events {
                    write_sse_event(&mut stream, &event)?;
                }
            }
            MailboxWake::Heartbeat => write_sse_heartbeat(&mut stream)?,
            MailboxWake::Closed => break,
        }
    }
    Ok(())
}

fn write_response(stream: &mut TcpStream, content_type: &str, body: &[u8]) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

fn write_preview_response(stream: &mut TcpStream, body: &[u8]) -> io::Result<()> {
    const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; object-src 'none'; frame-src 'none'; child-src 'none'; media-src 'none'; connect-src 'none'; worker-src 'none'; form-action 'none'; base-uri 'none'; navigate-to 'none'";
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nContent-Security-Policy: {CSP}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

fn write_json_error(stream: &mut TcpStream, status: &str, message: &str) -> io::Result<()> {
    let body = error_payload_v1(message, None).to_string();
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body.as_bytes())
}

fn write_json_response_status(
    stream: &mut TcpStream,
    status: &str,
    body: &serde_json::Value,
) -> io::Result<()> {
    let body = body.to_string();
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body.as_bytes())
}

fn write_method_not_allowed(stream: &mut TcpStream, allow: &str) -> io::Result<()> {
    let body = json!({ "error": format!("method not allowed; use {allow}") }).to_string();
    write!(
        stream,
        "HTTP/1.1 405 Method Not Allowed\r\nAllow: {allow}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body.as_bytes())
}

fn write_not_found(stream: &mut TcpStream) -> io::Result<()> {
    let body = b"not found";
    write!(
        stream,
        "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

fn write_bad_request(stream: &mut TcpStream, message: &str) -> io::Result<()> {
    write_json_error(stream, "400 Bad Request", message)
}

#[cfg(test)]
fn percent_decode(value: &str) -> Option<String> {
    percent_decode_component(value, true)
}

fn percent_decode_path(value: &str) -> Option<String> {
    percent_decode_component(value, false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PreviewBootstrapRoute {
    section: usize,
    detail: usize,
    step: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PreviewQuery {
    enabled: bool,
    route: Option<PreviewBootstrapRoute>,
    generation: Option<u64>,
}

fn parse_preview_query(query: &str) -> Result<PreviewQuery, &'static str> {
    if query.is_empty() {
        return Ok(PreviewQuery {
            enabled: false,
            route: None,
            generation: None,
        });
    }
    let mut preview = None;
    let mut section = None;
    let mut detail = None;
    let mut step = None;
    let mut generation = None;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (raw_name, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let name = percent_decode_component(raw_name, true)
            .ok_or("request query is not valid percent-encoded UTF-8")?;
        let value = percent_decode_component(raw_value, true)
            .ok_or("request query is not valid percent-encoded UTF-8")?;
        let slot = match name.as_str() {
            "zpres-preview" => Some((&mut preview, "zpres-preview must appear exactly once")),
            "zpres-preview-section" => Some((
                &mut section,
                "zpres-preview-section must appear exactly once",
            )),
            "zpres-preview-detail" => {
                Some((&mut detail, "zpres-preview-detail must appear exactly once"))
            }
            "zpres-preview-step" => {
                Some((&mut step, "zpres-preview-step must appear exactly once"))
            }
            "zpres-generation" => {
                Some((&mut generation, "zpres-generation must appear exactly once"))
            }
            _ => None,
        };
        if let Some((slot, duplicate_message)) = slot {
            if slot.is_some() {
                return Err(duplicate_message);
            }
            *slot = Some(value);
        }
    }
    let enabled = preview.as_deref() == Some("1");
    let has_route = section.is_some() || detail.is_some() || step.is_some();
    if (has_route || generation.is_some()) && !enabled {
        return Err("preview bootstrap parameters require zpres-preview=1");
    }
    let route = if has_route {
        let parse_coordinate = |value: Option<String>| {
            let value = value.ok_or("preview route requires Section, Detail, and Step")?;
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err("preview route coordinates must be unsigned decimal integers");
            }
            value
                .parse::<u32>()
                .map(|value| value as usize)
                .map_err(|_| "preview route coordinate is too large")
        };
        Some(PreviewBootstrapRoute {
            section: parse_coordinate(section)?,
            detail: parse_coordinate(detail)?,
            step: parse_coordinate(step)?,
        })
    } else {
        None
    };
    let generation = generation
        .map(|value| {
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err("zpres-generation must be an unsigned decimal integer");
            }
            value
                .parse::<u64>()
                .map_err(|_| "zpres-generation is too large")
        })
        .transpose()?;
    if route.is_some() != generation.is_some() {
        return Err("preview route and zpres-generation must be provided together");
    }
    Ok(PreviewQuery {
        enabled,
        route,
        generation,
    })
}

fn escape_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn percent_decode_component(value: &str, plus_as_space: bool) -> Option<String> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' if plus_as_space => {
                output.push(b' ');
                index += 1;
            }
            b'%' => {
                if index + 2 >= bytes.len() {
                    return None;
                }
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
                output.push(u8::from_str_radix(hex, 16).ok()?);
                index += 3;
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(output).ok()
}

fn write_sse_event(stream: &mut TcpStream, event: &SseEvent) -> io::Result<()> {
    let data = event.wire_data();
    write!(
        stream,
        "event: {}\ndata: {}\n\n",
        event.wire_name(),
        data.replace('\n', "\\n")
    )?;
    stream.flush()
}

fn write_sse_heartbeat(stream: &mut TcpStream) -> io::Result<()> {
    stream.write_all(b": heartbeat\n\n")?;
    stream.flush()
}

fn theme_switcher_state_json(state: &Arc<Mutex<ServerState>>) -> String {
    let state = state.lock().expect("server state poisoned");
    theme_switcher_state_value(&state).to_string()
}

fn theme_switcher_state_value(state: &ServerState) -> serde_json::Value {
    let current_theme = state
        .live_theme_override
        .as_ref()
        .map(|selection| selection.theme.clone())
        .or_else(|| state.last_good_theme.clone())
        .unwrap_or_else(|| theme::DEFAULT_THEME_NAME.to_string());
    let current_variant = state
        .live_theme_override
        .as_ref()
        .and_then(|selection| selection.variant.clone())
        .or_else(|| {
            state
                .last_good_theme_params
                .get(&state.last_good_palette_parameter)
                .cloned()
        })
        .unwrap_or_else(|| "defaults".to_string());
    json!({
        "enabled": state.theme_switcher_enabled,
        "themes": state.theme_switcher_catalog,
        "currentTheme": current_theme,
        "currentVariant": current_variant,
    })
}

fn discover_theme_switcher_catalog(
    deck_root: &Path,
    theme_search_paths: &[PathBuf],
) -> Vec<ThemeSwitcherTheme> {
    let mut manifest_paths = Vec::new();
    collect_theme_manifests(&deck_root.join("themes"), &mut manifest_paths);
    for search_path in theme_search_paths {
        if search_path.join("theme.toml").is_file() {
            manifest_paths.push(search_path.join("theme.toml"));
        }
        collect_theme_manifests(search_path, &mut manifest_paths);
    }
    collect_theme_manifests(&theme::builtin_theme_search_path(), &mut manifest_paths);

    let mut seen = BTreeSet::new();
    let mut themes = Vec::new();
    for manifest_path in manifest_paths {
        let Ok(manifest) = theme::load_theme_manifest(&manifest_path) else {
            continue;
        };
        if !seen.insert(manifest.name.clone()) {
            continue;
        }
        let mut variants = manifest.color_variants.keys().cloned().collect::<Vec<_>>();
        variants.sort();
        if variants.is_empty() {
            variants.push("defaults".to_string());
        } else if !variants.iter().any(|variant| variant == "defaults") {
            variants.insert(0, "defaults".to_string());
        }
        themes.push(ThemeSwitcherTheme {
            name: manifest.name,
            palette_parameter: manifest.palette_parameter,
            variants,
        });
    }
    themes.sort_by(|left, right| left.name.cmp(&right.name));
    themes
}

fn collect_theme_manifests(root: &Path, manifest_paths: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.join("theme.toml").is_file() {
            manifest_paths.push(path.join("theme.toml"));
        }
    }
}

fn theme_css_href(generation: u64) -> String {
    format!("assets/theme.css?zpres_live={generation}")
}

fn live_update_event(update: LiveUpdate, generation: u64) -> SseEvent {
    match update {
        LiveUpdate::Reload => SseEvent::Reload { generation },
        LiveUpdate::ThemeCss => SseEvent::ThemeCss {
            generation,
            href: theme_css_href(generation),
        },
        LiveUpdate::Assets => SseEvent::Assets {
            generation,
            version: generation,
            theme_css_href: theme_css_href(generation),
        },
    }
}

fn session_event_from_projection(
    snapshot: &crate::presentation_protocol::SessionSnapshotV1,
) -> SseEvent {
    project_session_event_v1(snapshot)
}

fn projected_session_v1(state: &ServerState) -> crate::presentation_protocol::SessionSnapshotV1 {
    project_snapshot_v1(
        state.session.snapshot(state.session_clock_origin.elapsed()),
        &state.last_good_speaker_notes,
    )
}

fn session_event(state: &ServerState) -> SseEvent {
    session_event_from_projection(&projected_session_v1(state))
}

fn diagnostics_event(diagnostics: &[Diagnostic]) -> SseEvent {
    SseEvent::Diagnostics {
        html: html::render_diagnostics_list(diagnostics),
    }
}

fn broadcast_event_batch_locked(state: &mut ServerState, events: &[SseEvent]) {
    state
        .clients
        .retain(|_, mailbox| mailbox.publish_batch(events) == PublishResult::Published);
}

fn decorate_live_html(html: &str, theme_switcher: bool, presentation_url: &str) -> String {
    let html = inject_live_client(html, presentation_url);
    if theme_switcher {
        inject_theme_switcher(&html)
    } else {
        html
    }
}

const LIVE_GENERATION_TOKEN: &str = "__ZPRES_ACCEPTED_GENERATION__";

fn resolve_live_generation(html: &str, generation: u64) -> String {
    html.replace(LIVE_GENERATION_TOKEN, &generation.to_string())
}

fn inject_live_client(html: &str, presentation_url: &str) -> String {
    let client = r#"<script>
(function () {
  let latestRevision = -1;
  let renderedGeneration = __ZPRES_ACCEPTED_GENERATION__;
  window.zpresRenderedGeneration = renderedGeneration;
  window.zpresLiveControl = async function (command, payload) {
    const response = await fetch("/zpres/control", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(Object.assign({ command }, payload || {})),
    });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || "presentation control failed");
    return result;
  };
  if (!window.EventSource) return;
  const events = new EventSource("/events");
  events.onopen = function () { latestRevision = -1; };
  function cacheBust(value, version) {
    const url = new URL(value, window.location.href);
    if (url.origin !== window.location.origin) return value;
    url.searchParams.set("zpres_live", version || Date.now());
    return url.pathname + url.search + url.hash;
  }
  function refreshThemeCss(href) {
    const nextHref = href || ("assets/theme.css?v=" + Date.now());
    for (const link of document.querySelectorAll('link[rel="stylesheet"]')) {
      const url = new URL(link.getAttribute("href") || link.href, window.location.href);
      if (url.pathname.endsWith("/assets/theme.css")) link.setAttribute("href", nextHref);
    }
  }
  function refreshAssetUrls(version) {
    for (const element of document.querySelectorAll("img[src], video[src], audio[src], iframe[src], source[src]")) {
      const attr = element.getAttribute("src");
      if (attr) element.setAttribute("src", cacheBust(attr, version));
    }
    for (const element of document.querySelectorAll("video[poster]")) {
      const poster = element.getAttribute("poster");
      if (poster) element.setAttribute("poster", cacheBust(poster, version));
    }
  }
  function eventGeneration(payload) {
    const generation = Number(payload?.generation);
    return Number.isSafeInteger(generation) && generation >= 0 ? generation : null;
  }
  function acceptGeneration(generation) {
    renderedGeneration = generation;
    window.zpresRenderedGeneration = generation;
  }
  events.addEventListener("session", function (event) {
    const payload = JSON.parse(event.data);
    if (!payload.current || payload.revision < latestRevision) return;
    latestRevision = payload.revision;
    window.zpresLiveGeneration = payload.generation;
    window.zpresPresentation?.navigate(payload.current).catch(function (error) {
      if (error?.name !== "AbortError") console.error(error);
    });
  });
  events.addEventListener("render-state", function (event) {
    const generation = eventGeneration(JSON.parse(event.data));
    if (generation !== null && generation !== renderedGeneration) window.location.reload();
  });
  events.addEventListener("reload", function (event) {
    const generation = eventGeneration(JSON.parse(event.data));
    if (generation !== null && generation > renderedGeneration) window.location.reload();
  });
  events.addEventListener("theme-css", function (event) {
    const payload = JSON.parse(event.data);
    const generation = eventGeneration(payload);
    if (generation === null || generation <= renderedGeneration) return;
    refreshThemeCss(payload.href);
    acceptGeneration(generation);
  });
  events.addEventListener("assets", function (event) {
    const payload = JSON.parse(event.data);
    const generation = eventGeneration(payload);
    if (generation === null || generation <= renderedGeneration) return;
    refreshThemeCss(payload.themeCssHref);
    refreshAssetUrls(payload.version);
    acceptGeneration(generation);
  });
  events.addEventListener("diagnostics", function (event) {
    const payload = JSON.parse(event.data);
    const current = document.querySelector(".debug-diagnostics");
    if (current && payload.html) current.outerHTML = payload.html;
  });
})();
</script>
</body>"#;
    let html = html.replace("</body>", &client);
    html.replace(
        "<head>",
        &format!(
            "<head>\n<meta name=\"zpres-live-presentation\" content=\"{}\">",
            escape_attr(presentation_url)
        ),
    )
}

fn inject_theme_switcher(html: &str) -> String {
    html.replace(
        "</body>",
        r##"<style>
.zpres-theme-switcher {
  position: fixed;
  z-index: 1000;
  right: 16px;
  top: 16px;
  display: grid;
  grid-template-columns: auto auto;
  gap: 8px;
  align-items: end;
  max-width: min(440px, calc(100vw - 32px));
  padding: 9px;
  color: #e5edf5;
  background: rgb(8 13 19 / 0.82);
  border: 1px solid rgb(148 163 184 / 0.36);
  border-radius: 8px;
  box-shadow: 0 18px 50px rgb(0 0 0 / 0.32);
  backdrop-filter: blur(14px);
}
.zpres-theme-switcher label {
  display: grid;
  gap: 3px;
  min-width: 128px;
  color: #94a3b8;
  font: 700 11px/1.2 ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  letter-spacing: 0;
  text-transform: uppercase;
}
.zpres-theme-switcher select {
  height: 32px;
  min-width: 144px;
  padding: 0 28px 0 9px;
  color: #f8fafc;
  background: #111827;
  border: 1px solid rgb(148 163 184 / 0.42);
  border-radius: 6px;
  font: 500 13px/1 ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}
.zpres-theme-switcher select:hover,
.zpres-theme-switcher select:focus {
  border-color: #38bdf8;
  outline: none;
}
@media (max-width: 520px) {
  .zpres-theme-switcher {
    left: 10px;
    right: 10px;
    top: 10px;
    grid-template-columns: 1fr 1fr;
  }
  .zpres-theme-switcher label,
  .zpres-theme-switcher select {
    min-width: 0;
    width: 100%;
  }
}
</style>
<form class="zpres-theme-switcher" aria-label="Theme switcher">
  <label>Theme<select id="zpres-theme-switcher-theme"></select></label>
  <label>Variant<select id="zpres-theme-switcher-variant"></select></label>
</form>
<script>
(function () {
  const root = document.querySelector(".zpres-theme-switcher");
  const themeSelect = document.querySelector("#zpres-theme-switcher-theme");
  const variantSelect = document.querySelector("#zpres-theme-switcher-variant");
  let catalog = [];
  let ready = false;
  function currentHash() {
    return /^#\/\d+\/\d+(?:\/\d+)?$/.test(window.location.hash) ? window.location.hash : "#/0/0";
  }
  function rememberHash() {
    try {
      window.sessionStorage.setItem("zpres-live-hash:" + window.location.pathname, currentHash());
    } catch (_) {}
  }
  function themeEntry(name) {
    return catalog.find((theme) => theme.name === name) || catalog[0];
  }
  function fillThemes(currentTheme) {
    themeSelect.replaceChildren();
    for (const theme of catalog) themeSelect.add(new Option(theme.name, theme.name));
    if (themeEntry(currentTheme)) themeSelect.value = themeEntry(currentTheme).name;
  }
  function fillVariants(currentVariant) {
    const selected = themeEntry(themeSelect.value);
    variantSelect.replaceChildren();
    if (!selected) return;
    for (const variant of selected.variants) variantSelect.add(new Option(variant, variant));
    variantSelect.value = selected.variants.includes(currentVariant) ? currentVariant : selected.variants[0];
  }
  async function loadState() {
    const response = await fetch("/zpres/theme-switcher.json", { cache: "no-store" });
    const state = await response.json();
    if (!state.enabled) {
      root.hidden = true;
      return;
    }
    catalog = state.themes || [];
    if (!catalog.length) {
      root.hidden = true;
      return;
    }
    fillThemes(state.currentTheme);
    fillVariants(state.currentVariant);
    ready = true;
  }
  async function switchTheme() {
    if (!ready) return;
    rememberHash();
    const response = await fetch("/zpres/theme-switch", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ theme: themeSelect.value, variant: variantSelect.value }),
      cache: "no-store",
    });
    if (!response.ok) {
      console.warn("zpres theme switch failed", await response.text());
      return;
    }
    window.location.reload();
  }
  themeSelect.addEventListener("change", function () {
    fillVariants("defaults");
    switchTheme();
  });
  variantSelect.addEventListener("change", switchTheme);
  loadState().catch((error) => {
    console.warn("zpres theme switcher failed", error);
    root.hidden = true;
  });
})();
</script>
</body>"##,
    )
}

fn initial_html() -> String {
    "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>zpres live server</title><link rel=\"stylesheet\" href=\"assets/reveal.css\"><link rel=\"stylesheet\" href=\"assets/theme.css\"></head><body><main class=\"debug-empty\">Waiting for first good deck.</main><aside class=\"debug-diagnostics\" aria-label=\"diagnostics\"><strong>diagnostics</strong><span>waiting</span></aside></body></html>".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    path: PathBuf,
    modified: Option<SystemTime>,
    len: Option<u64>,
}

fn live_watch_snapshot(options: &ServeOptions, state: &Arc<Mutex<ServerState>>) -> Vec<FileStamp> {
    let (active_root, live_selection) = {
        let state = state.lock().expect("server state poisoned");
        (
            state.last_good_theme_dependency_root.clone(),
            state
                .live_theme_override
                .as_ref()
                .map(|selection| selection.theme.clone()),
        )
    };
    let mut paths = Vec::new();
    if let Some(root) = active_root {
        collect_files(&root, &mut paths);
    }
    let deck_root = options.source.parent().unwrap_or_else(|| Path::new("."));
    for name in [
        live_selection.as_ref(),
        options.cli_theme_override.as_ref(),
        options.fallback_theme.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        // Keep watching a selected package even when its manifest is currently
        // invalid or the first build has not succeeded yet.
        if theme::validate_theme_name(name).is_ok() {
            collect_files(&deck_root.join("themes").join(name), &mut paths);
            collect_files(&deck_root.join(name), &mut paths);
        }
    }
    let mut snapshot = watch_snapshot(&options.source, &options.theme_search_paths);
    snapshot.extend(paths.into_iter().map(file_stamp));
    snapshot.sort_by(|left, right| left.path.cmp(&right.path));
    snapshot.dedup_by(|left, right| left.path == right.path);
    snapshot
}

fn watch_snapshot(source: &Path, theme_search_paths: &[PathBuf]) -> Vec<FileStamp> {
    let mut paths = vec![source.to_path_buf()];
    paths.extend(extract_local_dependency_paths(source));
    paths.extend(extract_deck_theme_watch_paths(source));
    for theme_path in theme_search_paths {
        collect_files(theme_path, &mut paths);
    }
    collect_files(&theme::builtin_theme_search_path(), &mut paths);
    paths.sort();
    paths.dedup();
    paths.into_iter().map(file_stamp).collect()
}

fn extract_deck_theme_watch_paths(source: &Path) -> Vec<PathBuf> {
    let deck_root = source.parent().unwrap_or_else(|| Path::new("."));
    let mut paths = Vec::new();
    if let Some(theme_name) = current_source_theme_name(source) {
        collect_files(&deck_root.join("themes").join(&theme_name), &mut paths);
        collect_files(&deck_root.join(&theme_name), &mut paths);
    } else {
        collect_files(&deck_root.join("themes"), &mut paths);
    }
    paths
}

fn current_source_theme_name(source: &Path) -> Option<String> {
    if let Ok(deck) = deck::parse_source_file(source)
        && let Some(theme) = deck.metadata.theme
    {
        return Some(theme);
    }
    let text = fs::read_to_string(source).ok()?;
    extract_textual_front_matter_theme(&text)
}

fn extract_textual_front_matter_theme(text: &str) -> Option<String> {
    let deck::FrontMatterSplit::Valid { front_matter, .. } = deck::split_front_matter(text) else {
        return None;
    };
    for line in front_matter.lines() {
        let trimmed = line.trim();
        let Some(value) = trimmed.strip_prefix("theme:") else {
            continue;
        };
        let theme = value.trim().trim_matches(['"', '\'']).to_string();
        if theme.is_empty() {
            return None;
        }
        return theme::validate_theme_name(&theme).is_ok().then_some(theme);
    }
    None
}

fn file_stamp(path: PathBuf) -> FileStamp {
    let metadata = fs::metadata(&path).ok();
    FileStamp {
        path,
        modified: metadata
            .as_ref()
            .and_then(|metadata| metadata.modified().ok()),
        len: metadata.map(|metadata| metadata.len()),
    }
}

fn collect_files(path: &Path, paths: &mut Vec<PathBuf>) {
    if path.is_file() {
        paths.push(path.to_path_buf());
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        collect_files(&entry.path(), paths);
    }
}

fn extract_local_dependency_paths(source: &Path) -> Vec<PathBuf> {
    let Ok(text) = fs::read_to_string(source) else {
        return Vec::new();
    };
    let deck_root = source.parent().unwrap_or_else(|| Path::new("."));
    let mut paths = Vec::new();
    if let Ok(deck) = deck::parse_source_file(source) {
        paths.extend(
            deck.local_dependency_references()
                .into_iter()
                .map(|reference| deck_root.join(reference)),
        );
    }
    for key in ["src", "data", "poster"] {
        let mut rest = text.as_str();
        let needle = format!("{key}=");
        while let Some(index) = rest.find(&needle) {
            rest = &rest[index + needle.len()..];
            let Some(value) = read_attribute_value(rest) else {
                continue;
            };
            if !looks_remote_or_fragment(&value) {
                paths.push(deck_root.join(value));
            }
        }
    }
    paths.extend(extract_markdown_image_paths(&text, deck_root));
    paths.extend(extract_json_url_dependency_paths(&text, deck_root));
    paths.sort();
    paths.dedup();
    paths
}

fn extract_markdown_image_paths(text: &str, deck_root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("![") else {
            continue;
        };
        let Some((_, rest)) = rest.split_once("](") else {
            continue;
        };
        let Some(target) = rest.strip_suffix(')') else {
            continue;
        };
        let value = target
            .split_once(" \"")
            .map_or(target.trim(), |(src, _)| src.trim());
        if !looks_remote_or_fragment(value) && is_safe_local_asset_path(value) {
            paths.push(deck_root.join(value));
        }
    }
    paths
}

fn extract_json_url_dependency_paths(text: &str, deck_root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut rest = text;
    while let Some(index) = rest.find("\"url\"") {
        rest = &rest[index + 5..];
        let Some(colon_index) = rest.find(':') else {
            break;
        };
        rest = &rest[colon_index + 1..];
        let trimmed = rest.trim_start();
        let Some(trimmed) = trimmed.strip_prefix('"') else {
            continue;
        };
        let Some(end) = trimmed.find('"') else {
            break;
        };
        let value = &trimmed[..end];
        if !looks_remote_or_fragment(value) && is_safe_local_asset_path(value) {
            paths.push(deck_root.join(value));
        }
        rest = &trimmed[end + 1..];
    }
    paths
}

fn read_attribute_value(rest: &str) -> Option<String> {
    if let Some(quoted) = rest.strip_prefix('"') {
        let end = quoted.find('"')?;
        return Some(quoted[..end].to_string());
    }
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

fn looks_remote_or_fragment(value: &str) -> bool {
    value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("data:")
        || value.starts_with('#')
}

fn is_safe_local_asset_path(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn content_type_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("svg") => "image/svg+xml",
        Some(extension) if extension.eq_ignore_ascii_case("png") => "image/png",
        Some(extension)
            if extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg") =>
        {
            "image/jpeg"
        }
        Some(extension) if extension.eq_ignore_ascii_case("gif") => "image/gif",
        Some(extension) if extension.eq_ignore_ascii_case("webp") => "image/webp",
        Some(extension) if extension.eq_ignore_ascii_case("css") => "text/css; charset=utf-8",
        Some(extension) if extension.eq_ignore_ascii_case("woff") => "font/woff",
        Some(extension) if extension.eq_ignore_ascii_case("woff2") => "font/woff2",
        Some(extension) if extension.eq_ignore_ascii_case("ttf") => "font/ttf",
        Some(extension) if extension.eq_ignore_ascii_case("otf") => "font/otf",
        Some(extension) if extension.eq_ignore_ascii_case("csv") => "text/csv; charset=utf-8",
        Some(extension) if extension.eq_ignore_ascii_case("json") => {
            "application/json; charset=utf-8"
        }
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chromium::{ChromiumSession, ChromiumSessionOptions, discover_chromium_executable};
    use crate::pdf::PngViewport;
    use std::io::Read as _;
    use std::net::SocketAddr;
    use tempfile::tempdir;

    const TEST_IO_TIMEOUT: Duration = Duration::from_secs(3);

    fn test_route(
        section: usize,
        detail: usize,
        step: usize,
        step_count: usize,
        slide_id: &str,
    ) -> PlannedScreenState {
        PlannedScreenState {
            section,
            detail,
            step,
            step_count,
            slide_id: slide_id.to_string(),
            role: if detail == 0 { "main" } else { "detail" },
            title: Some(slide_id.to_string()),
            generated: None,
        }
    }

    fn test_options(source: PathBuf) -> ServeOptions {
        ServeOptions {
            source,
            theme_search_paths: Vec::new(),
            fallback_theme: None,
            fallback_theme_params: BTreeMap::new(),
            cli_theme_override: None,
            cli_theme_params: BTreeMap::new(),
            port: 0,
            speaker_port: None,
            speaker_enabled: true,
            open: false,
            debounce: Duration::from_millis(1),
            theme_switcher: false,
        }
    }

    fn request_once(
        request: &[u8],
        role: ViewRole,
        options: ServeOptions,
        state: Arc<Mutex<ServerState>>,
    ) -> String {
        use std::io::Read as _;
        use std::net::Shutdown;
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_connection(stream, role, Arc::new(options), state).unwrap();
        });
        let mut client = TcpStream::connect(address).unwrap();
        client.write_all(request).unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        server.join().unwrap();
        response
    }

    fn live_address(url: &str) -> SocketAddr {
        url.strip_prefix("http://")
            .expect("live URL is HTTP")
            .parse()
            .expect("live URL has a socket address")
    }

    fn live_request(url: &str, request: &[u8]) -> String {
        let mut stream = TcpStream::connect_timeout(&live_address(url), TEST_IO_TIMEOUT).unwrap();
        stream.set_read_timeout(Some(TEST_IO_TIMEOUT)).unwrap();
        stream.set_write_timeout(Some(TEST_IO_TIMEOUT)).unwrap();
        stream.write_all(request).unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let mut reader = BufReader::new(stream);
        let mut headers = String::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(!line.is_empty(), "HTTP response ended before its headers");
            headers.push_str(&line);
            if line == "\r\n" || line == "\n" {
                break;
            }
        }
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.strip_prefix("Content-Length: ")
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .unwrap_or(0);
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body).unwrap();
        format!("{headers}{}", String::from_utf8(body).unwrap())
    }

    fn live_json_get(url: &str) -> serde_json::Value {
        let response = live_request(
            url,
            b"GET /zpres/session.json HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
    }

    fn live_control(url: &str, body: &str) -> serde_json::Value {
        let request = format!(
            "POST /zpres/control HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let response = live_request(url, request.as_bytes());
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
    }

    fn live_navigate_to_slide(
        url: &str,
        state: &Arc<Mutex<ServerState>>,
        slide_id: &str,
        step: usize,
    ) -> serde_json::Value {
        let (generation, section, detail) = {
            let state = state.lock().unwrap();
            let route = state
                .session
                .routes()
                .iter()
                .find(|route| route.slide_id == slide_id && route.step == step)
                .unwrap_or_else(|| panic!("missing route {slide_id} at Step {step}"));
            (state.session.generation(), route.section, route.detail)
        };
        live_control(
            url,
            &format!(
                r#"{{"command":"navigate","generation":{generation},"section":{section},"detail":{detail},"step":{step}}}"#
            ),
        )
    }

    struct TestSseClient {
        reader: BufReader<TcpStream>,
    }

    impl TestSseClient {
        fn connect(url: &str) -> Self {
            let mut stream =
                TcpStream::connect_timeout(&live_address(url), TEST_IO_TIMEOUT).unwrap();
            stream.set_read_timeout(Some(TEST_IO_TIMEOUT)).unwrap();
            stream.set_write_timeout(Some(TEST_IO_TIMEOUT)).unwrap();
            stream
                .write_all(b"GET /events HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .unwrap();
            let mut reader = BufReader::new(stream);
            let mut status = String::new();
            reader.read_line(&mut status).unwrap();
            assert_eq!(status.trim_end(), "HTTP/1.1 200 OK");
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line == "\n" {
                    break;
                }
                assert!(!line.is_empty(), "SSE headers ended before blank line");
            }
            Self { reader }
        }

        fn next_event(&mut self) -> (String, serde_json::Value) {
            let mut event: Option<String> = None;
            let mut data: Option<String> = None;
            loop {
                let mut line = String::new();
                let read = self.reader.read_line(&mut line).unwrap();
                assert_ne!(read, 0, "SSE stream closed before the expected event");
                let line = line.trim_end_matches(['\r', '\n']);
                if line.is_empty() {
                    if let (Some(event), Some(data)) = (event.take(), data.take()) {
                        return (event, serde_json::from_str(&data).unwrap());
                    }
                } else if let Some(value) = line.strip_prefix("event: ") {
                    event = Some(value.to_string());
                } else if let Some(value) = line.strip_prefix("data: ") {
                    data = Some(value.to_string());
                }
            }
        }

        fn initial_session(&mut self) -> serde_json::Value {
            assert_eq!(self.next_event().0, "diagnostics");
            let (event, session) = self.next_event();
            assert_eq!(event, "session");
            let (event, render_state) = self.next_event();
            assert_eq!(event, "render-state");
            assert_eq!(render_state["generation"], session["generation"]);
            session
        }

        fn next_session(&mut self) -> serde_json::Value {
            loop {
                let (event, data) = self.next_event();
                if event == "session" {
                    return data;
                }
            }
        }
    }

    struct RebuildCommitBlocker {
        observed: mpsc::Receiver<RebuildHookEvent>,
        release: mpsc::Sender<()>,
    }

    fn block_rebuild_before_commit(
        state: &Arc<Mutex<ServerState>>,
        source_text: &str,
    ) -> RebuildCommitBlocker {
        let fingerprint = format!("{:x}", Sha256::digest(source_text.as_bytes()));
        let (observed_tx, observed) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let blocked = Arc::new(AtomicBool::new(false));
        let hook: RebuildTestHook = Arc::new(move |event| {
            if event.phase == RebuildHookPhase::BeforeCommit
                && event.fingerprint.as_deref() == Some(fingerprint.as_str())
                && !blocked.swap(true, Ordering::SeqCst)
            {
                observed_tx.send(event).unwrap();
                release_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(TEST_IO_TIMEOUT)
                    .expect("timed out waiting to release blocked rebuild");
            }
        });
        let coordinator = state.lock().unwrap().rebuild_coordinator.clone();
        coordinator.lock().unwrap().test_hook = Some(hook);
        RebuildCommitBlocker { observed, release }
    }

    fn clear_rebuild_hook(state: &Arc<Mutex<ServerState>>) {
        let coordinator = state.lock().unwrap().rebuild_coordinator.clone();
        coordinator.lock().unwrap().test_hook = None;
    }

    fn register_test_mailbox(state: &Arc<Mutex<ServerState>>) -> Arc<SubscriberMailbox> {
        let mailbox = Arc::new(SubscriberMailbox::default());
        let mut state = state.lock().unwrap();
        let id = state.next_subscriber_id;
        state.next_subscriber_id += 1;
        state.clients.insert(id, Arc::clone(&mailbox));
        mailbox
    }

    fn drain_test_mailbox(mailbox: &SubscriberMailbox) -> Vec<SseEvent> {
        match mailbox.wait_and_drain(TEST_IO_TIMEOUT) {
            MailboxWake::Events(events) => events,
            MailboxWake::Heartbeat => panic!("mailbox heartbeat arrived before queued events"),
            MailboxWake::Closed => panic!("mailbox closed before queued events"),
        }
    }

    fn wait_for_test_condition(message: &str, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + TEST_IO_TIMEOUT;
        while !condition() {
            assert!(Instant::now() < deadline, "{message}");
            thread::yield_now();
        }
    }

    #[test]
    fn sse_event_classes_are_exhaustive_and_mailbox_is_bounded() {
        let variants = [
            SseEvent::Diagnostics {
                html: "diagnostic".to_string(),
            },
            SseEvent::Session {
                snapshot: r#"{"generation":2,"revision":4}"#.to_string(),
                generation: 2,
                revision: 4,
            },
            SseEvent::Reload { generation: 3 },
            SseEvent::ThemeCss {
                generation: 3,
                href: theme_css_href(3),
            },
            SseEvent::Assets {
                generation: 3,
                version: 3,
                theme_css_href: theme_css_href(3),
            },
            SseEvent::RenderState { generation: 3 },
        ];
        assert_eq!(
            variants
                .each_ref()
                .map(|event| (sse_delivery_class(event), event.wire_name())),
            [
                (SseDeliveryClass::DiagnosticsSnapshot, "diagnostics"),
                (SseDeliveryClass::SessionSnapshot, "session"),
                (SseDeliveryClass::RenderGeneration, "reload"),
                (SseDeliveryClass::RenderGeneration, "theme-css"),
                (SseDeliveryClass::RenderGeneration, "assets"),
                (SseDeliveryClass::RenderGeneration, "render-state"),
            ]
        );
        assert!(variants.iter().all(|event| !event.wire_data().is_empty()));

        let mailbox = SubscriberMailbox::default();
        assert_eq!(
            mailbox.publish_batch(&[
                SseEvent::Diagnostics {
                    html: "old".to_string(),
                },
                SseEvent::Session {
                    snapshot: "old".to_string(),
                    generation: 2,
                    revision: 5,
                },
                SseEvent::Reload { generation: 2 },
                SseEvent::Diagnostics {
                    html: "new".to_string(),
                },
                SseEvent::Session {
                    snapshot: "stale".to_string(),
                    generation: 2,
                    revision: 4,
                },
                SseEvent::ThemeCss {
                    generation: 3,
                    href: "ignored-by-coalescing".to_string(),
                },
            ]),
            PublishResult::Published
        );
        let events = drain_test_mailbox(&mailbox);
        assert_eq!(events.len(), SSE_MAILBOX_CAPACITY);
        assert_eq!(
            events,
            vec![
                SseEvent::Diagnostics {
                    html: "new".to_string()
                },
                SseEvent::Session {
                    snapshot: "old".to_string(),
                    generation: 2,
                    revision: 5,
                },
                SseEvent::Reload { generation: 3 },
            ]
        );
        let state = mailbox.state.lock().unwrap();
        assert_eq!(state.max_depth, SSE_MAILBOX_CAPACITY);
    }

    #[test]
    fn sse_resource_bounds_stalled_subscriber_and_shutdown_cleanup() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        write_dual_listener_source(&source);
        let mut host = start_live_host(test_options(source)).unwrap();
        {
            let mut state = host.state.lock().unwrap();
            state.sse_settings.heartbeat_interval = Duration::from_millis(20);
            state.sse_settings.write_timeout = Duration::from_millis(100);
        }
        let url = host.presentation_url().to_string();
        let mut stalled = TestSseClient::connect(&url);
        stalled.initial_session();
        let stalled_mailbox = {
            let state = host.state.lock().unwrap();
            state.clients.values().next_back().unwrap().clone()
        };
        let (observed_tx, observed_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let blocked = Arc::new(AtomicBool::new(false));
        stalled_mailbox.state.lock().unwrap().after_drain_hook = Some(Arc::new({
            let blocked = Arc::clone(&blocked);
            let release_rx = Arc::clone(&release_rx);
            move || {
                if !blocked.swap(true, Ordering::SeqCst) {
                    observed_tx.send(()).unwrap();
                    release_rx
                        .lock()
                        .unwrap()
                        .recv_timeout(TEST_IO_TIMEOUT)
                        .unwrap();
                }
            }
        }));

        let mut healthy = TestSseClient::connect(&url);
        healthy.initial_session();
        {
            let mut state = host.state.lock().unwrap();
            broadcast_event_batch_locked(
                &mut state,
                &[SseEvent::Session {
                    snapshot: r#"{"generation":1,"revision":1}"#.to_string(),
                    generation: 1,
                    revision: 1,
                }],
            );
        }
        observed_rx.recv_timeout(TEST_IO_TIMEOUT).unwrap();

        const FINAL_REVISION: u64 = 2_001;
        {
            let mut state = host.state.lock().unwrap();
            for revision in 2..=FINAL_REVISION {
                broadcast_event_batch_locked(
                    &mut state,
                    &[SseEvent::Session {
                        snapshot: json!({ "generation": 1, "revision": revision }).to_string(),
                        generation: 1,
                        revision,
                    }],
                );
            }
        }
        {
            let state = stalled_mailbox.state.lock().unwrap();
            assert!(mailbox_depth(&state) <= SSE_MAILBOX_CAPACITY);
            assert!(state.max_depth <= SSE_MAILBOX_CAPACITY);
        }
        release_tx.send(()).unwrap();

        while healthy.next_session()["revision"].as_u64() != Some(FINAL_REVISION) {}
        while stalled.next_session()["revision"].as_u64() != Some(FINAL_REVISION) {}

        let metrics = Arc::clone(&host.state.lock().unwrap().sse_metrics);
        drop(healthy);
        drop(stalled);
        wait_for_test_condition("SSE writers did not deregister after disconnect", || {
            metrics.active_writers.load(Ordering::SeqCst) == 0
        });
        assert_eq!(
            metrics.registrations.load(Ordering::SeqCst),
            metrics.deregistrations.load(Ordering::SeqCst)
        );

        let mut idle = TestSseClient::connect(&url);
        idle.initial_session();
        host.shutdown();
        assert_eq!(metrics.active_writers.load(Ordering::SeqCst), 0);
        assert_eq!(
            metrics.registrations.load(Ordering::SeqCst),
            metrics.deregistrations.load(Ordering::SeqCst)
        );
        assert!(stalled_mailbox.is_closed());
    }

    #[test]
    fn sse_resource_bounds_heartbeat_keeps_idle_stream_observable() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        write_dual_listener_source(&source);
        let mut host = start_live_host(test_options(source)).unwrap();
        host.state.lock().unwrap().sse_settings.heartbeat_interval = Duration::from_millis(20);
        let mut stream = TcpStream::connect(live_address(host.presentation_url())).unwrap();
        stream.set_read_timeout(Some(TEST_IO_TIMEOUT)).unwrap();
        stream
            .write_all(b"GET /events HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut saw_heartbeat = false;
        for _ in 0..32 {
            let mut line = String::new();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0);
            if line.trim_end() == ": heartbeat" {
                saw_heartbeat = true;
                break;
            }
        }
        assert!(saw_heartbeat, "idle SSE stream emitted no heartbeat");
        host.shutdown();
    }

    #[test]
    fn sse_resource_bounds_unread_socket_times_out_without_harming_healthy_clients() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        write_dual_listener_source(&source);
        let mut host = start_live_host(test_options(source)).unwrap();
        let metrics = {
            let mut state = host.state.lock().unwrap();
            state.sse_settings.heartbeat_interval = Duration::from_millis(20);
            state.sse_settings.write_timeout = Duration::from_millis(30);
            state.diagnostics = vec![Diagnostic::error(None, "x".repeat(16 * 1024 * 1024))];
            Arc::clone(&state.sse_metrics)
        };
        let mut unread = TcpStream::connect(live_address(host.presentation_url())).unwrap();
        unread
            .write_all(b"GET /events HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        wait_for_test_condition("unread SSE client was never registered", || {
            metrics.registrations.load(Ordering::SeqCst) >= 1
        });
        wait_for_test_condition("unread SSE writer did not hit its write timeout", || {
            metrics.active_writers.load(Ordering::SeqCst) == 0
        });
        assert_eq!(
            metrics.registrations.load(Ordering::SeqCst),
            metrics.deregistrations.load(Ordering::SeqCst)
        );
        drop(unread);

        host.state.lock().unwrap().diagnostics.clear();
        let mut healthy = TestSseClient::connect(host.presentation_url());
        healthy.initial_session();
        drop(healthy);
        host.shutdown();
        assert_eq!(metrics.active_writers.load(Ordering::SeqCst), 0);
        assert_eq!(
            metrics.registrations.load(Ordering::SeqCst),
            metrics.deregistrations.load(Ordering::SeqCst)
        );
    }

    fn live_theme_switch_response(url: &str, theme: &str, variant: &str) -> String {
        let body = json!({ "theme": theme, "variant": variant }).to_string();
        let authority = url.strip_prefix("http://").unwrap();
        let request = format!(
            "POST /zpres/theme-switch HTTP/1.1\r\nHost: {authority}\r\nOrigin: {url}\r\nSec-Fetch-Site: same-origin\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        live_request(url, request.as_bytes())
    }

    fn write_dual_listener_source(path: &Path) {
        fs::write(
            path,
            "# First\n\n::: steps\n1. Reveal\n:::\n\n--\n\n# Detail\n\n---\n\n# Last\n",
        )
        .unwrap();
    }

    fn write_playable_test_wav(path: &Path) {
        const SAMPLE_RATE: u32 = 8_000;
        const DURATION_SECONDS: u32 = 2;
        let sample_count = SAMPLE_RATE * DURATION_SECONDS;
        let data_len = sample_count;
        let mut wav = Vec::with_capacity(44 + data_len as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
        wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&8_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for sample in 0..sample_count {
            wav.push(if (sample / 9) % 2 == 0 { 32 } else { 224 });
        }
        fs::write(path, wav).unwrap();
    }

    fn serve_foreign_browser_page() -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(TEST_IO_TIMEOUT)).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line == "\n" || line.is_empty() {
                    break;
                }
            }
            let body = b"<!doctype html><html><body>foreign test origin</body></html>";
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        });
        (format!("http://{address}/"), worker)
    }

    #[derive(Clone, Copy)]
    enum TestPortPair {
        Adjacent,
        Distinct,
    }

    fn with_reserved_test_port<R>(mut attempt: impl FnMut(u16, TcpListener) -> Option<R>) -> R {
        let deadline = Instant::now() + TEST_IO_TIMEOUT;
        loop {
            let reservation = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = reservation.local_addr().unwrap().port();
            if port != u16::MAX
                && let Some(value) = attempt(port, reservation)
            {
                return value;
            }
            assert!(
                Instant::now() < deadline,
                "could not reserve and hand off a bounded loopback port before the test deadline"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn with_reserved_test_ports<R>(
        pair: TestPortPair,
        mut attempt: impl FnMut(u16, u16) -> Option<R>,
    ) -> R {
        let deadline = Instant::now() + TEST_IO_TIMEOUT;
        loop {
            let first = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let first_port = first.local_addr().unwrap().port();
            if first_port == u16::MAX {
                assert!(
                    Instant::now() < deadline,
                    "could not reserve two bounded loopback ports before the test deadline"
                );
                continue;
            }
            let second = match pair {
                TestPortPair::Adjacent => first_port
                    .checked_add(1)
                    .and_then(|port| TcpListener::bind(("127.0.0.1", port)).ok()),
                TestPortPair::Distinct => TcpListener::bind(("127.0.0.1", 0)).ok(),
            };
            if let Some(second) = second {
                let second_port = second.local_addr().unwrap().port();
                if second_port == u16::MAX {
                    assert!(
                        Instant::now() < deadline,
                        "could not reserve two bounded loopback ports before the test deadline"
                    );
                    continue;
                }
                // Retain both reservations until the attempt is ready to bind
                // them. The handoff cannot be atomic with the production API,
                // so a collision makes the whole attempt retry until deadline.
                drop((first, second));
                if let Some(value) = attempt(first_port, second_port) {
                    return value;
                }
            }
            assert!(
                Instant::now() < deadline,
                "could not hand off two reserved loopback ports before the test deadline"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_until_both_bindable(first_port: u16, second_port: u16) -> (TcpListener, TcpListener) {
        let deadline = Instant::now() + TEST_IO_TIMEOUT;
        loop {
            if let Ok(first) = TcpListener::bind(("127.0.0.1", first_port)) {
                if let Ok(second) = TcpListener::bind(("127.0.0.1", second_port)) {
                    return (first, second);
                }
                // Do not hold one port while waiting for the other. Each
                // successful return proves that both were bindable together.
                drop(first);
            }
            assert!(
                Instant::now() < deadline,
                "ports {first_port} and {second_port} were not both bindable before the test deadline"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn assert_session_equivalent(left: &serde_json::Value, right: &serde_json::Value) {
        let mut left = left.clone();
        let mut right = right.clone();
        left["timer"]["elapsed_ms"] = json!(0);
        right["timer"]["elapsed_ms"] = json!(0);
        assert_eq!(left, right);
    }

    #[test]
    fn dual_listener_http_controls_share_one_authoritative_session() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        write_dual_listener_source(&source);
        let mut host = start_live_host(test_options(source)).unwrap();
        let presentation = host.presentation_url().to_string();
        let speaker = host.speaker_url().unwrap().to_string();

        assert_eq!(live_json_get(&presentation), live_json_get(&speaker));
        let advanced = live_control(&speaker, r#"{"command":"advance"}"#);
        assert_eq!(advanced["current"]["step"], 1);
        assert_session_equivalent(&advanced, &live_json_get(&presentation));
        let retreated = live_control(&presentation, r#"{"command":"retreat"}"#);
        assert_eq!(retreated["current"]["step"], 0);
        let detail = live_control(&speaker, r#"{"command":"detail-next"}"#);
        assert_eq!(detail["current"]["detail"], 1);
        let main = live_control(&presentation, r#"{"command":"detail-previous"}"#);
        assert_eq!(main["current"]["detail"], 0);
        let started = live_control(&speaker, r#"{"command":"timer-start"}"#);
        assert_eq!(started["timer"]["status"], "running");
        let paused = live_control(&presentation, r#"{"command":"timer-pause"}"#);
        assert_eq!(paused["timer"]["status"], "paused");
        let reset = live_control(&speaker, r#"{"command":"timer-reset"}"#);
        assert_eq!(reset["timer"]["status"], "stopped");
        let generation = reset["generation"].as_u64().unwrap();
        let navigated = live_control(
            &presentation,
            &format!(
                r#"{{"command":"navigate","generation":{generation},"section":1,"detail":0,"step":0}}"#
            ),
        );
        assert_eq!(navigated["current"]["section"], 1);
        assert_session_equivalent(&navigated, &live_json_get(&speaker));

        host.shutdown();
    }

    #[test]
    fn dual_listener_protocol_golden_matrix_covers_controls_errors_and_sse() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        write_dual_listener_source(&source);
        let mut host = start_live_host(test_options(source)).unwrap();
        let presentation = host.presentation_url().to_string();
        let speaker = host.speaker_url().unwrap().to_string();
        let mut presentation_sse = TestSseClient::connect(&presentation);
        let mut speaker_sse = TestSseClient::connect(&speaker);

        let expected_diagnostics = json!({ "html": html::render_diagnostics_list(&[]) });
        for client in [&mut presentation_sse, &mut speaker_sse] {
            assert_eq!(
                client.next_event(),
                ("diagnostics".to_string(), expected_diagnostics.clone())
            );
            let (event, initial) = client.next_event();
            assert_eq!(event, "session");
            assert_eq!(
                initial,
                json!({
                    "protocol_version": 1,
                    "generation": 1,
                    "revision": 1,
                    "current": {
                        "section": 0, "detail": 0, "step": 0, "step_count": 1,
                        "slide_id": "section-1-main", "role": "main", "title": "First",
                        "notes_html": "",
                    },
                    "next": {
                        "section": 0, "detail": 0, "step": 1, "step_count": 1,
                        "slide_id": "section-1-main", "role": "main", "title": "First",
                        "notes_html": "",
                    },
                    "timer": { "status": "stopped", "elapsed_ms": 0 },
                })
            );
            assert_eq!(client.next_event().0, "render-state");
        }

        let mut accepted = |url: &str, body: &str, revision: u64| {
            let snapshot = live_control(url, body);
            assert_eq!(snapshot["revision"], revision);
            assert_session_equivalent(&presentation_sse.next_session(), &snapshot);
            assert_session_equivalent(&speaker_sse.next_session(), &snapshot);
            snapshot
        };

        let no_op = accepted(&presentation, r#"{"command":"retreat"}"#, 2);
        assert_eq!(no_op["current"]["step"], 0);
        let advanced = accepted(&speaker, r#"{"command":"advance"}"#, 3);
        assert_eq!(advanced["current"]["step"], 1);
        assert_eq!(advanced["timer"]["status"], "running");
        let retreated = accepted(&presentation, r#"{"command":"retreat"}"#, 4);
        assert_eq!(retreated["current"]["step"], 0);
        let detail = accepted(&speaker, r#"{"command":"detail-next"}"#, 5);
        assert_eq!(detail["current"]["detail"], 1);
        let main = accepted(&presentation, r#"{"command":"detail-previous"}"#, 6);
        assert_eq!(main["current"]["detail"], 0);
        let paused = accepted(&speaker, r#"{"command":"timer-pause"}"#, 7);
        assert_eq!(paused["timer"]["status"], "paused");
        let started = accepted(&presentation, r#"{"command":"timer-start"}"#, 8);
        assert_eq!(started["timer"]["status"], "running");
        let reset = accepted(&speaker, r#"{"command":"timer-reset"}"#, 9);
        assert_eq!(
            reset["timer"],
            json!({ "status": "stopped", "elapsed_ms": 0 })
        );
        let navigated = accepted(
            &presentation,
            r#"{"command":"navigate","generation":1,"section":1,"detail":0,"step":0}"#,
            10,
        );
        assert_eq!(navigated["current"]["slide_id"], "section-2-main");
        assert!(navigated["next"].is_null());
        let end_no_op = accepted(&speaker, r#"{"command":"advance"}"#, 11);
        assert_eq!(end_no_op["current"]["slide_id"], "section-2-main");
        assert!(end_no_op["next"].is_null());

        drop(accepted);
        let rejected = |url: &str, body: &str, status: &str| {
            let request = format!(
                "POST /zpres/control HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let response = live_request(url, request.as_bytes());
            assert!(response.starts_with(status), "{response}");
            serde_json::from_str::<serde_json::Value>(response.split_once("\r\n\r\n").unwrap().1)
                .unwrap()
        };
        let unknown = rejected(
            &presentation,
            r#"{"command":"unknown"}"#,
            "HTTP/1.1 400 Bad Request",
        );
        assert_eq!(
            unknown,
            json!({
                "error": "unknown variant `unknown`, expected one of `advance`, `retreat`, `detail-next`, `detail-previous`, `navigate`, `timer-start`, `timer-pause`, `timer-reset` at line 1 column 20"
            })
        );
        let malformed = rejected(&speaker, "{", "HTTP/1.1 400 Bad Request");
        assert_eq!(
            malformed,
            json!({ "error": "EOF while parsing an object at line 1 column 1" })
        );

        let method_body = json!({ "error": "method not allowed; use POST" }).to_string();
        let wrong_method = live_request(
            &presentation,
            b"GET /zpres/control HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(
            wrong_method,
            format!(
                "HTTP/1.1 405 Method Not Allowed\r\nAllow: POST\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{method_body}",
                method_body.len()
            )
        );

        let oversized_body = json!({ "error": "request body is too large" }).to_string();
        let oversized = live_request(
            &speaker,
            b"POST /zpres/control HTTP/1.1\r\nHost: localhost\r\nContent-Length: 65537\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(
            oversized,
            format!(
                "HTTP/1.1 413 Content Too Large\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n{oversized_body}",
                oversized_body.len()
            )
        );
        let stale = rejected(
            &presentation,
            r#"{"command":"navigate","generation":0,"section":0,"detail":0,"step":0}"#,
            "HTTP/1.1 409 Conflict",
        );
        assert_eq!(stale["session"], end_no_op);
        let invalid = rejected(
            &speaker,
            r#"{"command":"navigate","generation":1,"section":99,"detail":0,"step":0}"#,
            "HTTP/1.1 409 Conflict",
        );
        assert_eq!(invalid["session"], end_no_op);
        assert_eq!(live_json_get(&presentation), end_no_op);
        assert_eq!(live_json_get(&speaker), end_no_op);

        host.shutdown();
    }

    #[test]
    fn control_request_security_uses_both_real_host_origins() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        write_dual_listener_source(&source);
        let mut host = start_live_host(test_options(source)).unwrap();
        let presentation = host.presentation_url().to_string();
        let speaker = host.speaker_url().unwrap().to_string();

        let body = r#"{"command":"advance"}"#;
        let request = format!(
            "POST /zpres/control HTTP/1.1\r\nHost: localhost\r\nOrigin: {presentation}\r\nSec-Fetch-Site: same-site\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let response = live_request(&speaker, request.as_bytes());
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");

        let body = r#"{"command":"retreat"}"#;
        let request = format!(
            "POST /zpres/control HTTP/1.1\r\nHost: localhost\r\nOrigin: {speaker}\r\nSec-Fetch-Site: same-site\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let response = live_request(&presentation, request.as_bytes());
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");

        let localhost_presentation = presentation.replace("127.0.0.1", "localhost");
        let localhost_speaker = speaker.replace("127.0.0.1", "localhost");
        let body = r#"{"command":"advance"}"#;
        let request = format!(
            "POST /zpres/control HTTP/1.1\r\nHost: localhost\r\nOrigin: {localhost_presentation}\r\nSec-Fetch-Site: same-site\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let response = live_request(&speaker, request.as_bytes());
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");

        let body = r#"{"command":"retreat"}"#;
        let request = format!(
            "POST /zpres/control HTTP/1.1\r\nHost: localhost\r\nOrigin: {localhost_speaker}\r\nSec-Fetch-Site: same-site\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let response = live_request(&presentation, request.as_bytes());
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");

        let before = live_json_get(&presentation);
        let body = r#"{"command":"timer-reset"}"#;
        let request = format!(
            "POST /zpres/control HTTP/1.1\r\nHost: localhost\r\nOrigin: http://127.0.0.1:9\r\nSec-Fetch-Site: cross-site\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let response = live_request(&speaker, request.as_bytes());
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        let after = live_json_get(&speaker);
        assert_eq!(after["revision"], before["revision"]);
        assert_eq!(after["current"], before["current"]);
        assert_eq!(after["timer"]["status"], before["timer"]["status"]);

        host.shutdown();
    }

    #[test]
    fn live_origin_parser_treats_only_localhost_and_ipv4_loopback_as_aliases() {
        assert_eq!(
            parse_http_origin("http://localhost:3004"),
            parse_http_origin("http://127.0.0.1:3004")
        );
        assert_eq!(
            parse_http_origin("http://LOCALHOST"),
            Some(HttpOrigin { port: 80 })
        );
        for rejected in [
            "https://localhost:3004",
            "http://localhost.example:3004",
            "http://127.0.0.2:3004",
            "http://user@localhost:3004",
            "http://localhost:3004/path",
        ] {
            assert_eq!(parse_http_origin(rejected), None, "accepted {rejected}");
        }
    }

    #[test]
    fn dual_listener_sse_streams_match_and_reconnect_to_current_state() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        write_dual_listener_source(&source);
        let mut host = start_live_host(test_options(source)).unwrap();
        let presentation = host.presentation_url().to_string();
        let speaker = host.speaker_url().unwrap().to_string();
        let mut audience_events = TestSseClient::connect(&presentation);
        let mut speaker_events = TestSseClient::connect(&speaker);
        assert_eq!(
            audience_events.initial_session(),
            speaker_events.initial_session()
        );

        let accepted = live_control(&speaker, r#"{"command":"advance"}"#);
        let audience_session = audience_events.next_session();
        let speaker_session = speaker_events.next_session();
        assert_session_equivalent(&audience_session, &accepted);
        assert_eq!(speaker_session, audience_session);

        drop(speaker_events);
        let mut reconnected = TestSseClient::connect(&speaker);
        assert_session_equivalent(&reconnected.initial_session(), &audience_session);
        let accepted = live_control(&presentation, r#"{"command":"retreat"}"#);
        let audience_session = audience_events.next_session();
        let speaker_session = reconnected.next_session();
        assert_session_equivalent(&audience_session, &accepted);
        assert_eq!(speaker_session, audience_session);

        host.shutdown();
    }

    #[test]
    fn live_host_port_modes_labels_and_shutdown_are_deterministic() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        write_dual_listener_source(&source);

        let mut host = start_live_host(test_options(source.clone())).unwrap();
        assert_ne!(host.presentation_port(), 0);
        assert_ne!(host.speaker_port(), Some(0));
        assert_ne!(host.speaker_port(), Some(host.presentation_port()));
        let labels = startup_messages(&test_options(source.clone()), &host);
        assert!(labels.contains(&format!("Presentation: {}", host.presentation_url())));
        assert!(labels.contains(&format!("Speaker: {}", host.speaker_url().unwrap())));
        assert_eq!(labels.last().unwrap(), "Press Ctrl-C to stop.");
        let presentation_address = live_address(host.presentation_url());
        let speaker_address = live_address(host.speaker_url().unwrap());
        let mut events = TestSseClient::connect(host.presentation_url());
        events.initial_session();
        host.shutdown();
        assert!(host.listener_handles.is_empty());
        assert!(host.connection_handles.lock().unwrap().is_empty());
        assert!(
            TcpStream::connect_timeout(&presentation_address, Duration::from_millis(100)).is_err()
        );
        assert!(TcpStream::connect_timeout(&speaker_address, Duration::from_millis(100)).is_err());

        let mut no_speaker = test_options(source);
        no_speaker.speaker_enabled = false;
        let mut host = start_live_host(no_speaker).unwrap();
        assert!(host.speaker_url().is_none());
        host.shutdown();
    }

    #[test]
    fn live_host_selects_default_explicit_and_next_free_ports() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        write_dual_listener_source(&source);

        let (mut host, requested_presentation, requested_speaker) = with_reserved_test_ports(
            TestPortPair::Adjacent,
            |requested_presentation, requested_speaker| {
                let mut options = test_options(source.clone());
                options.port = requested_presentation;
                let Ok(mut host) = start_live_host(options) else {
                    return None;
                };
                if host.presentation_port() == requested_presentation
                    && host.speaker_port() == Some(requested_speaker)
                {
                    Some((host, requested_presentation, requested_speaker))
                } else {
                    host.shutdown();
                    None
                }
            },
        );
        assert_eq!(host.presentation_port(), requested_presentation);
        assert_eq!(host.speaker_port(), Some(requested_speaker));
        host.shutdown();

        let (mut host, presentation_port, speaker_port) =
            with_reserved_test_ports(TestPortPair::Distinct, |presentation_port, speaker_port| {
                let mut options = test_options(source.clone());
                options.port = presentation_port;
                options.speaker_port = Some(speaker_port);
                let Ok(mut host) = start_live_host(options) else {
                    return None;
                };
                if host.presentation_port() == presentation_port
                    && host.speaker_port() == Some(speaker_port)
                {
                    Some((host, presentation_port, speaker_port))
                } else {
                    host.shutdown();
                    None
                }
            });
        assert_eq!(host.presentation_port(), presentation_port);
        assert_eq!(host.speaker_port(), Some(speaker_port));
        host.shutdown();

        let (mut host, busy_port) = with_reserved_test_port(|busy_port, busy_presentation| {
            let mut options = test_options(source.clone());
            options.port = busy_port;
            let host = start_live_host(options).ok()?;
            // The reservation remains live throughout startup, proving the
            // production listener actually had to advance past it.
            drop(busy_presentation);
            (host.presentation_port() > busy_port).then_some((host, busy_port))
        });
        assert!(host.presentation_port() > busy_port);
        host.shutdown();

        let (mut host, busy_port) = with_reserved_test_port(|busy_port, busy_speaker| {
            let mut options = test_options(source.clone());
            options.speaker_port = Some(busy_port);
            let host = start_live_host(options).ok()?;
            // Keep the collision owned until both production listeners have
            // finished binding.
            drop(busy_speaker);
            (host.speaker_port().is_some_and(|port| port > busy_port)).then_some((host, busy_port))
        });
        assert!(host.speaker_port().unwrap() > busy_port);
        host.shutdown();
    }

    #[test]
    fn speaker_startup_failure_releases_the_presentation_listener() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        write_dual_listener_source(&source);
        let busy_speaker = TcpListener::bind(("127.0.0.1", u16::MAX)).unwrap();
        let (rebound, requested_presentation) =
            with_reserved_test_port(|requested_presentation, presentation_reservation| {
                let mut options = test_options(source.clone());
                options.port = requested_presentation;
                options.speaker_port = Some(u16::MAX);

                // This is the only unavoidable bind handoff. Retrying the
                // whole attempt and proving immediate rebound makes a stolen
                // port a bounded retry instead of a false positive.
                drop(presentation_reservation);
                if start_live_host(options).is_ok() {
                    return None;
                }
                TcpListener::bind(("127.0.0.1", requested_presentation))
                    .ok()
                    .map(|rebound| (rebound, requested_presentation))
            });
        assert_eq!(rebound.local_addr().unwrap().port(), requested_presentation);
        drop(rebound);
        drop(busy_speaker);
    }

    #[test]
    fn listener_configuration_failure_starts_no_threads_and_releases_both_ports() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        write_dual_listener_source(&source);
        let (result, configured_roles, presentation_port, speaker_port) =
            with_reserved_test_ports(TestPortPair::Distinct, |presentation_port, speaker_port| {
                let mut options = test_options(source.clone());
                options.port = presentation_port;
                options.speaker_port = Some(speaker_port);
                let configured_roles = Mutex::new(Vec::new());

                let result = start_live_host_with_listener_config(options, |listener, role| {
                    configured_roles
                        .lock()
                        .unwrap()
                        .push((role, listener.local_addr().unwrap().port()));
                    if role == ViewRole::Speaker {
                        return Err(io::Error::other("injected speaker configuration failure"));
                    }
                    listener.set_nonblocking(true)
                });
                let configured_roles = configured_roles.into_inner().unwrap();
                let used_requested_ports = configured_roles
                    == vec![
                        (ViewRole::Presentation, presentation_port),
                        (ViewRole::Speaker, speaker_port),
                    ];
                used_requested_ports.then_some((
                    result,
                    configured_roles,
                    presentation_port,
                    speaker_port,
                ))
            });

        assert!(matches!(result, Err(ServerError::Io(_))));
        assert_eq!(
            configured_roles,
            vec![
                (ViewRole::Presentation, presentation_port),
                (ViewRole::Speaker, speaker_port)
            ]
        );
        let (presentation, speaker) = wait_until_both_bindable(presentation_port, speaker_port);
        drop((presentation, speaker));
    }

    #[test]
    fn idle_preconnection_timeout_closes_without_an_http_response() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        let options = Arc::new(test_options(source));
        let state = Arc::new(Mutex::new(ServerState::new(temp.path().to_path_buf())));
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_connection_with_timeout(
                stream,
                ViewRole::Presentation,
                options,
                state,
                Duration::from_millis(25),
            )
            .unwrap();
        });
        let mut client = TcpStream::connect(address).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut response = Vec::new();
        match client.read_to_end(&mut response) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
            Err(error) => panic!("unexpected read failure: {error}"),
        }
        worker.join().unwrap();

        assert!(
            response.is_empty(),
            "idle socket returned HTTP: {response:?}"
        );
    }

    #[test]
    fn failed_connection_registration_never_spawns_a_worker() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        write_dual_listener_source(&source);
        let mut host = start_live_host(test_options(source)).unwrap();
        host.active_connections
            .fail_registration
            .store(true, Ordering::Release);
        let mut client = TcpStream::connect(live_address(host.presentation_url())).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        client
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        // Registration failure is allowed to close the peer before the client
        // finishes its half-close.
        let _ = client.shutdown(Shutdown::Write);
        let mut response = Vec::new();
        match client.read_to_end(&mut response) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
            Err(error) => panic!("unexpected read failure: {error}"),
        }

        assert!(response.is_empty());
        assert!(host.connection_handles.lock().unwrap().is_empty());
        assert!(host.active_connections.streams.lock().unwrap().is_empty());
        host.shutdown();
    }

    #[test]
    fn completed_short_connections_are_reaped_during_production_accepts() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        write_dual_listener_source(&source);
        let mut host = start_live_host(test_options(source)).unwrap();
        let presentation = host.presentation_url().to_string();

        for _ in 0..128 {
            let session = live_json_get(&presentation);
            assert_eq!(session["protocol_version"], 1);
        }
        let deadline = Instant::now() + TEST_IO_TIMEOUT;
        while !host.active_connections.streams.lock().unwrap().is_empty() {
            assert!(
                Instant::now() < deadline,
                "short HTTP workers did not finish before the test deadline"
            );
            thread::sleep(Duration::from_millis(5));
        }

        // One more production accept reaps every previously completed worker.
        // Its own handle may remain until the next accept or host shutdown.
        live_json_get(&presentation);
        let deadline = Instant::now() + TEST_IO_TIMEOUT;
        while !host.active_connections.streams.lock().unwrap().is_empty() {
            assert!(
                Instant::now() < deadline,
                "final HTTP worker did not finish before the test deadline"
            );
            thread::sleep(Duration::from_millis(5));
        }
        let retained = host.connection_handles.lock().unwrap().len();
        assert!(
            retained <= 1,
            "completed short requests left {retained} worker handles retained"
        );

        host.shutdown();
        assert!(host.connection_handles.lock().unwrap().is_empty());
    }

    #[test]
    fn accepted_live_socket_waits_for_a_complete_http_request() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        write_dual_listener_source(&source);
        let mut host = start_live_host(test_options(source)).unwrap();
        let mut client = TcpStream::connect(live_address(host.presentation_url())).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();

        // Give the nonblocking listener time to accept the socket, then finish
        // the request in a second write. A worker that inherited nonblocking
        // mode would see WouldBlock after the first line and close early.
        client
            .write_all(b"GET /zpres/session.json HTTP/1.1\r\n")
            .unwrap();
        thread::sleep(Duration::from_millis(50));
        client.write_all(b"Host: localhost\r\n\r\n").unwrap();
        client.shutdown(Shutdown::Write).unwrap();

        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
        assert!(response.contains("\"protocol_version\":1"), "{response}");
        host.shutdown();
    }

    #[test]
    fn failed_browser_open_is_nonfatal_and_visits_every_active_view() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        write_dual_listener_source(&source);
        let mut host = start_live_host(test_options(source)).unwrap();
        let visited = Mutex::new(Vec::new());
        let warnings = open_active_views(&host, |url| {
            visited.lock().unwrap().push(url.to_string());
            Err(io::Error::other("injected opener failure"))
        });
        assert_eq!(visited.lock().unwrap().len(), 2);
        assert_eq!(warnings.len(), 2);
        assert!(warnings.iter().all(|warning| {
            warning.starts_with("warning: could not open http://127.0.0.1:")
                && warning.ends_with("injected opener failure")
        }));
        host.shutdown();
    }

    #[test]
    fn production_serve_boundary_starts_opens_and_stops_both_views() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        write_dual_listener_source(&source);
        let mut options = test_options(source);
        options.open = true;
        let visited = Mutex::new(Vec::new());
        let runtime_called = AtomicBool::new(false);

        let result = serve_with_runtime(
            options,
            |url| {
                visited.lock().unwrap().push(url.to_string());
                Err(io::Error::other("injected opener failure"))
            },
            |_, state| {
                runtime_called.store(true, Ordering::Release);
                assert!(state.lock().unwrap().speaker_url.is_some());
                Ok(())
            },
        );

        assert!(result.is_ok(), "browser launch failures must be nonfatal");
        assert!(runtime_called.load(Ordering::Acquire));
        let visited = visited.into_inner().unwrap();
        assert_eq!(visited.len(), 2);
        assert!(visited[0].starts_with("http://127.0.0.1:"));
        assert!(visited[1].starts_with("http://127.0.0.1:"));
        assert_ne!(visited[0], visited[1]);
    }

    #[test]
    fn speaker_live_browser_synchronizes_both_view_roles() {
        if discover_chromium_executable().executable.is_none() {
            eprintln!("skipping speaker_live_browser: Chromium is unavailable");
            return;
        }
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        let theme_dir = temp.path().join("themes").join("live-theme");
        write_live_theme(&theme_dir, "red");
        write_playable_test_wav(&temp.path().join("clip.wav"));
        let initial_source = r##"---
theme: "live-theme"
---

::: slide
id: browser-first
:::
# First

![audio autoplay autoadvance alt="Silent preview"](clip.wav "Silent preview audio")

::: steps
1. Reveal
:::

::: notes
Remember the result.

[click] Reveal the evidence.
:::

--

::: slide
id: browser-detail
:::
# Detail

---

::: slide
id: browser-last
:::
# Last
"##;
        fs::write(&source, initial_source).unwrap();
        let options = test_options(source.clone());
        let mut host = start_live_host(options.clone()).unwrap();
        let presentation = host.presentation_url().to_string();
        let speaker = host.speaker_url().unwrap().to_string();
        let mut browser = ChromiumSession::launch(ChromiumSessionOptions {
            startup_timeout: Duration::from_secs(10),
            protocol_timeout: Duration::from_secs(10),
        })
        .unwrap();
        browser
            .load_screen_document(
                &speaker,
                PngViewport {
                    width: 1280,
                    height: 720,
                },
            )
            .unwrap();
        let initial = browser
            .evaluate_for_test(
                r##"(async () => {
                  const deadline = performance.now() + 3000;
                  while (performance.now() < deadline) {
                    const current = document.querySelector("#current-route")?.textContent || "";
                    const next = document.querySelector("#next-route")?.textContent || "";
                    const marker = document.querySelector("[data-note-click-index='1']");
                    const preview = (id) => {
                      const frame = document.querySelector(id);
                      return {
                        route: JSON.parse(frame?.dataset.previewRoute || "null"),
                        evidence: JSON.parse(frame?.dataset.previewEvidence || "null"),
                      };
                    };
                    const currentPreview = preview("#current-frame");
                    const nextPreview = preview("#next-frame");
                    if (current.includes("Step 0/1") && next.includes("Step 1/1") && marker
                        && currentPreview.route?.step === 0 && nextPreview.route?.step === 1
                        && currentPreview.evidence?.active_slide_id === "browser-first"
                        && nextPreview.evidence?.active_slide_id === "browser-first") {
                      return {
                        current, next,
                        notes: document.querySelector("#speaker-notes")?.textContent || "",
                        markerClass: marker.className,
                        connected: document.querySelector("#connection-status")?.dataset.connected,
                        currentSlideId: currentPreview.evidence.active_slide_id,
                        nextSlideId: nextPreview.evidence.active_slide_id,
                        currentEvidence: currentPreview.evidence,
                        nextEvidence: nextPreview.evidence,
                        layout: (() => {
                          const box = (element) => {
                            const rect = element.getBoundingClientRect();
                            return { x:rect.x, y:rect.y, width:rect.width, height:rect.height,
                              right:rect.right, bottom:rect.bottom };
                          };
                          return {
                            current: box(document.querySelector("#current-frame").closest(".speaker-panel")),
                            next: box(document.querySelector("#next-frame").closest(".speaker-panel")),
                            notes: box(document.querySelector(".notes-panel")),
                          };
                        })(),
                      };
                    }
                    await new Promise(resolve => setTimeout(resolve, 20));
                  }
                  throw new Error("speaker view did not receive its initial session");
                })()"##,
            )
            .unwrap();
        assert!(
            initial["notes"]
                .as_str()
                .unwrap()
                .contains("Remember the result")
        );
        assert_eq!(initial["connected"], "true");
        assert_eq!(initial["currentSlideId"], "browser-first");
        assert_eq!(initial["nextSlideId"], "browser-first");
        assert_eq!(initial["currentEvidence"]["current_step"], 0);
        assert_eq!(initial["currentEvidence"]["visible_steps"], 0);
        assert_eq!(initial["nextEvidence"]["current_step"], 1);
        assert_eq!(initial["nextEvidence"]["visible_steps"], 1);
        assert!(
            initial["layout"]["next"]["x"].as_f64().unwrap()
                > initial["layout"]["current"]["x"].as_f64().unwrap(),
            "Next preview must occupy the right rail instead of shrinking below Current"
        );
        assert_eq!(
            initial["layout"]["next"]["x"],
            initial["layout"]["notes"]["x"]
        );
        assert!(
            initial["layout"]["next"]["bottom"].as_f64().unwrap()
                <= initial["layout"]["notes"]["y"].as_f64().unwrap(),
            "Next preview must sit above Notes without overlap"
        );
        for preview in ["currentEvidence", "nextEvidence"] {
            assert_eq!(initial[preview]["media_elements"], 0);
            assert_eq!(initial[preview]["nested_contexts"], 0);
            assert_eq!(initial[preview]["inert_placeholders"], 1);
            assert_eq!(initial[preview]["font_status"], "loaded");
            assert!(initial[preview]["stylesheet_count"].as_u64().unwrap() >= 3);
        }
        assert!(
            initial["markerClass"]
                .as_str()
                .unwrap()
                .contains("is-pending")
        );

        let advanced = browser
            .evaluate_for_test(
                r#"(async () => {
                  document.querySelector('[data-command="advance"]').click();
                  const deadline = performance.now() + 3000;
                  while (performance.now() < deadline) {
                    const session = await fetch('/zpres/session.json', {cache:'no-store'}).then(r => r.json());
                    const current = document.querySelector('#current-route')?.textContent || '';
                    const marker = document.querySelector("[data-note-click-index='1']");
                    const frame = document.querySelector('#current-frame');
                    const route = JSON.parse(frame?.dataset.previewRoute || 'null');
                    const evidence = JSON.parse(frame?.dataset.previewEvidence || 'null');
                    if (session.current?.step === 1 && current.includes('Step 1/1')
                        && marker?.classList.contains('is-active') && route?.step === 1
                        && evidence?.visible_steps === 1) {
                      return { session, route, evidence };
                    }
                    await new Promise(resolve => setTimeout(resolve, 20));
                  }
                  throw new Error('speaker advance did not synchronize');
                })()"#,
            )
            .unwrap();
        assert_eq!(advanced["session"]["timer"]["status"], "running");
        assert_eq!(advanced["session"]["current"]["slide_id"], "browser-first");
        assert_eq!(advanced["route"]["step"], 1);
        assert_eq!(advanced["evidence"]["visible_steps"], 1);

        let interpolated_timer = browser
            .evaluate_for_test(
                r#"(async () => {
                  const before = await fetch('/zpres/session.json', {cache:'no-store'}).then(r => r.json());
                  const initialText = document.querySelector('#elapsed-timer')?.textContent;
                  const deadline = performance.now() + 3000;
                  while (performance.now() < deadline) {
                    const currentText = document.querySelector('#elapsed-timer')?.textContent;
                    if (currentText !== initialText && currentText !== '00:00') {
                      const after = await fetch('/zpres/session.json', {cache:'no-store'}).then(r => r.json());
                      return { initialText, currentText, beforeRevision: before.revision, afterRevision: after.revision };
                    }
                    await new Promise(resolve => setTimeout(resolve, 50));
                  }
                  throw new Error('speaker timer did not interpolate locally');
                })()"#,
            )
            .unwrap();
        assert_eq!(
            interpolated_timer["beforeRevision"],
            interpolated_timer["afterRevision"]
        );

        browser
            .evaluate_for_test(
                r#"(async () => {
                  document.querySelector('[data-command="detail-next"]').click();
                  const deadline = performance.now() + 3000;
                  while (performance.now() < deadline) {
                    const session = await fetch('/zpres/session.json', {cache:'no-store'}).then(r => r.json());
                    if (session.current?.detail === 1) return true;
                    await new Promise(resolve => setTimeout(resolve, 20));
                  }
                  throw new Error('speaker Detail navigation did not synchronize');
                })()"#,
            )
            .unwrap();

        browser
            .load_screen_document(
                &presentation,
                PngViewport {
                    width: 1280,
                    height: 720,
                },
            )
            .unwrap();
        let audience = browser
            .evaluate_for_test(
                r#"(async () => {
                  document.dispatchEvent(new KeyboardEvent('keydown', {key:'ArrowUp', bubbles:true}));
                  const deadline = performance.now() + 3000;
                  while (performance.now() < deadline) {
                    const session = await fetch('/zpres/session.json', {cache:'no-store'}).then(r => r.json());
                    if (session.current?.detail === 0 && session.current?.step === 0 && location.hash === '#/0/0') return session;
                    await new Promise(resolve => setTimeout(resolve, 20));
                  }
                  throw new Error('audience keyboard control did not synchronize');
                })()"#,
            )
            .unwrap();
        assert_eq!(audience["current"]["detail"], 0);
        browser.reload_screen_document().unwrap();
        let reconnected = browser
            .evaluate_for_test(
                r#"(async () => {
                  const deadline = performance.now() + 3000;
                  while (performance.now() < deadline) {
                    const session = await fetch('/zpres/session.json', {cache:'no-store'}).then(r => r.json());
                    if (window.zpresLiveGeneration === session.generation && location.hash === '#/0/0') return session;
                    await new Promise(resolve => setTimeout(resolve, 20));
                  }
                  throw new Error('audience SSE reconnect did not restore the session');
                })()"#,
            )
            .unwrap();
        assert_eq!(reconnected["revision"], audience["revision"]);

        let generation_before_missed_updates = reconnected["generation"].as_u64().unwrap();
        browser
            .load_screen_document(
                "about:blank",
                PngViewport {
                    width: 1280,
                    height: 720,
                },
            )
            .unwrap();
        for color in ["green", "yellow", "purple", "red"] {
            write_live_theme(&theme_dir, color);
            rebuild(&options, &host.state, false);
        }
        browser
            .load_screen_document(
                &presentation,
                PngViewport {
                    width: 1280,
                    height: 720,
                },
            )
            .unwrap();
        let missed_update_reconnect = browser
            .evaluate_for_test(
                r#"(async () => {
                  const deadline = performance.now() + 3000;
                  while (performance.now() < deadline) {
                    const session = await fetch('/zpres/session.json', {cache:'no-store'}).then(r => r.json());
                    const rendered = window.zpresRenderedGeneration;
                    const live = window.zpresLiveGeneration;
                    if (rendered === session.generation && live === session.generation && location.hash === '#/0/0') {
                      return { generation: session.generation, rendered, live };
                    }
                    await new Promise(resolve => setTimeout(resolve, 20));
                  }
                  throw new Error('audience reconnect did not converge after missed render generations');
                })()"#,
            )
            .unwrap();
        assert!(
            missed_update_reconnect["generation"].as_u64().unwrap()
                >= generation_before_missed_updates + 4
        );

        browser
            .load_screen_document(
                &speaker,
                PngViewport {
                    width: 1280,
                    height: 720,
                },
            )
            .unwrap();
        let final_state = browser
            .evaluate_for_test(
                r#"(async () => {
                  const waitFor = async predicate => {
                    const deadline = performance.now() + 3000;
                    while (performance.now() < deadline) {
                      const session = await fetch('/zpres/session.json', {cache:'no-store'}).then(r => r.json());
                      if (predicate(session)) return session;
                      await new Promise(resolve => setTimeout(resolve, 20));
                    }
                    throw new Error('speaker did not reach requested final state');
                  };
                  await waitFor(session => session.current?.section === 0 && session.current?.step === 0);
                  document.querySelector('[data-command="advance"]').click();
                  await waitFor(session => session.current?.section === 0 && session.current?.step === 1);
                  document.querySelector('[data-command="advance"]').click();
                  const session = await waitFor(session => session.current?.section === 1);
                  const deadline = performance.now() + 3000;
                  while (performance.now() < deadline) {
                    const empty = document.querySelector('#next-empty');
                    if (!empty.hidden && empty.textContent.includes('End of presentation')) {
                      return { session, route: document.querySelector('#current-route').textContent, end: empty.textContent };
                    }
                    await new Promise(resolve => setTimeout(resolve, 20));
                  }
                  throw new Error('speaker did not render the end-of-presentation preview');
                })()"#,
            )
            .unwrap();
        assert_eq!(final_state["session"]["next"], serde_json::Value::Null);
        assert_eq!(
            final_state["session"]["current"]["slide_id"],
            "browser-last"
        );
        assert!(
            final_state["end"]
                .as_str()
                .unwrap()
                .contains("End of presentation")
        );

        let before_theme_generation = live_json_get(&speaker)["generation"].as_u64().unwrap();
        write_live_theme(&theme_dir, "blue");
        rebuild(&options, &host.state, false);
        let theme_refresh = browser
            .evaluate_for_test(
                r#"(async () => {
                  const deadline = performance.now() + 3000;
                  while (performance.now() < deadline) {
                    const frame = document.querySelector('#current-frame');
                    const evidence = JSON.parse(frame?.dataset.previewEvidence || 'null');
                    const session = await fetch('/zpres/session.json', {cache:'no-store'}).then(r => r.json());
                    if (evidence?.slide_color === 'rgb(0, 0, 255)') {
                      return { generation: session.generation, color: evidence.slide_color };
                    }
                    await new Promise(resolve => setTimeout(resolve, 20));
                  }
                  throw new Error('Theme CSS event did not refresh speaker previews');
                })()"#,
            )
            .unwrap();
        assert!(theme_refresh["generation"].as_u64().unwrap() > before_theme_generation);
        assert_eq!(theme_refresh["color"], "rgb(0, 0, 255)");

        let rebuilt_source = initial_source.replace("# Last", "# Last rebuilt");
        let before_valid_generation = live_json_get(&speaker)["generation"].as_u64().unwrap();
        fs::write(&source, &rebuilt_source).unwrap();
        rebuild(&options, &host.state, false);
        let valid_rebuild = browser
            .evaluate_for_test(
                r#"(async () => {
                  const deadline = performance.now() + 3000;
                  while (performance.now() < deadline) {
                    const session = await fetch('/zpres/session.json', {cache:'no-store'}).then(r => r.json());
                    const title = document.querySelector('#slide-title')?.textContent || '';
                    if (title.includes('Last rebuilt') && session.current?.section === 1) return { session, title };
                    await new Promise(resolve => setTimeout(resolve, 20));
                  }
                  throw new Error('valid Source rebuild was not reflected in speaker state');
                })()"#,
            )
            .unwrap();
        assert!(valid_rebuild["session"]["generation"].as_u64().unwrap() > before_valid_generation);
        assert_eq!(
            valid_rebuild["session"]["current"]["slide_id"],
            "browser-last"
        );
        let preserved_generation = valid_rebuild["session"]["generation"].as_u64().unwrap();
        let preserved_revision = valid_rebuild["session"]["revision"].as_u64().unwrap();

        fs::write(&source, "---\ntheme: [\n---\n# Broken\n").unwrap();
        rebuild(&options, &host.state, false);
        let invalid_rebuild = browser
            .evaluate_for_test(
                r#"(async () => {
                  const deadline = performance.now() + 3000;
                  while (performance.now() < deadline) {
                    const session = await fetch('/zpres/session.json', {cache:'no-store'}).then(r => r.json());
                    const diagnostics = document.querySelector('#diagnostic-status')?.textContent || '';
                    const title = document.querySelector('#slide-title')?.textContent || '';
                    if (diagnostics.includes('Last good') && diagnostics.includes('diagnostic') && title.includes('Last rebuilt')) {
                      return { session, diagnostics, title };
                    }
                    await new Promise(resolve => setTimeout(resolve, 20));
                  }
                  throw new Error('invalid Source diagnostics did not preserve the Last good deck');
                })()"#,
            )
            .unwrap();
        assert_eq!(
            invalid_rebuild["session"]["generation"].as_u64(),
            Some(preserved_generation)
        );
        assert_eq!(
            invalid_rebuild["session"]["revision"].as_u64(),
            Some(preserved_revision)
        );
        assert_eq!(invalid_rebuild["session"]["current"]["section"], 1);

        let before_foreign = live_json_get(&speaker);
        let before_theme_override = host.state.lock().unwrap().live_theme_override.clone();
        let (foreign_url, foreign_server) = serve_foreign_browser_page();
        browser
            .load_screen_document(
                &foreign_url,
                PngViewport {
                    width: 1280,
                    height: 720,
                },
            )
            .unwrap();
        foreign_server.join().unwrap();
        let control_url = serde_json::to_string(&format!("{presentation}/zpres/control")).unwrap();
        let theme_url =
            serde_json::to_string(&format!("{presentation}/zpres/theme-switch")).unwrap();
        let foreign_submissions = browser
            .evaluate_for_test(&format!(
                r#"(async () => {{
                  function observeNavigation(frame, name, trigger) {{
                    return new Promise((resolve, reject) => {{
                      const timeout = setTimeout(
                        () => reject(new Error(name + ' navigation did not complete')),
                        3000
                      );
                      const finish = (event) => {{
                        clearTimeout(timeout);
                        frame.removeEventListener('load', loaded);
                        frame.removeEventListener('error', failed);
                        resolve({{ name, completion: event }});
                      }};
                      const loaded = () => finish('load');
                      const failed = () => finish('error');
                      frame.addEventListener('load', loaded);
                      frame.addEventListener('error', failed);
                      trigger();
                    }});
                  }}
                  async function submit(action, fields, name) {{
                    const frame = document.createElement('iframe');
                    frame.name = name;
                    frame.hidden = true;
                    await observeNavigation(frame, name + '-initial', () => document.body.append(frame));
                    const form = document.createElement('form');
                    form.method = 'POST';
                    form.action = action;
                    form.target = name;
                    for (const [key, value] of Object.entries(fields)) {{
                      const input = document.createElement('input');
                      input.name = key;
                      input.value = value;
                      form.append(input);
                    }}
                    document.body.append(form);
                    const completion = await observeNavigation(frame, name, () => form.submit());
                    form.remove();
                    frame.remove();
                    return completion;
                  }}
                  const control = await submit(
                    {control_url},
                    {{command:'timer-reset'}},
                    'foreign-control'
                  );
                  const theme = await submit(
                    {theme_url},
                    {{theme:'science',variant:'defaults'}},
                    'foreign-theme'
                  );
                  return {{ control, theme }};
                }})()"#
            ))
            .unwrap();
        assert_eq!(foreign_submissions["control"]["name"], "foreign-control");
        assert_eq!(foreign_submissions["control"]["completion"], "load");
        assert_eq!(foreign_submissions["theme"]["name"], "foreign-theme");
        assert_eq!(foreign_submissions["theme"]["completion"], "load");
        let after_foreign = live_json_get(&speaker);
        assert_eq!(after_foreign["revision"], before_foreign["revision"]);
        assert_eq!(after_foreign["current"], before_foreign["current"]);
        assert_eq!(
            after_foreign["timer"]["status"],
            before_foreign["timer"]["status"]
        );
        assert_eq!(
            host.state.lock().unwrap().live_theme_override,
            before_theme_override
        );

        host.shutdown();
    }

    #[test]
    fn preview_isolation_browser_denies_content_requests_and_forged_control() {
        if discover_chromium_executable().executable.is_none() {
            eprintln!("skipping preview_isolation_browser: Chromium is unavailable");
            return;
        }
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        let initial = r#"# Isolated preview

::: audio src="https://hostile.invalid/probe-audio.wav" autoplay=true autoadvance=true controls=true
Hostile audio.
:::

::: iframe src="https://hostile.invalid/probe-frame" title="Hostile frame"
Hostile frame.
:::

::: html
<object data="https://hostile.invalid/probe-object"><embed src="https://hostile.invalid/probe-embed"></object>
:::

::: steps
1. Safe next state
:::
"#;
        fs::write(&source, initial).unwrap();
        let options = test_options(source.clone());
        let mut host = start_live_host(options.clone()).unwrap();
        let speaker = host.speaker_url().unwrap().to_string();
        let presentation = host.presentation_url().to_string();
        let adversarial_source = initial.replace("https://hostile.invalid", &presentation);
        fs::write(&source, &adversarial_source).unwrap();
        rebuild(&options, &host.state, false);
        let audience_html = live_request(
            &presentation,
            b"GET /presentation.html HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert!(audience_html.contains(&format!("{presentation}/probe-audio.wav")));
        assert!(audience_html.contains(&format!("{presentation}/probe-frame")));
        assert!(audience_html.contains(&format!("{presentation}/probe-object")));
        host.state.lock().unwrap().request_paths.clear();

        let mut browser = ChromiumSession::launch(ChromiumSessionOptions {
            startup_timeout: Duration::from_secs(10),
            protocol_timeout: Duration::from_secs(10),
        })
        .unwrap();
        browser
            .load_screen_document(
                &speaker,
                PngViewport {
                    width: 1280,
                    height: 720,
                },
            )
            .unwrap();
        let isolated = browser
            .evaluate_for_test(
                r#"(async () => {
                  const deadline = performance.now() + 3000;
                  while (performance.now() < deadline) {
                    const current = document.querySelector('#current-frame');
                    const next = document.querySelector('#next-frame');
                    const currentEvidence = JSON.parse(current?.dataset.previewEvidence || 'null');
                    const nextEvidence = JSON.parse(next?.dataset.previewEvidence || 'null');
                    if (currentEvidence && nextEvidence) {
                      const before = await fetch('/zpres/session.json', {cache:'no-store'}).then(r => r.json());
                      const oldUrl = new URL(current.src);
                      const forged = {
                        type:'zpres-preview-route', protocol_version:1,
                        nonce:'0'.repeat(32),
                        epoch:Number(oldUrl.searchParams.get('zpres-preview-epoch')),
                        role:'current',
                        route:{section:0,detail:0,step:1},
                      };
                      current.contentWindow.postMessage(forged, '*');
                      const currentUrl = new URL(current.src);
                      next.contentWindow.postMessage({
                        type:'zpres-preview-route', protocol_version:1,
                        nonce:'0'.repeat(32),
                        epoch:Number(currentUrl.searchParams.get('zpres-preview-epoch')),
                        role:'current', route:{section:0,detail:0,step:0},
                      }, '*');
                      current.dispatchEvent(new MouseEvent('click', {bubbles:false}));
                      current.dispatchEvent(new KeyboardEvent('keydown', {key:'ArrowRight',bubbles:false}));
                      current.focus();
                      await new Promise(resolve => setTimeout(resolve, 250));
                      const after = await fetch('/zpres/session.json', {cache:'no-store'}).then(r => r.json());
                      return {
                        before, after, oldUrl:oldUrl.href,
                        currentEvidence, nextEvidence,
                        currentRoute:JSON.parse(current.dataset.previewRoute),
                        nextRoute:JSON.parse(next.dataset.previewRoute),
                        topUrl:location.href,
                        activeTag:document.activeElement?.tagName || '',
                        currentSandbox:current.getAttribute('sandbox'),
                        nonceExposed:currentUrl.searchParams.has('zpres-preview-nonce'),
                      };
                    }
                    await new Promise(resolve => setTimeout(resolve, 20));
                  }
                  throw new Error('isolated previews did not authenticate');
                })()"#,
            )
            .unwrap();
        assert_eq!(
            isolated["before"]["revision"],
            isolated["after"]["revision"]
        );
        assert_eq!(isolated["after"]["current"]["step"], 0);
        assert_eq!(isolated["currentRoute"]["step"], 0);
        assert_eq!(isolated["nextRoute"]["step"], 1);
        assert!(isolated["topUrl"].as_str().unwrap().starts_with(&speaker));
        assert_eq!(isolated["currentSandbox"], "allow-scripts");
        assert_eq!(isolated["nonceExposed"], false);
        for evidence in ["currentEvidence", "nextEvidence"] {
            assert_eq!(isolated[evidence]["media_elements"], 0);
            assert_eq!(isolated[evidence]["nested_contexts"], 0);
            assert_eq!(isolated[evidence]["inert_placeholders"], 3);
            assert_eq!(isolated[evidence]["font_status"], "loaded");
        }
        assert!(
            host.state
                .lock()
                .unwrap()
                .request_paths
                .iter()
                .all(|path| !path.contains("probe-"))
        );

        let old_url = isolated["oldUrl"].as_str().unwrap().to_string();
        fs::write(
            &source,
            adversarial_source.replace("Isolated preview", "Isolated preview rebuilt"),
        )
        .unwrap();
        rebuild(&options, &host.state, false);
        let stale = browser
            .evaluate_for_test(&format!(
                r#"(async () => {{
                  const oldUrl = new URL({});
                  const frame = document.querySelector('#current-frame');
                  const deadline = performance.now() + 3000;
                  while (performance.now() < deadline) {{
                    const current = new URL(frame.src);
                    if (Number(current.searchParams.get('zpres-preview-epoch')) > Number(oldUrl.searchParams.get('zpres-preview-epoch'))
                        && frame.dataset.previewRoute) {{
                      const before = frame.dataset.previewRoute;
                      frame.contentWindow.postMessage({{
                        type:'zpres-preview-route', protocol_version:1,
                        nonce:'0'.repeat(32),
                        epoch:Number(oldUrl.searchParams.get('zpres-preview-epoch')),
                        role:'current', route:{{section:0,detail:0,step:1}},
                      }}, '*');
                      await new Promise(resolve => setTimeout(resolve, 200));
                      return {{ before, after:frame.dataset.previewRoute, src:frame.src }};
                    }}
                    await new Promise(resolve => setTimeout(resolve, 20));
                  }}
                  throw new Error('preview did not re-handshake after rebuild');
                }})()"#,
                serde_json::to_string(&old_url).unwrap()
            ))
            .unwrap();
        assert_eq!(stale["before"], stale["after"]);
        host.shutdown();
    }

    fn assert_preview_paint(png: &[u8], rect: &serde_json::Value, expected: [u8; 3], label: &str) {
        let image = image::load_from_memory(png).unwrap().to_rgb8();
        let x = rect["x"].as_f64().unwrap().max(0.0) as u32;
        let y = rect["y"].as_f64().unwrap().max(0.0) as u32;
        let width = rect["width"].as_f64().unwrap().max(1.0) as u32;
        let height = rect["height"].as_f64().unwrap().max(1.0) as u32;
        let inset_x = width / 6;
        let inset_y = height / 6;
        let mut matching = 0usize;
        let mut sampled = 0usize;
        for sample_y in ((y + inset_y)..(y + height - inset_y).min(image.height())).step_by(3) {
            for sample_x in ((x + inset_x)..(x + width - inset_x).min(image.width())).step_by(3) {
                let pixel = image.get_pixel(sample_x, sample_y).0;
                sampled += 1;
                if pixel
                    .iter()
                    .zip(expected)
                    .all(|(actual, expected)| actual.abs_diff(expected) <= 12)
                {
                    matching += 1;
                }
            }
        }
        let fraction = matching as f64 / sampled.max(1) as f64;
        assert!(
            fraction >= 0.45,
            "{label} painted fraction {fraction:.3} did not show expected rgb({},{},{})",
            expected[0],
            expected[1],
            expected[2]
        );
    }

    #[test]
    fn speaker_preview_v1_cdp_pixels_follow_main_detail_and_final_routes() {
        if discover_chromium_executable().executable.is_none() {
            eprintln!("skipping speaker preview paint test: Chromium is unavailable");
            return;
        }
        let temp = tempdir().unwrap();
        let source = temp.path().join("paint.zp.md");
        let theme_dir = temp.path().join("themes").join("live-theme");
        write_live_theme_v1(&theme_dir, "white");
        fs::write(
            theme_dir.join("theme.css.tmpl"),
            r#".zpres-slide { color:white; }
.zpres-slide-canvas { background:inherit !important; }
.zpres-block {}
.zpres-slide[data-slide-id="paint-main"] { background:rgb(220,38,38) !important; }
.zpres-slide[data-slide-id="paint-detail"] { background:rgb(22,163,74) !important; }
.zpres-slide[data-slide-id="paint-final"] { background:rgb(37,99,235) !important; }
"#,
        )
        .unwrap();
        let mut paint_source = r#"---
theme: live-theme
---
::: slide
id: paint-main
:::
# Long notes state

::: steps
1. Revealed paint state
:::

::: notes
This deliberately long note provides enough text to occupy the notes pane and makes the initial state visually distinct. It continues with a second sentence for deterministic paint evidence.
:::

--

::: slide
id: paint-detail
:::
# Detail paint

---

::: slide
id: paint-final
:::
# Final paint
"#
        .to_string();
        fs::write(&source, &paint_source).unwrap();
        let options = test_options(source.clone());
        let mut host = start_live_host(options.clone()).unwrap();
        let speaker = host.speaker_url().unwrap().to_string();
        let presentation = host.presentation_url().to_string();

        for viewport in [
            PngViewport {
                width: 1280,
                height: 720,
            },
            PngViewport {
                width: 1440,
                height: 900,
            },
        ] {
            let session = live_json_get(&presentation);
            live_control(
                &presentation,
                &format!(
                    r#"{{"command":"navigate","generation":{},"section":0,"detail":0,"step":0}}"#,
                    session["generation"].as_u64().unwrap()
                ),
            );
            let mut browser = ChromiumSession::launch(ChromiumSessionOptions {
                startup_timeout: Duration::from_secs(10),
                protocol_timeout: Duration::from_secs(10),
            })
            .unwrap();
            browser.load_screen_document(&speaker, viewport).unwrap();

            let current_rect = browser
                .evaluate_for_test(
                    r#"(async () => {
                      const frame = document.querySelector('#current-frame');
                      const deadline = performance.now() + 3000;
                      while (performance.now() < deadline) {
                        const route = JSON.parse(frame.dataset.previewRoute || 'null');
                        if (route?.slide_id === 'paint-main') {
                          const rect = frame.getBoundingClientRect();
                          const evidence = JSON.parse(frame.dataset.previewEvidence || 'null');
                          return {x:rect.x,y:rect.y,width:rect.width,height:rect.height,evidence};
                        }
                        await new Promise(resolve => requestAnimationFrame(resolve));
                      }
                      throw new Error('main preview route was not acknowledged');
                    })()"#,
                )
                .unwrap();
            let main_png = browser.capture_screenshot_for_test().unwrap();
            assert_eq!(current_rect["evidence"]["active_display"], "grid");
            assert_eq!(current_rect["evidence"]["inactive_display"], "none");
            assert_preview_paint(&main_png, &current_rect, [220, 38, 38], "main preview");

            let rapid_route = browser
                .evaluate_for_test(
                    r#"(async () => {
                      const frame = document.querySelector('#current-frame');
                      const initialSrc = frame.src;
                      const control = command => fetch('/zpres/control', {
                        method:'POST', headers:{'Content-Type':'application/json'},
                        body:JSON.stringify({command}),
                      }).then(response => {
                        if (!response.ok) throw new Error('rapid control failed');
                        return response.json();
                      });
                      await control('detail-next');
                      const finalSession = await control('detail-previous');
                      const deadline = performance.now() + 3000;
                      while (performance.now() < deadline) {
                        const route = JSON.parse(frame.dataset.previewRoute || 'null');
                        const evidence = JSON.parse(frame.dataset.previewEvidence || 'null');
                        if (finalSession.current.slide_id === 'paint-main'
                            && route?.slide_id === 'paint-main' && route.step === 0
                            && evidence?.active_slide_id === 'paint-main') {
                          await new Promise(resolve => setTimeout(resolve, 100));
                          return {
                            initialSrc, finalSrc:frame.src,
                            route:JSON.parse(frame.dataset.previewRoute || 'null'),
                            evidence:JSON.parse(frame.dataset.previewEvidence || 'null'),
                          };
                        }
                        await new Promise(resolve => requestAnimationFrame(resolve));
                      }
                      throw new Error('rapid Detail round trip did not settle on the desired route');
                    })()"#,
                )
                .unwrap();
            assert_eq!(rapid_route["initialSrc"], rapid_route["finalSrc"]);
            assert_eq!(rapid_route["route"]["slide_id"], "paint-main");
            assert_eq!(rapid_route["evidence"]["active_slide_id"], "paint-main");
            assert_eq!(rapid_route["evidence"]["active_display"], "grid");
            assert_eq!(rapid_route["evidence"]["inactive_display"], "none");
            let rapid_png = browser.capture_screenshot_for_test().unwrap();
            assert_preview_paint(
                &rapid_png,
                &current_rect,
                [220, 38, 38],
                "rapid route final preview",
            );

            let route_reload_invariants = browser
                .evaluate_for_test(
                    r#"(async () => {
                      const current = document.querySelector('#current-frame');
                      const next = document.querySelector('#next-frame');
                      const initial = { current:current.src, next:next.src };
                      document.querySelector('[data-command="timer-start"]').click();
                      const timerDeadline = performance.now() + 3000;
                      let pauseRequested = false;
                      let timerPaused = false;
                      while (performance.now() < timerDeadline) {
                        const session = await fetch('/zpres/session.json', {cache:'no-store'}).then(response => response.json());
                        if (session.timer.status === 'running' && !pauseRequested) {
                          pauseRequested = true;
                          document.querySelector('[data-command="timer-pause"]').click();
                        }
                        if (session.timer.status === 'paused') {
                          timerPaused = true;
                          break;
                        }
                        await new Promise(resolve => requestAnimationFrame(resolve));
                      }
                      if (!timerPaused) throw new Error('timer did not pause');
                      if (current.src !== initial.current || next.src !== initial.next) {
                        throw new Error('timer-only revisions reloaded a preview');
                      }
                      document.querySelector('[data-command="advance"]').click();
                      const stepDeadline = performance.now() + 3000;
                      while (performance.now() < stepDeadline) {
                        const current = document.querySelector('#current-frame');
                        const next = document.querySelector('#next-frame');
                        const currentRoute = JSON.parse(current.dataset.previewRoute || 'null');
                        const nextRoute = JSON.parse(next.dataset.previewRoute || 'null');
                        if (currentRoute?.slide_id === 'paint-main' && currentRoute.step === 1
                            && nextRoute?.slide_id === 'paint-final') {
                          const currentRect = current.getBoundingClientRect();
                          const nextRect = next.getBoundingClientRect();
                          return {
                            initial,
                            stepped:{current:current.src,next:next.src},
                            currentRect:{x:currentRect.x,y:currentRect.y,width:currentRect.width,height:currentRect.height},
                            nextRect:{x:nextRect.x,y:nextRect.y,width:nextRect.width,height:nextRect.height},
                          };
                        }
                        await new Promise(resolve => requestAnimationFrame(resolve));
                      }
                      throw new Error('Step route previews did not navigate and acknowledge');
                    })()"#,
                )
                .unwrap();
            assert_eq!(
                route_reload_invariants["initial"]["current"],
                route_reload_invariants["stepped"]["current"]
            );
            assert_eq!(
                route_reload_invariants["initial"]["next"],
                route_reload_invariants["stepped"]["next"]
            );
            let stepped_png = browser.capture_screenshot_for_test().unwrap();
            assert_preview_paint(
                &stepped_png,
                &route_reload_invariants["currentRect"],
                [220, 38, 38],
                "current Step preview",
            );
            assert_preview_paint(
                &stepped_png,
                &route_reload_invariants["nextRect"],
                [37, 99, 235],
                "next-after-Step preview",
            );
            browser
                .evaluate_for_test(
                    r#"(async () => {
                      document.querySelector('[data-command="retreat"]').click();
                      const deadline = performance.now() + 3000;
                      while (performance.now() < deadline) {
                        const frame = document.querySelector('#current-frame');
                        const route = JSON.parse(frame.dataset.previewRoute || 'null');
                        if (route?.slide_id === 'paint-main' && route.step === 0) return true;
                        await new Promise(resolve => requestAnimationFrame(resolve));
                      }
                      throw new Error('Step retreat did not settle');
                    })()"#,
                )
                .unwrap();

            browser
                .evaluate_for_test(
                    r#"(async () => {
                      document.querySelector('[data-command="detail-next"]').click();
                      const deadline = performance.now() + 3000;
                      while (performance.now() < deadline) {
                        const frame = document.querySelector('#current-frame');
                        const route = JSON.parse(frame.dataset.previewRoute || 'null');
                        if (route?.slide_id === 'paint-detail') {
                          const rect = frame.getBoundingClientRect();
                          return {x:rect.x,y:rect.y,width:rect.width,height:rect.height};
                        }
                        await new Promise(resolve => requestAnimationFrame(resolve));
                      }
                      throw new Error('Detail preview route was not acknowledged');
                    })()"#,
                )
                .unwrap();
            let generation_before_detail_rebuild =
                live_json_get(&presentation)["generation"].as_u64().unwrap();
            paint_source = paint_source.replace(
                "# Detail paint",
                &format!(
                    "# Detail paint rebuilt {}x{}",
                    viewport.width, viewport.height
                ),
            );
            fs::write(&source, &paint_source).unwrap();
            rebuild(&options, &host.state, false);
            let detail_rect = browser
                .evaluate_for_test(&format!(
                    r#"(async () => {{
                      const deadline = performance.now() + 3000;
                      while (performance.now() < deadline) {{
                        const frame = document.querySelector('#current-frame');
                        const route = JSON.parse(frame.dataset.previewRoute || 'null');
                        const url = new URL(frame.src);
                        if (route?.slide_id === 'paint-detail'
                            && Number(url.searchParams.get('zpres-generation')) > {generation_before_detail_rebuild}) {{
                          const rect = frame.getBoundingClientRect();
                          return {{x:rect.x,y:rect.y,width:rect.width,height:rect.height}};
                        }}
                        await new Promise(resolve => requestAnimationFrame(resolve));
                      }}
                      throw new Error('Detail preview did not repaint after generation rebuild');
                    }})()"#
                ))
                .unwrap();
            let detail_png = browser.capture_screenshot_for_test().unwrap();
            assert_preview_paint(&detail_png, &detail_rect, [22, 163, 74], "Detail preview");

            browser
                .evaluate_for_test(
                    r#"(async () => {
                      document.querySelector('[data-command="detail-previous"]').click();
                      const mainDeadline = performance.now() + 3000;
                      while (performance.now() < mainDeadline) {
                        const frame = document.querySelector('#current-frame');
                        const route = JSON.parse(frame.dataset.previewRoute || 'null');
                        if (route?.slide_id === 'paint-main') break;
                        await new Promise(resolve => requestAnimationFrame(resolve));
                      }
                      document.querySelector('[data-command="advance"]').click();
                      const stepDeadline = performance.now() + 3000;
                      while (performance.now() < stepDeadline) {
                        const frame = document.querySelector('#current-frame');
                        const route = JSON.parse(frame.dataset.previewRoute || 'null');
                        if (route?.slide_id === 'paint-main' && route.step === 1) break;
                        await new Promise(resolve => requestAnimationFrame(resolve));
                      }
                      document.querySelector('[data-command="advance"]').click();
                      const deadline = performance.now() + 3000;
                      while (performance.now() < deadline) {
                        const frame = document.querySelector('#current-frame');
                        const route = JSON.parse(frame.dataset.previewRoute || 'null');
                        if (route?.slide_id === 'paint-final') {
                          const rect = frame.getBoundingClientRect();
                          return {x:rect.x,y:rect.y,width:rect.width,height:rect.height};
                        }
                        await new Promise(resolve => requestAnimationFrame(resolve));
                      }
                      throw new Error('final preview route was not acknowledged');
                    })()"#,
                )
                .unwrap();
            let generation_before_final_rebuild =
                live_json_get(&presentation)["generation"].as_u64().unwrap();
            paint_source = paint_source.replace(
                "# Final paint",
                &format!(
                    "# Final paint rebuilt {}x{}",
                    viewport.width, viewport.height
                ),
            );
            fs::write(&source, &paint_source).unwrap();
            rebuild(&options, &host.state, false);
            let final_rect = browser
                .evaluate_for_test(&format!(
                    r#"(async () => {{
                      const deadline = performance.now() + 3000;
                      while (performance.now() < deadline) {{
                        const frame = document.querySelector('#current-frame');
                        const route = JSON.parse(frame.dataset.previewRoute || 'null');
                        const url = new URL(frame.src);
                        if (route?.slide_id === 'paint-final'
                            && Number(url.searchParams.get('zpres-generation')) > {generation_before_final_rebuild}) {{
                          const rect = frame.getBoundingClientRect();
                          return {{x:rect.x,y:rect.y,width:rect.width,height:rect.height}};
                        }}
                        await new Promise(resolve => requestAnimationFrame(resolve));
                      }}
                      throw new Error('final preview did not repaint after generation rebuild');
                    }})()"#
                ))
                .unwrap();
            let final_png = browser.capture_screenshot_for_test().unwrap();
            assert_preview_paint(&final_png, &final_rect, [37, 99, 235], "final preview");
        }
        host.shutdown();
    }

    #[test]
    fn both_view_roles_share_the_control_session() {
        let temp = tempdir().unwrap();
        let state = Arc::new(Mutex::new(ServerState::new(temp.path().to_path_buf())));
        state.lock().unwrap().session.install_routes(vec![
            test_route(0, 0, 0, 1, "main-1"),
            test_route(0, 0, 1, 1, "main-1"),
            test_route(1, 0, 0, 0, "main-2"),
        ]);
        let options = test_options(temp.path().join("talk.zp.md"));
        let body = br#"{"command":"advance"}"#;
        let request = format!(
            "POST /zpres/control HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            String::from_utf8_lossy(body)
        );
        let response = request_once(
            request.as_bytes(),
            ViewRole::Speaker,
            options.clone(),
            Arc::clone(&state),
        );
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert_eq!(
            state.lock().unwrap().session.current_route().unwrap().step,
            1
        );

        let response = request_once(
            b"GET /zpres/session.json HTTP/1.1\r\nHost: localhost\r\n\r\n",
            ViewRole::Presentation,
            options,
            state,
        );
        assert!(response.contains("\"step\":1"));
        assert!(response.contains("\"status\":\"running\""));
    }

    #[test]
    fn http_request_security_headers_are_normalized_and_duplicates_are_rejected() {
        let temp = tempdir().unwrap();
        let options = test_options(temp.path().join("talk.zp.md"));
        let state = Arc::new(Mutex::new(ServerState::new(temp.path().to_path_buf())));
        state.lock().unwrap().session.install_routes(vec![
            test_route(0, 0, 0, 1, "main-1"),
            test_route(0, 0, 1, 1, "main-1"),
        ]);
        let body = r#"{"command":"advance"}"#;
        let accepted = format!(
            "POST /zpres/control HTTP/1.1\r\nhOsT: localhost \r\noRiGiN:  http://127.0.0.1:3000 \r\ncOnTeNt-TyPe: Application/JSON ; charset=utf-8 \r\nsEc-FeTcH-sItE: SAME-ORIGIN \r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let response = request_once(
            accepted.as_bytes(),
            ViewRole::Presentation,
            options.clone(),
            Arc::clone(&state),
        );
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        let accepted_revision = state.lock().unwrap().session.revision();

        for (name, first, second) in [
            ("Host", "localhost", "localhost"),
            ("Origin", "http://127.0.0.1:3000", "http://127.0.0.1:3000"),
            ("Content-Type", "application/json", "application/json"),
            ("Sec-Fetch-Site", "same-origin", "same-origin"),
        ] {
            let request = format!(
                "POST /zpres/control HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\n{name}: {first}\r\n{name}: {second}\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let response = request_once(
                request.as_bytes(),
                ViewRole::Presentation,
                options.clone(),
                Arc::clone(&state),
            );
            assert!(
                response.starts_with("HTTP/1.1 400 Bad Request"),
                "duplicate {name} was not rejected: {response}"
            );
            assert_eq!(state.lock().unwrap().session.revision(), accepted_revision);
        }
    }

    #[test]
    fn control_request_security_rejects_browser_forgeries_without_mutation() {
        let temp = tempdir().unwrap();
        let options = test_options(temp.path().join("talk.zp.md"));
        let state = Arc::new(Mutex::new(ServerState::new(temp.path().to_path_buf())));
        {
            let mut state = state.lock().unwrap();
            state.speaker_url = Some("http://127.0.0.1:3001".to_string());
            state.session.install_routes(vec![
                test_route(0, 0, 0, 1, "main-1"),
                test_route(0, 0, 1, 1, "main-1"),
            ]);
        }
        let body = r#"{"command":"advance"}"#;
        for (origin, fetch_site) in [
            (Some("http://127.0.0.1:3000"), Some("same-origin")),
            (Some("http://127.0.0.1:3001"), Some("same-site")),
            (Some("http://127.0.0.1:3000"), None),
            (None, None),
        ] {
            let origin = origin.map_or_else(String::new, |value| format!("Origin: {value}\r\n"));
            let fetch_site =
                fetch_site.map_or_else(String::new, |value| format!("Sec-Fetch-Site: {value}\r\n"));
            let request = format!(
                "POST /zpres/control HTTP/1.1\r\nHost: localhost\r\n{origin}{fetch_site}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let response = request_once(
                request.as_bytes(),
                ViewRole::Presentation,
                options.clone(),
                Arc::clone(&state),
            );
            assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        }

        let before = {
            let state = state.lock().unwrap();
            (
                state.session.revision(),
                state.session.current_index(),
                state.session.timer_status(),
            )
        };
        let reset = r#"{"command":"timer-reset"}"#;
        for (headers, expected) in [
            (
                "Origin: http://127.0.0.1:3999\r\nContent-Type: application/json\r\n",
                "403 Forbidden",
            ),
            (
                "Origin: null\r\nContent-Type: application/json\r\n",
                "403 Forbidden",
            ),
            (
                "Origin: http://127.0.0.1:3000/\r\nContent-Type: application/json\r\n",
                "403 Forbidden",
            ),
            (
                "Origin: http://127.0.0.1:3000\r\nSec-Fetch-Site: cross-site\r\nContent-Type: application/json\r\n",
                "403 Forbidden",
            ),
            (
                "Sec-Fetch-Site: same-origin\r\nContent-Type: application/json\r\n",
                "403 Forbidden",
            ),
            (
                "Origin: http://127.0.0.1:3000\r\n",
                "415 Unsupported Media Type",
            ),
            (
                "Origin: http://127.0.0.1:3000\r\nContent-Type: text/plain\r\n",
                "415 Unsupported Media Type",
            ),
        ] {
            let request = format!(
                "POST /zpres/control HTTP/1.1\r\nHost: localhost\r\n{headers}Content-Length: {}\r\n\r\n{reset}",
                reset.len()
            );
            let response = request_once(
                request.as_bytes(),
                ViewRole::Speaker,
                options.clone(),
                Arc::clone(&state),
            );
            assert!(
                response.starts_with(&format!("HTTP/1.1 {expected}")),
                "unexpected response for {headers:?}: {response}"
            );
            assert!(!response.contains("Access-Control-Allow-Origin"));
            let after = state.lock().unwrap();
            assert_eq!(
                (
                    after.session.revision(),
                    after.session.current_index(),
                    after.session.timer_status(),
                ),
                before
            );
        }
    }

    #[test]
    fn control_api_rejects_methods_and_oversized_bodies() {
        let temp = tempdir().unwrap();
        let options = test_options(temp.path().join("talk.zp.md"));
        let state = Arc::new(Mutex::new(ServerState::new(temp.path().to_path_buf())));
        let response = request_once(
            b"GET /zpres/control HTTP/1.1\r\nHost: localhost\r\n\r\n",
            ViewRole::Presentation,
            options.clone(),
            Arc::clone(&state),
        );
        assert!(response.starts_with("HTTP/1.1 405 Method Not Allowed"));
        assert!(response.contains("Allow: POST"));
        let response = request_once(
            b"POST /zpres/control HTTP/1.1\r\nHost: localhost\r\nContent-Length: 65537\r\n\r\n",
            ViewRole::Presentation,
            options,
            state,
        );
        assert!(response.starts_with("HTTP/1.1 413 Content Too Large"));
    }

    #[test]
    fn control_api_rejects_malformed_and_stale_navigation() {
        let temp = tempdir().unwrap();
        let options = test_options(temp.path().join("talk.zp.md"));
        let state = Arc::new(Mutex::new(ServerState::new(temp.path().to_path_buf())));
        state
            .lock()
            .unwrap()
            .session
            .install_routes(vec![test_route(0, 0, 0, 0, "main-1")]);

        let malformed = br#"{"command":"unknown"}"#;
        let request = format!(
            "POST /zpres/control HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            malformed.len(),
            String::from_utf8_lossy(malformed)
        );
        let response = request_once(
            request.as_bytes(),
            ViewRole::Presentation,
            options.clone(),
            Arc::clone(&state),
        );
        assert!(response.starts_with("HTTP/1.1 400 Bad Request"));

        let stale = br#"{"command":"navigate","generation":0,"section":0,"detail":0,"step":0}"#;
        let request = format!(
            "POST /zpres/control HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            stale.len(),
            String::from_utf8_lossy(stale)
        );
        let response = request_once(
            request.as_bytes(),
            ViewRole::Speaker,
            options,
            Arc::clone(&state),
        );
        assert!(response.starts_with("HTTP/1.1 409 Conflict"));
        assert!(response.contains("\"session\""));
        assert!(response.contains("\"generation\":1"));

        let revision = state.lock().unwrap().session.revision();
        let advance = br#"{"command":"advance"}"#;
        let request = format!(
            "POST /zpres/control HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            advance.len(),
            String::from_utf8_lossy(advance)
        );
        let response = request_once(
            request.as_bytes(),
            ViewRole::Presentation,
            test_options(temp.path().join("talk.zp.md")),
            Arc::clone(&state),
        );
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert_eq!(state.lock().unwrap().session.revision(), revision + 1);
    }

    #[test]
    fn preview_mode_query_is_exact_and_response_has_restrictive_csp() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        fs::write(&source, "# Preview route\n\n::: steps\n1. Revealed\n:::\n").unwrap();
        let options = test_options(source);
        let state = Arc::new(Mutex::new(ServerState::new(temp.path().to_path_buf())));
        rebuild(&options, &state, true);
        {
            let mut state = state.lock().unwrap();
            state.last_good_html = "AUDIENCE".to_string();
            state.last_good_preview_html = "PREVIEW".to_string();
        }
        let request = |target: &str| {
            request_once(
                format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes(),
                ViewRole::Presentation,
                options.clone(),
                Arc::clone(&state),
            )
        };

        let preview = request("/presentation.html?zpres-preview=1&zpres-preview-role=current");
        assert!(preview.starts_with("HTTP/1.1 200 OK"));
        assert!(preview.contains("Content-Security-Policy: default-src 'none';"));
        assert!(preview.contains("script-src 'self'"));
        assert!(preview.contains("object-src 'none'"));
        assert!(preview.contains("frame-src 'none'"));
        assert!(preview.contains("media-src 'none'"));
        assert!(preview.contains("connect-src 'none'"));
        assert!(preview.ends_with("PREVIEW"));
        assert!(!preview.contains("unsafe-inline"));

        let generation = state.lock().unwrap().session.generation();
        let bootstrap = request(&format!(
            "/presentation.html?zpres-preview=1&zpres-preview-section=0&zpres-preview-detail=0&zpres-preview-step=1&zpres-generation={generation}"
        ));
        assert!(bootstrap.starts_with("HTTP/1.1 200 OK"));
        assert!(bootstrap.contains("data-zpres-output-target=\"screen\""));
        assert!(bootstrap.contains("data-zpres-view-role=\"preview\""));
        assert!(bootstrap.contains("zpres-section-stack is-active"));
        assert!(bootstrap.contains("zpres-slide is-active"));
        assert!(bootstrap.contains("zpres-step fragment is-visible"));

        let substring = request("/presentation.html?value=zpres-preview%3D1");
        assert!(substring.ends_with("AUDIENCE"));
        assert!(
            request("/presentation.html?zpres-preview=1&zpres-preview=1")
                .starts_with("HTTP/1.1 400 Bad Request")
        );
        assert!(
            request("/presentation.html?zpres-preview=%").starts_with("HTTP/1.1 400 Bad Request")
        );
        for invalid in [
            "/presentation.html?zpres-preview=1&zpres-preview-section=0&zpres-preview-detail=0&zpres-generation=1",
            "/presentation.html?zpres-preview=1&zpres-preview-section=-1&zpres-preview-detail=0&zpres-preview-step=0&zpres-generation=1",
            "/presentation.html?zpres-preview=1&zpres-preview-section=0&zpres-preview-section=0&zpres-preview-detail=0&zpres-preview-step=0&zpres-generation=1",
            "/presentation.html?zpres-generation=1",
        ] {
            assert!(
                request(invalid).starts_with("HTTP/1.1 400 Bad Request"),
                "accepted invalid preview bootstrap: {invalid}"
            );
        }
        assert!(
            request(&format!(
                "/presentation.html?zpres-preview=1&zpres-preview-section=0&zpres-preview-detail=0&zpres-preview-step=0&zpres-generation={}",
                generation.saturating_sub(1)
            ))
            .starts_with("HTTP/1.1 400 Bad Request")
        );
        assert!(
            request(&format!(
                "/presentation.html?zpres-preview=1&zpres-preview-section=99&zpres-preview-detail=0&zpres-preview-step=0&zpres-generation={generation}"
            ))
            .starts_with("HTTP/1.1 400 Bad Request")
        );
        assert!(request("/presentation.html?%7Apres-preview=1").ends_with("PREVIEW"));
    }

    #[test]
    fn invalid_rebuild_preserves_the_last_good_session_generation_and_route() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        fs::write(&source, "# First\n\n::: steps\n1. Reveal\n:::\n").unwrap();
        let options = test_options(source.clone());
        let state = Arc::new(Mutex::new(ServerState::new(temp.path().to_path_buf())));
        rebuild(&options, &state, true);
        {
            let mut state = state.lock().unwrap();
            let now = state.session_clock_origin.elapsed();
            state.session.apply(SessionCommand::Advance, now).unwrap();
        }
        let before = {
            let state = state.lock().unwrap();
            (
                state.session.generation(),
                state.session.current_route().cloned(),
            )
        };
        fs::write(&source, "---\ntheme: [\n---\n# Broken\n").unwrap();

        rebuild(&options, &state, false);

        let state = state.lock().unwrap();
        assert_eq!(state.session.generation(), before.0);
        assert_eq!(state.session.current_route(), before.1.as_ref());
        assert!(state.diagnostics.iter().any(Diagnostic::is_fatal));
    }

    #[test]
    fn rebuild_ordering_source_matrix_keeps_only_the_newest_attempt() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        let initial_source = "::: slide\nid: initial-route\n:::\n# Initial\n";
        fs::write(&source, initial_source).unwrap();
        let options = test_options(source.clone());
        let mut host = start_live_host(options.clone()).unwrap();
        let presentation = host.presentation_url().to_string();
        let speaker = host.speaker_url().unwrap().to_string();
        let mut events = TestSseClient::connect(&presentation);
        let initial = events.initial_session();
        live_control(&speaker, r#"{"command":"timer-start"}"#);
        let timer_event = events.next_event();
        assert_eq!(timer_event.0, "session");
        assert_eq!(timer_event.1["timer"]["status"], "running");

        let older_valid = "::: slide\nid: older-valid\n:::\n# Older valid\n";
        let newer_valid = "::: slide\nid: newest-valid\n:::\n# Newest valid\n";
        fs::write(&source, older_valid).unwrap();
        let blocker = block_rebuild_before_commit(&host.state, older_valid);
        let older_options = options.clone();
        let older_state = Arc::clone(&host.state);
        let older = thread::spawn(move || rebuild(&older_options, &older_state, false));
        let observed = blocker.observed.recv_timeout(TEST_IO_TIMEOUT).unwrap();
        assert!(observed.ticket > initial["generation"].as_u64().unwrap());
        assert_eq!(observed.valid, Some(true));
        assert!(observed.requested_theme_override.is_none());

        fs::write(&source, newer_valid).unwrap();
        assert!(matches!(
            rebuild(&options, &host.state, false),
            CommitOutcome::AcceptedValid
        ));
        let diagnostic_event = events.next_event();
        assert_eq!(diagnostic_event.0, "diagnostics");
        let session_event = events.next_event();
        assert_eq!(session_event.0, "session");
        assert_eq!(session_event.1["current"]["slide_id"], "newest-valid");
        assert_eq!(events.next_event().0, "reload");
        blocker.release.send(()).unwrap();
        assert!(matches!(older.join().unwrap(), CommitOutcome::Superseded));
        clear_rebuild_hook(&host.state);

        let newest = live_json_get(&speaker);
        assert_eq!(
            newest["generation"],
            initial["generation"].as_u64().unwrap() + 1
        );
        assert_eq!(
            newest["revision"],
            initial["revision"].as_u64().unwrap() + 2
        );
        assert_eq!(newest["current"]["slide_id"], "newest-valid");
        assert_eq!(newest["timer"]["status"], "running");
        assert!(
            host.state
                .lock()
                .unwrap()
                .last_good_html
                .contains("Newest valid")
        );

        let older_valid_after_newest =
            "::: slide\nid: stale-after-invalid\n:::\n# Stale after invalid\n";
        let newer_invalid = "---\ntheme: [\n---\n# Newest invalid\n";
        fs::write(&source, older_valid_after_newest).unwrap();
        let blocker = block_rebuild_before_commit(&host.state, older_valid_after_newest);
        let older_options = options.clone();
        let older_state = Arc::clone(&host.state);
        let older = thread::spawn(move || rebuild(&older_options, &older_state, false));
        assert_eq!(
            blocker
                .observed
                .recv_timeout(TEST_IO_TIMEOUT)
                .unwrap()
                .valid,
            Some(true)
        );
        let before_invalid = live_json_get(&presentation);
        fs::write(&source, newer_invalid).unwrap();
        assert!(matches!(
            rebuild(&options, &host.state, false),
            CommitOutcome::AcceptedInvalid { .. }
        ));
        let invalid_event = events.next_event();
        assert_eq!(invalid_event.0, "diagnostics");
        assert!(
            invalid_event.1["html"]
                .as_str()
                .unwrap()
                .contains("invalid source front matter")
        );
        blocker.release.send(()).unwrap();
        assert!(matches!(older.join().unwrap(), CommitOutcome::Superseded));
        clear_rebuild_hook(&host.state);
        let after_invalid = live_json_get(&speaker);
        assert_eq!(after_invalid["generation"], before_invalid["generation"]);
        assert_eq!(after_invalid["revision"], before_invalid["revision"]);
        assert_eq!(after_invalid["current"], before_invalid["current"]);
        assert!(
            host.state
                .lock()
                .unwrap()
                .last_good_html
                .contains("Newest valid")
        );

        let older_invalid = "---\ntheme: [\n---\n# Older invalid\n";
        let final_valid = "::: slide\nid: final-valid\n:::\n# Final valid\n";
        fs::write(&source, older_invalid).unwrap();
        let blocker = block_rebuild_before_commit(&host.state, older_invalid);
        let older_options = options.clone();
        let older_state = Arc::clone(&host.state);
        let older = thread::spawn(move || rebuild(&older_options, &older_state, false));
        assert_eq!(
            blocker
                .observed
                .recv_timeout(TEST_IO_TIMEOUT)
                .unwrap()
                .valid,
            Some(false)
        );
        fs::write(&source, final_valid).unwrap();
        assert!(matches!(
            rebuild(&options, &host.state, false),
            CommitOutcome::AcceptedValid
        ));
        assert_eq!(events.next_event().0, "diagnostics");
        let final_session = events.next_session();
        assert_eq!(final_session["current"]["slide_id"], "final-valid");
        assert_eq!(events.next_event().0, "reload");
        blocker.release.send(()).unwrap();
        assert!(matches!(older.join().unwrap(), CommitOutcome::Superseded));
        clear_rebuild_hook(&host.state);
        let final_presentation = live_json_get(&presentation);
        assert_session_equivalent(&final_presentation, &live_json_get(&speaker));
        assert_eq!(final_presentation["current"]["slide_id"], "final-valid");
        assert_eq!(final_presentation["timer"]["status"], "running");
        assert!(host.state.lock().unwrap().diagnostics.is_empty());

        host.shutdown();
    }

    #[test]
    fn rebuild_commit_publishes_atomically_before_waiting_control() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        let initial_source = "# Initial\n";
        let rebuilt_source = "# Rebuilt atomically\n\nChanged body.\n";
        fs::write(&source, initial_source).unwrap();
        let options = test_options(source.clone());
        let mut host = start_live_host(options.clone()).unwrap();
        let presentation = host.presentation_url().to_string();
        let mut events = TestSseClient::connect(&presentation);
        let initial = events.initial_session();

        let fingerprint = format!("{:x}", Sha256::digest(rebuilt_source.as_bytes()));
        let (installed_tx, installed_rx) = mpsc::channel();
        let (rebuild_release_tx, rebuild_release_rx) = mpsc::channel();
        let rebuild_release_rx = Arc::new(Mutex::new(rebuild_release_rx));
        let installed_once = Arc::new(AtomicBool::new(false));
        let rebuild_hook: RebuildTestHook = Arc::new(move |event| {
            if event.phase == RebuildHookPhase::AfterStateInstalledBeforeBroadcast
                && event.fingerprint.as_deref() == Some(fingerprint.as_str())
                && !installed_once.swap(true, Ordering::SeqCst)
            {
                installed_tx.send(()).unwrap();
                rebuild_release_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(TEST_IO_TIMEOUT)
                    .expect("timed out waiting to publish installed rebuild state");
            }
        });

        let (control_reached_tx, control_reached_rx) = mpsc::channel();
        let (control_release_tx, control_release_rx) = mpsc::channel();
        let control_release_rx = Arc::new(Mutex::new(control_release_rx));
        let (control_boundary_tx, control_boundary_rx) = mpsc::channel();
        let control_hook: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            control_reached_tx.send(()).unwrap();
            control_release_rx
                .lock()
                .unwrap()
                .recv_timeout(TEST_IO_TIMEOUT)
                .expect("timed out waiting to release control at the state-lock boundary");
            control_boundary_tx.send(()).unwrap();
        });
        {
            let mut coordinator = host.rebuild_coordinator.lock().unwrap();
            coordinator.test_hook = Some(rebuild_hook);
            coordinator.control_before_state_lock_hook = Some(control_hook);
        }

        let control_url = presentation.clone();
        let control =
            thread::spawn(move || live_control(&control_url, r#"{"command":"timer-reset"}"#));
        control_reached_rx
            .recv_timeout(TEST_IO_TIMEOUT)
            .expect("control did not reach its pre-lock hook");

        fs::write(&source, rebuilt_source).unwrap();
        let rebuild_options = options.clone();
        let rebuild_state = Arc::clone(&host.state);
        let rebuild = thread::spawn(move || rebuild(&rebuild_options, &rebuild_state, false));
        installed_rx
            .recv_timeout(TEST_IO_TIMEOUT)
            .expect("rebuild did not install state before publication");
        assert!(matches!(
            host.state.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        ));

        control_release_tx.send(()).unwrap();
        control_boundary_rx
            .recv_timeout(TEST_IO_TIMEOUT)
            .expect("control did not reach the ServerState lock boundary");
        rebuild_release_tx.send(()).unwrap();

        assert!(matches!(
            rebuild.join().unwrap(),
            CommitOutcome::AcceptedValid
        ));
        let control_snapshot = control.join().unwrap();

        let (diagnostic_event, _) = events.next_event();
        assert_eq!(diagnostic_event, "diagnostics");
        let (rebuild_event, rebuild_snapshot) = events.next_event();
        assert_eq!(rebuild_event, "session");
        let (update_event, _) = events.next_event();
        assert_eq!(update_event, "reload");
        let (control_event, published_control_snapshot) = events.next_event();
        assert_eq!(control_event, "session");
        assert_eq!(
            rebuild_snapshot["revision"].as_u64().unwrap() + 1,
            published_control_snapshot["revision"].as_u64().unwrap()
        );
        assert!(
            rebuild_snapshot["revision"].as_u64().unwrap() > initial["revision"].as_u64().unwrap()
        );
        assert_eq!(control_snapshot, published_control_snapshot);
        assert_eq!(control_snapshot, live_json_get(&presentation));

        {
            let mut coordinator = host.rebuild_coordinator.lock().unwrap();
            coordinator.test_hook = None;
            coordinator.control_before_state_lock_hook = None;
        }
        host.shutdown();
    }

    #[test]
    fn rebuild_ordering_theme_and_source_share_one_ticket_authority() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        let source_text = "::: slide\nid: theme-route\n:::\n# Theme route\n";
        fs::write(&source, source_text).unwrap();
        let mut options = test_options(source.clone());
        options.theme_switcher = true;
        options.theme_search_paths = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("themes")];
        let mut host = start_live_host(options.clone()).unwrap();
        let presentation = host.presentation_url().to_string();
        let speaker = host.speaker_url().unwrap().to_string();

        let blocker = block_rebuild_before_commit(&host.state, source_text);
        let older_url = presentation.clone();
        let older =
            thread::spawn(move || live_theme_switch_response(&older_url, "science", "light"));
        let first = blocker.observed.recv_timeout(TEST_IO_TIMEOUT).unwrap();
        assert_eq!(
            first
                .requested_theme_override
                .as_ref()
                .map(|value| value.theme.as_str()),
            Some("science")
        );
        let newer = live_theme_switch_response(&speaker, "dark-splash", "cyan");
        assert!(newer.starts_with("HTTP/1.1 200 OK"), "{newer}");
        blocker.release.send(()).unwrap();
        let older = older.join().unwrap();
        assert!(older.starts_with("HTTP/1.1 409 Conflict"), "{older}");
        assert!(older.contains("\"outcome\":\"superseded\""));
        clear_rebuild_hook(&host.state);
        {
            let state = host.state.lock().unwrap();
            assert_eq!(state.last_good_theme.as_deref(), Some("dark-splash"));
            assert_eq!(
                state.live_theme_override,
                Some(LiveThemeOverride {
                    theme: "dark-splash".to_string(),
                    variant: Some("cyan".to_string()),
                })
            );
            assert!(state.last_good_theme_css.contains("#060b0f"));
        }

        let source_from_theme =
            "::: slide\nid: source-captures-theme\n:::\n# Source captures requested Theme\n";
        fs::write(&source, source_from_theme).unwrap();
        let blocker = block_rebuild_before_commit(&host.state, source_from_theme);
        let older_url = speaker.clone();
        let older =
            thread::spawn(move || live_theme_switch_response(&older_url, "science", "dark"));
        assert_eq!(
            blocker
                .observed
                .recv_timeout(TEST_IO_TIMEOUT)
                .unwrap()
                .requested_theme_override
                .unwrap()
                .theme,
            "science"
        );
        assert!(matches!(
            rebuild(&options, &host.state, false),
            CommitOutcome::AcceptedValid
        ));
        blocker.release.send(()).unwrap();
        let older = older.join().unwrap();
        assert!(older.starts_with("HTTP/1.1 409 Conflict"), "{older}");
        clear_rebuild_hook(&host.state);
        {
            let state = host.state.lock().unwrap();
            assert_eq!(state.last_good_theme.as_deref(), Some("science"));
            assert_eq!(
                state.session.current_route().unwrap().slide_id,
                "source-captures-theme"
            );
        }

        let source_before_theme =
            "::: slide\nid: source-before-theme\n:::\n# Source before Theme\n";
        fs::write(&source, source_before_theme).unwrap();
        let blocker = block_rebuild_before_commit(&host.state, source_before_theme);
        let old_options = options.clone();
        let old_state = Arc::clone(&host.state);
        let older = thread::spawn(move || rebuild(&old_options, &old_state, false));
        blocker.observed.recv_timeout(TEST_IO_TIMEOUT).unwrap();
        let newer = live_theme_switch_response(&presentation, "dark-splash", "violet");
        assert!(newer.starts_with("HTTP/1.1 200 OK"), "{newer}");
        blocker.release.send(()).unwrap();
        assert!(matches!(older.join().unwrap(), CommitOutcome::Superseded));
        clear_rebuild_hook(&host.state);
        assert_eq!(
            host.state.lock().unwrap().live_theme_override,
            Some(LiveThemeOverride {
                theme: "dark-splash".to_string(),
                variant: Some("violet".to_string()),
            })
        );

        let before_invalid = live_json_get(&presentation);
        fs::write(&source, "---\ntheme: [\n---\n# Invalid for Theme\n").unwrap();
        let invalid = live_theme_switch_response(&speaker, "science", "light");
        assert!(
            invalid.starts_with("HTTP/1.1 422 Unprocessable Content"),
            "{invalid}"
        );
        assert!(invalid.contains("\"outcome\":\"invalid\""));
        let after_invalid = live_json_get(&speaker);
        assert_eq!(after_invalid["generation"], before_invalid["generation"]);
        assert_eq!(after_invalid["revision"], before_invalid["revision"]);
        assert_eq!(
            host.state.lock().unwrap().live_theme_override,
            Some(LiveThemeOverride {
                theme: "dark-splash".to_string(),
                variant: Some("violet".to_string()),
            })
        );

        let newest_ticket = host.rebuild_coordinator.lock().unwrap().newest_requested;
        let bad_variant = live_theme_switch_response(&presentation, "science", "not-a-variant");
        assert!(bad_variant.starts_with("HTTP/1.1 400 Bad Request"));
        assert_eq!(
            host.rebuild_coordinator.lock().unwrap().newest_requested,
            newest_ticket
        );

        host.rebuild_coordinator.lock().unwrap().closed = true;
        let shutting_down = live_theme_switch_response(&presentation, "science", "light");
        assert!(
            shutting_down.starts_with("HTTP/1.1 503 Service Unavailable"),
            "{shutting_down}"
        );
        assert!(shutting_down.contains("\"outcome\":\"shutting-down\""));

        host.shutdown();
    }

    #[test]
    fn rebuild_ordering_shutdown_discards_an_inflight_preparation() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        let source_text = "::: slide\nid: shutdown-route\n:::\n# Shutdown route\n";
        fs::write(&source, source_text).unwrap();
        let options = test_options(source);
        let mut host = start_live_host(options.clone()).unwrap();
        let presentation = host.presentation_url().to_string();
        let state = Arc::clone(&host.state);
        let before = live_json_get(&presentation);
        let blocker = block_rebuild_before_commit(&state, source_text);
        let (closed_tx, closed_rx) = mpsc::channel();
        host.rebuild_coordinator.lock().unwrap().closed_hook = Some(closed_tx);
        let rebuild_state = Arc::clone(&state);
        let request = thread::spawn(move || rebuild(&options, &rebuild_state, false));
        blocker.observed.recv_timeout(TEST_IO_TIMEOUT).unwrap();

        let shutdown = thread::spawn(move || {
            host.shutdown();
            host
        });
        closed_rx.recv_timeout(TEST_IO_TIMEOUT).unwrap();
        blocker.release.send(()).unwrap();
        assert!(matches!(
            request.join().unwrap(),
            CommitOutcome::ShuttingDown
        ));
        let host = shutdown.join().unwrap();
        let state = host.state.lock().unwrap();
        assert_eq!(snapshot_json_v1(&projected_session_v1(&state)), before);
        assert!(state.live_theme_override.is_none());
    }

    #[test]
    fn rebuild_preservation_tracks_authored_ids_and_clamps_deleted_routes_on_real_host() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        let background = temp.path().join("background.svg");
        fs::write(
            &background,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="9"></svg>"#,
        )
        .unwrap();
        fs::write(
            &source,
            r#"# First

--

::: slide
id: stable-detail
:::
# Detail

::: steps
1. Detail one
2. Detail two
:::

---

::: slide
id: stable-target
:::
# Target

::: steps
1. Target one
2. Target two
3. Target three
:::
"#,
        )
        .unwrap();
        let options = test_options(source.clone());
        let mut host = start_live_host(options.clone()).unwrap();
        let presentation = host.presentation_url().to_string();
        let speaker = host.speaker_url().unwrap().to_string();
        let navigated = live_navigate_to_slide(&presentation, &host.state, "stable-target", 2);
        assert_eq!(navigated["current"]["section"], 1);
        live_control(&speaker, r#"{"command":"timer-start"}"#);

        fs::write(
            &source,
            r#"# Inserted

---

# First

--

::: slide
id: stable-detail
:::
# Detail

::: steps
1. Detail one
2. Detail two
:::

---

::: slide
id: stable-target
:::
# Target

::: steps
1. Target one
:::
"#,
        )
        .unwrap();
        let before = live_json_get(&presentation);
        rebuild(&options, &host.state, false);
        let inserted = live_json_get(&speaker);
        assert_eq!(
            inserted["generation"].as_u64(),
            Some(before["generation"].as_u64().unwrap() + 1)
        );
        assert_eq!(
            inserted["revision"].as_u64(),
            Some(before["revision"].as_u64().unwrap() + 1)
        );
        assert_eq!(inserted["current"]["slide_id"], "stable-target");
        assert_eq!(inserted["current"]["section"], 2);
        assert_eq!(inserted["current"]["step"], 1);
        assert_eq!(inserted["timer"]["status"], "running");

        fs::write(
            &source,
            r#"---
background_image:
  src: background.svg
  splash: true
  intent: decorative
---

# Inserted

---

# First

--

::: slide
id: stable-detail
:::
# Detail

::: steps
1. Detail one
2. Detail two
:::

---

::: slide
id: stable-target
:::
# Target

::: steps
1. Target one
:::
"#,
        )
        .unwrap();
        rebuild(&options, &host.state, false);
        let splash = live_json_get(&presentation);
        assert_eq!(splash["current"]["slide_id"], "stable-target");
        assert_eq!(splash["current"]["section"], 3);
        assert_eq!(splash["current"]["step"], 1);
        assert_eq!(splash["timer"]["status"], "running");
        assert!(
            host.state
                .lock()
                .unwrap()
                .session
                .routes()
                .iter()
                .any(|route| route.slide_id == "background-image-splash")
        );

        live_navigate_to_slide(&speaker, &host.state, "stable-detail", 2);
        fs::write(
            &source,
            r#"---
background_image:
  src: background.svg
  splash: true
  intent: decorative
---

# Inserted

---

# First

--

::: slide
id: inserted-detail
:::
# Inserted detail

--

::: slide
id: stable-detail
:::
# Detail

::: steps
1. Detail one
2. Detail two
:::

---

::: slide
id: stable-target
:::
# Target
"#,
        )
        .unwrap();
        rebuild(&options, &host.state, false);
        let detail_inserted = live_json_get(&presentation);
        assert_eq!(detail_inserted["current"]["slide_id"], "stable-detail");
        assert_eq!(detail_inserted["current"]["detail"], 2);
        assert_eq!(detail_inserted["current"]["step"], 2);
        assert_eq!(detail_inserted["timer"]["status"], "running");

        fs::write(
            &source,
            r#"---
background_image:
  src: background.svg
  splash: true
  intent: decorative
---

# Inserted

---

# First

--

::: slide
id: inserted-detail
:::
# Inserted detail

---

::: slide
id: stable-target
:::
# Target
"#,
        )
        .unwrap();
        rebuild(&options, &host.state, false);
        let deleted = live_json_get(&speaker);
        assert_eq!(deleted["current"]["slide_id"], "inserted-detail");
        assert_eq!(deleted["current"]["section"], 2);
        assert_eq!(deleted["current"]["detail"], 1);
        assert_eq!(deleted["current"]["step"], 0);
        assert_eq!(deleted["timer"]["status"], "running");
        assert_session_equivalent(&deleted, &live_json_get(&presentation));

        let preserved_generation = deleted["generation"].as_u64().unwrap();
        let preserved_revision = deleted["revision"].as_u64().unwrap();
        fs::write(
            &source,
            r#"::: slide
id: duplicate-route
:::
# First

---

::: slide
id: duplicate-route
:::
# Second
"#,
        )
        .unwrap();
        rebuild(&options, &host.state, false);
        let invalid = live_json_get(&speaker);
        assert_eq!(invalid["generation"], preserved_generation);
        assert_eq!(invalid["revision"], preserved_revision);
        assert_eq!(invalid["current"], deleted["current"]);
        assert_eq!(invalid["timer"]["status"], "running");
        assert!(
            host.state
                .lock()
                .unwrap()
                .diagnostics
                .iter()
                .any(|diagnostic| {
                    diagnostic.is_fatal() && diagnostic.message.contains("duplicate slide ID")
                })
        );

        host.shutdown();
    }

    #[test]
    fn speaker_listener_avoids_the_presentation_and_busy_ports() {
        let presentation = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let presentation_port = presentation.local_addr().unwrap().port();
        let busy = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let busy_port = busy.local_addr().unwrap().port();
        let speaker = bind_live_listener_avoiding(busy_port, presentation_port).unwrap();
        assert_ne!(speaker.local_addr().unwrap().port(), presentation_port);
        assert!(speaker.local_addr().unwrap().port() > busy_port);
    }

    fn write_live_theme(root: &Path, color: &str) {
        fs::create_dir_all(root).unwrap();
        fs::write(
            root.join("theme.toml"),
            r#"[theme]
name = "live-theme"
version = "0.1.0"
api_version = 1
output_targets = ["html", "pdf"]
"#,
        )
        .unwrap();
        fs::write(
            root.join("theme.css.tmpl"),
            format!(
                ".zpres-slide {{ color: {color}; }}\n.zpres-slide-canvas {{}}\n.zpres-block {{}}\n"
            ),
        )
        .unwrap();
        fs::write(
            root.join("print.css.tmpl"),
            format!(
                ".zpres-print-slide {{}}\n.zpres-slide {{ color: {color}; }}\n.zpres-slide-canvas {{}}\n.zpres-block {{}}\n"
            ),
        )
        .unwrap();
    }

    fn write_live_theme_v1(root: &Path, color: &str) {
        fs::create_dir_all(root).unwrap();
        fs::write(
            root.join("theme.toml"),
            r#"[theme]
name = "live-theme"
version = "0.1.0"
api_version = 1
stylesheet = "theme.css.tmpl"
print_stylesheet = "print.css.tmpl"
output_targets = ["html", "pdf"]
"#,
        )
        .unwrap();
        fs::write(
            root.join("theme.css.tmpl"),
            format!(
                ".zpres-slide {{ color: {color}; }}\n.zpres-slide-canvas {{}}\n.zpres-block {{}}\n"
            ),
        )
        .unwrap();
        fs::write(
            root.join("print.css.tmpl"),
            format!(
                ".zpres-print-slide {{}}\n.zpres-slide {{ color: {color}; }}\n.zpres-slide-canvas {{}}\n.zpres-block {{}}\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn textual_theme_fallback_uses_shared_front_matter_boundaries() {
        assert_eq!(
            extract_textual_front_matter_theme(
                "\u{feff}---\r\ntheme: \"local-theme\"\r\ninvalid: [\r\n---\r\n# Talk\r\n"
            )
            .as_deref(),
            Some("local-theme")
        );
        assert_eq!(
            extract_textual_front_matter_theme(
                "\u{feff}---\r\ntheme: \"local-theme\"\r\n# Missing close\r\n"
            ),
            None
        );
    }

    #[test]
    fn extracts_local_dependency_paths_from_source() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        fs::write(
            &source,
            r#"---
background_image:
  src: "assets/frontmatter-background.svg"
---

::: background src="assets/slide-background.svg"
:::
::: figure src="assets/plot.svg"
:::
::: vega-lite data="data/runtime.csv"
:::
::: vega-lite
{ "data": { "url": "data/from-spec.csv" }, "mark": "line" }
:::
::: figure src="https://example.com/plot.svg"
:::
![width=70% alt="Markdown figure"](assets/markdown.svg)
"#,
        )
        .unwrap();

        let paths = extract_local_dependency_paths(&source);

        assert_eq!(
            paths,
            vec![
                temp.path().join("assets/frontmatter-background.svg"),
                temp.path().join("assets/markdown.svg"),
                temp.path().join("assets/plot.svg"),
                temp.path().join("assets/slide-background.svg"),
                temp.path().join("data/from-spec.csv"),
                temp.path().join("data/runtime.csv"),
            ]
        );
    }

    #[test]
    fn extracts_textual_dependency_paths_when_source_cannot_parse() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        fs::write(
            &source,
            r#"---
theme: [
---

::: figure src="assets/plot.svg"
:::
"#,
        )
        .unwrap();

        let paths = extract_local_dependency_paths(&source);

        assert_eq!(paths, vec![temp.path().join("assets/plot.svg")]);
    }

    #[test]
    fn watch_snapshot_includes_deck_root_theme_package_files() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        let nested_theme = temp.path().join("themes").join("local-theme");
        let peer_theme = temp.path().join("local-theme");
        fs::create_dir_all(&nested_theme).unwrap();
        fs::create_dir_all(&peer_theme).unwrap();
        fs::write(
            &source,
            r##"---
theme: "local-theme"
---

# Local theme
"##,
        )
        .unwrap();
        fs::write(nested_theme.join("theme.toml"), "nested").unwrap();
        fs::write(nested_theme.join("theme.css.tmpl"), "nested css").unwrap();
        fs::create_dir_all(nested_theme.join("fonts")).unwrap();
        fs::write(nested_theme.join("fonts/body.woff2"), "font").unwrap();
        fs::write(peer_theme.join("theme.toml"), "peer").unwrap();
        fs::write(peer_theme.join("print.css.tmpl"), "peer css").unwrap();

        let snapshot = watch_snapshot(&source, &[]);
        let paths = snapshot
            .into_iter()
            .map(|stamp| stamp.path)
            .collect::<Vec<_>>();

        assert!(paths.contains(&nested_theme.join("theme.toml")));
        assert!(paths.contains(&nested_theme.join("theme.css.tmpl")));
        assert!(paths.contains(&nested_theme.join("fonts/body.woff2")));
        assert!(paths.contains(&peer_theme.join("theme.toml")));
        assert!(paths.contains(&peer_theme.join("print.css.tmpl")));
    }

    #[test]
    fn watch_snapshot_falls_back_to_deck_themes_directory_without_theme_name() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        let theme_dir = temp.path().join("themes").join("draft-theme");
        fs::create_dir_all(&theme_dir).unwrap();
        fs::write(&source, "# Unthemed\n").unwrap();
        fs::write(theme_dir.join("theme.toml"), "draft").unwrap();

        let snapshot = watch_snapshot(&source, &[]);
        let paths = snapshot
            .into_iter()
            .map(|stamp| stamp.path)
            .collect::<Vec<_>>();

        assert!(paths.contains(&theme_dir.join("theme.toml")));
    }

    #[test]
    fn live_rebuild_allows_declared_theme_fonts_and_assets_only() {
        use std::io::Read as _;
        use std::net::Shutdown;

        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        let theme_dir = temp.path().join("themes").join("live-theme");
        write_live_theme(&theme_dir, "red");
        fs::create_dir_all(theme_dir.join("fonts")).unwrap();
        fs::create_dir_all(theme_dir.join("textures")).unwrap();
        let font_path = "fonts/body # % ü.woff2";
        fs::write(theme_dir.join(font_path), b"font bytes").unwrap();
        fs::write(theme_dir.join("textures/paper.png"), b"paper bytes").unwrap();
        fs::write(theme_dir.join("textures/undeclared.png"), b"private").unwrap();
        fs::write(
            theme_dir.join("theme.toml"),
            r#"[theme]
name = "live-theme"
version = "0.1.0"
api_version = 1
fonts = ["fonts/body # % ü.woff2"]
assets = ["textures/paper.png"]
output_targets = ["html", "pdf"]
"#,
        )
        .unwrap();
        fs::write(
            &source,
            r##"---
theme: "live-theme"
---

# Live dependencies
"##,
        )
        .unwrap();
        let state = Arc::new(Mutex::new(ServerState::new(temp.path().to_path_buf())));
        let options = ServeOptions {
            source,
            theme_search_paths: Vec::new(),
            fallback_theme: None,
            fallback_theme_params: BTreeMap::new(),
            cli_theme_override: None,
            cli_theme_params: BTreeMap::new(),
            port: 0,
            speaker_port: None,
            speaker_enabled: false,
            open: false,
            debounce: Duration::from_millis(1),
            theme_switcher: false,
        };

        rebuild(&options, &state, true);

        assert_eq!(
            state.lock().unwrap().last_good_theme_dependencies,
            vec![font_path, "textures/paper.png"]
        );
        assert_eq!(
            read_theme_asset(&format!("assets/{font_path}"), &state),
            Some(("font/woff2", b"font bytes".to_vec()))
        );
        assert_eq!(
            read_theme_asset("assets/textures/paper.png", &state),
            Some(("image/png", b"paper bytes".to_vec()))
        );
        assert_eq!(
            read_theme_asset("assets/textures/undeclared.png", &state),
            None
        );

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server_state = Arc::clone(&state);
        let server_options = Arc::new(options);
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_connection(stream, ViewRole::Presentation, server_options, server_state)
                .unwrap();
        });
        let mut client = TcpStream::connect(address).unwrap();
        client
            .write_all(
                b"GET /assets/fonts/body%20%23%20%25%20%C3%BC.woff2?cache=one+two HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        server.join().unwrap();

        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("Content-Type: font/woff2\r\n"));
        assert!(response.ends_with("font bytes"));
    }

    #[test]
    fn dependency_content_types_cover_theme_css_and_fonts() {
        for (name, expected) in [
            ("theme.css", "text/css; charset=utf-8"),
            ("THEME.CSS", "text/css; charset=utf-8"),
            ("body.woff", "font/woff"),
            ("body.woff2", "font/woff2"),
            ("body.WoFf2", "font/woff2"),
            ("body.ttf", "font/ttf"),
            ("body.otf", "font/otf"),
            ("paper.SVG", "image/svg+xml"),
        ] {
            assert_eq!(content_type_for_path(Path::new(name)), expected);
        }
        assert_eq!(
            percent_decode_path("assets/a+b.woff2").as_deref(),
            Some("assets/a+b.woff2")
        );
        assert_eq!(percent_decode("one+two").as_deref(), Some("one two"));
        let traversal = percent_decode_path("assets/%2e%2e/secret.woff2").unwrap();
        assert!(!is_safe_local_asset_path(
            traversal.strip_prefix("assets/").unwrap()
        ));
    }

    #[cfg(unix)]
    #[test]
    fn deck_assets_allow_normal_files_but_reject_symlink_escape() {
        use std::os::unix::fs::symlink;

        let deck = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let assets = deck.path().join("assets");
        fs::create_dir(&assets).unwrap();
        fs::write(assets.join("inside.txt"), b"inside").unwrap();
        fs::write(outside.path().join("secret.txt"), b"outside").unwrap();
        symlink(outside.path().join("secret.txt"), assets.join("escape.txt")).unwrap();

        let state = Arc::new(Mutex::new(ServerState::new(deck.path().to_path_buf())));
        assert_eq!(
            read_deck_asset("assets/inside.txt", &state),
            Some(("application/octet-stream", b"inside".to_vec()))
        );
        assert_eq!(read_deck_asset("assets/escape.txt", &state), None);
    }

    fn runtime_test_options(source: PathBuf) -> ServeOptions {
        ServeOptions {
            source,
            theme_search_paths: Vec::new(),
            fallback_theme: None,
            fallback_theme_params: BTreeMap::new(),
            cli_theme_override: Some("live-theme".to_string()),
            cli_theme_params: BTreeMap::new(),
            port: 0,
            speaker_port: None,
            speaker_enabled: true,
            open: false,
            debounce: Duration::from_millis(1),
            theme_switcher: true,
        }
    }

    #[test]
    fn served_runtime_matches_publication_after_theme_api_switch() {
        use std::io::Read as _;
        use std::net::Shutdown;

        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        fs::write(&source, "---\ntheme: debug\n---\n# Runtime parity\n").unwrap();
        write_live_theme(&temp.path().join("live-theme"), "red");
        let options = runtime_test_options(source.clone());
        let state = Arc::new(Mutex::new(ServerState::new(temp.path().to_path_buf())));
        for (index, selection) in [None, Some("debug"), Some("live-theme")]
            .into_iter()
            .enumerate()
        {
            state.lock().unwrap().live_theme_override = selection.map(|name| LiveThemeOverride {
                theme: name.to_string(),
                variant: None,
            });
            rebuild(&options, &state, index == 0);
            let mut deck = deck::parse_source_file(&source).unwrap();
            let selected = state.lock().unwrap().live_theme_override.clone();
            let theme = resolve_live_theme(&mut deck, &options, selected.as_ref()).unwrap();
            let publication = html::write_debug_html_bundle(
                &deck,
                &theme,
                &temp.path().join(format!("out-{index}")),
            )
            .unwrap();
            let published =
                fs::read_to_string(publication.generation_path.join("assets/reveal.js")).unwrap();

            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let address = listener.local_addr().unwrap();
            let server_state = Arc::clone(&state);
            let server_options = Arc::new(options.clone());
            let server = thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                handle_connection(stream, ViewRole::Presentation, server_options, server_state)
                    .unwrap();
            });
            let mut client = TcpStream::connect(address).unwrap();
            client
                .write_all(b"GET /assets/reveal.js HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .unwrap();
            client.shutdown(Shutdown::Write).unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            server.join().unwrap();
            let (_, served) = response.split_once("\r\n\r\n").unwrap();
            assert_eq!(
                served, published,
                "runtime drift after selection {selection:?}"
            );
        }
    }

    #[test]
    fn watcher_tracks_cli_and_live_selected_local_themes_through_invalid_edits() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        fs::write(&source, "---\ntheme: debug\n---\n# Selected package\n").unwrap();
        let cli_root = temp.path().join("live-theme");
        write_live_theme(&cli_root, "red");
        let other_root = temp.path().join("live-other");
        write_live_theme(&other_root, "blue");
        let manifest = fs::read_to_string(other_root.join("theme.toml"))
            .unwrap()
            .replace("live-theme", "live-other");
        fs::write(other_root.join("theme.toml"), manifest).unwrap();
        let options = runtime_test_options(source.clone());
        let state = Arc::new(Mutex::new(ServerState::new(temp.path().to_path_buf())));
        rebuild(&options, &state, true);
        let before = live_watch_snapshot(&options, &state);
        fs::write(
            cli_root.join("theme.css.tmpl"),
            ".zpres-slide { color: green; }",
        )
        .unwrap();
        assert_ne!(before, live_watch_snapshot(&options, &state));

        state.lock().unwrap().live_theme_override = Some(LiveThemeOverride {
            theme: "live-other".to_string(),
            variant: None,
        });
        rebuild(&options, &state, false);
        let before = live_watch_snapshot(&options, &state);
        fs::write(other_root.join("theme.toml"), "invalid [").unwrap();
        rebuild(&options, &state, false);
        let invalid = live_watch_snapshot(&options, &state);
        assert_ne!(before, invalid);
        fs::write(&source, "---\ninvalid: [\n---\n").unwrap();
        fs::write(
            other_root.join("theme.css.tmpl"),
            ".zpres-slide { color: purple; }",
        )
        .unwrap();
        assert_ne!(invalid, live_watch_snapshot(&options, &state));
        assert!(
            live_watch_snapshot(&options, &state)
                .iter()
                .any(|stamp| stamp.path == other_root.join("theme.css.tmpl"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn deck_assets_allow_internal_symlinks_but_reject_external_targets() {
        use std::os::unix::fs::symlink;
        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(root.path().join("inside.txt"), "inside").unwrap();
        fs::write(outside.path().join("outside.txt"), "outside").unwrap();
        symlink(
            root.path().join("inside.txt"),
            root.path().join("internal.txt"),
        )
        .unwrap();
        symlink(
            outside.path().join("outside.txt"),
            root.path().join("external.txt"),
        )
        .unwrap();
        symlink(outside.path(), root.path().join("external-dir")).unwrap();
        let state = Arc::new(Mutex::new(ServerState::new(root.path().to_path_buf())));
        assert_eq!(
            read_deck_asset("internal.txt", &state).unwrap().1,
            b"inside"
        );
        assert!(read_deck_asset("external.txt", &state).is_none());
        assert!(read_deck_asset("external-dir/outside.txt", &state).is_none());
    }

    #[test]
    fn v1_runtime_preserves_control_keys_and_citation_routes() {
        use crate::chromium::{
            ChromiumSession, ChromiumSessionOptions, discover_chromium_executable,
        };
        if discover_chromium_executable().executable.is_none() {
            return;
        }
        let temp = tempdir().unwrap();
        let path = temp.path().join("runtime.html");
        let document = format!(
            r##"<!doctype html><body>
<main class="reveal"><div class="slides"><section class="zpres-section-stack">
<section class="zpres-slide" data-slide-id="main" data-slide-role="main">
<a id="link" href="#note" data-zpres-footnote-target="note"><span id="link-child">Citation</span></a>
<video id="video" controls></video><p id="note">Source note</p>
<p class="zpres-step fragment" data-step-index="1">First Step</p>
</section></section></div></main><script>{}</script></body>"##,
            html::theme_runtime_js(theme::ThemeApiVersion::V1)
        );
        fs::write(&path, document).unwrap();
        let mut browser = ChromiumSession::launch(ChromiumSessionOptions::default()).unwrap();
        browser
            .load_screen_document(
                &crate::file_url::file_url(&path),
                crate::pdf::PngViewport::default(),
            )
            .unwrap();
        let result = browser.evaluate_for_test(r#"(async () => {
          await window.zpresPresentation.ready;
          const dispatch = (target) => {
            const event = new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true });
            target.dispatchEvent(event);
            return event.defaultPrevented;
          };
          const linkHandled = dispatch(document.getElementById('link-child'));
          const videoHandled = dispatch(document.getElementById('video'));
          const controlStep = window.zpresPresentation.current().step;
          const bodyHandled = dispatch(document.body);
          await window.zpresPresentation.navigate({ section: 0, detail: 0, step: 1 });
          const before = location.hash;
          document.getElementById('link-child').click();
          await new Promise(resolve => setTimeout(resolve, 0));
          return { linkHandled, videoHandled, controlStep, bodyHandled,
            current: document.querySelector('[data-step-index]').getAttribute('aria-current'),
            routePreserved: location.hash === before,
            citationFocused: document.activeElement.id === 'note',
            step: window.zpresPresentation.current().step };
        })()"#).unwrap();
        assert_eq!(
            result,
            json!({
                "linkHandled": false, "videoHandled": false, "controlStep": 0,
                "bodyHandled": true, "current": "step", "routePreserved": true,
                "citationFocused": true, "step": 1,
            })
        );
    }

    #[test]
    fn live_client_is_injected_into_html() {
        let html = inject_live_client("<html><body>deck</body></html>", "http://127.0.0.1:3000");

        assert!(html.contains("EventSource(\"/events\")"));
        assert!(html.contains("theme-css"));
        assert!(html.contains("assets"));
        assert!(html.contains("zpres_live"));
        assert!(html.contains("assets/theme.css?v="));
        assert!(html.contains("/zpres/control"));
        assert!(html.contains("addEventListener(\"session\""));
        assert!(!html.contains("Open speaker view"));
        assert!(!html.contains("zpres-live-view-link"));
        assert!(html.contains("deck"));
    }

    #[test]
    fn theme_switcher_is_injected_when_enabled() {
        let html = decorate_live_html(
            "<html><body>deck</body></html>",
            true,
            "http://127.0.0.1:3000",
        );

        assert!(html.contains("zpres-theme-switcher"));
        assert!(html.contains("/zpres/theme-switcher.json"));
        assert!(html.contains("fetch(\"/zpres/theme-switch\""));
        assert!(html.contains("method: \"POST\""));
        assert!(html.contains("Content-Type\": \"application/json"));
        assert!(!html.contains("/zpres/theme-switch?"));
        assert!(html.contains("zpres-live-hash:"));
        assert!(html.contains("deck"));
    }

    #[test]
    fn theme_switch_request_security_requires_guarded_json_post() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        fs::write(&source, "# Theme security\n").unwrap();
        let options = ServeOptions {
            source,
            theme_search_paths: vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("themes")],
            fallback_theme: None,
            fallback_theme_params: BTreeMap::new(),
            cli_theme_override: None,
            cli_theme_params: BTreeMap::new(),
            port: 0,
            speaker_port: None,
            speaker_enabled: true,
            open: false,
            debounce: Duration::from_millis(1),
            theme_switcher: true,
        };
        let state = Arc::new(Mutex::new(ServerState::new_for_options(
            &options,
            temp.path().to_path_buf(),
            "http://127.0.0.1:3000".to_string(),
            Some("http://127.0.0.1:3001".to_string()),
        )));
        let body = r#"{"theme":"dark-splash","variant":"cyan"}"#;
        let request = format!(
            "POST /zpres/theme-switch HTTP/1.1\r\nHost: localhost\r\nOrigin: http://127.0.0.1:3001\r\nSec-Fetch-Site: same-origin\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let response = request_once(
            request.as_bytes(),
            ViewRole::Speaker,
            options.clone(),
            Arc::clone(&state),
        );
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        let accepted = state.lock().unwrap().live_theme_override.clone();
        assert_eq!(
            accepted,
            Some(LiveThemeOverride {
                theme: "dark-splash".to_string(),
                variant: Some("cyan".to_string()),
            })
        );

        let response = request_once(
            b"GET /zpres/theme-switch?theme=science HTTP/1.1\r\nHost: localhost\r\n\r\n",
            ViewRole::Presentation,
            options.clone(),
            Arc::clone(&state),
        );
        assert!(response.starts_with("HTTP/1.1 405 Method Not Allowed"));
        assert!(response.contains("Allow: POST"));
        assert_eq!(state.lock().unwrap().live_theme_override, accepted);

        let body = r#"{"theme":"science","variant":"defaults"}"#;
        let request = format!(
            "POST /zpres/theme-switch HTTP/1.1\r\nHost: localhost\r\nOrigin: http://127.0.0.1:3999\r\nSec-Fetch-Site: cross-site\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let response = request_once(
            request.as_bytes(),
            ViewRole::Presentation,
            options,
            Arc::clone(&state),
        );
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        assert!(!response.contains("Access-Control-Allow-Origin"));
        assert_eq!(state.lock().unwrap().live_theme_override, accepted);
    }

    #[test]
    fn theme_switcher_catalog_discovers_built_in_variants() {
        let catalog = discover_theme_switcher_catalog(
            Path::new(env!("CARGO_MANIFEST_DIR")),
            &[PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("themes")],
        );
        let science = catalog
            .iter()
            .find(|theme| theme.name == "science")
            .expect("science theme should be discoverable");
        let dark_splash = catalog
            .iter()
            .find(|theme| theme.name == "dark-splash")
            .expect("dark-splash theme should be discoverable");

        assert_eq!(science.palette_parameter, "mode");
        assert!(science.variants.contains(&"light".to_string()));
        assert!(science.variants.contains(&"dark".to_string()));
        assert_eq!(dark_splash.palette_parameter, "variant");
        assert!(dark_splash.variants.contains(&"cyan".to_string()));
        assert!(dark_splash.variants.contains(&"violet".to_string()));
    }

    #[test]
    fn live_server_auto_increments_when_requested_port_is_busy() {
        let occupied = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let occupied_port = occupied.local_addr().unwrap().port();

        let listener = bind_live_listener(occupied_port).unwrap();
        let bound_port = listener.local_addr().unwrap().port();

        assert!(bound_port > occupied_port);
    }

    #[test]
    fn live_rebuild_uses_theme_switcher_override() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        fs::write(
            &source,
            r##"---
theme: "science"
theme_params:
  mode: "light"
---

# Switch me
"##,
        )
        .unwrap();
        let options = ServeOptions {
            source,
            theme_search_paths: vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("themes")],
            fallback_theme: None,
            fallback_theme_params: BTreeMap::new(),
            cli_theme_override: None,
            cli_theme_params: BTreeMap::new(),
            port: 0,
            speaker_port: None,
            speaker_enabled: false,
            open: false,
            debounce: Duration::from_millis(1),
            theme_switcher: true,
        };
        let state = Arc::new(Mutex::new(ServerState::new_for_options(
            &options,
            temp.path().to_path_buf(),
            "http://127.0.0.1:3000".to_string(),
            Some("http://127.0.0.1:3001".to_string()),
        )));
        state.lock().unwrap().live_theme_override = Some(LiveThemeOverride {
            theme: "dark-splash".to_string(),
            variant: Some("cyan".to_string()),
        });

        rebuild(&options, &state, true);

        let state = state.lock().unwrap();
        assert_eq!(state.last_good_theme.as_deref(), Some("dark-splash"));
        assert!(state.last_good_html.contains("zpres-theme-switcher"));
        assert!(state.last_good_html.contains("zpres-theme-dark-splash"));
        assert!(
            state
                .last_good_theme_css
                .contains("--zpres-color-background: #060b0f")
        );
    }

    #[test]
    fn live_rebuild_sends_theme_css_event_when_only_theme_css_changes() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        let theme_dir = temp.path().join("themes").join("live-theme");
        write_live_theme(&theme_dir, "red");
        fs::write(
            &source,
            r##"---
theme: "live-theme"
---

# Live CSS
"##,
        )
        .unwrap();
        let state = Arc::new(Mutex::new(ServerState::new(temp.path().to_path_buf())));
        let options = ServeOptions {
            source,
            theme_search_paths: Vec::new(),
            fallback_theme: None,
            fallback_theme_params: BTreeMap::new(),
            cli_theme_override: None,
            cli_theme_params: BTreeMap::new(),
            port: 0,
            speaker_port: None,
            speaker_enabled: false,
            open: false,
            debounce: Duration::from_millis(1),
            theme_switcher: false,
        };

        rebuild(&options, &state, true);
        let mailbox = register_test_mailbox(&state);
        write_live_theme(&theme_dir, "blue");

        rebuild(&options, &state, false);

        let events = drain_test_mailbox(&mailbox);
        assert!(matches!(events[0], SseEvent::Diagnostics { .. }));
        assert!(matches!(events[1], SseEvent::Session { .. }));
        assert!(matches!(events[2], SseEvent::ThemeCss { .. }));
        assert!(
            events[2]
                .wire_data()
                .contains("assets/theme.css?zpres_live=")
        );
    }

    #[test]
    fn live_rebuild_sends_reload_event_when_deck_html_changes() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        let theme_dir = temp.path().join("themes").join("live-theme");
        write_live_theme(&theme_dir, "red");
        fs::write(
            &source,
            r##"---
theme: "live-theme"
---

# First
"##,
        )
        .unwrap();
        let state = Arc::new(Mutex::new(ServerState::new(temp.path().to_path_buf())));
        let options = ServeOptions {
            source: source.clone(),
            theme_search_paths: Vec::new(),
            fallback_theme: None,
            fallback_theme_params: BTreeMap::new(),
            cli_theme_override: None,
            cli_theme_params: BTreeMap::new(),
            port: 0,
            speaker_port: None,
            speaker_enabled: false,
            open: false,
            debounce: Duration::from_millis(1),
            theme_switcher: false,
        };

        rebuild(&options, &state, true);
        let mailbox = register_test_mailbox(&state);
        fs::write(
            &source,
            r##"---
theme: "live-theme"
---

# Second
"##,
        )
        .unwrap();

        rebuild(&options, &state, false);

        let events = drain_test_mailbox(&mailbox);
        assert!(matches!(events[0], SseEvent::Diagnostics { .. }));
        assert!(matches!(events[1], SseEvent::Session { .. }));
        assert!(matches!(events[2], SseEvent::Reload { .. }));
    }

    #[test]
    fn live_rebuild_sends_assets_event_when_only_local_assets_change() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        let theme_dir = temp.path().join("themes").join("live-theme");
        let asset_dir = temp.path().join("assets");
        fs::create_dir_all(&asset_dir).unwrap();
        fs::write(asset_dir.join("plot.svg"), "<svg></svg>").unwrap();
        write_live_theme(&theme_dir, "red");
        fs::write(
            &source,
            r##"---
theme: "live-theme"
---

# Asset update

![Plot](assets/plot.svg)
"##,
        )
        .unwrap();
        let state = Arc::new(Mutex::new(ServerState::new(temp.path().to_path_buf())));
        let options = ServeOptions {
            source,
            theme_search_paths: Vec::new(),
            fallback_theme: None,
            fallback_theme_params: BTreeMap::new(),
            cli_theme_override: None,
            cli_theme_params: BTreeMap::new(),
            port: 0,
            speaker_port: None,
            speaker_enabled: false,
            open: false,
            debounce: Duration::from_millis(1),
            theme_switcher: false,
        };

        rebuild(&options, &state, true);
        let mailbox = register_test_mailbox(&state);
        fs::write(asset_dir.join("plot.svg"), "<svg><path /></svg>").unwrap();

        rebuild(&options, &state, false);

        let events = drain_test_mailbox(&mailbox);
        assert!(matches!(events[0], SseEvent::Diagnostics { .. }));
        assert!(matches!(events[1], SseEvent::Session { .. }));
        assert!(matches!(events[2], SseEvent::Assets { .. }));
        assert!(
            events[2]
                .wire_data()
                .contains("assets/theme.css?zpres_live=")
        );
        assert!(events[2].wire_data().contains("version"));
    }

    #[test]
    fn live_rebuild_can_switch_themes_from_front_matter() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        fs::write(
            &source,
            r##"---
theme: "science"
---

# Science
"##,
        )
        .unwrap();
        let state = Arc::new(Mutex::new(ServerState::new(temp.path().to_path_buf())));
        let options = ServeOptions {
            source: source.clone(),
            theme_search_paths: vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("themes")],
            fallback_theme: None,
            fallback_theme_params: BTreeMap::new(),
            cli_theme_override: None,
            cli_theme_params: BTreeMap::new(),
            port: 0,
            speaker_port: None,
            speaker_enabled: false,
            open: false,
            debounce: Duration::from_millis(1),
            theme_switcher: false,
        };

        rebuild(&options, &state, true);
        {
            let state = state.lock().unwrap();
            assert_eq!(state.last_good_theme.as_deref(), Some("science"));
            assert!(state.last_good_html.contains("zpres-theme-science"));
        }

        fs::write(
            &source,
            r##"---
theme: "dark-splash"
theme_params:
  variant: "cyan"
---

# Dark splash
"##,
        )
        .unwrap();

        rebuild(&options, &state, false);
        let state = state.lock().unwrap();
        assert_eq!(state.last_good_theme.as_deref(), Some("dark-splash"));
        assert!(state.last_good_html.contains("zpres-theme-dark-splash"));
        assert!(
            state
                .last_good_theme_css
                .contains("--zpres-color-background: #060b0f")
        );
    }

    #[test]
    fn live_rebuild_keeps_cli_theme_override_pinned() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        fs::write(
            &source,
            r##"---
theme: "dark-splash"
theme_params:
  variant: "cyan"
---

# Forced Science
"##,
        )
        .unwrap();
        let state = Arc::new(Mutex::new(ServerState::new(temp.path().to_path_buf())));
        let options = ServeOptions {
            source,
            theme_search_paths: vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("themes")],
            fallback_theme: None,
            fallback_theme_params: BTreeMap::new(),
            cli_theme_override: Some("science".to_string()),
            cli_theme_params: BTreeMap::new(),
            port: 0,
            speaker_port: None,
            speaker_enabled: false,
            open: false,
            debounce: Duration::from_millis(1),
            theme_switcher: false,
        };

        rebuild(&options, &state, true);

        let state = state.lock().unwrap();
        assert_eq!(state.last_good_theme.as_deref(), Some("science"));
        assert!(state.last_good_html.contains("zpres-theme-science"));
    }

    #[test]
    fn live_rebuild_does_not_carry_old_theme_params_when_cli_switches_theme() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        fs::write(
            &source,
            r##"---
theme: "science"
theme_params:
  accent: "#0f766e"
  mode: "light"
---

# Forced Dark Splash
"##,
        )
        .unwrap();
        let state = Arc::new(Mutex::new(ServerState::new(temp.path().to_path_buf())));
        let options = ServeOptions {
            source,
            theme_search_paths: vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("themes")],
            fallback_theme: Some("dark-splash".to_string()),
            fallback_theme_params: BTreeMap::new(),
            cli_theme_override: Some("dark-splash".to_string()),
            cli_theme_params: BTreeMap::new(),
            port: 0,
            speaker_port: None,
            speaker_enabled: false,
            open: false,
            debounce: Duration::from_millis(1),
            theme_switcher: false,
        };

        rebuild(&options, &state, true);

        let state = state.lock().unwrap();
        assert_eq!(state.last_good_theme.as_deref(), Some("dark-splash"));
        assert!(
            state
                .last_good_theme_css
                .contains("--zpres-color-accent: #9b6cff")
        );
        assert!(!state.last_good_theme_css.contains("#0f766e"));
    }

    #[test]
    fn live_rebuild_reports_discarded_theme_params_as_warnings() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("talk.zp.md");
        fs::write(
            &source,
            r##"---
theme: "science"
theme_params:
  accent: "teal"
  unknown: "value"
---

# Theme diagnostics
"##,
        )
        .unwrap();
        let state = Arc::new(Mutex::new(ServerState::new(temp.path().to_path_buf())));
        let options = ServeOptions {
            source,
            theme_search_paths: vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("themes")],
            fallback_theme: None,
            fallback_theme_params: BTreeMap::new(),
            cli_theme_override: None,
            cli_theme_params: BTreeMap::new(),
            port: 0,
            speaker_port: None,
            speaker_enabled: false,
            open: false,
            debounce: Duration::from_millis(1),
            theme_switcher: false,
        };

        rebuild(&options, &state, true);

        let state = state.lock().unwrap();
        assert_eq!(state.last_good_theme.as_deref(), Some("science"));
        assert!(
            state
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("accent"))
        );
        assert!(
            state
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("unknown"))
        );
    }
}
