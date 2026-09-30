use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::json;
use thiserror::Error;

use crate::deck::{self, Diagnostic};
use crate::html;
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
    let listener = bind_live_listener(options.port)?;
    let address = listener.local_addr()?;
    let bound_port = address.port();
    if options.port != 0 && bound_port != options.port {
        println!(
            "Port {} is busy; using next available port {}.",
            options.port, bound_port
        );
    }

    let deck_root = options
        .source
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let state = Arc::new(Mutex::new(ServerState::new_for_options(
        &options, deck_root,
    )));
    rebuild(&options, &state, true);

    let server_state = Arc::clone(&state);
    let server_options = Arc::new(options.clone());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                continue;
            };
            let state = Arc::clone(&server_state);
            let options = Arc::clone(&server_options);
            thread::spawn(move || {
                let _ = handle_connection(stream, options, state);
            });
        }
    });

    println!("Serving {} at http://{}", options.source.display(), address);
    println!("Press Ctrl-C to stop.");

    let mut snapshot = live_watch_snapshot(&options, &state);
    loop {
        thread::sleep(options.debounce);
        let next_snapshot = live_watch_snapshot(&options, &state);
        if next_snapshot != snapshot {
            snapshot = next_snapshot;
            // Let editors finish atomic writes before parsing the source file.
            thread::sleep(options.debounce);
            rebuild(&options, &state, false);
        }
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

#[derive(Debug)]
struct ServerState {
    last_good_html: String,
    last_good_theme: Option<String>,
    last_good_theme_api: theme::ThemeApiVersion,
    last_good_theme_params: BTreeMap<String, String>,
    last_good_palette_parameter: String,
    last_good_theme_css: String,
    last_good_theme_dependency_root: Option<PathBuf>,
    last_good_theme_dependencies: Vec<String>,
    diagnostics: Vec<Diagnostic>,
    clients: Vec<Sender<SseEvent>>,
    deck_root: PathBuf,
    theme_switcher_enabled: bool,
    theme_switcher_catalog: Vec<ThemeSwitcherTheme>,
    live_theme_override: Option<LiveThemeOverride>,
}

impl ServerState {
    #[cfg(test)]
    fn new(deck_root: PathBuf) -> Self {
        Self {
            last_good_html: decorate_live_html(&initial_html(), false),
            last_good_theme: None,
            last_good_theme_api: theme::ThemeApiVersion::V1,
            last_good_theme_params: BTreeMap::new(),
            last_good_palette_parameter: "variant".to_string(),
            last_good_theme_css: String::new(),
            last_good_theme_dependency_root: None,
            last_good_theme_dependencies: Vec::new(),
            diagnostics: Vec::new(),
            clients: Vec::new(),
            theme_switcher_enabled: false,
            theme_switcher_catalog: Vec::new(),
            live_theme_override: None,
            deck_root,
        }
    }

    fn new_for_options(options: &ServeOptions, deck_root: PathBuf) -> Self {
        Self {
            last_good_html: decorate_live_html(&initial_html(), options.theme_switcher),
            last_good_theme: None,
            last_good_theme_api: theme::ThemeApiVersion::V1,
            last_good_theme_params: BTreeMap::new(),
            last_good_palette_parameter: "variant".to_string(),
            last_good_theme_css: String::new(),
            last_good_theme_dependency_root: None,
            last_good_theme_dependencies: Vec::new(),
            diagnostics: Vec::new(),
            clients: Vec::new(),
            theme_switcher_enabled: options.theme_switcher,
            theme_switcher_catalog: discover_theme_switcher_catalog(
                &deck_root,
                &options.theme_search_paths,
            ),
            live_theme_override: None,
            deck_root,
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

#[derive(Debug, Clone)]
struct SseEvent {
    event: String,
    data: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveUpdate {
    Reload,
    ThemeCss,
    Assets,
}

fn rebuild(options: &ServeOptions, state: &Arc<Mutex<ServerState>>, initial: bool) {
    match deck::parse_source_file(&options.source) {
        Ok(mut deck) => {
            let live_theme_override = {
                let state = state.lock().expect("server state poisoned");
                state.live_theme_override.clone()
            };
            let rendered_theme =
                match resolve_live_theme(&mut deck, options, live_theme_override.as_ref()) {
                    Ok(rendered_theme) => rendered_theme,
                    Err(error) => {
                        let diagnostics = vec![Diagnostic::error(None, error.to_string())];
                        eprintln!("warning: could not resolve live theme: {error}");
                        update_diagnostics(state, diagnostics);
                        return;
                    }
                };
            print_diagnostics(&deck.diagnostics);
            if deck.diagnostics.iter().any(Diagnostic::is_fatal) {
                let diagnostics = deck.diagnostics.clone();
                println!(
                    "Current source has {} fatal diagnostic(s); keeping last good deck.",
                    diagnostics
                        .iter()
                        .filter(|diagnostic| diagnostic.is_fatal())
                        .count()
                );
                update_diagnostics(state, diagnostics);
                return;
            }

            let html = decorate_live_html(
                &html::render_debug_html(&deck, &rendered_theme),
                options.theme_switcher,
            );
            let theme = deck.metadata.theme.clone();
            let theme_params = rendered_theme.params.clone();
            let palette_parameter = rendered_theme.manifest.palette_parameter.clone();
            let theme_dependency_root =
                rendered_theme.manifest.path.parent().map(Path::to_path_buf);
            let theme_dependencies = theme::theme_dependency_paths(&rendered_theme.manifest)
                .map(str::to_string)
                .collect();
            let theme_api = rendered_theme.manifest.api();
            let theme_css = rendered_theme.screen_css;
            let diagnostics = deck.diagnostics.clone();
            let live_update = {
                let mut state = state.lock().expect("server state poisoned");
                let html_changed = state.last_good_html != html;
                let theme_css_changed = state.last_good_theme_css != theme_css;
                state.last_good_html = html;
                state.last_good_theme = theme;
                state.last_good_theme_api = theme_api;
                state.last_good_theme_params = theme_params;
                state.last_good_palette_parameter = palette_parameter;
                state.last_good_theme_css = theme_css;
                state.last_good_theme_dependency_root = theme_dependency_root;
                state.last_good_theme_dependencies = theme_dependencies;
                state.diagnostics = diagnostics.clone();
                if initial {
                    None
                } else if html_changed {
                    Some(LiveUpdate::Reload)
                } else if theme_css_changed {
                    Some(LiveUpdate::ThemeCss)
                } else {
                    Some(LiveUpdate::Assets)
                }
            };
            broadcast_diagnostics(state, &diagnostics);
            match live_update {
                Some(LiveUpdate::Reload) => broadcast_reload(state),
                Some(LiveUpdate::ThemeCss) => broadcast_theme_css_reload(state),
                Some(LiveUpdate::Assets) => broadcast_assets_reload(state),
                None => {}
            }
            println!("Rebuilt {} section(s).", deck.sections.len());
        }
        Err(error) => {
            let diagnostics = vec![Diagnostic::error(None, error.to_string())];
            eprintln!("{}", error);
            update_diagnostics(state, diagnostics);
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

fn update_diagnostics(state: &Arc<Mutex<ServerState>>, diagnostics: Vec<Diagnostic>) {
    {
        let mut state = state.lock().expect("server state poisoned");
        state.diagnostics = diagnostics.clone();
    }
    broadcast_diagnostics(state, &diagnostics);
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
    mut stream: TcpStream,
    options: Arc<ServeOptions>,
    state: Arc<Mutex<ServerState>>,
) -> io::Result<()> {
    let mut request_line = String::new();
    {
        let mut reader = BufReader::new(stream.try_clone()?);
        reader.read_line(&mut request_line)?;
    }
    let uri = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .trim_start_matches('/');
    let (encoded_path, query) = uri.split_once('?').unwrap_or((uri, ""));
    let Some(path) = percent_decode_path(encoded_path) else {
        return write_bad_request(
            &mut stream,
            "request path is not valid percent-encoded UTF-8",
        );
    };

    match path.as_str() {
        "" | "index.html" => {
            let html = {
                let state = state.lock().expect("server state poisoned");
                state.last_good_html.clone()
            };
            write_response(&mut stream, "text/html; charset=utf-8", html.as_bytes())
        }
        "zpres/theme-switcher.json" => {
            let body = theme_switcher_state_json(&state);
            write_response(
                &mut stream,
                "application/json; charset=utf-8",
                body.as_bytes(),
            )
        }
        "zpres/theme-switch" => handle_theme_switch(&mut stream, &options, &state, query),
        "events" => handle_events(stream, state),
        asset_path => {
            let theme_api = state
                .lock()
                .expect("server state poisoned")
                .last_good_theme_api;
            if let Some((content_type, body)) = html::core_runtime_asset(asset_path, theme_api) {
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

fn handle_theme_switch(
    stream: &mut TcpStream,
    options: &ServeOptions,
    state: &Arc<Mutex<ServerState>>,
    query: &str,
) -> io::Result<()> {
    let enabled = {
        let state = state.lock().expect("server state poisoned");
        state.theme_switcher_enabled
    };
    if !enabled {
        return write_bad_request(stream, "theme switcher is not enabled");
    }
    let params = parse_query_params(query);
    let Some(theme_name) = params.get("theme").filter(|value| !value.is_empty()) else {
        return write_bad_request(stream, "missing theme");
    };
    if let Err(error) = theme::validate_theme_name(theme_name) {
        return write_bad_request(stream, &error.to_string());
    }
    let variant = params
        .get("variant")
        .filter(|value| value.as_str() != "defaults")
        .cloned();
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
    {
        let mut state = state.lock().expect("server state poisoned");
        state.live_theme_override = Some(LiveThemeOverride {
            theme: manifest.name.clone(),
            variant,
        });
    }
    rebuild(options, state, false);
    let body = theme_switcher_state_json(state);
    write_response(stream, "application/json; charset=utf-8", body.as_bytes())
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

fn handle_events(mut stream: TcpStream, state: Arc<Mutex<ServerState>>) -> io::Result<()> {
    stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n",
    )?;
    stream.write_all(b"retry: 500\n\n")?;

    let (sender, receiver) = mpsc::channel();
    let diagnostics = {
        let mut state = state.lock().expect("server state poisoned");
        state.clients.push(sender);
        state.diagnostics.clone()
    };
    write_sse_event(&mut stream, &diagnostics_event(&diagnostics))?;

    while let Ok(event) = receiver.recv() {
        if write_sse_event(&mut stream, &event).is_err() {
            break;
        }
    }
    Ok(())
}

fn write_response(stream: &mut TcpStream, content_type: &str, body: &[u8]) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
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
    let body = json!({ "error": message }).to_string();
    write!(
        stream,
        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body.as_bytes())
}

fn parse_query_params(query: &str) -> BTreeMap<String, String> {
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .filter_map(|part| {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            Some((percent_decode(key)?, percent_decode(value)?))
        })
        .collect()
}

fn percent_decode(value: &str) -> Option<String> {
    percent_decode_component(value, true)
}

fn percent_decode_path(value: &str) -> Option<String> {
    percent_decode_component(value, false)
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
    write!(
        stream,
        "event: {}\ndata: {}\n\n",
        event.event,
        event.data.replace('\n', "\\n")
    )?;
    stream.flush()
}

fn theme_switcher_state_json(state: &Arc<Mutex<ServerState>>) -> String {
    let state = state.lock().expect("server state poisoned");
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
    .to_string()
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

fn broadcast_reload(state: &Arc<Mutex<ServerState>>) {
    broadcast(
        state,
        SseEvent {
            event: "reload".to_string(),
            data: "{}".to_string(),
        },
    );
}

fn broadcast_theme_css_reload(state: &Arc<Mutex<ServerState>>) {
    let version = live_update_version();
    broadcast(
        state,
        SseEvent {
            event: "theme-css".to_string(),
            data: json!({
                "href": format!("assets/theme.css?v={version}"),
            })
            .to_string(),
        },
    );
}

fn broadcast_assets_reload(state: &Arc<Mutex<ServerState>>) {
    let version = live_update_version();
    broadcast(
        state,
        SseEvent {
            event: "assets".to_string(),
            data: json!({
                "version": version,
                "themeCssHref": format!("assets/theme.css?v={version}"),
            })
            .to_string(),
        },
    );
}

fn live_update_version() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn broadcast_diagnostics(state: &Arc<Mutex<ServerState>>, diagnostics: &[Diagnostic]) {
    broadcast(state, diagnostics_event(diagnostics));
}

fn diagnostics_event(diagnostics: &[Diagnostic]) -> SseEvent {
    SseEvent {
        event: "diagnostics".to_string(),
        data: json!({
            "html": html::render_diagnostics_list(diagnostics),
        })
        .to_string(),
    }
}

fn broadcast(state: &Arc<Mutex<ServerState>>, event: SseEvent) {
    let mut state = state.lock().expect("server state poisoned");
    state
        .clients
        .retain(|client| client.send(event.clone()).is_ok());
}

fn decorate_live_html(html: &str, theme_switcher: bool) -> String {
    let html = inject_live_client(html);
    if theme_switcher {
        inject_theme_switcher(&html)
    } else {
        html
    }
}

fn inject_live_client(html: &str) -> String {
    html.replace(
        "</body>",
        r#"<script>
(function () {
  const liveHashKey = "zpres-live-hash:" + window.location.pathname;
  const liveHashPattern = /^#\/\d+\/\d+(?:\/\d+)?$/;
  function readStoredHash() {
    try {
      return window.sessionStorage.getItem(liveHashKey);
    } catch (_) {
      return null;
    }
  }
  function clearStoredHash() {
    try {
      window.sessionStorage.removeItem(liveHashKey);
    } catch (_) {}
  }
  function rememberLiveHash() {
    try {
      if (window.location.hash && liveHashPattern.test(window.location.hash)) {
        window.sessionStorage.setItem(liveHashKey, window.location.hash);
      }
    } catch (_) {}
  }
  function restoreLiveHash() {
    const storedHash = readStoredHash();
    if (!window.location.hash && storedHash && liveHashPattern.test(storedHash)) {
      window.history.replaceState(null, "", storedHash);
    }
    clearStoredHash();
  }
  restoreLiveHash();
  if (!window.EventSource) return;
  const events = new EventSource("/events");
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
	  events.addEventListener("reload", function () {
	    rememberLiveHash();
	    window.location.reload();
	  });
	  events.addEventListener("theme-css", function (event) {
	    const payload = JSON.parse(event.data);
	    refreshThemeCss(payload.href);
	  });
  events.addEventListener("assets", function (event) {
    const payload = JSON.parse(event.data);
    refreshThemeCss(payload.themeCssHref);
    refreshAssetUrls(payload.version);
  });
	  events.addEventListener("diagnostics", function (event) {
	    const payload = JSON.parse(event.data);
	    const current = document.querySelector(".debug-diagnostics");
	    if (current && payload.html) current.outerHTML = payload.html;
	  });
	})();
</script>
</body>"#,
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
    const params = new URLSearchParams({
      theme: themeSelect.value,
      variant: variantSelect.value,
    });
    const response = await fetch("/zpres/theme-switch?" + params.toString(), { cache: "no-store" });
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
    use tempfile::tempdir;

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
            handle_connection(stream, server_options, server_state).unwrap();
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

    fn runtime_test_options(source: PathBuf) -> ServeOptions {
        ServeOptions {
            source,
            theme_search_paths: Vec::new(),
            fallback_theme: None,
            fallback_theme_params: BTreeMap::new(),
            cli_theme_override: Some("live-theme".to_string()),
            cli_theme_params: BTreeMap::new(),
            port: 0,
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
                handle_connection(stream, server_options, server_state).unwrap();
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
            assert_eq!(
                state.lock().unwrap().last_good_theme_api,
                theme.manifest.api()
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
        let html = inject_live_client("<html><body>deck</body></html>");

        assert!(html.contains("EventSource(\"/events\")"));
        assert!(html.contains("theme-css"));
        assert!(html.contains("assets"));
        assert!(html.contains("zpres_live"));
        assert!(html.contains("assets/theme.css?v="));
        assert!(html.contains("zpres-live-hash:"));
        assert!(html.contains("sessionStorage.setItem(liveHashKey, window.location.hash)"));
        assert!(html.contains("window.history.replaceState(null, \"\", storedHash)"));
        assert!(html.contains("deck"));
    }

    #[test]
    fn theme_switcher_is_injected_when_enabled() {
        let html = decorate_live_html("<html><body>deck</body></html>", true);

        assert!(html.contains("zpres-theme-switcher"));
        assert!(html.contains("/zpres/theme-switcher.json"));
        assert!(html.contains("/zpres/theme-switch?"));
        assert!(html.contains("zpres-live-hash:"));
        assert!(html.contains("deck"));
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
            debounce: Duration::from_millis(1),
            theme_switcher: true,
        };
        let state = Arc::new(Mutex::new(ServerState::new_for_options(
            &options,
            temp.path().to_path_buf(),
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
            debounce: Duration::from_millis(1),
            theme_switcher: false,
        };

        rebuild(&options, &state, true);
        let (sender, receiver) = mpsc::channel();
        state.lock().unwrap().clients.push(sender);
        write_live_theme(&theme_dir, "blue");

        rebuild(&options, &state, false);

        let diagnostics = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        let update = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(diagnostics.event, "diagnostics");
        assert_eq!(update.event, "theme-css");
        assert!(update.data.contains("assets/theme.css?v="));
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
            debounce: Duration::from_millis(1),
            theme_switcher: false,
        };

        rebuild(&options, &state, true);
        let (sender, receiver) = mpsc::channel();
        state.lock().unwrap().clients.push(sender);
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

        let diagnostics = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        let update = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(diagnostics.event, "diagnostics");
        assert_eq!(update.event, "reload");
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
            debounce: Duration::from_millis(1),
            theme_switcher: false,
        };

        rebuild(&options, &state, true);
        let (sender, receiver) = mpsc::channel();
        state.lock().unwrap().clients.push(sender);
        fs::write(asset_dir.join("plot.svg"), "<svg><path /></svg>").unwrap();

        rebuild(&options, &state, false);

        let diagnostics = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        let update = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(diagnostics.event, "diagnostics");
        assert_eq!(update.event, "assets");
        assert!(update.data.contains("assets/theme.css?v="));
        assert!(update.data.contains("version"));
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
