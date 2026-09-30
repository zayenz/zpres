use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket, connect};

use crate::pdf::PngViewport;

const STDERR_CAPTURE_LIMIT: usize = 64 * 1024;
const DEVTOOLS_POLL_INTERVAL: Duration = Duration::from_millis(20);
const PRINT_PAGE_LAYOUT_TOLERANCE_PX: f64 = 2.0;
const MAX_PLATFORM_FONT_PROBES: usize = 24;
const PLATFORM_FONT_PROBE_ATTRIBUTE_PREFIX: &str = "data-zpres-platform-font-probe-";
const PLATFORM_FONT_CANDIDATE_ATTRIBUTE_PREFIX: &str = "data-zpres-platform-font-candidate-";
static PLATFORM_FONT_BRIDGE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
pub(crate) const STATIC_READINESS_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Clone)]
pub(crate) struct ChromiumDiscovery {
    pub executable: Option<PathBuf>,
    pub requested: Option<PathBuf>,
    pub searched: Vec<PathBuf>,
}

pub(crate) fn discover_chromium_executable() -> ChromiumDiscovery {
    let requested = std::env::var_os("ZPRES_CHROMIUM")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    if let Some(path) = &requested {
        return ChromiumDiscovery {
            executable: path.exists().then(|| path.clone()),
            requested,
            searched: Vec::new(),
        };
    }

    let searched = chromium_candidates()
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let executable = searched.iter().find(|path| path.is_file()).cloned();
    ChromiumDiscovery {
        executable,
        requested: None,
        searched,
    }
}

fn chromium_candidates() -> Vec<&'static str> {
    vec![
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
    ]
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ChromiumSessionOptions {
    pub startup_timeout: Duration,
    pub protocol_timeout: Duration,
}

impl Default for ChromiumSessionOptions {
    fn default() -> Self {
        Self {
            startup_timeout: Duration::from_secs(10),
            // Static readiness supplies a separate, longer command deadline.
            protocol_timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ChromiumVersion {
    pub product: String,
    pub revision: String,
    pub user_agent: String,
    pub js_version: String,
    pub protocol_version: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ChromiumCapture {
    pub observation: BrowserPageObservation,
    pub png: Vec<u8>,
    pub semantic_text_suppressed_png: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum BrowserPromiseStatus {
    Missing,
    Fulfilled,
    Rejected,
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct BrowserStaticReadiness {
    pub promise_present: bool,
    pub promise_status: BrowserPromiseStatus,
    pub document_ready_state: String,
    pub ready: bool,
    pub target: Option<String>,
    pub declared_page_count: Option<usize>,
    pub observed_page_count: usize,
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub state: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct BrowserPrintPage {
    pub index: usize,
    pub page: Option<usize>,
    #[serde(default)]
    pub slide_id: String,
    #[serde(default)]
    pub role: String,
    pub bounds: BrowserRect,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct BrowserScreenRoute {
    pub hash: String,
    pub section: usize,
    pub detail: usize,
    pub step: usize,
    pub step_count: usize,
    #[serde(default)]
    pub slide_id: String,
    #[serde(default)]
    pub role: String,
    pub generated: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct BrowserDiagnostic {
    pub kind: String,
    pub level: String,
    pub text: String,
    pub url: Option<String>,
    pub line: Option<u64>,
    pub column: Option<u64>,
}

pub(crate) struct ChromiumSession {
    executable: PathBuf,
    child: Child,
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    profile_dir: PathBuf,
    next_id: u64,
    events: VecDeque<Value>,
    diagnostics: VecDeque<BrowserDiagnostic>,
    target_id: String,
    session_id: String,
    loaded_url: Option<String>,
    loaded_viewport: Option<PngViewport>,
    protocol_timeout: Duration,
    version: ChromiumVersion,
}

impl ChromiumSession {
    pub(crate) fn launch(options: ChromiumSessionOptions) -> Result<Self, ChromiumError> {
        let discovery = discover_chromium_executable();
        let executable = discovery.executable.ok_or(ChromiumError::MissingChromium {
            requested: discovery.requested,
            searched: discovery.searched,
        })?;
        Self::launch_with_executable(executable, options)
    }

    pub(crate) fn launch_with_executable(
        executable: PathBuf,
        options: ChromiumSessionOptions,
    ) -> Result<Self, ChromiumError> {
        if !executable.is_file() {
            return Err(ChromiumError::MissingChromium {
                requested: Some(executable),
                searched: Vec::new(),
            });
        }
        let profile_dir = create_profile_dir()?;
        let stderr_path = profile_dir.join("chromium-stderr.log");
        let stderr = match fs::File::create(&stderr_path) {
            Ok(stderr) => stderr,
            Err(source) => {
                let _ = fs::remove_dir_all(&profile_dir);
                return Err(ChromiumError::CreateDiagnostics {
                    path: stderr_path,
                    source,
                });
            }
        };
        let scratch_root = profile_dir
            .parent()
            .expect("a Chromium profile always has the fixed scratch parent");
        let child_result = Command::new(&executable)
            .arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--no-sandbox")
            .arg("--hide-scrollbars")
            .arg("--disable-background-networking")
            .arg("--disable-component-update")
            .arg("--disable-default-apps")
            .arg("--disable-sync")
            .arg("--metrics-recording-only")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--allow-file-access-from-files")
            .arg("--remote-allow-origins=*")
            .arg("--remote-debugging-address=127.0.0.1")
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", profile_dir.display()))
            .arg("about:blank")
            .env("TMPDIR", scratch_root)
            .env("TMP", scratch_root)
            .env("TEMP", scratch_root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr))
            .spawn();
        let mut child = match child_result {
            Ok(child) => child,
            Err(source) => {
                let _ = fs::remove_dir_all(&profile_dir);
                return Err(ChromiumError::Launch {
                    executable: executable.clone(),
                    source,
                });
            }
        };
        let devtools_file = profile_dir.join("DevToolsActivePort");
        let devtools = match wait_for_devtools(&mut child, &devtools_file, options.startup_timeout)
        {
            Ok(devtools) => devtools,
            Err(error) => {
                stop_child(&mut child);
                let diagnostics = read_stderr(&stderr_path);
                let _ = fs::remove_dir_all(&profile_dir);
                return Err(error.with_diagnostics(diagnostics));
            }
        };
        let websocket_url = format!("ws://127.0.0.1:{}{}", devtools.port, devtools.browser_path);
        let (mut socket, _) = match connect(websocket_url.as_str()) {
            Ok(connection) => connection,
            Err(source) => {
                stop_child(&mut child);
                let diagnostics = read_stderr(&stderr_path);
                let _ = fs::remove_dir_all(&profile_dir);
                return Err(ChromiumError::Connect {
                    url: websocket_url,
                    diagnostics,
                    source,
                });
            }
        };
        if let Err(error) = set_socket_timeout(&mut socket, options.protocol_timeout) {
            stop_child(&mut child);
            let _ = fs::remove_dir_all(&profile_dir);
            return Err(error);
        }

        let mut session = Self {
            executable,
            child,
            socket,
            profile_dir,
            next_id: 1,
            events: VecDeque::new(),
            diagnostics: VecDeque::new(),
            target_id: String::new(),
            session_id: String::new(),
            loaded_url: None,
            loaded_viewport: None,
            protocol_timeout: options.protocol_timeout,
            version: ChromiumVersion {
                product: String::new(),
                revision: String::new(),
                user_agent: String::new(),
                js_version: String::new(),
                protocol_version: String::new(),
            },
        };

        session.version = session.read_browser_version()?;
        let target =
            session.send_command("Target.createTarget", json!({ "url": "about:blank" }), None)?;
        session.target_id = required_string(&target, "targetId")?;
        let attached = session.send_command(
            "Target.attachToTarget",
            json!({ "targetId": session.target_id, "flatten": true }),
            None,
        )?;
        session.session_id = required_string(&attached, "sessionId")?;
        session.send_page_command("Page.enable", json!({}))?;
        session.send_page_command("Runtime.enable", json!({}))?;
        // CSS.getPlatformFontsForNode requires both domains. Keeping them
        // enabled for the target lets every print page and screen route report
        // the fonts Chrome actually used, not only the declared CSS families.
        session.send_page_command("DOM.enable", json!({}))?;
        session.send_page_command("CSS.enable", json!({}))?;
        session.send_page_command("Log.enable", json!({}))?;
        Ok(session)
    }

    pub(crate) fn version(&self) -> &ChromiumVersion {
        &self.version
    }

    pub(crate) fn executable(&self) -> &Path {
        &self.executable
    }

    /// Loads one complete print document into the session's single page target.
    /// Subsequent readiness, observation, capture, and PDF operations all use
    /// this target without navigating again.
    pub(crate) fn load_print_document(
        &mut self,
        url: &str,
        viewport: PngViewport,
    ) -> Result<(), ChromiumError> {
        self.navigate(url, viewport)
    }

    /// Loads the interactive presentation document into the session's single
    /// page target. Screen routes can then be inventoried and captured without
    /// reloading the Deck between states.
    pub(crate) fn load_screen_document(
        &mut self,
        url: &str,
        viewport: PngViewport,
    ) -> Result<(), ChromiumError> {
        self.navigate(url, viewport)
    }

    #[cfg(test)]
    pub(crate) fn reload_screen_document(&mut self) -> Result<(), ChromiumError> {
        self.require_loaded_document()?;
        let previous_loader = self.main_frame_loader_id(self.protocol_timeout)?;
        self.diagnostics.clear();
        self.events.retain(|event| {
            event.get("method").and_then(Value::as_str) != Some("Page.loadEventFired")
        });
        self.send_page_command("Page.reload", json!({ "ignoreCache": true }))?;
        self.wait_for_final_document(Some(&previous_loader))
    }

    #[cfg(test)]
    pub(crate) fn full_accessibility_tree(&mut self) -> Result<Value, ChromiumError> {
        self.require_loaded_document()?;
        self.send_page_command("Accessibility.getFullAXTree", json!({}))
    }

    #[cfg(test)]
    pub(crate) fn evaluate_for_test(&mut self, expression: &str) -> Result<Value, ChromiumError> {
        self.require_loaded_document()?;
        self.evaluate(expression)
    }

    /// Returns every state exposed by the production presentation runtime.
    /// The runtime owns route enumeration so the browser gate follows the same
    /// Section, Detail-slide, generated-slide, and Step semantics as live use.
    pub(crate) fn screen_routes(&mut self) -> Result<Vec<BrowserScreenRoute>, ChromiumError> {
        self.require_loaded_document()?;
        let value = self.evaluate(SCREEN_ROUTES_JS)?;
        let routes: Vec<BrowserScreenRoute> =
            serde_json::from_value(value).map_err(|source| ChromiumError::InvalidScreenRoutes {
                source,
                url: self.loaded_url.clone().unwrap_or_default(),
                diagnostics: self.diagnostic_summary(),
            })?;
        validate_screen_route_inventory(&routes).map_err(|message| {
            ChromiumError::InvalidScreenRouteInventory {
                message,
                url: self.loaded_url.clone().unwrap_or_default(),
                diagnostics: self.diagnostic_summary(),
            }
        })?;
        Ok(routes)
    }

    /// Navigates through the production runtime, verifies that it activated the
    /// requested state, measures the active Slide, and captures the full logical
    /// presentation viewport.
    pub(crate) fn capture_screen_route(
        &mut self,
        route: &BrowserScreenRoute,
    ) -> Result<ChromiumCapture, ChromiumError> {
        self.require_loaded_document()?;
        let expression = SCREEN_NAVIGATE_JS
            .replace("__ZPRES_SCREEN_SECTION__", &route.section.to_string())
            .replace("__ZPRES_SCREEN_DETAIL__", &route.detail.to_string())
            .replace("__ZPRES_SCREEN_STEP__", &route.step.to_string());
        let value = self.evaluate(&expression)?;
        let active: BrowserScreenRoute = serde_json::from_value(value).map_err(|source| {
            ChromiumError::InvalidScreenRouteState {
                source,
                url: self.loaded_url.clone().unwrap_or_default(),
                diagnostics: self.diagnostic_summary(),
            }
        })?;
        if active != *route {
            return Err(ChromiumError::ScreenRouteMismatch {
                expected: Box::new(route.clone()),
                observed: Box::new(active),
                diagnostics: self.diagnostic_summary(),
            });
        }

        // Navigation can resolve once the runtime state is correct but before
        // Chromium has committed the newly visible Step. This matters after
        // the preceding route's semantic-text suppression capture: without a
        // fresh paint boundary, the next ordinary screenshot can contain a
        // mixture of the restored and newly activated states.
        self.settle_paint_frames()?;

        let active_json = serde_json::to_string(&active).map_err(|source| {
            ChromiumError::InvalidProtocolJson {
                source,
                message: active.hash.clone(),
            }
        })?;
        let expression = format!(
            "window.__zpresVisualObservationMode = 'screen';\nwindow.__zpresActiveScreenRoute = {active_json};\n{VISUAL_OBSERVATION_JS}"
        );
        let mut observation = self.observe_visual_page(&expression)?;
        let accessibility_preferences = self.accessibility_preference_evidence()?;
        if self.debug_inspection_enabled()? {
            self.settle_visual_paint()?;
            observation = self.observe_visual_page(&expression)?;
        }
        observation.accessibility_preferences = accessibility_preferences;
        let png = self.capture_screenshot()?;
        self.suppress_semantic_text()?;
        let suppressed = self.capture_screenshot();
        let restore = self.restore_semantic_text();
        let semantic_text_suppressed_png = suppressed?;
        restore?;
        Ok(ChromiumCapture {
            observation,
            png,
            semantic_text_suppressed_png,
        })
    }

    /// Waits for the page-owned `window.zpresStaticReady` promise. The promise
    /// may resolve to a structured object, which is retained in `state`; the
    /// stable fields also fall back to the print body's
    /// `data-zpres-*` attributes for compatibility with simple fixtures.
    pub(crate) fn await_static_readiness(
        &mut self,
        timeout: Duration,
    ) -> Result<BrowserStaticReadiness, ChromiumError> {
        self.require_loaded_document()?;
        let bounded_timeout = timeout
            .max(Duration::from_millis(1))
            .min(Duration::from_secs(5 * 60 - 1));
        let timeout_ms = u64::try_from(bounded_timeout.as_millis()).unwrap_or(u64::MAX);
        let expression =
            AWAIT_STATIC_READINESS_JS.replace("__ZPRES_TIMEOUT_MS__", &timeout_ms.to_string());
        let value = self.evaluate_with_timeout(
            &expression,
            bounded_timeout.saturating_add(Duration::from_secs(1)),
        )?;
        serde_json::from_value(value).map_err(|source| ChromiumError::InvalidReadiness {
            source,
            url: self.loaded_url.clone().unwrap_or_default(),
            diagnostics: self.diagnostic_summary(),
        })
    }

    /// Returns the print pages in document order. `index` is zero-based and is
    /// the value accepted by `observe_print_page` and `capture_print_page`.
    pub(crate) fn print_pages(&mut self) -> Result<Vec<BrowserPrintPage>, ChromiumError> {
        self.require_loaded_document()?;
        let value = self.evaluate(PRINT_PAGES_JS)?;
        let pages: Vec<BrowserPrintPage> =
            serde_json::from_value(value).map_err(|source| ChromiumError::InvalidPrintPages {
                source,
                url: self.loaded_url.clone().unwrap_or_default(),
                diagnostics: self.diagnostic_summary(),
            })?;
        validate_print_page_inventory(&pages).map_err(|message| {
            ChromiumError::InvalidPrintPageLayout {
                message,
                url: self.loaded_url.clone().unwrap_or_default(),
                diagnostics: self.diagnostic_summary(),
            }
        })?;
        Ok(pages)
    }

    pub(crate) fn observe_print_page(
        &mut self,
        page_index: usize,
    ) -> Result<BrowserPageObservation, ChromiumError> {
        self.require_print_page(page_index)?;
        let expression = format!(
            "window.__zpresVisualObservationMode = 'print';\nwindow.__zpresRequestedPrintPageIndex = {page_index};\n{VISUAL_OBSERVATION_JS}"
        );
        self.observe_visual_page(&expression)
    }

    pub(crate) fn capture_print_page(
        &mut self,
        page_index: usize,
    ) -> Result<ChromiumCapture, ChromiumError> {
        let output_viewport = self
            .loaded_viewport
            .ok_or(ChromiumError::NoLoadedDocument)?;
        self.capture_print_page_with_output_viewport(page_index, output_viewport)
    }

    /// Captures a loaded logical print canvas at a different output-pixel size.
    /// The browser viewport and observations remain unchanged; CDP scales the
    /// logical clip uniformly when it encodes the screenshot.
    pub(crate) fn capture_print_page_with_output_viewport(
        &mut self,
        page_index: usize,
        output_viewport: PngViewport,
    ) -> Result<ChromiumCapture, ChromiumError> {
        self.require_print_page(page_index)?;
        let accessibility_preferences = self.accessibility_preference_evidence()?;
        // Media-preference emulation can schedule another print autoscale pass.
        // Finish it and measure the same settled geometry that the raster will
        // paint, rather than retaining bounds from an intermediate scale.
        self.settle_visual_paint()?;
        let mut observation = self.observe_print_page(page_index)?;
        observation.accessibility_preferences = accessibility_preferences;
        let clip = observation
            .slide_bounds
            .filter(|bounds| bounds.width > 0.0 && bounds.height > 0.0)
            .ok_or_else(|| ChromiumError::InvalidPageClip {
                page_index,
                slide_id: observation.slide_id.clone(),
                diagnostics: self.diagnostic_summary(),
            })?;
        let logical_viewport = self
            .loaded_viewport
            .ok_or(ChromiumError::NoLoadedDocument)?;
        let scale = f64::from(output_viewport.width) / f64::from(logical_viewport.width);
        let screenshot_clip = BrowserRect {
            width: f64::from(logical_viewport.width),
            height: f64::from(logical_viewport.height),
            right: clip.x + f64::from(logical_viewport.width),
            bottom: clip.y + f64::from(logical_viewport.height),
            ..clip
        };
        let png = self.capture_screenshot_clip(screenshot_clip, scale)?;
        self.suppress_semantic_text()?;
        let suppressed = self.capture_screenshot_clip(screenshot_clip, scale);
        let restore = self.restore_semantic_text();
        let semantic_text_suppressed_png = suppressed?;
        restore?;
        Ok(ChromiumCapture {
            observation,
            png,
            semantic_text_suppressed_png,
        })
    }

    fn suppress_semantic_text(&mut self) -> Result<(), ChromiumError> {
        self.evaluate(
            r#"(() => {
              document.getElementById('zpres-semantic-text-suppression')?.remove();
              document.querySelectorAll('[data-zpres-semantic-text-suppressed]').forEach((element) => {
                element.removeAttribute('data-zpres-semantic-text-suppressed');
              });
              const mode = window.__zpresVisualObservationMode;
              const target = mode === 'print'
                ? Array.from(document.querySelectorAll('.zpres-print-slide'))[
                    Number(window.__zpresRequestedPrintPageIndex)
                  ]
                : document.querySelector('.zpres-slide.is-active, .zpres-slide.present');
              if (!target) throw new Error(`cannot locate ${mode ?? 'unknown'} slide for semantic-text suppression`);
              target.setAttribute('data-zpres-semantic-text-suppressed', 'true');
              const style = document.createElement('style');
              style.id = 'zpres-semantic-text-suppression';
              // Important declarations reverse cascade-layer precedence. Keep
              // this validator override in the renderer-owned first layer so
              // Theme-layer `!important` SVG paint cannot survive suppression.
              style.textContent = `
                @layer zpres-reset {
                  [data-zpres-semantic-text-suppressed] .zpres-slide-header,
                  [data-zpres-semantic-text-suppressed] .zpres-slide-header *,
                  [data-zpres-semantic-text-suppressed] .zpres-slide-body,
                  [data-zpres-semantic-text-suppressed] .zpres-slide-body *,
                  [data-zpres-semantic-text-suppressed] .zpres-slide-footer,
                  [data-zpres-semantic-text-suppressed] .zpres-slide-footer * {
                    color: transparent !important;
                    -webkit-text-fill-color: transparent !important;
                    text-shadow: none !important;
                  }
                  [data-zpres-semantic-text-suppressed] .zpres-slide-header svg text,
                  [data-zpres-semantic-text-suppressed] .zpres-slide-header svg text *,
                  [data-zpres-semantic-text-suppressed] .zpres-slide-body svg text,
                  [data-zpres-semantic-text-suppressed] .zpres-slide-body svg text *,
                  [data-zpres-semantic-text-suppressed] .zpres-slide-footer svg text,
                  [data-zpres-semantic-text-suppressed] .zpres-slide-footer svg text * {
                    fill: transparent !important;
                    stroke: transparent !important;
                  }
                }
              `;
              document.head.append(style);
              return true;
            })()"#,
        )?;
        // The visual gate captures the ordinary page and a text-suppressed
        // companion from the same long-lived browser target. Wait for the
        // suppression style to paint before taking the companion image; a
        // half-painted frame can otherwise leak into this capture and the
        // next print page.
        self.settle_paint_frames()?;
        Ok(())
    }

    fn restore_semantic_text(&mut self) -> Result<(), ChromiumError> {
        self.evaluate(
            r#"(() => {
              document.getElementById('zpres-semantic-text-suppression')?.remove();
              document.querySelectorAll('[data-zpres-semantic-text-suppressed]').forEach((element) => {
                element.removeAttribute('data-zpres-semantic-text-suppressed');
              });
              return true;
            })()"#,
        )?;
        // Print pages are captured back-to-back without navigation. Make the
        // restored text a completed frame before the next page is observed or
        // captured, rather than relying on an incidental repaint.
        self.settle_paint_frames()?;
        Ok(())
    }

    fn settle_paint_frames(&mut self) -> Result<(), ChromiumError> {
        self.evaluate(
            r#"(async () => {
              // Force style/layout resolution, then wait for the runtime's
              // finite reveal/visibility transitions. Navigation resolves
              // before those 160ms transitions finish, so two animation
              // frames alone can capture a half-painted Step. Keep the wait
              // bounded in case a Theme owns a deliberately looping animation.
              void document.documentElement.offsetHeight;
              await new Promise((resolve) => requestAnimationFrame(resolve));
              const active = document.getAnimations().filter((animation) =>
                animation.playState === 'running' || animation.playState === 'pending'
              );
              if (active.length > 0) {
                await Promise.race([
                  Promise.allSettled(active.map((animation) => animation.finished)),
                  new Promise((resolve) => setTimeout(resolve, 1000)),
                ]);
              }
              await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
              return true;
            })()"#,
        )?;
        Ok(())
    }

    fn accessibility_preference_evidence(
        &mut self,
    ) -> Result<BrowserAccessibilityPreferenceEvidence, ChromiumError> {
        let focus_visible = self
            .evaluate(
                r#"(() => {
                  if (document.body?.getAttribute('data-zpres-output-target') === 'print') return true;
                  const target = Array.from(document.querySelectorAll('[data-nav], a[href], button, [tabindex]:not([tabindex="-1"])')).find((element) => {
                    const style = getComputedStyle(element);
                    const bounds = element.getBoundingClientRect();
                    return style.display !== 'none' && style.visibility !== 'hidden' && bounds.width > 0 && bounds.height > 0;
                  });
                  if (!target) return true;
                  const x = window.scrollX, y = window.scrollY;
                  target.focus({ focusVisible: true });
                  const style = getComputedStyle(target);
                  const visible = target.matches(':focus-visible')
                    && style.outlineStyle !== 'none'
                    && Number.parseFloat(style.outlineWidth) >= 2;
                  target.blur();
                  window.scrollTo(x, y);
                  return visible;
                })()"#,
            )?
            .as_bool()
            .unwrap_or(false);

        let preference = |session: &mut Self,
                          name: &str,
                          value: &str,
                          expression: &str|
         -> Result<bool, ChromiumError> {
            session.send_page_command(
                "Emulation.setEmulatedMedia",
                json!({
                    "media": "screen",
                    "features": [{ "name": name, "value": value }],
                }),
            )?;
            Ok(session.evaluate(expression)?.as_bool().unwrap_or(false))
        };
        let state_expression = r#"(() => {
          const active = document.querySelector('.zpres-step[data-step-state="active"]');
          if (!active) return true;
          const style = getComputedStyle(active);
          return active.getAttribute('aria-current') === 'step'
            && (style.outlineStyle !== 'none' || Number.parseFloat(style.borderInlineStartWidth) >= 3);
        })()"#;
        let reduced_motion_preserves_state = preference(
            self,
            "prefers-reduced-motion",
            "reduce",
            r#"(() => {
              if (!matchMedia('(prefers-reduced-motion: reduce)').matches) return false;
              const elements = document.querySelectorAll('.zpres-slide.is-active, .fragment, .zpres-step, .zpres-code-line');
              return Array.from(elements).every((element) => {
                const style = getComputedStyle(element);
                return style.animationName === 'none'
                  && style.transitionDuration.split(',').every((duration) => Number.parseFloat(duration) === 0);
              });
            })()"#,
        )?;
        let increased_contrast_preserves_state =
            preference(self, "prefers-contrast", "more", state_expression)?;
        let forced_colors_preserves_state =
            preference(self, "forced-colors", "active", state_expression)?;
        let restore_media = self
            .evaluate("document.body?.getAttribute('data-zpres-output-target') || 'screen'")?
            .as_str()
            .unwrap_or("screen")
            .to_string();
        self.send_page_command(
            "Emulation.setEmulatedMedia",
            json!({ "media": restore_media, "features": [] }),
        )?;
        let current_step_has_non_color_cue =
            self.evaluate(state_expression)?.as_bool().unwrap_or(false);
        let min_non_text_contrast = self
            .evaluate(
                r#"(() => {
                  const parse = (value) => {
                    const match = String(value || '').match(/rgba?\(([^)]+)\)/i);
                    if (!match) return null;
                    const parts = match[1].split(/[ ,/]+/).filter(Boolean).map(Number);
                    if (parts.length < 3 || parts.some((part) => !Number.isFinite(part))) return null;
                    return { r: parts[0], g: parts[1], b: parts[2], a: Number.isFinite(parts[3]) ? parts[3] : 1 };
                  };
                  const luminance = (color) => {
                    const channel = (value) => {
                      const normalized = value / 255;
                      return normalized <= 0.04045 ? normalized / 12.92 : Math.pow((normalized + 0.055) / 1.055, 2.4);
                    };
                    return 0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b);
                  };
                  const background = (element) => {
                    for (let current = element; current instanceof Element; current = current.parentElement) {
                      const color = parse(getComputedStyle(current).backgroundColor);
                      if (color?.a >= 0.999) return color;
                    }
                    return parse(getComputedStyle(document.body).backgroundColor);
                  };
                  const ratio = (first, second) => {
                    if (!first || !second || first.a < 0.999 || second.a < 0.999) return null;
                    const a = luminance(first), b = luminance(second);
                    return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
                  };
                  const indicators = [document.querySelector(':focus-visible'), document.querySelector('.zpres-step[data-step-state="active"]')]
                    .filter(Boolean)
                    .map((element) => {
                      const style = getComputedStyle(element);
                      const color = style.outlineStyle !== 'none' ? style.outlineColor : style.borderInlineStartColor;
                      return { element, color: parse(color) };
                    });
                  const graphics = Array.from(document.querySelectorAll('[data-chart-role] :is(path, line, circle, rect, polygon, polyline), .zpres-block-diagram svg :is(path, line, circle, rect, polygon, polyline)'))
                    .filter((element) => {
                      const bounds = element.getBoundingClientRect();
                      return bounds.width > 0 && bounds.height > 0;
                    })
                    .map((element) => {
                      const style = getComputedStyle(element);
                      return { element, color: parse(style.stroke) || parse(style.fill) };
                    })
                    .filter((sample) => sample.color?.a >= 0.999);
                  const ratios = [...indicators, ...graphics]
                    .map((sample) => ratio(sample.color, background(sample.element)))
                    .filter(Number.isFinite);
                  return ratios.length ? Math.min(...ratios) : null;
                })()"#,
            )?
            .as_f64();
        let non_color_state_cues_present = self
            .evaluate(
                r#"(() => {
                  const visible = (element) => element && getComputedStyle(element).display !== 'none' && getComputedStyle(element).visibility !== 'hidden';
                  const callouts = Array.from(document.querySelectorAll('.zpres-slide.is-active .zpres-block-callout')).filter(visible);
                  const comparisons = Array.from(document.querySelectorAll('.zpres-slide.is-active [data-comparison-role]')).filter(visible);
                  const legends = Array.from(document.querySelectorAll('.zpres-slide.is-active [data-chart-role="legend"]')).filter(visible);
                  return callouts.every((element) => Boolean(element.querySelector('.zpres-callout-title')?.textContent?.trim()) && Number.parseFloat(getComputedStyle(element).borderInlineStartWidth) > 0)
                    && comparisons.every((element) => Boolean(element.getAttribute('data-comparison-cue')) && Boolean(element.querySelector('.zpres-layout-region-title')?.textContent?.trim()))
                    && legends.every((element) => Boolean(element.textContent?.trim()));
                })()"#,
            )?
            .as_bool()
            .unwrap_or(false);
        Ok(BrowserAccessibilityPreferenceEvidence {
            focus_visible,
            reduced_motion_preserves_state,
            increased_contrast_preserves_state,
            forced_colors_preserves_state,
            current_step_has_non_color_cue,
            min_non_text_contrast,
            non_color_state_cues_present,
        })
    }

    /// Produces PDF bytes from the currently loaded target through CDP. The
    /// caller is responsible for awaiting and validating static readiness first.
    pub(crate) fn print_pdf(&mut self) -> Result<Vec<u8>, ChromiumError> {
        self.require_loaded_document()?;
        let result = self.send_page_command(
            "Page.printToPDF",
            json!({
                "displayHeaderFooter": false,
                "printBackground": true,
                "preferCSSPageSize": true,
                "transferMode": "ReturnAsStream",
            }),
        )?;
        let handle = result
            .get("stream")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| ChromiumError::Protocol {
                message: format!(
                    "Page.printToPDF returned no stream: {result}. {}",
                    self.diagnostic_summary()
                ),
            })?;
        let mut pdf = Vec::new();
        loop {
            let chunk = self
                .send_page_command("IO.read", json!({ "handle": handle, "size": 1024 * 1024 }))?;
            let data = chunk.get("data").and_then(Value::as_str).ok_or_else(|| {
                ChromiumError::Protocol {
                    message: format!(
                        "IO.read returned no PDF data: {chunk}. {}",
                        self.diagnostic_summary()
                    ),
                }
            })?;
            if chunk
                .get("base64Encoded")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                pdf.extend(
                    base64::engine::general_purpose::STANDARD
                        .decode(data)
                        .map_err(ChromiumError::PdfDecode)?,
                );
            } else {
                pdf.extend_from_slice(data.as_bytes());
            }
            if chunk.get("eof").and_then(Value::as_bool).unwrap_or(false) {
                break;
            }
        }
        self.send_page_command("IO.close", json!({ "handle": handle }))?;
        Ok(pdf)
    }

    pub(crate) fn diagnostics(&self) -> Vec<BrowserDiagnostic> {
        self.diagnostics.iter().cloned().collect()
    }

    #[cfg(test)]
    pub(crate) fn capture_page(
        &mut self,
        url: &str,
        viewport: PngViewport,
    ) -> Result<ChromiumCapture, ChromiumError> {
        self.load_print_document(url, viewport)?;
        let observation = self.observe_print_page(0)?;
        let png = self.capture_screenshot()?;
        self.suppress_semantic_text()?;
        let suppressed = self.capture_screenshot();
        let restore = self.restore_semantic_text();
        let semantic_text_suppressed_png = suppressed?;
        restore?;
        Ok(ChromiumCapture {
            observation,
            png,
            semantic_text_suppressed_png,
        })
    }

    pub(crate) fn capture_document(
        &mut self,
        url: &str,
        viewport: PngViewport,
    ) -> Result<Vec<u8>, ChromiumError> {
        self.load_print_document(url, viewport)?;
        let result = self.evaluate(DOCUMENT_READY_JS)?;
        if result
            .get("errors")
            .and_then(Value::as_array)
            .is_some_and(|errors| !errors.is_empty())
        {
            return Err(ChromiumError::Evaluation {
                message: format!("document assets did not become ready: {result}"),
            });
        }
        self.capture_screenshot()
    }

    fn navigate(&mut self, url: &str, viewport: PngViewport) -> Result<(), ChromiumError> {
        self.loaded_url = None;
        self.loaded_viewport = None;
        self.diagnostics.clear();
        let previous_loader = self.main_frame_loader_id(self.protocol_timeout)?;
        self.send_page_command(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": viewport.width,
                "height": viewport.height,
                "deviceScaleFactor": 1,
                "mobile": false,
            }),
        )?;
        self.events.retain(|event| {
            event.get("method").and_then(Value::as_str) != Some("Page.loadEventFired")
        });
        let result = self.send_page_command("Page.navigate", json!({ "url": url }))?;
        if let Some(error_text) = result.get("errorText").and_then(Value::as_str) {
            return Err(ChromiumError::Navigation {
                url: url.to_string(),
                reason: format!("{error_text}. {}", self.diagnostic_summary()),
            });
        }
        self.wait_for_final_document(Some(&previous_loader))?;
        self.loaded_url = Some(url.to_string());
        self.loaded_viewport = Some(viewport);
        Ok(())
    }

    fn wait_for_final_document(
        &mut self,
        previous_loader: Option<&str>,
    ) -> Result<(), ChromiumError> {
        const READY: &str = r#"(() => document.readyState === "complete"
          && !document.querySelector('meta[http-equiv="refresh"]'))()"#;
        let deadline = Instant::now() + self.protocol_timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ChromiumError::ProtocolTimeout {
                    operation: "final non-redirect document readiness".to_string(),
                    timeout: self.protocol_timeout,
                    diagnostics: self.diagnostic_summary(),
                });
            }
            let attempt_timeout = remaining.min(Duration::from_millis(250));
            let loader_changed = self
                .main_frame_loader_id(attempt_timeout)
                .is_ok_and(|loader| previous_loader.is_none_or(|previous| loader != previous));
            match self.evaluate_with_timeout(READY, attempt_timeout) {
                Ok(Value::Bool(true)) if loader_changed => return Ok(()),
                Ok(_) => {}
                Err(
                    ChromiumError::Evaluation { .. }
                    | ChromiumError::Protocol { .. }
                    | ChromiumError::ProtocolTimeout { .. },
                ) => {
                    // Redirects can destroy the current JavaScript context
                    // between command dispatch and evaluation. Retry against
                    // the final document until the navigation deadline.
                }
                Err(error) => return Err(error),
            }
            std::thread::sleep(Duration::from_millis(10).min(remaining));
        }
    }

    fn main_frame_loader_id(&mut self, timeout: Duration) -> Result<String, ChromiumError> {
        let result =
            self.send_page_command_with_timeout("Page.getFrameTree", json!({}), timeout)?;
        result
            .pointer("/frameTree/frame/loaderId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| ChromiumError::Protocol {
                message: format!(
                    "Page.getFrameTree returned no main-frame loader id: {result}. {}",
                    self.diagnostic_summary()
                ),
            })
    }

    fn require_loaded_document(&self) -> Result<(), ChromiumError> {
        if self.loaded_url.is_some() {
            Ok(())
        } else {
            Err(ChromiumError::NoLoadedDocument)
        }
    }

    fn require_print_page(&mut self, page_index: usize) -> Result<BrowserPrintPage, ChromiumError> {
        let pages = self.print_pages()?;
        let available_pages = pages.len();
        pages
            .into_iter()
            .find(|page| page.index == page_index)
            .ok_or_else(|| ChromiumError::MissingPrintPage {
                page_index,
                available_pages,
                diagnostics: self.diagnostic_summary(),
            })
    }

    fn evaluate(&mut self, expression: &str) -> Result<Value, ChromiumError> {
        self.evaluate_with_timeout(expression, self.protocol_timeout)
    }

    fn observe_visual_page(
        &mut self,
        expression: &str,
    ) -> Result<BrowserPageObservation, ChromiumError> {
        let evaluated = self.evaluate(expression)?;
        let mut observation: BrowserPageObservation =
            serde_json::from_value(evaluated).map_err(|source| {
                ChromiumError::InvalidObservation {
                    source,
                    url: self.loaded_url.clone().unwrap_or_default(),
                    diagnostics: self.diagnostic_summary(),
                }
            })?;
        self.attach_platform_font_evidence(&mut observation)?;
        Ok(observation)
    }

    fn settle_visual_paint(&mut self) -> Result<(), ChromiumError> {
        self.evaluate(
            r#"(async () => {
              await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
              if (typeof window.zpresAutoscaleAll === "function") {
                await window.zpresAutoscaleAll();
              }
              if (typeof window.zpresRefreshDebugInspection === "function") {
                window.zpresRefreshDebugInspection();
              }
              await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
              return true;
            })()"#,
        )?;
        Ok(())
    }

    fn debug_inspection_enabled(&mut self) -> Result<bool, ChromiumError> {
        Ok(self
            .evaluate("typeof window.zpresRefreshDebugInspection === 'function'")?
            .as_bool()
            .unwrap_or(false))
    }

    fn attach_platform_font_evidence(
        &mut self,
        observation: &mut BrowserPageObservation,
    ) -> Result<(), ChromiumError> {
        let bridge = PlatformFontBridge::new();
        let cleanup_js = clear_platform_font_probes_js(&bridge);
        let setup_value =
            match self.evaluate(&platform_font_probes_js(MAX_PLATFORM_FONT_PROBES, &bridge)) {
                Ok(value) => value,
                Err(error) => {
                    let _ = self.evaluate(&cleanup_js);
                    return Err(error);
                }
            };
        let setup: BrowserPlatformFontProbeSetup = match serde_json::from_value(setup_value) {
            Ok(setup) => setup,
            Err(source) => {
                let _ = self.evaluate(&cleanup_js);
                return Err(ChromiumError::InvalidProtocolJson {
                    source,
                    message: "platform-font probe setup".to_string(),
                });
            }
        };

        let result = self.collect_platform_fonts(&setup, &bridge);
        // Probe attributes are deliberately temporary: they must not affect
        // the screenshot or remain on the shared target for the next route.
        let cleanup = self.evaluate(&cleanup_js);
        let (probes, fonts) = match result {
            Ok(evidence) => {
                cleanup?;
                evidence
            }
            Err(error) => {
                let _ = cleanup;
                return Err(error);
            }
        };

        observation.platform_font_probes = probes;
        observation.platform_fonts = fonts;
        observation.platform_font_probe_count = observation.platform_font_probes.len();
        observation.platform_font_candidate_count = setup.candidate_count;
        observation.platform_font_probe_truncated = setup.truncated;
        observation.platform_font_evidence =
            BrowserPlatformFontEvidence::from_fonts(&observation.platform_fonts);
        for measurement in &mut observation.design_measurements {
            if let Some(probe) = observation
                .platform_font_probes
                .iter()
                .find(|probe| probe.element == measurement.element)
            {
                measurement.actual_font_families =
                    probe.fonts.iter().map(|font| font.family.clone()).collect();
                measurement.actual_font_families.sort();
                measurement.actual_font_families.dedup();
            }
        }
        Ok(())
    }

    fn collect_platform_fonts(
        &mut self,
        setup: &BrowserPlatformFontProbeSetup,
        bridge: &PlatformFontBridge,
    ) -> Result<(Vec<BrowserPlatformFontProbe>, Vec<BrowserPlatformFont>), ChromiumError> {
        if !setup.slide_present {
            return Ok((Vec::new(), Vec::new()));
        }

        let document = self.send_page_command("DOM.getDocument", json!({ "depth": 0 }))?;
        let root_node_id = document
            .pointer("/root/nodeId")
            .and_then(Value::as_u64)
            .ok_or_else(|| ChromiumError::Protocol {
                message: format!(
                    "DOM.getDocument returned no root node id: {document}. {}",
                    self.diagnostic_summary()
                ),
            })?;
        let candidate_matches = self.send_page_command(
            "DOM.querySelectorAll",
            json!({
                "nodeId": root_node_id,
                "selector": bridge.candidate_selector(),
            }),
        )?;
        let candidate_node_ids = platform_font_node_ids(
            &candidate_matches,
            "DOM.querySelectorAll returned no platform-font candidate nodes",
            self,
        )?;
        if candidate_node_ids.len() != setup.candidate_count {
            return Err(ChromiumError::Protocol {
                message: format!(
                    "platform-font candidate inventory changed between Runtime and DOM inspection: setup found {}, DOM returned {}. {}",
                    setup.candidate_count,
                    candidate_node_ids.len(),
                    self.diagnostic_summary()
                ),
            });
        }

        let matches = self.send_page_command(
            "DOM.querySelectorAll",
            json!({
                "nodeId": root_node_id,
                "selector": bridge.probe_selector(),
            }),
        )?;
        let node_ids = platform_font_node_ids(
            &matches,
            "DOM.querySelectorAll returned no platform-font probe nodes",
            self,
        )?;
        if node_ids.len() != setup.probes.len() || node_ids.len() != setup.selected_count {
            return Err(ChromiumError::Protocol {
                message: format!(
                    "platform-font probe inventory changed between Runtime and DOM inspection: setup selected {}, described {}, DOM returned {}. {}",
                    setup.selected_count,
                    setup.probes.len(),
                    node_ids.len(),
                    self.diagnostic_summary()
                ),
            });
        }

        // CDP reports fonts used for a node's direct child TextNodes, not its
        // whole subtree. Every visible direct TextNode belongs to exactly one
        // candidate parent, so these responses can be aggregated without the
        // overlapping-subtree double counting that element probes would cause.
        let mut all_usages = Vec::new();
        let mut fonts_by_node = BTreeMap::new();
        for node_id in candidate_node_ids {
            let response = self
                .send_page_command("CSS.getPlatformFontsForNode", json!({ "nodeId": node_id }))?;
            let response: CdpPlatformFontsResponse =
                serde_json::from_value(response).map_err(|source| {
                    ChromiumError::InvalidProtocolJson {
                        source,
                        message: "CSS.getPlatformFontsForNode response".to_string(),
                    }
                })?;
            all_usages.extend(response.fonts.iter().cloned());
            fonts_by_node.insert(node_id, aggregate_platform_fonts(response.fonts));
        }
        let fonts = aggregate_platform_fonts(all_usages);

        let mut probes = Vec::with_capacity(node_ids.len());
        for (node_id, metadata) in node_ids.iter().zip(&setup.probes) {
            let fonts = fonts_by_node.get(node_id).cloned().unwrap_or_default();
            let multiple_faces_observed = fonts.len() > 1;
            let secondary_face_glyph_count = secondary_face_glyph_count(&fonts);
            probes.push(BrowserPlatformFontProbe {
                index: metadata.index,
                element: metadata.element.clone(),
                text_excerpt: metadata.text_excerpt.clone(),
                requested_family: metadata.requested_family.clone(),
                requested_style: metadata.requested_style.clone(),
                requested_weight: metadata.requested_weight.clone(),
                requested_font_synthesis: metadata.requested_font_synthesis.clone(),
                fonts,
                multiple_faces_observed,
                secondary_face_glyph_count,
            });
        }

        probes.sort_by_key(|probe| probe.index);
        Ok((probes, fonts))
    }

    fn evaluate_with_timeout(
        &mut self,
        expression: &str,
        timeout: Duration,
    ) -> Result<Value, ChromiumError> {
        let result = self.send_page_command_with_timeout(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "awaitPromise": true,
                "returnByValue": true,
                "userGesture": false,
            }),
            timeout,
        )?;
        if let Some(exception) = result.get("exceptionDetails") {
            return Err(ChromiumError::Evaluation {
                message: format!("{exception}. {}", self.diagnostic_summary()),
            });
        }
        result
            .pointer("/result/value")
            .cloned()
            .ok_or_else(|| ChromiumError::Evaluation {
                message: format!(
                    "Runtime.evaluate returned no value: {result}. {}",
                    self.diagnostic_summary()
                ),
            })
    }

    fn capture_screenshot(&mut self) -> Result<Vec<u8>, ChromiumError> {
        let result = self.send_page_command(
            "Page.captureScreenshot",
            json!({
                "format": "png",
                "fromSurface": true,
                "captureBeyondViewport": false,
            }),
        )?;
        let data =
            result
                .get("data")
                .and_then(Value::as_str)
                .ok_or_else(|| ChromiumError::Protocol {
                    message: format!("Page.captureScreenshot returned no data: {result}"),
                })?;
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(ChromiumError::ScreenshotDecode)
    }

    fn capture_screenshot_clip(
        &mut self,
        clip: BrowserRect,
        scale: f64,
    ) -> Result<Vec<u8>, ChromiumError> {
        let scroll = self.evaluate("({ x: window.scrollX, y: window.scrollY })")?;
        self.evaluate(&format!(
            r#"(async () => {{
              window.scrollTo({{ left: {x}, top: {y}, behavior: 'instant' }});
              await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
              return {{ x: window.scrollX, y: window.scrollY }};
            }})()"#,
            x = clip.x,
            y = clip.y,
        ))?;
        let result = self.send_page_command(
            "Page.captureScreenshot",
            json!({
                "format": "png",
                "fromSurface": true,
                // Capturing beyond the viewport temporarily resizes Chromium's
                // surface and can restart print autoscaling. The requested page
                // is scrolled into the existing viewport instead, so companion
                // rasters retain the observation's settled geometry.
                "captureBeyondViewport": false,
                "clip": {
                    "x": clip.x,
                    "y": clip.y,
                    "width": clip.width,
                    "height": clip.height,
                    "scale": scale,
                },
            }),
        );
        let restore = self.evaluate(&format!(
            "window.scrollTo({{ left: {}, top: {}, behavior: 'instant' }}); true",
            scroll.get("x").and_then(Value::as_f64).unwrap_or(0.0),
            scroll.get("y").and_then(Value::as_f64).unwrap_or(0.0),
        ));
        let result = result?;
        restore?;
        let data =
            result
                .get("data")
                .and_then(Value::as_str)
                .ok_or_else(|| ChromiumError::Protocol {
                    message: format!(
                        "Page.captureScreenshot returned no clipped data: {result}. {}",
                        self.diagnostic_summary()
                    ),
                })?;
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(ChromiumError::ScreenshotDecode)
    }

    fn read_browser_version(&mut self) -> Result<ChromiumVersion, ChromiumError> {
        let result = self.send_command("Browser.getVersion", json!({}), None)?;
        Ok(ChromiumVersion {
            product: optional_string(&result, "product"),
            revision: optional_string(&result, "revision"),
            user_agent: optional_string(&result, "userAgent"),
            js_version: optional_string(&result, "jsVersion"),
            protocol_version: optional_string(&result, "protocolVersion"),
        })
    }

    fn send_page_command(&mut self, method: &str, params: Value) -> Result<Value, ChromiumError> {
        self.send_page_command_with_timeout(method, params, self.protocol_timeout)
    }

    fn send_page_command_with_timeout(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, ChromiumError> {
        let session_id = self.session_id.clone();
        self.send_command_with_timeout(method, params, Some(&session_id), timeout)
    }

    fn send_command(
        &mut self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, ChromiumError> {
        self.send_command_with_timeout(method, params, session_id, self.protocol_timeout)
    }

    fn send_command_with_timeout(
        &mut self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
        timeout: Duration,
    ) -> Result<Value, ChromiumError> {
        let timeout = timeout
            .max(Duration::from_millis(1))
            .min(Duration::from_secs(5 * 60));
        let id = self.next_id;
        self.next_id += 1;
        let mut message = json!({
            "id": id,
            "method": method,
            "params": params,
        });
        if let Some(session_id) = session_id {
            message["sessionId"] = Value::String(session_id.to_string());
        }
        self.socket
            .send(Message::Text(message.to_string().into()))
            .map_err(ChromiumError::WebSocket)?;

        let deadline = Instant::now() + timeout;
        loop {
            if Instant::now() >= deadline {
                return Err(ChromiumError::ProtocolTimeout {
                    operation: method.to_string(),
                    timeout,
                    diagnostics: self.diagnostic_summary(),
                });
            }
            let Some(value) = self.read_protocol_message(deadline)? else {
                continue;
            };
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = value.get("error") {
                    return Err(ChromiumError::Protocol {
                        message: format!("{method}: {error}. {}", self.diagnostic_summary()),
                    });
                }
                return Ok(value.get("result").cloned().unwrap_or(Value::Null));
            }
            if value.get("method").is_some() {
                self.queue_event(value);
            }
        }
    }

    fn queue_event(&mut self, event: Value) {
        const MAX_QUEUED_EVENTS: usize = 1024;
        const MAX_DIAGNOSTICS: usize = 256;
        if let Some(diagnostic) = browser_diagnostic_from_event(&event) {
            if self.diagnostics.len() == MAX_DIAGNOSTICS {
                self.diagnostics.pop_front();
            }
            self.diagnostics.push_back(diagnostic);
        }
        if self.events.len() == MAX_QUEUED_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    fn diagnostic_summary(&self) -> String {
        if self.diagnostics.is_empty() {
            return "no browser log, console, or runtime diagnostics captured".to_string();
        }
        self.diagnostics
            .iter()
            .rev()
            .take(12)
            .rev()
            .map(|diagnostic| {
                let location = diagnostic
                    .url
                    .as_deref()
                    .filter(|url| !url.is_empty())
                    .map(|url| {
                        let line = diagnostic.line.map(|value| value + 1).unwrap_or(0);
                        let column = diagnostic.column.map(|value| value + 1).unwrap_or(0);
                        if line > 0 {
                            format!(" {url}:{line}:{column}")
                        } else {
                            format!(" {url}")
                        }
                    })
                    .unwrap_or_default();
                format!(
                    "[{}:{}{}] {}",
                    diagnostic.kind, diagnostic.level, location, diagnostic.text
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn read_protocol_message(&mut self, deadline: Instant) -> Result<Option<Value>, ChromiumError> {
        loop {
            if Instant::now() >= deadline {
                return Ok(None);
            }
            match self.socket.read() {
                Ok(Message::Text(text)) => {
                    return serde_json::from_str(text.as_str())
                        .map(Some)
                        .map_err(|source| ChromiumError::InvalidProtocolJson {
                            source,
                            message: text.to_string(),
                        });
                }
                Ok(Message::Ping(payload)) => {
                    self.socket
                        .send(Message::Pong(payload))
                        .map_err(ChromiumError::WebSocket)?;
                }
                Ok(Message::Close(frame)) => {
                    return Err(ChromiumError::Protocol {
                        message: format!("Chromium closed the DevTools socket: {frame:?}"),
                    });
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(source))
                    if matches!(
                        source.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(source) => return Err(ChromiumError::WebSocket(source)),
            }
        }
    }
}

impl Drop for ChromiumSession {
    fn drop(&mut self) {
        stop_child(&mut self.child);
        let _ = fs::remove_dir_all(&self.profile_dir);
    }
}

#[derive(Debug, Error)]
pub(crate) enum ChromiumError {
    #[error(
        "Chrome/Chromium was not found. Install Google Chrome or Chromium, or set ZPRES_CHROMIUM to the executable path. Requested: {requested:?}. Searched: {searched:?}"
    )]
    MissingChromium {
        requested: Option<PathBuf>,
        searched: Vec<PathBuf>,
    },
    #[error("failed to create a temporary Chromium profile: {0}")]
    CreateProfile(io::Error),
    #[error("failed to create Chromium diagnostics file at {path}: {source}")]
    CreateDiagnostics { path: PathBuf, source: io::Error },
    #[error("failed to launch Chromium at {executable}: {source}")]
    Launch {
        executable: PathBuf,
        source: io::Error,
    },
    #[error("Chromium exited during startup with {status}. Diagnostics: {diagnostics}")]
    StartupExited {
        status: ExitStatus,
        diagnostics: String,
    },
    #[error("Chromium did not expose DevTools within {timeout:?}. Diagnostics: {diagnostics}")]
    StartupTimeout {
        timeout: Duration,
        diagnostics: String,
    },
    #[error("Chromium wrote an invalid DevToolsActivePort file at {path}: {contents:?}")]
    InvalidDevToolsPort { path: PathBuf, contents: String },
    #[error(
        "failed to connect to Chromium DevTools at {url}: {source}. Diagnostics: {diagnostics}"
    )]
    Connect {
        url: String,
        diagnostics: String,
        source: tungstenite::Error,
    },
    #[error("failed to configure the Chromium DevTools socket: {0}")]
    Socket(io::Error),
    #[error("Chromium DevTools websocket failed: {0}")]
    WebSocket(tungstenite::Error),
    #[error(
        "Chromium DevTools operation '{operation}' exceeded {timeout:?}. Diagnostics: {diagnostics}"
    )]
    ProtocolTimeout {
        operation: String,
        timeout: Duration,
        diagnostics: String,
    },
    #[error("Chromium DevTools protocol error: {message}")]
    Protocol { message: String },
    #[error("Chromium DevTools returned invalid JSON {message:?}: {source}")]
    InvalidProtocolJson {
        source: serde_json::Error,
        message: String,
    },
    #[error("Chromium could not navigate to {url}: {reason}")]
    Navigation { url: String, reason: String },
    #[error("no document is loaded in the Chromium target")]
    NoLoadedDocument,
    #[error("Chromium page evaluation failed: {message}")]
    Evaluation { message: String },
    #[error(
        "Chromium returned an invalid static-readiness state for {url}: {source}. Diagnostics: {diagnostics}"
    )]
    InvalidReadiness {
        source: serde_json::Error,
        url: String,
        diagnostics: String,
    },
    #[error(
        "Chromium returned an invalid print-page inventory for {url}: {source}. Diagnostics: {diagnostics}"
    )]
    InvalidPrintPages {
        source: serde_json::Error,
        url: String,
        diagnostics: String,
    },
    #[error(
        "Chromium returned an invalid print-page layout for {url}: {message}. Diagnostics: {diagnostics}"
    )]
    InvalidPrintPageLayout {
        message: String,
        url: String,
        diagnostics: String,
    },
    #[error(
        "Chromium returned an invalid interactive route inventory for {url}: {source}. Diagnostics: {diagnostics}"
    )]
    InvalidScreenRoutes {
        source: serde_json::Error,
        url: String,
        diagnostics: String,
    },
    #[error(
        "Chromium returned an invalid interactive route inventory for {url}: {message}. Diagnostics: {diagnostics}"
    )]
    InvalidScreenRouteInventory {
        message: String,
        url: String,
        diagnostics: String,
    },
    #[error(
        "Chromium returned an invalid active interactive route state for {url}: {source}. Diagnostics: {diagnostics}"
    )]
    InvalidScreenRouteState {
        source: serde_json::Error,
        url: String,
        diagnostics: String,
    },
    #[error(
        "interactive navigation activated a different state: expected {expected:?}, observed {observed:?}. Diagnostics: {diagnostics}"
    )]
    ScreenRouteMismatch {
        expected: Box<BrowserScreenRoute>,
        observed: Box<BrowserScreenRoute>,
        diagnostics: String,
    },
    #[error(
        "print page index {page_index} does not exist in the loaded Chromium document ({available_pages} page(s) available). Diagnostics: {diagnostics}"
    )]
    MissingPrintPage {
        page_index: usize,
        available_pages: usize,
        diagnostics: String,
    },
    #[error(
        "print page index {page_index} (slide '{slide_id}') has no positive screenshot bounds. Diagnostics: {diagnostics}"
    )]
    InvalidPageClip {
        page_index: usize,
        slide_id: String,
        diagnostics: String,
    },
    #[error(
        "Chromium returned an invalid visual observation for {url}: {source}. Diagnostics: {diagnostics}"
    )]
    InvalidObservation {
        source: serde_json::Error,
        url: String,
        diagnostics: String,
    },
    #[error("Chromium returned invalid base64 screenshot data: {0}")]
    ScreenshotDecode(base64::DecodeError),
    #[error("Chromium returned invalid base64 PDF data: {0}")]
    PdfDecode(base64::DecodeError),
}

impl ChromiumError {
    fn with_diagnostics(self, diagnostics: String) -> Self {
        match self {
            Self::StartupExited { status, .. } => Self::StartupExited {
                status,
                diagnostics,
            },
            Self::StartupTimeout { timeout, .. } => Self::StartupTimeout {
                timeout,
                diagnostics,
            },
            error => error,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct BrowserPageObservation {
    #[serde(default)]
    pub document_ready_state: String,
    #[serde(default)]
    pub autoscale_ready: bool,
    #[serde(default)]
    pub slide_present: bool,
    pub page: Option<usize>,
    #[serde(default)]
    pub slide_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub role: String,
    pub variant: Option<String>,
    pub generated: Option<String>,
    pub step_state: Option<String>,
    pub pdf_step: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_inspection: Option<BrowserDebugInspectionObservation>,
    pub slide_bounds: Option<BrowserRect>,
    pub canvas_bounds: Option<BrowserRect>,
    pub content_bounds: Option<BrowserRect>,
    pub screen_bounds: Option<BrowserRect>,
    pub root_bounds: Option<BrowserRect>,
    pub stage_bounds: Option<BrowserRect>,
    pub active_stack_bounds: Option<BrowserRect>,
    pub footer_bounds: Option<BrowserRect>,
    pub document_scroll: Option<BrowserOverflowDeltas>,
    pub active_stack_count: Option<usize>,
    pub active_slide_count: Option<usize>,
    pub visible_stack_count: Option<usize>,
    pub visible_slide_count: Option<usize>,
    pub routed_stack_visible: Option<bool>,
    pub routed_slide_visible: Option<bool>,
    pub screen_route: Option<String>,
    pub screen_section: Option<usize>,
    pub screen_detail: Option<usize>,
    pub screen_step: Option<usize>,
    pub screen_step_count: Option<usize>,
    pub content_union: Option<BrowserRect>,
    pub visible_content_bounds: Option<BrowserRect>,
    #[serde(default)]
    pub block_bounds: Vec<BrowserBlockBounds>,
    pub outside_canvas: Option<BrowserOverflowDeltas>,
    pub occupancy: Option<f64>,
    pub whitespace: Option<BrowserWhitespace>,
    pub autoscale_factor: Option<f64>,
    #[serde(default)]
    pub clip_marker: bool,
    #[serde(default)]
    pub unresolved: Vec<BrowserUnresolved>,
    #[serde(default)]
    pub images: Vec<BrowserImageObservation>,
    #[serde(default)]
    pub background_images: Vec<BrowserImageObservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authored_background: Option<BrowserAuthoredBackgroundObservation>,
    #[serde(default)]
    pub font_faces: Vec<BrowserFontFace>,
    #[serde(default)]
    pub text_styles: Vec<BrowserTextStyle>,
    /// Fonts Chrome actually used for visible direct TextNodes in the active
    /// Slide. Each TextNode parent contributes exactly once, so representative
    /// probes cannot double-count aggregate glyphs. Entries are deterministic.
    #[serde(default)]
    pub platform_fonts: Vec<BrowserPlatformFont>,
    /// A bounded representative sample of per-element requested CSS and actual
    /// platform-font use. A probe covers one visible element with a direct,
    /// non-whitespace TextNode; it does not contribute to aggregate counts.
    #[serde(default)]
    pub platform_font_probes: Vec<BrowserPlatformFontProbe>,
    #[serde(default)]
    pub platform_font_probe_count: usize,
    #[serde(default)]
    pub platform_font_candidate_count: usize,
    #[serde(default)]
    pub platform_font_probe_truncated: bool,
    #[serde(default)]
    pub platform_font_evidence: BrowserPlatformFontEvidence,
    pub min_visible_text_px: Option<f64>,
    pub min_body_text_px: Option<f64>,
    #[serde(default)]
    pub design_measurements: Vec<BrowserDesignMeasurement>,
    #[serde(default)]
    pub semantic_text_regions: Vec<BrowserSemanticTextRegion>,
    #[serde(default)]
    pub overflow_elements: Vec<BrowserElementOverflow>,
    #[serde(default)]
    pub geometry_violations: Vec<BrowserGeometryViolation>,
    #[serde(default)]
    pub step_visibility_violations: Vec<BrowserStepVisibilityViolation>,
    #[serde(default)]
    pub readiness_errors: Vec<String>,
    #[serde(default)]
    pub accessibility_preferences: BrowserAccessibilityPreferenceEvidence,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct BrowserDebugInspectionObservation {
    pub identity: String,
    pub role: String,
    pub route: String,
    pub target: String,
    #[serde(default)]
    pub step_summary: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub diagnostics: String,
    #[serde(default)]
    pub print_rail: String,
    #[serde(default)]
    pub painted_rail: String,
    #[serde(default)]
    pub regions: Vec<BrowserDebugRegionObservation>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct BrowserDebugRegionObservation {
    pub role: String,
    pub bounds: String,
    #[serde(default)]
    pub placement: String,
    pub label_bounds: Option<BrowserRect>,
    #[serde(default)]
    pub authored_text_intersections: Vec<BrowserRect>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct BrowserSemanticTextRegion {
    pub region: String,
    pub element: String,
    pub authored_text: String,
    #[serde(default)]
    pub text_bounds: Vec<BrowserRect>,
    pub raster_ink: Option<BrowserRasterInkObservation>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct BrowserRasterInkObservation {
    pub sampled_pixels: usize,
    pub ink_pixels: usize,
    pub ink_ratio: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct BrowserAccessibilityPreferenceEvidence {
    #[serde(default)]
    pub focus_visible: bool,
    #[serde(default)]
    pub reduced_motion_preserves_state: bool,
    #[serde(default)]
    pub increased_contrast_preserves_state: bool,
    #[serde(default)]
    pub forced_colors_preserves_state: bool,
    #[serde(default)]
    pub current_step_has_non_color_cue: bool,
    pub min_non_text_contrast: Option<f64>,
    #[serde(default)]
    pub non_color_state_cues_present: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct BrowserDesignMeasurement {
    pub element: String,
    pub type_role: String,
    pub content_role: String,
    pub surface: String,
    pub palette: String,
    pub slide_id: String,
    pub step: usize,
    pub text_excerpt: String,
    #[serde(default)]
    pub text_sample: bool,
    pub font_size_px: Option<f64>,
    pub font_weight: Option<u16>,
    #[serde(default)]
    pub wcag_large_text: bool,
    #[serde(default)]
    pub essential_content: bool,
    pub line_height_px: Option<f64>,
    pub cap_height_proxy_px: Option<f64>,
    pub line_count: Option<usize>,
    pub prose_characters: Option<usize>,
    pub prose_measure_ch: Option<f64>,
    pub occupancy: Option<f64>,
    pub contrast_ratio: Option<f64>,
    #[serde(default)]
    pub image_backed_text: bool,
    pub rendered_width: Option<f64>,
    pub rendered_height: Option<f64>,
    pub natural_width: Option<u32>,
    pub natural_height: Option<u32>,
    pub resolution_scale: Option<f64>,
    pub figure_alternative_status: Option<String>,
    #[serde(default)]
    pub actual_font_families: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct BrowserRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub right: f64,
    pub bottom: f64,
}

fn validate_print_page_inventory(pages: &[BrowserPrintPage]) -> Result<(), String> {
    let mut previous: Option<&BrowserPrintPage> = None;
    for (position, page) in pages.iter().enumerate() {
        let expected_page = position + 1;
        if page.index != position {
            return Err(format!(
                "page inventory entry {expected_page} reports zero-based index {} instead of {position}",
                page.index
            ));
        }
        if page.page != Some(expected_page) {
            return Err(format!(
                "page inventory entry {expected_page} (slide '{}') reports data-page {:?}",
                page.slide_id, page.page
            ));
        }
        let bounds = page.bounds;
        if ![
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            bounds.right,
            bounds.bottom,
        ]
        .into_iter()
        .all(f64::is_finite)
        {
            return Err(format!(
                "page {expected_page} (slide '{}') has non-finite browser bounds",
                page.slide_id
            ));
        }
        if bounds.width <= 0.0 || bounds.height <= 0.0 {
            return Err(format!(
                "page {expected_page} (slide '{}') has collapsed browser bounds {:.1}x{:.1}",
                page.slide_id, bounds.width, bounds.height
            ));
        }
        if (bounds.right - (bounds.x + bounds.width)).abs() > PRINT_PAGE_LAYOUT_TOLERANCE_PX
            || (bounds.bottom - (bounds.y + bounds.height)).abs() > PRINT_PAGE_LAYOUT_TOLERANCE_PX
        {
            return Err(format!(
                "page {expected_page} (slide '{}') reports internally inconsistent browser bounds",
                page.slide_id
            ));
        }
        if let Some(previous) = previous
            && bounds.y < previous.bounds.bottom - PRINT_PAGE_LAYOUT_TOLERANCE_PX
        {
            return Err(format!(
                "page {expected_page} (slide '{}') starts at y={:.1} before page {} (slide '{}') ends at y={:.1}; print pages must be ordered and non-overlapping",
                page.slide_id, bounds.y, position, previous.slide_id, previous.bounds.bottom
            ));
        }
        previous = Some(page);
    }
    Ok(())
}

fn validate_screen_route_inventory(routes: &[BrowserScreenRoute]) -> Result<(), String> {
    if routes.is_empty() {
        return Err(
            "the production presentation runtime returned no interactive routes".to_string(),
        );
    }

    let mut hashes = BTreeSet::new();
    let mut groups = BTreeMap::<(usize, usize, String), Vec<&BrowserScreenRoute>>::new();
    for route in routes {
        if route.hash.is_empty() || !route.hash.starts_with("#/") {
            return Err(format!(
                "slide '{}' returned invalid interactive hash {:?}",
                route.slide_id, route.hash
            ));
        }
        if !hashes.insert(route.hash.clone()) {
            return Err(format!(
                "the production presentation runtime returned duplicate hash '{}'",
                route.hash
            ));
        }
        if route.slide_id.is_empty() {
            return Err(format!(
                "interactive hash '{}' has no Slide identifier",
                route.hash
            ));
        }
        if route.role.is_empty() {
            return Err(format!(
                "interactive hash '{}' (slide '{}') has no Slide role",
                route.hash, route.slide_id
            ));
        }
        if route.step > route.step_count {
            return Err(format!(
                "interactive hash '{}' reports step {} beyond step count {}",
                route.hash, route.step, route.step_count
            ));
        }
        groups
            .entry((route.section, route.detail, route.slide_id.clone()))
            .or_default()
            .push(route);
    }

    for ((section, detail, slide_id), group) in groups {
        let first = group[0];
        if group.iter().any(|route| {
            route.step_count != first.step_count
                || route.role != first.role
                || route.generated != first.generated
        }) {
            return Err(format!(
                "interactive routes for slide '{slide_id}' at section {section}, detail {detail} disagree about role, generated state, or step count"
            ));
        }
        let observed_steps = group
            .iter()
            .map(|route| route.step)
            .collect::<BTreeSet<_>>();
        let expected_steps = (0..=first.step_count).collect::<BTreeSet<_>>();
        if observed_steps != expected_steps {
            return Err(format!(
                "interactive routes for slide '{slide_id}' at section {section}, detail {detail} report steps {observed_steps:?}; expected {expected_steps:?}"
            ));
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct BrowserBlockBounds {
    pub block_type: String,
    pub bounds: BrowserRect,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct BrowserOverflowDeltas {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl BrowserOverflowDeltas {
    pub(crate) fn maximum(self) -> f64 {
        self.left.max(self.top).max(self.right).max(self.bottom)
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct BrowserWhitespace {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct BrowserUnresolved {
    pub marker: String,
    pub excerpt: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct BrowserImageObservation {
    pub source: String,
    #[serde(default)]
    pub visible: bool,
    #[serde(default)]
    pub complete: bool,
    pub natural_width: u32,
    pub natural_height: u32,
    #[serde(default)]
    pub decoded: bool,
    pub error: Option<String>,
    pub bounds: Option<BrowserRect>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct BrowserAuthoredBackgroundObservation {
    pub phase: Option<String>,
    pub split: Option<String>,
    #[serde(default)]
    pub layer_count: usize,
    #[serde(default)]
    pub visible: bool,
    #[serde(default)]
    pub intersects_slide: bool,
    pub source: Option<String>,
    pub declared_image: Option<String>,
    pub computed_image: Option<String>,
    #[serde(default)]
    pub source_preserved: bool,
    pub bounds: Option<BrowserRect>,
    pub display: Option<String>,
    pub visibility: Option<String>,
    pub content_visibility: Option<String>,
    pub opacity: Option<String>,
    pub intent: Option<String>,
    #[serde(default)]
    pub semantic_layer_count: usize,
    #[serde(default)]
    pub short_alternative_present: bool,
    #[serde(default)]
    pub long_description_present: bool,
    #[serde(default)]
    pub decorative_hidden: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct BrowserFontFace {
    pub family: String,
    pub status: String,
    pub style: String,
    pub weight: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct BrowserTextStyle {
    pub family: String,
    pub size_px: f64,
    pub line_height: String,
    pub weight: String,
    pub count: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct BrowserPlatformFont {
    pub family: String,
    pub postscript_name: String,
    #[serde(default)]
    pub custom: bool,
    pub glyph_count: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct BrowserPlatformFontProbe {
    pub index: usize,
    pub element: String,
    pub text_excerpt: String,
    pub requested_family: String,
    pub requested_style: String,
    pub requested_weight: String,
    pub requested_font_synthesis: String,
    #[serde(default)]
    pub fonts: Vec<BrowserPlatformFont>,
    /// True means Chrome reported more than one platform face for this sampled
    /// element. The protocol does not establish why multiple faces were used.
    #[serde(default)]
    pub multiple_faces_observed: bool,
    /// Glyphs reported for faces other than the sampled element's face with
    /// greatest coverage. This is not a missing-glyph or fallback diagnosis.
    #[serde(default)]
    pub secondary_face_glyph_count: usize,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum BrowserFontEvidenceStatus {
    Observed,
    NotObserved,
    #[default]
    Unavailable,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct BrowserPlatformFontEvidence {
    #[serde(default)]
    pub actual_font_use: BrowserFontEvidenceStatus,
    #[serde(default)]
    pub multiple_faces_observed: BrowserFontEvidenceStatus,
    #[serde(default)]
    pub fallback_activation: BrowserFontEvidenceStatus,
    #[serde(default)]
    pub missing_glyph_cause: BrowserFontEvidenceStatus,
    #[serde(default)]
    pub synthesized_face_activation: BrowserFontEvidenceStatus,
    #[serde(default)]
    pub limitations: Vec<String>,
}

impl BrowserPlatformFontEvidence {
    fn from_fonts(fonts: &[BrowserPlatformFont]) -> Self {
        Self {
            actual_font_use: if fonts.is_empty() {
                BrowserFontEvidenceStatus::NotObserved
            } else {
                BrowserFontEvidenceStatus::Observed
            },
            multiple_faces_observed: if fonts.len() > 1 {
                BrowserFontEvidenceStatus::Observed
            } else {
                BrowserFontEvidenceStatus::NotObserved
            },
            // CSS.getPlatformFontsForNode reports chosen faces and glyph counts,
            // but not why a face was selected or whether one requested family
            // fell back to another. It also does not expose synthesized state.
            fallback_activation: BrowserFontEvidenceStatus::Unavailable,
            missing_glyph_cause: BrowserFontEvidenceStatus::Unavailable,
            synthesized_face_activation: BrowserFontEvidenceStatus::Unavailable,
            limitations: vec![
                "multiple observed faces do not prove that font fallback occurred"
                    .to_string(),
                "Chrome DevTools does not map platform-font glyph counts back to source characters"
                    .to_string(),
                "Chrome DevTools does not report why a requested family resolved to an actual face"
                    .to_string(),
                "Chrome DevTools does not report whether a bold or italic face was synthesized"
                    .to_string(),
                "the aggregate covers visible direct DOM text in the active Slide; representative probes cover at most 24 of those elements"
                    .to_string(),
                "platform-font evidence does not cover canvas or pseudo-element text"
                    .to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlatformFontBridge {
    probe_attribute: String,
    candidate_attribute: String,
}

impl PlatformFontBridge {
    fn new() -> Self {
        let sequence = PLATFORM_FONT_BRIDGE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let token = format!("{:x}-{nanos:x}-{sequence:x}", std::process::id());
        Self {
            probe_attribute: format!("{PLATFORM_FONT_PROBE_ATTRIBUTE_PREFIX}{token}"),
            candidate_attribute: format!("{PLATFORM_FONT_CANDIDATE_ATTRIBUTE_PREFIX}{token}"),
        }
    }

    fn probe_selector(&self) -> String {
        format!("[{}]", self.probe_attribute)
    }

    fn candidate_selector(&self) -> String {
        format!("[{}]", self.candidate_attribute)
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
struct BrowserPlatformFontProbeSetup {
    #[serde(default)]
    slide_present: bool,
    #[serde(default)]
    candidate_count: usize,
    #[serde(default)]
    selected_count: usize,
    #[serde(default)]
    truncated: bool,
    #[serde(default)]
    probes: Vec<BrowserPlatformFontProbeMetadata>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
struct BrowserPlatformFontProbeMetadata {
    index: usize,
    #[serde(default)]
    element: String,
    #[serde(default)]
    text_excerpt: String,
    #[serde(default)]
    requested_family: String,
    #[serde(default)]
    requested_style: String,
    #[serde(default)]
    requested_weight: String,
    #[serde(default)]
    requested_font_synthesis: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
struct CdpPlatformFontsResponse {
    #[serde(default)]
    fonts: Vec<CdpPlatformFontUsage>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
struct CdpPlatformFontUsage {
    #[serde(rename = "familyName", default)]
    family_name: String,
    #[serde(rename = "postScriptName", default)]
    postscript_name: String,
    #[serde(rename = "isCustomFont", default)]
    custom: bool,
    #[serde(rename = "glyphCount", default)]
    glyph_count: usize,
}

fn aggregate_platform_fonts(
    usages: impl IntoIterator<Item = CdpPlatformFontUsage>,
) -> Vec<BrowserPlatformFont> {
    let mut aggregated = BTreeMap::<(String, String, bool), usize>::new();
    for usage in usages {
        if usage.glyph_count == 0 {
            continue;
        }
        let key = (
            usage.family_name.trim().to_string(),
            usage.postscript_name.trim().to_string(),
            usage.custom,
        );
        let glyph_count = aggregated.entry(key).or_default();
        *glyph_count = glyph_count.saturating_add(usage.glyph_count);
    }
    aggregated
        .into_iter()
        .map(
            |((family, postscript_name, custom), glyph_count)| BrowserPlatformFont {
                family,
                postscript_name,
                custom,
                glyph_count,
            },
        )
        .collect()
}

fn platform_font_node_ids(
    response: &Value,
    message: &str,
    session: &ChromiumSession,
) -> Result<Vec<u64>, ChromiumError> {
    response
        .get("nodeIds")
        .and_then(Value::as_array)
        .map(|node_ids| node_ids.iter().filter_map(Value::as_u64).collect())
        .ok_or_else(|| ChromiumError::Protocol {
            message: format!("{message}: {response}. {}", session.diagnostic_summary()),
        })
}

fn secondary_face_glyph_count(fonts: &[BrowserPlatformFont]) -> usize {
    let total = fonts
        .iter()
        .fold(0usize, |total, font| total.saturating_add(font.glyph_count));
    let primary = fonts
        .iter()
        .map(|font| font.glyph_count)
        .max()
        .unwrap_or_default();
    total.saturating_sub(primary)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct BrowserElementOverflow {
    pub element: String,
    pub horizontal_px: f64,
    pub vertical_px: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct BrowserGeometryViolation {
    pub kind: String,
    pub element: String,
    pub boundary: String,
    pub element_bounds: BrowserRect,
    pub boundary_bounds: BrowserRect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intersection_bounds: Option<BrowserRect>,
    pub deltas: BrowserOverflowDeltas,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct BrowserStepVisibilityViolation {
    pub kind: String,
    pub element: String,
    pub step_index: Option<usize>,
    pub expected_visible: bool,
    pub visible: bool,
}

#[derive(Debug)]
struct DevToolsAddress {
    port: u16,
    browser_path: String,
}

fn wait_for_devtools(
    child: &mut Child,
    path: &Path,
    timeout: Duration,
) -> Result<DevToolsAddress, ChromiumError> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(ChromiumError::CreateProfile)? {
            return Err(ChromiumError::StartupExited {
                status,
                diagnostics: "see Chromium stderr".to_string(),
            });
        }
        if let Ok(contents) = fs::read_to_string(path) {
            let mut lines = contents.lines();
            let port = lines.next().and_then(|line| line.parse::<u16>().ok());
            let browser_path = lines.next().filter(|line| line.starts_with('/'));
            if let (Some(port), Some(browser_path)) = (port, browser_path) {
                return Ok(DevToolsAddress {
                    port,
                    browser_path: browser_path.to_string(),
                });
            }
            if started.elapsed() >= timeout {
                return Err(ChromiumError::InvalidDevToolsPort {
                    path: path.to_path_buf(),
                    contents,
                });
            }
        }
        if started.elapsed() >= timeout {
            return Err(ChromiumError::StartupTimeout {
                timeout,
                diagnostics: "see Chromium stderr".to_string(),
            });
        }
        thread::sleep(DEVTOOLS_POLL_INTERVAL);
    }
}

fn create_profile_dir() -> Result<PathBuf, ChromiumError> {
    let root = fixed_profile_root()?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for attempt in 0..32 {
        let path = root.join(format!(
            "zpres-chromium-{}-{nonce}-{attempt}",
            std::process::id()
        ));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&path) {
            Ok(()) => return Ok(path),
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(ChromiumError::CreateProfile(source)),
        }
    }
    Err(ChromiumError::CreateProfile(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique profile directory",
    )))
}

fn fixed_profile_root() -> Result<PathBuf, ChromiumError> {
    let root = fs::canonicalize("/tmp").map_err(ChromiumError::CreateProfile)?;
    let metadata = fs::symlink_metadata(&root).map_err(ChromiumError::CreateProfile)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ChromiumError::CreateProfile(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the fixed Chromium profile root is not a directory",
        )));
    }
    Ok(root)
}

fn set_socket_timeout(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    timeout: Duration,
) -> Result<(), ChromiumError> {
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => {
            stream
                .set_read_timeout(Some(timeout.min(Duration::from_millis(250))))
                .map_err(ChromiumError::Socket)?;
            stream
                .set_write_timeout(Some(timeout))
                .map_err(ChromiumError::Socket)?;
        }
        _ => {
            return Err(ChromiumError::Protocol {
                message: "unexpected TLS stream for local Chromium DevTools".to_string(),
            });
        }
    }
    Ok(())
}

fn read_stderr(path: &Path) -> String {
    fs::read(path)
        .map(|captured| {
            String::from_utf8_lossy(&captured[..captured.len().min(STDERR_CAPTURE_LIMIT)])
                .into_owned()
        })
        .unwrap_or_default()
}

fn stop_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
}

fn required_string(value: &Value, name: &str) -> Result<String, ChromiumError> {
    value
        .get(name)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| ChromiumError::Protocol {
            message: format!("missing string field '{name}' in {value}"),
        })
}

fn optional_string(value: &Value, name: &str) -> String {
    value
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn browser_diagnostic_from_event(event: &Value) -> Option<BrowserDiagnostic> {
    match event.get("method").and_then(Value::as_str)? {
        "Log.entryAdded" => {
            let entry = event.pointer("/params/entry")?;
            Some(BrowserDiagnostic {
                kind: "log".to_string(),
                level: optional_string(entry, "level"),
                text: optional_string(entry, "text"),
                url: optional_nonempty_string(entry, "url"),
                line: entry.get("lineNumber").and_then(Value::as_u64),
                column: entry.get("columnNumber").and_then(Value::as_u64),
            })
        }
        "Runtime.consoleAPICalled" => {
            let params = event.get("params")?;
            let text = params
                .get("args")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(remote_object_text)
                .collect::<Vec<_>>()
                .join(" ");
            let frame = params
                .pointer("/stackTrace/callFrames/0")
                .unwrap_or(&Value::Null);
            Some(BrowserDiagnostic {
                kind: "console".to_string(),
                level: optional_string(params, "type"),
                text,
                url: optional_nonempty_string(frame, "url"),
                line: frame.get("lineNumber").and_then(Value::as_u64),
                column: frame.get("columnNumber").and_then(Value::as_u64),
            })
        }
        "Runtime.exceptionThrown" => {
            let details = event.pointer("/params/exceptionDetails")?;
            let exception = details
                .get("exception")
                .map(remote_object_text)
                .filter(|text| !text.is_empty());
            let text = exception.unwrap_or_else(|| optional_string(details, "text"));
            Some(BrowserDiagnostic {
                kind: "runtime".to_string(),
                level: "error".to_string(),
                text,
                url: optional_nonempty_string(details, "url"),
                line: details.get("lineNumber").and_then(Value::as_u64),
                column: details.get("columnNumber").and_then(Value::as_u64),
            })
        }
        _ => None,
    }
}

fn optional_nonempty_string(value: &Value, name: &str) -> Option<String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
}

fn remote_object_text(value: &Value) -> String {
    value
        .get("value")
        .map(|value| match value {
            Value::String(text) => text.clone(),
            value => value.to_string(),
        })
        .or_else(|| {
            value
                .get("description")
                .and_then(Value::as_str)
                .map(String::from)
        })
        .or_else(|| {
            value
                .get("unserializableValue")
                .and_then(Value::as_str)
                .map(String::from)
        })
        .unwrap_or_else(|| optional_string(value, "type"))
}

const AWAIT_STATIC_READINESS_JS: &str = r#"
(async () => {
  const timeoutMs = __ZPRES_TIMEOUT_MS__;
  const body = document.body;
  const promisePresent = Boolean(window.zpresStaticReady && typeof window.zpresStaticReady.then === "function");
  let promiseStatus = "missing";
  let state = window.zpresStaticReadyState || null;
  const errors = [];
  const describeError = (error) => {
    if (typeof error === "string") return error;
    if (!error || typeof error !== "object") return String(error);
    const parts = [error.stage, error.kind, error.message].filter(Boolean);
    return parts.length ? parts.join(": ") : JSON.stringify(error);
  };

  if (!promisePresent) {
    errors.push("window.zpresStaticReady is missing or is not Promise-like");
  } else {
    const timedOut = { kind: "timed-out" };
    const settled = await Promise.race([
      Promise.resolve(window.zpresStaticReady).then(
        (value) => ({ kind: "fulfilled", value }),
        (error) => ({ kind: "rejected", error }),
      ),
      new Promise((resolve) => setTimeout(() => resolve(timedOut), timeoutMs)),
    ]);
    if (settled === timedOut || settled.kind === "timed-out") {
      promiseStatus = "timed-out";
      state = window.zpresStaticReadyState || state;
      errors.push("window.zpresStaticReady did not settle within " + timeoutMs + "ms");
    } else if (settled.kind === "rejected") {
      promiseStatus = "rejected";
      state = settled.error?.details || window.zpresStaticReadyError || window.zpresStaticReadyState || state;
      errors.push(describeError(settled.error));
    } else {
      promiseStatus = "fulfilled";
      state = settled.value ?? window.zpresStaticReadyState ?? state;
    }
  }

  if (!state || typeof state !== "object") state = {};
  if (Array.isArray(state.errors)) errors.push(...state.errors.map(describeError));
  const stateStatus = typeof state.status === "string" ? state.status : null;
  const bodyReady = body?.getAttribute("data-zpres-ready") === "true";
  const stateReady = typeof state.ready === "boolean" ? state.ready : stateStatus === "ready";
  const ready = promiseStatus === "fulfilled" && (stateReady || bodyReady);
  const rawPageCount = state.pageCount ?? state.page_count ?? body?.getAttribute("data-zpres-page-count");
  const parsedPageCount = Number(rawPageCount);
  const hasPageCount = rawPageCount !== null && rawPageCount !== undefined && rawPageCount !== "";
  const target = state.target ?? body?.getAttribute("data-zpres-ready-target") ?? null;
  return {
    promise_present: promisePresent,
    promise_status: promiseStatus,
    document_ready_state: document.readyState,
    ready,
    target: target === null ? null : String(target),
    declared_page_count: hasPageCount && Number.isFinite(parsedPageCount) ? parsedPageCount : null,
    observed_page_count: document.querySelectorAll(".zpres-print-slide").length,
    errors,
    state,
  };
})()
"#;

const SCREEN_ROUTES_JS: &str = r#"
(async () => {
  const presentation = window.zpresPresentation;
  if (!presentation || typeof presentation.routes !== "function") {
    throw new Error("window.zpresPresentation.routes() is unavailable");
  }
  if (presentation.ready && typeof presentation.ready.then === "function") {
    await presentation.ready;
  }
  const routes = await Promise.resolve(presentation.routes());
  if (!Array.isArray(routes)) {
    throw new Error("window.zpresPresentation.routes() did not return an array");
  }
  return routes;
})()
"#;

const SCREEN_NAVIGATE_JS: &str = r#"
(async () => {
  const presentation = window.zpresPresentation;
  if (!presentation || typeof presentation.navigate !== "function") {
    throw new Error("window.zpresPresentation.navigate() is unavailable");
  }
  return await presentation.navigate({
    section: __ZPRES_SCREEN_SECTION__,
    detail: __ZPRES_SCREEN_DETAIL__,
    step: __ZPRES_SCREEN_STEP__,
  });
})()
"#;

const PRINT_PAGES_JS: &str = r#"
(() => Array.from(document.querySelectorAll(".zpres-print-slide")).map((slide, index) => {
  const bounds = slide.getBoundingClientRect();
  const round = (value) => Math.round(value * 1000) / 1000;
  return {
    index,
    page: slide.dataset.page ? Number(slide.dataset.page) : null,
    slide_id: slide.dataset.slideId || "",
    role: slide.dataset.slideRole || "",
    bounds: {
      x: round(bounds.x), y: round(bounds.y), width: round(bounds.width), height: round(bounds.height),
      right: round(bounds.right), bottom: round(bounds.bottom),
    },
  };
}))()
"#;

const DOCUMENT_READY_JS: &str = r#"
(async () => {
  const errors = [];
  const withTimeout = (promise, label) => Promise.race([
    promise,
    new Promise((_, reject) => setTimeout(() => reject(new Error(label + " timed out")), 7000)),
  ]);
  try { await withTimeout(document.fonts.ready, "fonts"); } catch (error) { errors.push(String(error)); }
  await Promise.all(Array.from(document.images).map(async (image) => {
    try {
      if (!image.complete) {
        await withTimeout(new Promise((resolve, reject) => {
          image.addEventListener("load", resolve, { once: true });
          image.addEventListener("error", () => reject(new Error("image failed: " + image.currentSrc)), { once: true });
        }), "image");
      }
      if (image.naturalWidth === 0) throw new Error("image has no natural size: " + image.currentSrc);
      await withTimeout(image.decode(), "image decode");
    } catch (error) { errors.push(String(error)); }
  }));
  if (window.zpresAutoscaleAll) {
    try { await withTimeout(Promise.resolve(window.zpresAutoscaleAll()), "autoscale"); }
    catch (error) { errors.push(String(error)); }
  }
  await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  return { errors };
})()
"#;

fn platform_font_probes_js(max_probes: usize, bridge: &PlatformFontBridge) -> String {
    PLATFORM_FONT_PROBES_JS
        .replace("__ZPRES_MAX_FONT_PROBES__", &max_probes.to_string())
        .replace("__ZPRES_PROBE_ATTRIBUTE__", &bridge.probe_attribute)
        .replace("__ZPRES_CANDIDATE_ATTRIBUTE__", &bridge.candidate_attribute)
}

fn clear_platform_font_probes_js(bridge: &PlatformFontBridge) -> String {
    CLEAR_PLATFORM_FONT_PROBES_JS
        .replace("__ZPRES_PROBE_ATTRIBUTE__", &bridge.probe_attribute)
        .replace("__ZPRES_CANDIDATE_ATTRIBUTE__", &bridge.candidate_attribute)
}

const PLATFORM_FONT_PROBES_JS: &str = r##"
(() => {
  const probeAttribute = "__ZPRES_PROBE_ATTRIBUTE__";
  const candidateAttribute = "__ZPRES_CANDIDATE_ATTRIBUTE__";
  const clearAttributes = () => {
    for (const element of document.querySelectorAll("[" + probeAttribute + "]")) {
      element.removeAttribute(probeAttribute);
    }
    for (const element of document.querySelectorAll("[" + candidateAttribute + "]")) {
      element.removeAttribute(candidateAttribute);
    }
  };
  clearAttributes();

  try {
    const observationMode = window.__zpresVisualObservationMode === "screen" ? "screen" : "print";
    const requestedPageIndex = Number(window.__zpresRequestedPrintPageIndex ?? 0);
    const slides = observationMode === "screen"
      ? Array.from(document.querySelectorAll(".zpres-slide.is-active, .debug-slide.is-active"))
      : Array.from(document.querySelectorAll(".zpres-print-slide, .debug-print-slide"));
    const slide = observationMode === "screen"
      ? (slides.length === 1 ? slides[0] : null)
      : (Number.isInteger(requestedPageIndex) ? (slides[requestedPageIndex] || null) : null);
    if (!slide) return {
      slide_present: false, candidate_count: 0, selected_count: 0, truncated: false, probes: [],
    };
    const slideBounds = slide.getBoundingClientRect();
    const isVisible = (element) => {
      if (!(element instanceof Element)) return false;
      let current = element;
      while (current instanceof Element) {
        const style = getComputedStyle(current);
        if (style.display === "none" || style.visibility === "hidden"
          || style.visibility === "collapse" || style.contentVisibility === "hidden"
          || Number(style.opacity || 1) === 0) return false;
        current = current.parentElement;
      }
      const bounds = element.getBoundingClientRect();
      return bounds.width > 0 && bounds.height > 0
        && bounds.right > slideBounds.left && bounds.left < slideBounds.right
        && bounds.bottom > slideBounds.top && bounds.top < slideBounds.bottom;
    };
    const hasDirectText = (element) => Array.from(element.childNodes).some((node) =>
      node.nodeType === Node.TEXT_NODE && /\S/.test(node.textContent || ""));
    const describe = (element) => {
      const id = element.id ? "#" + element.id : "";
      const classes = Array.from(element.classList).slice(0, 3)
        .map((value) => "." + value).join("");
      return element.tagName.toLowerCase() + id + classes;
    };
    const directText = (element) => Array.from(element.childNodes)
      .filter((node) => node.nodeType === Node.TEXT_NODE)
      .map((node) => node.textContent || "").join(" ")
      .trim().replace(/\s+/g, " ");
    const candidates = [slide, ...Array.from(slide.querySelectorAll("*"))]
      .filter((element) => hasDirectText(element) && isVisible(element));
    candidates.forEach((element, index) => {
      element.setAttribute(candidateAttribute, String(index));
    });
    const sampleSize = Math.min(candidates.length, __ZPRES_MAX_FONT_PROBES__);
    const selected = sampleSize === candidates.length
      ? candidates
      : Array.from({ length: sampleSize }, (_, index) => {
          if (sampleSize === 1) return candidates[0];
          const candidateIndex = Math.round(index * (candidates.length - 1) / (sampleSize - 1));
          return candidates[candidateIndex];
        });
    const probes = selected.map((element, index) => {
      element.setAttribute(probeAttribute, String(index));
      const style = getComputedStyle(element);
      const text = directText(element);
      return {
        index,
        element: describe(element),
        text_excerpt: text.length > 160 ? text.slice(0, 159) + "…" : text,
        requested_family: style.fontFamily || "",
        requested_style: style.fontStyle || "",
        requested_weight: style.fontWeight || "",
        requested_font_synthesis: style.fontSynthesis || "",
      };
    });
    return {
      slide_present: true,
      candidate_count: candidates.length,
      selected_count: selected.length,
      truncated: candidates.length > selected.length,
      probes,
    };
  } catch (error) {
    clearAttributes();
    throw error;
  }
})()
"##;

const CLEAR_PLATFORM_FONT_PROBES_JS: &str = r#"
(() => {
  const probeAttribute = "__ZPRES_PROBE_ATTRIBUTE__";
  const candidateAttribute = "__ZPRES_CANDIDATE_ATTRIBUTE__";
  for (const element of document.querySelectorAll("[" + probeAttribute + "]")) {
    element.removeAttribute(probeAttribute);
  }
  for (const element of document.querySelectorAll("[" + candidateAttribute + "]")) {
    element.removeAttribute(candidateAttribute);
  }
  return true;
})()
"#;

#[cfg(test)]
const COUNT_PLATFORM_FONT_BRIDGE_ATTRIBUTES_JS: &str = r#"
(() => {
  let count = 0;
  for (const element of document.querySelectorAll("*")) {
    for (const name of element.getAttributeNames()) {
      if (name.startsWith("data-zpres-platform-font-probe-")
          || name.startsWith("data-zpres-platform-font-candidate-")) count += 1;
    }
  }
  return count;
})()
"#;

const VISUAL_OBSERVATION_JS: &str = r##"
(async () => {
  const observationMode = window.__zpresVisualObservationMode === "screen" ? "screen" : "print";
  const readinessErrors = [];
  const geometryViolations = [];
  const geometryViolationKeys = new Set();
  const stepVisibilityViolations = [];
  const MAX_GEOMETRY_VIOLATIONS = 64;
  // Text Range boxes can extend a few subpixels beyond their line box because
  // of glyph metrics. Four pixels filters that noise while retaining layout
  // crossings large enough to threaten a presentation boundary.
  const GEOMETRY_TOLERANCE = 4;
  const withTimeout = (promise, label) => Promise.race([
    promise,
    new Promise((_, reject) => setTimeout(() => reject(new Error(label + " timed out")), 7000)),
  ]);
  const round = (value) => Math.round(value * 1000) / 1000;
  const rect = (value) => value ? ({
    x: round(value.x), y: round(value.y), left: round(value.left), top: round(value.top),
    width: round(value.width), height: round(value.height),
    right: round(value.right), bottom: round(value.bottom),
  }) : null;
  const isVisible = (element) => {
    if (!(element instanceof Element)) return false;
    const elementStyle = getComputedStyle(element);
    if (elementStyle.display === "none"
      || elementStyle.visibility === "hidden" || elementStyle.visibility === "collapse"
      || elementStyle.contentVisibility === "hidden" || Number(elementStyle.opacity || 1) === 0) {
      return false;
    }
    let current = element.parentElement;
    while (current instanceof Element) {
      const style = getComputedStyle(current);
      if (style.display === "none" || style.contentVisibility === "hidden"
        || Number(style.opacity || 1) === 0) {
        return false;
      }
      current = current.parentElement;
    }
    const bounds = element.getBoundingClientRect();
    return bounds.width > 0 && bounds.height > 0;
  };
  const subtreeHasVisiblePixels = (element) => isVisible(element)
    || Array.from(element?.querySelectorAll("*") || []).some(isVisible);
  const describe = (element) => {
    if (!(element instanceof Element)) return "unknown";
    const id = element.id ? "#" + element.id : "";
    const classes = Array.from(element.classList).slice(0, 3).map((value) => "." + value).join("");
    const step = element.hasAttribute("data-step-index")
      ? "[data-step-index=\"" + element.getAttribute("data-step-index") + "\"]"
      : "";
    return element.tagName.toLowerCase() + id + classes + step;
  };
  const compactSource = (source) => source.length > 320
    ? source.slice(0, 240) + "… (" + source.length + " characters)"
    : source;
  const unionRects = (elements) => {
    const bounds = elements.filter(isVisible).map((element) => element.getBoundingClientRect());
    return unionBounds(bounds);
  };
  const unionBounds = (bounds) => {
    if (!bounds.length) return null;
    const left = Math.min(...bounds.map((value) => value.left));
    const top = Math.min(...bounds.map((value) => value.top));
    const right = Math.max(...bounds.map((value) => value.right));
    const bottom = Math.max(...bounds.map((value) => value.bottom));
    return { x: left, y: top, left, top, right, bottom, width: right - left, height: bottom - top };
  };
  const outsideDeltas = (subject, boundary) => ({
    left: round(Math.max(0, boundary.left - subject.left)),
    top: round(Math.max(0, boundary.top - subject.top)),
    right: round(Math.max(0, subject.right - boundary.right)),
    bottom: round(Math.max(0, subject.bottom - boundary.bottom)),
  });
  const maximumDelta = (deltas) => Math.max(deltas.left, deltas.top, deltas.right, deltas.bottom);
  const intersect = (first, second) => {
    const left = Math.max(first.left, second.left);
    const top = Math.max(first.top, second.top);
    const right = Math.min(first.right, second.right);
    const bottom = Math.min(first.bottom, second.bottom);
    const width = Math.max(0, right - left);
    const height = Math.max(0, bottom - top);
    return { x: left, y: top, left, top, right, bottom, width, height };
  };
  const textNodeBounds = (region) => {
    const bounds = [];
    const walker = document.createTreeWalker(region, NodeFilter.SHOW_TEXT);
    let node;
    while ((node = walker.nextNode())) {
      if (!node.nodeValue.trim() || !node.parentElement || !isVisible(node.parentElement)
        || node.parentElement.closest(".zpres-debug-boundary-label, .zpres-block-label")) continue;
      const range = document.createRange();
      range.selectNodeContents(node);
      for (const value of Array.from(range.getClientRects())) {
        if (value.width > 0 && value.height > 0) bounds.push(value);
      }
    }
    return bounds;
  };
  const authoredRegionText = (region) => {
    const fragments = [];
    const walker = document.createTreeWalker(region, NodeFilter.SHOW_TEXT);
    let node;
    while ((node = walker.nextNode())) {
      if (!node.nodeValue.trim() || !node.parentElement
        || node.parentElement.closest(".zpres-debug-boundary-label, .zpres-block-label")) continue;
      // Unrevealed Steps contain authored text, but owe no visible ink yet.
      const step = node.parentElement.closest('.zpres-step, .zpres-code-line');
      if (step && (observationMode === 'screen'
        ? Number(step.dataset.stepIndex || 0) > Number(activeRoute?.step || 0)
        : step.classList.contains('zpres-print-step-hidden'))) continue;
      fragments.push(node.nodeValue.trim());
    }
    return fragments.join(' ').replace(/\s+/g, ' ').slice(0, 320);
  };
  const addViolation = (kind, element, elementBounds, boundary, boundaryBounds, deltas, intersectionBounds = null) => {
    if (!elementBounds || !boundaryBounds || maximumDelta(deltas) <= GEOMETRY_TOLERANCE) return;
    const key = [kind, element, boundary, round(elementBounds.left), round(elementBounds.top),
      round(elementBounds.right), round(elementBounds.bottom)].join("|");
    if (geometryViolationKeys.has(key) || geometryViolations.length >= MAX_GEOMETRY_VIOLATIONS) return;
    geometryViolationKeys.add(key);
    geometryViolations.push({
      kind, element, boundary,
      element_bounds: rect(elementBounds),
      boundary_bounds: rect(boundaryBounds),
      intersection_bounds: rect(intersectionBounds),
      deltas,
    });
  };
  const clipsDescendants = (element) => {
    if (!(element instanceof Element)) return false;
    const style = getComputedStyle(element);
    return style.overflowX !== "visible" || style.overflowY !== "visible";
  };
  const containmentBoundary = (subject) => {
    let boundary = subject.element.closest(".zpres-block, .zpres-layout-region");
    if (boundary === subject.element) {
      boundary = subject.element.parentElement?.closest(".zpres-block, .zpres-layout-region") || null;
    }
    while (boundary) {
      if (boundary.matches(".zpres-layout-region") || !subject.text || clipsDescendants(boundary)) {
        return boundary;
      }
      boundary = boundary.parentElement?.closest(".zpres-block, .zpres-layout-region") || null;
    }
    return null;
  };

  const requestedPageIndex = Number(window.__zpresRequestedPrintPageIndex ?? 0);
  const printSlides = Array.from(document.querySelectorAll(".zpres-print-slide, .debug-print-slide"));
  const presentationStacks = observationMode === "screen"
    ? Array.from(document.querySelectorAll(".zpres-section-stack, .debug-section-stack"))
    : [];
  const presentationSlides = observationMode === "screen"
    ? Array.from(document.querySelectorAll(".zpres-slide, .debug-slide"))
    : [];
  const activeStacks = observationMode === "screen"
    ? Array.from(document.querySelectorAll(".zpres-section-stack.is-active, .debug-section-stack.is-active"))
    : [];
  const activeSlides = observationMode === "screen"
    ? Array.from(document.querySelectorAll(".zpres-slide.is-active, .debug-slide.is-active"))
    : [];
  const slide = observationMode === "screen"
    ? (activeSlides.length === 1 ? activeSlides[0] : null)
    : (Number.isInteger(requestedPageIndex) ? (printSlides[requestedPageIndex] || null) : null);
  const activeStack = activeStacks.length === 1 ? activeStacks[0] : null;
  const activeRoute = observationMode === "screen" ? (window.__zpresActiveScreenRoute || null) : null;
  const root = observationMode === "screen" ? document.querySelector(".reveal") : null;
  const stage = observationMode === "screen" ? root?.querySelector(".slides") || null : null;
  const slideElements = slide ? [slide, ...Array.from(slide.querySelectorAll("*"))] : [];

  try { await withTimeout(document.fonts.ready, "fonts"); } catch (error) { readinessErrors.push(String(error)); }

  const imageResults = await Promise.all((slide ? Array.from(slide.querySelectorAll("img")) : []).map(async (image) => {
    let decoded = false;
    let error = null;
    try {
      if (!image.complete) {
        await withTimeout(new Promise((resolve, reject) => {
          image.addEventListener("load", resolve, { once: true });
          image.addEventListener("error", () => reject(new Error("load failed")), { once: true });
        }), "image " + (image.currentSrc || image.src));
      }
      if (image.naturalWidth === 0) throw new Error("image has no natural size");
      await withTimeout(image.decode(), "decode " + (image.currentSrc || image.src));
      decoded = true;
    } catch (caught) {
      error = String(caught);
    }
    return {
      source: compactSource(image.currentSrc || image.src || ""),
      visible: isVisible(image), complete: image.complete,
      natural_width: image.naturalWidth || 0, natural_height: image.naturalHeight || 0,
      decoded, error, bounds: isVisible(image) ? rect(image.getBoundingClientRect()) : null,
    };
  }));

  const backgroundUrls = (value) => {
    const sources = [];
    const pattern = /url\((?:"([^"]*)"|'([^']*)'|([^)]*))\)/g;
    let match;
    while ((match = pattern.exec(value || "")) !== null) {
      const raw = (match[1] || match[2] || match[3] || "").trim();
      if (!raw) continue;
      let source = raw;
      try { source = new URL(raw, document.baseURI).href; } catch (_) {}
      sources.push(source);
    }
    return sources;
  };
  const backgroundSources = new Map();
  const collectBackground = (owner, pseudo) => {
    let style;
    try { style = getComputedStyle(owner, pseudo); } catch (_) { return; }
    for (const source of backgroundUrls(style.backgroundImage)) {
      if (!backgroundSources.has(source)) backgroundSources.set(source, { owner, pseudo });
    }
  };
  for (const element of slideElements) {
    collectBackground(element, null);
    collectBackground(element, "::before");
    collectBackground(element, "::after");
  }
  const backgroundResults = await Promise.all(Array.from(backgroundSources.entries()).map(async ([source, context]) => {
    const image = new Image();
    let decoded = false;
    let error = null;
    try {
      await withTimeout(new Promise((resolve, reject) => {
        image.addEventListener("load", resolve, { once: true });
        image.addEventListener("error", () => reject(new Error("background load failed")), { once: true });
        image.src = source;
      }), "background " + source);
      if (image.naturalWidth === 0) throw new Error("background has no natural size");
      await withTimeout(image.decode(), "background decode " + source);
      decoded = true;
    } catch (caught) { error = String(caught); }
    return {
      source: compactSource(source), visible: isVisible(context.owner), complete: image.complete,
      natural_width: image.naturalWidth || 0, natural_height: image.naturalHeight || 0,
      decoded, error, bounds: isVisible(context.owner) ? rect(context.owner.getBoundingClientRect()) : null,
    };
  }));

  let authoredBackground = null;
  if (document.body?.getAttribute("data-zpres-theme-api") === "1" && slide) {
    const layers = Array.from(slide.children).filter((element) =>
      element.matches(".zpres-slide-background"));
    const layer = layers[0] || null;
    const semanticLayers = Array.from(slide.children).filter((element) =>
      element.matches(".zpres-background-semantic"));
    const semanticLayer = semanticLayers[0] || null;
    const semanticImage = semanticLayer?.querySelector("img") || null;
    const layerStyle = layer ? getComputedStyle(layer) : null;
    const layerBounds = layer ? layer.getBoundingClientRect() : null;
    const currentSlideBounds = slide.getBoundingClientRect();
    const overlap = layerBounds ? intersect(layerBounds, currentSlideBounds) : { width: 0, height: 0 };
    const declaredImage = layer?.style.backgroundImage || "";
    const computedImage = layerStyle?.backgroundImage || "";
    const declaredSources = backgroundUrls(declaredImage);
    const computedSources = new Set(backgroundUrls(computedImage));
    authoredBackground = {
      phase: slide.getAttribute("data-background-phase"),
      split: slide.getAttribute("data-background-split"),
      layer_count: layers.length,
      visible: Boolean(layer && isVisible(layer)),
      intersects_slide: overlap.width > 0 && overlap.height > 0,
      source: layer?.getAttribute("data-zpres-background-source") || null,
      declared_image: declaredImage || null,
      computed_image: computedImage || null,
      source_preserved: layers.length === 1 && declaredSources.length > 0
        && declaredSources.every((source) => computedSources.has(source)),
      bounds: layerBounds ? rect(layerBounds) : null,
      display: layerStyle?.display || null,
      visibility: layerStyle?.visibility || null,
      content_visibility: layerStyle?.contentVisibility || null,
      opacity: layerStyle?.opacity || null,
      intent: semanticLayer?.getAttribute("data-zpres-background-intent")
        || (layer?.getAttribute("aria-hidden") === "true" ? "decorative" : null),
      semantic_layer_count: semanticLayers.length,
      short_alternative_present: Boolean(semanticImage?.getAttribute("alt")?.trim()),
      long_description_present: Boolean(semanticLayer?.querySelector("figcaption")?.textContent?.trim()),
      decorative_hidden: layer?.getAttribute("aria-hidden") === "true" && semanticLayers.length === 0,
    };
  }

  if (window.zpresAutoscaleAll) {
    try { await withTimeout(Promise.resolve(window.zpresAutoscaleAll()), "autoscale"); }
    catch (error) { readinessErrors.push(String(error)); }
  }
  await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));

  const canvas = slide?.querySelector(".zpres-slide-canvas, .debug-slide-canvas") || null;
  const content = canvas?.querySelector(".zpres-slide-content") || null;
  const footer = slide?.querySelector(".zpres-slide-footer") || null;
  const slideRect = slide?.getBoundingClientRect() || null;
  const footerRect = footer && isVisible(footer) ? footer.getBoundingClientRect() : null;
  const contentChildren = content ? Array.from(content.children).filter(isVisible) : [];
  const blockUnion = unionRects(contentChildren);
  const visibleContentElements = content ? Array.from(content.querySelectorAll("*")).filter((element) => {
    if (!isVisible(element) || element.closest(".zpres-debug-boundary-label")) return false;
    const graphicsRoot = element.closest("svg, math");
    return !graphicsRoot || graphicsRoot === element;
  }) : [];
  const excludedText = (element) => element.closest(
    ".zpres-debug-boundary-label, .zpres-block-label, .zpres-slide-meta, .zpres-slide-footer, script, style"
  );
  const textElements = new Set();
  const subjects = [];
  if (content) {
    for (const element of visibleContentElements) {
      if (excludedText(element)) continue;
      const graphicsRoot = element.closest("svg, math");
      if (graphicsRoot && graphicsRoot !== element) continue;
      const hasVisibleChild = Array.from(element.children).some(isVisible);
      const hasDirectText = Array.from(element.childNodes).some((node) =>
        node.nodeType === Node.TEXT_NODE && Boolean(node.nodeValue?.trim()));
      const explicitVisual = element.matches("img, video, audio, iframe, canvas, svg, math, table, pre, hr, .zpres-media-fallback");
      const positioned = getComputedStyle(element).position !== "static";
      if (explicitVisual || positioned || hasDirectText || (!hasVisibleChild && !hasDirectText)) {
        subjects.push({ element, locator: describe(element), bounds: element.getBoundingClientRect(), text: false });
      }
    }

    const walker = document.createTreeWalker(content, NodeFilter.SHOW_TEXT);
    let node;
    while ((node = walker.nextNode())) {
      if (!node.nodeValue.trim()) continue;
      const element = node.parentElement;
      if (!element || excludedText(element) || !isVisible(element)) continue;
      const graphicsRoot = element.closest("svg, math");
      if (graphicsRoot && graphicsRoot !== element) continue;
      textElements.add(element);
      const range = document.createRange();
      range.selectNodeContents(node);
      Array.from(range.getClientRects()).forEach((bounds, index) => {
        if (bounds.width <= 0 || bounds.height <= 0) return;
        subjects.push({ element, locator: describe(element) + "::text[" + index + "]", bounds, text: true });
      });
    }
  }
  const visibleContentUnion = unionBounds([
    ...visibleContentElements.map((element) => element.getBoundingClientRect()),
    ...subjects.map((subject) => subject.bounds),
  ]);
  const canvasRect = canvas?.getBoundingClientRect() || null;

  if (slideRect && canvasRect) {
    addViolation(
      "canvas-outside-slide",
      describe(canvas),
      canvasRect,
      describe(slide),
      slideRect,
      outsideDeltas(canvasRect, slideRect),
    );
  }
  if (slideRect && footerRect) {
    addViolation(
      "footer-outside-slide",
      describe(footer),
      footerRect,
      describe(slide),
      slideRect,
      outsideDeltas(footerRect, slideRect),
    );
  }

  for (const subject of subjects) {
    const boundary = containmentBoundary(subject);
    if (boundary && isVisible(boundary)) {
      const boundaryRect = boundary.getBoundingClientRect();
      const kind = boundary.matches(".zpres-layout-region")
        ? "outside-layout-region"
        : "outside-content-block";
      addViolation(
        kind,
        subject.locator,
        subject.bounds,
        describe(boundary),
        boundaryRect,
        outsideDeltas(subject.bounds, boundaryRect),
      );
    }
    if (canvasRect) {
      addViolation(
        "outside-canvas",
        subject.locator,
        subject.bounds,
        describe(canvas),
        canvasRect,
        outsideDeltas(subject.bounds, canvasRect),
      );
    }
  }

  if (content) {
    for (const container of Array.from(content.querySelectorAll(".zpres-block, .zpres-layout-region")).filter(isVisible)) {
      const boundary = container.parentElement?.closest(".zpres-block, .zpres-layout-region") || null;
      if (!boundary || !isVisible(boundary)
        || (!boundary.matches(".zpres-layout-region") && !clipsDescendants(boundary))) continue;
      const containerRect = container.getBoundingClientRect();
      const boundaryRect = boundary.getBoundingClientRect();
      addViolation(
        boundary.matches(".zpres-layout-region") ? "outside-layout-region" : "outside-content-block",
        describe(container),
        containerRect,
        describe(boundary),
        boundaryRect,
        outsideDeltas(containerRect, boundaryRect),
      );
    }
  }

  if (slide) {
    for (const figure of Array.from(slide.querySelectorAll("figure")).filter(isVisible)) {
      const caption = Array.from(figure.children).find((element) =>
        element.tagName === "FIGCAPTION" && isVisible(element));
      if (!caption) continue;
      const captionRect = caption.getBoundingClientRect();
      const visuals = Array.from(figure.children).filter((element) =>
        isVisible(element) && element !== caption
          && element.matches("img, video, audio, iframe, canvas, svg, math, .zpres-media-fallback"));
      for (const visual of visuals) {
        const visualRect = visual.getBoundingClientRect();
        const overlap = intersect(visualRect, captionRect);
        if (overlap.width <= GEOMETRY_TOLERANCE || overlap.height <= GEOMETRY_TOLERANCE) continue;
        addViolation(
          "caption-collision",
          describe(visual),
          visualRect,
          describe(caption),
          captionRect,
          { left: 0, top: 0, right: 0, bottom: round(overlap.height) },
        );
      }
    }

    const overlayAnnotations = Array.from(slide.querySelectorAll(
      '.zpres-block-layout[data-layout-kind="overlay"] .zpres-layout-region[data-region-role="annotation"]'
    )).filter(isVisible);
    const slideBody = slide.querySelector('.zpres-slide-body');
    const slideHeader = slide.querySelector('.zpres-slide-header');
    const visibleCaptions = Array.from(slide.querySelectorAll('figcaption')).filter(isVisible);
    for (const annotation of overlayAnnotations) {
      const annotationRect = annotation.getBoundingClientRect();
      if (slideBody && isVisible(slideBody)) {
        const bodyRect = slideBody.getBoundingClientRect();
        addViolation(
          'overlay-safe-area-collision',
          describe(annotation),
          annotationRect,
          describe(slideBody),
          bodyRect,
          outsideDeltas(annotationRect, bodyRect),
        );
      }
      const forbidden = [
        ...(slideHeader && isVisible(slideHeader) ? [['overlay-title-collision', slideHeader]] : []),
        ...(footer && isVisible(footer) ? [['overlay-footer-collision', footer]] : []),
        ...visibleCaptions.map((caption) => ['overlay-caption-collision', caption]),
      ];
      for (const [kind, boundary] of forbidden) {
        const boundaryRect = boundary.getBoundingClientRect();
        const overlap = intersect(annotationRect, boundaryRect);
        if (overlap.width <= GEOMETRY_TOLERANCE || overlap.height <= GEOMETRY_TOLERANCE) continue;
        addViolation(
          kind,
          describe(annotation),
          annotationRect,
          describe(boundary),
          boundaryRect,
          { left: 0, top: 0, right: round(overlap.width), bottom: round(overlap.height) },
        );
      }
    }
  }

  if (footerRect) {
    for (const subject of subjects) {
      const overlap = intersect(subject.bounds, footerRect);
      if (overlap.width <= GEOMETRY_TOLERANCE || overlap.height <= GEOMETRY_TOLERANCE) continue;
      addViolation(
        "footer-overlap",
        subject.locator,
        subject.bounds,
        describe(footer),
        footerRect,
        { left: 0, top: 0, right: 0, bottom: round(overlap.height) },
      );
    }
  }

  if (slide) {
    const pdfState = slide.getAttribute("data-pdf-step-state");
    const pdfStep = Number(slide.getAttribute("data-pdf-step") || "0");
    const routeStep = Number(activeRoute?.step ?? 0);
    const indexedSteps = Array.from(slide.querySelectorAll("[data-step-index]"));
    for (const element of indexedSteps) {
      const stepIndex = Number(element.getAttribute("data-step-index"));
      if (!Number.isSafeInteger(stepIndex) || stepIndex < 0) continue;
      const expectedVisible = observationMode === "screen"
        ? stepIndex <= routeStep
        : pdfState === "final" || (pdfState === "up-to" && stepIndex <= pdfStep);
      const visible = expectedVisible ? isVisible(element) : subtreeHasVisiblePixels(element);
      if (visible === expectedVisible) continue;
      stepVisibilityViolations.push({
        kind: visible ? "unexpected-visible-step" : "missing-visible-step",
        element: describe(element),
        step_index: stepIndex,
        expected_visible: expectedVisible,
        visible,
      });
    }
    if (observationMode === "screen") {
      for (const element of Array.from(slide.querySelectorAll(".is-step-gated"))) {
        if (!isVisible(element)) continue;
        stepVisibilityViolations.push({
          kind: "visible-step-gated-content",
          element: describe(element),
          step_index: null,
          expected_visible: false,
          visible: true,
        });
      }
    }
  }

  let outside = null;
  let occupancy = null;
  let whitespace = null;
  if (visibleContentUnion && canvasRect && canvasRect.width > 0 && canvasRect.height > 0) {
    outside = {
      left: round(Math.max(0, canvasRect.left - visibleContentUnion.left)),
      top: round(Math.max(0, canvasRect.top - visibleContentUnion.top)),
      right: round(Math.max(0, visibleContentUnion.right - canvasRect.right)),
      bottom: round(Math.max(0, visibleContentUnion.bottom - canvasRect.bottom)),
    };
  }
  if (blockUnion && canvasRect && canvasRect.width > 0 && canvasRect.height > 0) {
    const intersectionWidth = Math.max(0, Math.min(blockUnion.right, canvasRect.right) - Math.max(blockUnion.left, canvasRect.left));
    const intersectionHeight = Math.max(0, Math.min(blockUnion.bottom, canvasRect.bottom) - Math.max(blockUnion.top, canvasRect.top));
    occupancy = round((intersectionWidth * intersectionHeight) / (canvasRect.width * canvasRect.height));
    const clamp = (value) => Math.max(0, Math.min(1, value));
    whitespace = {
      left: round(clamp((blockUnion.left - canvasRect.left) / canvasRect.width)),
      top: round(clamp((blockUnion.top - canvasRect.top) / canvasRect.height)),
      right: round(clamp((canvasRect.right - blockUnion.right) / canvasRect.width)),
      bottom: round(clamp((canvasRect.bottom - blockUnion.bottom) / canvasRect.height)),
    };
  }

  const unresolvedSelector = "[data-zpres-unresolved], .debug-unresolved-math, .zpres-unresolved-math, .debug-placeholder";
  const unresolved = (slide ? [
    ...(slide.matches(unresolvedSelector) ? [slide] : []),
    ...Array.from(slide.querySelectorAll(unresolvedSelector)),
  ] : []).filter(isVisible).map((element) => ({
    marker: element.getAttribute("data-zpres-unresolved") || describe(element),
    excerpt: (element.textContent || "").trim().replace(/\s+/g, " ").slice(0, 160),
  }));

  const overflowElements = content ? Array.from(content.querySelectorAll("*")).filter((element) => {
    return isVisible(element) && !element.closest(".zpres-debug-boundary-label")
      && element.namespaceURI === "http://www.w3.org/1999/xhtml";
  }).map((element) => {
    const style = getComputedStyle(element);
    const clipsX = ["auto", "scroll", "hidden", "clip"].includes(style.overflowX);
    const checksVertical = element.matches(".zpres-slide-content, .zpres-layout-region, .zpres-block, pre, table");
    const clipsY = checksVertical && ["auto", "scroll", "hidden", "clip"].includes(style.overflowY);
    const horizontal = clipsX ? Math.max(0, element.scrollWidth - element.clientWidth) : 0;
    const vertical = clipsY ? Math.max(0, element.scrollHeight - element.clientHeight) : 0;
    return { element: describe(element), horizontal_px: round(horizontal), vertical_px: round(vertical) };
  }).filter((value) => value.horizontal_px > 2 || value.vertical_px > 2).slice(0, 24) : [];

  const textStyleMap = new Map();
  let minVisibleText = null;
  for (const element of textElements) {
    const style = getComputedStyle(element);
    const size = Number.parseFloat(style.fontSize);
    if (Number.isFinite(size)) minVisibleText = minVisibleText === null ? size : Math.min(minVisibleText, size);
    const key = [style.fontFamily, size, style.lineHeight, style.fontWeight].join("|");
    const current = textStyleMap.get(key) || {
      family: style.fontFamily, size_px: round(size || 0), line_height: style.lineHeight,
      weight: style.fontWeight, count: 0,
    };
    current.count += 1;
    textStyleMap.set(key, current);
  }
  let minBodyText = null;
  if (content) {
    for (const element of Array.from(content.querySelectorAll("p, li, td, th, blockquote, pre, code, figcaption"))) {
      if (!isVisible(element) || excludedText(element)) continue;
      const size = Number.parseFloat(getComputedStyle(element).fontSize);
      if (Number.isFinite(size)) minBodyText = minBodyText === null ? size : Math.min(minBodyText, size);
    }
  }

  const parseRgb = (value) => {
    const match = String(value || "").match(/rgba?\(([^)]+)\)/i);
    if (!match) return null;
    const parts = match[1].split(/[ ,/]+/).filter(Boolean).map(Number);
    if (parts.length < 3 || parts.slice(0, 3).some((part) => !Number.isFinite(part))) return null;
    return { r: parts[0], g: parts[1], b: parts[2], a: Number.isFinite(parts[3]) ? parts[3] : 1 };
  };
  const luminance = (color) => {
    const channel = (value) => {
      const normalized = value / 255;
      return normalized <= 0.04045 ? normalized / 12.92 : Math.pow((normalized + 0.055) / 1.055, 2.4);
    };
    return 0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b);
  };
  const contrast = (foreground, background) => {
    if (!foreground || !background || foreground.a < 0.999 || background.a < 0.999) return null;
    const first = luminance(foreground);
    const second = luminance(background);
    return round((Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05));
  };
  const backgroundFor = (element) => {
    let current = element;
    const backgroundLayer = slide?.querySelector(".zpres-slide-background");
    const layerStyle = backgroundLayer ? getComputedStyle(backgroundLayer) : null;
    let imageBacked = Boolean(backgroundLayer && isVisible(backgroundLayer)
      && layerStyle?.backgroundImage?.includes("url("));
    while (current instanceof Element) {
      const style = getComputedStyle(current);
      if (style.backgroundImage?.includes("url(")) imageBacked = true;
      const color = parseRgb(style.backgroundColor);
      if (color && color.a >= 0.999) return { color, imageBacked };
      current = current.parentElement;
    }
    const bodyColor = parseRgb(getComputedStyle(document.body).backgroundColor);
    return {
      color: bodyColor && bodyColor.a >= 0.999 ? bodyColor : { r: 255, g: 255, b: 255, a: 1 },
      imageBacked,
    };
  };
  const designElements = Array.from(new Set([
    ...textElements,
    ...(slide ? Array.from(slide.querySelectorAll("[data-zpres-content-role], img")) : []),
  ])).filter(isVisible);
  const measurementStep = observationMode === "screen"
    ? Number(activeRoute?.step ?? 0)
    : Number(slide?.dataset.pdfStep || 0);
  const measurementPalette = slide?.getAttribute("data-theme-param-mode")
    || document.body?.getAttribute("data-zpres-palette") || "default";
  const textLineCount = (element, text, lineHeight, bounds) => {
    if (!text) return null;
    const range = document.createRange();
    range.selectNodeContents(element);
    const tops = [];
    for (const line of Array.from(range.getClientRects())) {
      if (line.width <= 0 || line.height <= 0) continue;
      if (!tops.some((top) => Math.abs(top - line.top) < 2)) tops.push(line.top);
    }
    if (tops.length) return tops.length;
    return lineHeight ? Math.max(1, Math.round(bounds.height / lineHeight)) : 1;
  };
  const designMeasurements = designElements.map((element) => {
    const style = getComputedStyle(element);
    const bounds = element.getBoundingClientRect();
    const text = (element.textContent || "").trim().replace(/\s+/g, " ");
    const textSample = textElements.has(element);
    const fontSize = Number.parseFloat(style.fontSize);
    const parsedWeight = Number.parseInt(style.fontWeight, 10);
    const fontWeight = Number.isFinite(parsedWeight)
      ? parsedWeight : (style.fontWeight === "bold" ? 700 : 400);
    const wcagLargeText = Number.isFinite(fontSize)
      && (fontSize >= 24 || (fontWeight >= 700 && fontSize >= (14 * 96 / 72)));
    const parsedLineHeight = Number.parseFloat(style.lineHeight);
    const lineHeight = Number.isFinite(parsedLineHeight)
      ? parsedLineHeight : (Number.isFinite(fontSize) ? fontSize * 1.2 : null);
    const zeroWidth = Number.isFinite(fontSize) ? Math.max(1, fontSize * 0.5) : null;
    const background = backgroundFor(element);
    const foreground = parseRgb(style.color);
    const image = element instanceof HTMLImageElement ? element : null;
    const vectorImage = image && /(?:\.svg(?:[?#]|$)|^data:image\/svg\+xml)/i.test(image.currentSrc || image.src || "");
    const renderedScale = image && !vectorImage && bounds.width > 0 && bounds.height > 0
      && image.naturalWidth > 0 && image.naturalHeight > 0
      ? Math.min(image.naturalWidth / bounds.width, image.naturalHeight / bounds.height)
      : null;
    let alternativeStatus = null;
    if (image && image.closest("figure, [data-zpres-content-role=\"evidence\"]")) {
      alternativeStatus = !image.hasAttribute("alt") ? "missing"
        : image.getAttribute("alt").trim() ? "present" : "decorative";
    }
    const typeOwner = element.closest("[data-zpres-type-role]");
    const contentOwner = element.closest("[data-zpres-content-role]");
    const inferredType = element.matches(".zpres-slide-title, h1")
      ? (slide?.dataset.slideVariant === "section-title" ? "display" : "title")
      : element.matches("h2, h3") ? "heading"
      : element.matches("figcaption, .zpres-slide-sources, .zpres-slide-footer") ? "micro"
      : "body";
    const ownType = element.getAttribute("data-zpres-type-role");
    const resolvedType = ownType || (element.matches("h1, h2, h3")
      ? inferredType
      : (typeOwner?.getAttribute("data-zpres-type-role") || inferredType));
    const nonessential = Boolean(element.closest("figcaption, .zpres-footnote-ref, .zpres-slide-sources, .zpres-slide-footer, .zpres-slide-meta, .zpres-speaker-notes-panel"));
    return {
      element: describe(element),
      type_role: resolvedType,
      content_role: contentOwner?.getAttribute("data-zpres-content-role")
        || (image ? "evidence" : "text"),
      surface: observationMode,
      palette: measurementPalette,
      slide_id: slide?.dataset.slideId || "",
      step: measurementStep,
      text_excerpt: text.slice(0, 160),
      text_sample: textSample,
      font_size_px: Number.isFinite(fontSize) && textSample ? round(fontSize) : null,
      font_weight: textSample ? fontWeight : null,
      wcag_large_text: Boolean(textSample && wcagLargeText),
      essential_content: Boolean(textSample && resolvedType !== "micro" && !nonessential),
      line_height_px: Number.isFinite(lineHeight) && textSample ? round(lineHeight) : null,
      cap_height_proxy_px: Number.isFinite(fontSize) && textSample ? round(fontSize * 0.7) : null,
      line_count: textSample ? textLineCount(element, text, lineHeight, bounds) : null,
      prose_characters: textSample ? text.length : null,
      prose_measure_ch: zeroWidth && textSample ? round(bounds.width / zeroWidth) : null,
      occupancy: canvasRect && canvasRect.width > 0 && canvasRect.height > 0
        ? round((bounds.width * bounds.height) / (canvasRect.width * canvasRect.height)) : null,
      contrast_ratio: textSample && !background.imageBacked ? contrast(foreground, background.color) : null,
      image_backed_text: Boolean(textSample && background.imageBacked),
      rendered_width: image ? round(bounds.width) : null,
      rendered_height: image ? round(bounds.height) : null,
      natural_width: image ? image.naturalWidth : null,
      natural_height: image ? image.naturalHeight : null,
      resolution_scale: renderedScale === null ? null : round(renderedScale),
      figure_alternative_status: alternativeStatus,
      actual_font_families: [],
    };
  });

  const heading = slide ? Array.from(slide.querySelectorAll("h1, h2, h3")).find(isVisible) : null;
  const semanticTextRegions = slide ? [
    ['header', slide.querySelector('.zpres-slide-header')],
    ['body', slide.querySelector('.zpres-slide-body')],
    ['footer', slide.querySelector('.zpres-slide-footer')],
  ].map(([name, region]) => [name, region, region ? authoredRegionText(region) : ''])
    .filter(([, region, authoredText]) => region && authoredText)
    .map(([name, region, authoredText]) => {
    const textBounds = textNodeBounds(region).map(rect);
    return {
      region: name,
      element: describe(region),
      authored_text: authoredText,
      text_bounds: textBounds,
      raster_ink: null,
    };
  }) : [];
  const semanticHeader = semanticTextRegions.find((region) => region.region === 'header');
  const semanticBody = semanticTextRegions.find((region) => region.region === 'body');
  if (semanticHeader && semanticBody) {
    const intersections = semanticHeader.text_bounds.flatMap((headerBounds) =>
      semanticBody.text_bounds.map((bodyBounds) => intersect(headerBounds, bodyBounds))
        .filter((overlap) => overlap.width > GEOMETRY_TOLERANCE && overlap.height > GEOMETRY_TOLERANCE));
    const intersectionBounds = unionBounds(intersections);
    if (intersectionBounds) {
      const headerBounds = unionBounds(semanticHeader.text_bounds);
      const bodyBounds = unionBounds(semanticBody.text_bounds);
      addViolation(
        'header-body-overlap',
        semanticHeader.element,
        headerBounds,
        semanticBody.element,
        bodyBounds,
        {
          left: 0,
          top: 0,
          right: round(intersectionBounds.width),
          bottom: round(intersectionBounds.height),
        },
        intersectionBounds,
      );
    }
  }
  const rawFactor = slide?.getAttribute("data-autoscale-factor");
  const screenRect = observationMode === "screen" ? {
    x: 0, y: 0, left: 0, top: 0,
    width: window.innerWidth, height: window.innerHeight,
    right: window.innerWidth, bottom: window.innerHeight,
  } : null;
  const documentScroll = observationMode === "screen" ? {
    left: round(Math.max(0, window.scrollX || 0)),
    top: round(Math.max(0, window.scrollY || 0)),
    right: round(Math.max(0, Math.max(
      document.documentElement.scrollWidth,
      document.body?.scrollWidth || 0,
    ) - window.innerWidth)),
    bottom: round(Math.max(0, Math.max(
      document.documentElement.scrollHeight,
      document.body?.scrollHeight || 0,
    ) - window.innerHeight)),
  } : null;
  const debugInspection = document.body?.dataset.zpresDebug === "enabled" && slide
    ? {
        identity: slide.dataset.zpresDebugIdentity || "",
        role: slide.dataset.zpresDebugRole || "",
        route: slide.dataset.zpresDebugRoute || "",
        target: slide.dataset.zpresDebugTarget || "",
        step_summary: slide.dataset.zpresDebugStepSummary || "",
        status: slide.dataset.zpresDebugStatus || "",
        diagnostics: slide.dataset.zpresDebugDiagnostics || "",
        print_rail: slide.dataset.zpresDebugPrintRail || "",
        painted_rail: getComputedStyle(slide, "::after").content || "",
        regions: Array.from(slide.querySelectorAll("[data-zpres-debug-region]"))
          .map((region) => {
            const label = slide.querySelector(`:scope > .zpres-debug-boundary-label[data-zpres-debug-label-role="${region.dataset.zpresDebugRegion || ""}"]`);
            const labelBounds = label?.getBoundingClientRect() || null;
            const authoredTextBounds = [];
            const walker = document.createTreeWalker(slide, NodeFilter.SHOW_TEXT);
            for (let node = walker.nextNode(); node; node = walker.nextNode()) {
              const parent = node.parentElement;
              if (!node.nodeValue.trim() || !parent || !isVisible(parent)
                || parent.closest(".zpres-debug-boundary-label, .zpres-slide-meta, .zpres-block-label, script, style")) continue;
              const range = document.createRange();
              range.selectNodeContents(node);
              for (const textBounds of Array.from(range.getClientRects())) {
                const overlap = labelBounds ? intersect(labelBounds, textBounds) : null;
                if (overlap && overlap.width > 1 && overlap.height > 1) authoredTextBounds.push(rect(overlap));
              }
            }
            return {
              role: region.dataset.zpresDebugRegion || "",
              bounds: region.dataset.zpresDebugBounds || "",
              placement: label?.dataset.zpresDebugLabelPlacement || "",
              label_bounds: rect(labelBounds),
              authored_text_intersections: authoredTextBounds,
            };
          }),
      }
    : null;
  return {
    document_ready_state: document.readyState,
    autoscale_ready: document.body?.getAttribute("data-zpres-autoscale-ready") === "true",
    slide_present: Boolean(slide),
    page: slide?.dataset.page ? Number(slide.dataset.page) : null,
    slide_id: slide?.dataset.slideId || "",
    title: (heading?.textContent || "").trim().replace(/\s+/g, " "),
    role: slide?.dataset.slideRole || "",
    variant: slide?.dataset.slideVariant || null,
    generated: slide?.dataset.generatedSlide || null,
    step_state: slide?.dataset.pdfStepState || null,
    pdf_step: slide?.dataset.pdfStep ? Number(slide.dataset.pdfStep) : null,
    debug_inspection: debugInspection,
    slide_bounds: slide ? rect(slide.getBoundingClientRect()) : null,
    canvas_bounds: canvasRect ? rect(canvasRect) : null,
    content_bounds: content ? rect(content.getBoundingClientRect()) : null,
    screen_bounds: screenRect ? rect(screenRect) : null,
    root_bounds: root ? rect(root.getBoundingClientRect()) : null,
    stage_bounds: stage ? rect(stage.getBoundingClientRect()) : null,
    active_stack_bounds: activeStack ? rect(activeStack.getBoundingClientRect()) : null,
    footer_bounds: footerRect ? rect(footerRect) : null,
    document_scroll: documentScroll,
    active_stack_count: observationMode === "screen" ? activeStacks.length : null,
    active_slide_count: observationMode === "screen" ? activeSlides.length : null,
    visible_stack_count: observationMode === "screen" ? presentationStacks.filter(isVisible).length : null,
    visible_slide_count: observationMode === "screen" ? presentationSlides.filter(isVisible).length : null,
    routed_stack_visible: observationMode === "screen" ? Boolean(activeStack && isVisible(activeStack)) : null,
    routed_slide_visible: observationMode === "screen" ? Boolean(slide && isVisible(slide)) : null,
    screen_route: observationMode === "screen" ? location.hash : null,
    screen_section: observationMode === "screen" ? (activeRoute?.section ?? null) : null,
    screen_detail: observationMode === "screen" ? (activeRoute?.detail ?? null) : null,
    screen_step: observationMode === "screen" ? (activeRoute?.step ?? null) : null,
    screen_step_count: observationMode === "screen" ? (activeRoute?.step_count ?? null) : null,
    content_union: blockUnion ? rect(blockUnion) : null,
    visible_content_bounds: visibleContentUnion ? rect(visibleContentUnion) : null,
    block_bounds: Array.from(content?.querySelectorAll('.zpres-block') || []).filter(isVisible).map((element) => ({
      block_type: element.getAttribute("data-block-type") || describe(element),
      bounds: rect(element.getBoundingClientRect()),
    })),
    outside_canvas: outside,
    occupancy,
    whitespace,
    autoscale_factor: rawFactor ? Number(rawFactor) : null,
    clip_marker: slide?.getAttribute("data-zpres-overflow") === "clipped",
    unresolved,
    images: imageResults,
    background_images: backgroundResults,
    authored_background: authoredBackground,
    font_faces: Array.from(document.fonts).map((font) => ({
      family: font.family, status: font.status, style: font.style, weight: font.weight,
    })),
    text_styles: Array.from(textStyleMap.values()),
    min_visible_text_px: minVisibleText === null ? null : round(minVisibleText),
    min_body_text_px: minBodyText === null ? null : round(minBodyText),
    design_measurements: designMeasurements,
    semantic_text_regions: semanticTextRegions,
    overflow_elements: overflowElements,
    geometry_violations: geometryViolations,
    step_visibility_violations: stepVisibilityViolations,
    readiness_errors: readinessErrors,
  };
})()
"##;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_url::file_url;
    use image::GenericImageView;
    use tempfile::tempdir;

    const TEST_FONT_BASE64: &str = "AAEAAAAKAIAAAwAgT1MvMkUAQ7AAAAEoAAAAYGNtYXAAdABcAAABlAAAADxnbHlmpjZxAAAAAdgAAAA8aGVhZC4/TwIAAACsAAAANmhoZWEFJQHfAAAA5AAAACRobXR4BXgASwAAAYgAAAAMbG9jYQANACsAAAHQAAAACG1heHAABQAJAAABCAAAACBuYW1lsCwxeAAAAhQAAAHOcG9zdAAIACQAAAPkAAAAKAABAAAAAQAA+/4pQ18PPPUAAQPoAAAAAOZ4hbMAAAAA5niFswAyAAACCAK8AAAAAwACAAAAAAAAAAEAAAMg/zgAAAJYABkASwHqAAEAAAAAAAAAAAAAAAAAAAADAAEAAAADAAcAAQAAAAAAAgAAAAAAAAAAAAAAAAAAAAAAAwHTAZAABQAEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAAAAAAAAAAAAAAAAPz8/PwAAACAAQQMg/zgAAAMgAMgAAAAAAAAAAAAAAAAAAAAgAAACWAAyASwAAAH0ABkAAAACAAAAAwAAABQAAwABAAAAFAAEACgAAAAGAAQAAQACACAAQf//AAAAIABB////4f/BAAEAAAAAAAAAAAANAA0AHgABAFAAAAIIArwAAwAAMxEhEVABuAK8/UQAAAEAMgAAAcICvAAGAAAzExMjJyMHMsjIbihkKAK8/USgoAAAAAwAlgABAAAAAAABAA8AAAABAAAAAAACAAcADwABAAAAAAADABsAFgABAAAAAAAEABcAMQABAAAAAAAFAAsASAABAAAAAAAGABUAUwADAAEECQABAB4AaAADAAEECQACAA4AhgADAAEECQADADYAlAADAAEECQAEAC4AygADAAEECQAFABYA+AADAAEECQAGACoBDlpwcmVzIFRlc3QgRmFjZVJlZ3VsYXJacHJlcyBUZXN0IEZhY2UgUmVndWxhciAxLjBacHJlcyBUZXN0IEZhY2UgUmVndWxhclZlcnNpb24gMS4wWnByZXNUZXN0RmFjZS1SZWd1bGFyAFoAcAByAGUAcwAgAFQAZQBzAHQAIABGAGEAYwBlAFIAZQBnAHUAbABhAHIAWgBwAHIAZQBzACAAVABlAHMAdAAgAEYAYQBjAGUAIABSAGUAZwB1AGwAYQByACAAMQAuADAAWgBwAHIAZQBzACAAVABlAHMAdAAgAEYAYQBjAGUAIABSAGUAZwB1AGwAYQByAFYAZQByAHMAaQBvAG4AIAAxAC4AMABaAHAAcgBlAHMAVABlAHMAdABGAGEAYwBlAC0AUgBlAGcAdQBsAGEAcgAAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAwAAAAMAJA==";

    #[test]
    fn overflow_delta_reports_largest_edge() {
        let overflow = BrowserOverflowDeltas {
            left: 1.0,
            top: 7.5,
            right: 2.0,
            bottom: 4.0,
        };
        assert_eq!(overflow.maximum(), 7.5);
    }

    #[test]
    fn browser_observation_deserializes_minimal_measurement() {
        let observation: BrowserPageObservation = serde_json::from_value(json!({
            "slide_present": true,
            "page": 2,
            "slide_id": "section-2-main",
            "clip_marker": false,
        }))
        .unwrap();
        assert!(observation.slide_present);
        assert_eq!(observation.page, Some(2));
        assert_eq!(observation.slide_id, "section-2-main");
        assert!(observation.images.is_empty());
        assert!(observation.platform_fonts.is_empty());
        assert!(observation.platform_font_probes.is_empty());
        assert_eq!(observation.platform_font_probe_count, 0);
        assert_eq!(observation.platform_font_candidate_count, 0);
        assert!(!observation.platform_font_probe_truncated);
        assert_eq!(
            observation.platform_font_evidence.missing_glyph_cause,
            BrowserFontEvidenceStatus::Unavailable
        );
    }

    #[test]
    fn chromium_profiles_use_the_fixed_host_scratch_root() {
        let expected = fs::canonicalize("/tmp").unwrap();
        let profile = create_profile_dir().unwrap();

        assert_eq!(profile.parent(), Some(expected.as_path()));
        fs::remove_dir(&profile).unwrap();
    }

    #[test]
    fn platform_font_usage_is_deduplicated_sorted_and_saturating() {
        let fonts = aggregate_platform_fonts([
            CdpPlatformFontUsage {
                family_name: " Zulu ".to_string(),
                postscript_name: "Zulu-Regular".to_string(),
                custom: false,
                glyph_count: 2,
            },
            CdpPlatformFontUsage {
                family_name: "Alpha".to_string(),
                postscript_name: "Alpha-Regular".to_string(),
                custom: true,
                glyph_count: 3,
            },
            CdpPlatformFontUsage {
                family_name: "Zulu".to_string(),
                postscript_name: "Zulu-Regular".to_string(),
                custom: false,
                glyph_count: 5,
            },
            CdpPlatformFontUsage {
                family_name: "Unused".to_string(),
                glyph_count: 0,
                ..Default::default()
            },
        ]);

        assert_eq!(
            fonts,
            vec![
                BrowserPlatformFont {
                    family: "Alpha".to_string(),
                    postscript_name: "Alpha-Regular".to_string(),
                    custom: true,
                    glyph_count: 3,
                },
                BrowserPlatformFont {
                    family: "Zulu".to_string(),
                    postscript_name: "Zulu-Regular".to_string(),
                    custom: false,
                    glyph_count: 7,
                },
            ]
        );
        assert_eq!(secondary_face_glyph_count(&fonts), 3);
    }

    #[test]
    fn platform_font_bridge_uses_fresh_author_invisible_attribute_names() {
        let first = PlatformFontBridge::new();
        let second = PlatformFontBridge::new();

        assert_ne!(first, second);
        assert!(
            first
                .probe_attribute
                .starts_with(PLATFORM_FONT_PROBE_ATTRIBUTE_PREFIX)
        );
        assert!(
            first
                .candidate_attribute
                .starts_with(PLATFORM_FONT_CANDIDATE_ATTRIBUTE_PREFIX)
        );
        assert_ne!(first.probe_attribute, "data-zpres-platform-font-probe");
        assert_ne!(
            first.candidate_attribute,
            "data-zpres-platform-font-candidate"
        );
        assert!(!platform_font_probes_js(1, &first).contains("__ZPRES_"));
        assert!(!clear_platform_font_probes_js(&first).contains("__ZPRES_"));
    }

    #[test]
    fn platform_font_evidence_serializes_objective_face_counts_and_protocol_limits() {
        let aggregate_fonts = vec![
            BrowserPlatformFont {
                family: "Talk Sans".to_string(),
                postscript_name: "TalkSans-Regular".to_string(),
                custom: true,
                glyph_count: 24,
            },
            BrowserPlatformFont {
                family: "Fallback Sans".to_string(),
                postscript_name: "FallbackSans".to_string(),
                custom: false,
                glyph_count: 1,
            },
        ];
        let probe = BrowserPlatformFontProbe {
            index: 0,
            element: "p.lead".to_string(),
            text_excerpt: "Text rendered with two faces".to_string(),
            requested_family: "Talk Sans, sans-serif".to_string(),
            requested_style: "normal".to_string(),
            requested_weight: "400".to_string(),
            requested_font_synthesis: "weight style".to_string(),
            fonts: aggregate_fonts.clone(),
            multiple_faces_observed: true,
            secondary_face_glyph_count: 1,
        };
        let observation = BrowserPageObservation {
            platform_font_evidence: BrowserPlatformFontEvidence::from_fonts(&aggregate_fonts),
            platform_fonts: aggregate_fonts,
            platform_font_probes: vec![probe],
            platform_font_probe_count: 1,
            platform_font_candidate_count: 1,
            ..Default::default()
        };

        let serialized = serde_json::to_value(observation).unwrap();
        assert_eq!(
            serialized.pointer("/platform_font_probes/0/multiple_faces_observed"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            serialized.pointer("/platform_font_probes/0/secondary_face_glyph_count"),
            Some(&json!(1))
        );
        assert_eq!(
            serialized.pointer("/platform_font_evidence/multiple_faces_observed"),
            Some(&json!("observed"))
        );
        assert_eq!(
            serialized.pointer("/platform_font_evidence/fallback_activation"),
            Some(&json!("unavailable"))
        );
        assert_eq!(
            serialized.pointer("/platform_font_evidence/missing_glyph_cause"),
            Some(&json!("unavailable"))
        );
        assert_eq!(
            serialized.pointer("/platform_font_evidence/synthesized_face_activation"),
            Some(&json!("unavailable"))
        );
    }

    #[test]
    fn chromium_candidates_include_macos_and_linux_paths() {
        let candidates = chromium_candidates();
        assert!(
            candidates
                .iter()
                .any(|path| path.starts_with("/Applications/"))
        );
        assert!(candidates.iter().any(|path| path.starts_with("/usr/bin/")));
    }

    #[test]
    fn print_page_inventory_rejects_collapsed_and_overlapping_bounds() {
        let page = |index, y, height| BrowserPrintPage {
            index,
            page: Some(index + 1),
            slide_id: format!("slide-{}", index + 1),
            role: "main".to_string(),
            bounds: BrowserRect {
                y,
                width: 1280.0,
                height,
                right: 1280.0,
                bottom: y + height,
                ..Default::default()
            },
        };

        assert!(
            validate_print_page_inventory(&[page(0, 0.0, 720.0), page(1, 720.0, 720.0)]).is_ok()
        );
        assert!(
            validate_print_page_inventory(&[page(0, 0.0, 720.0), page(1, 0.0, 720.0)])
                .unwrap_err()
                .contains("ordered and non-overlapping")
        );
        assert!(
            validate_print_page_inventory(&[page(0, 0.0, 0.0)])
                .unwrap_err()
                .contains("collapsed browser bounds")
        );
    }

    #[test]
    fn screen_route_inventory_requires_unique_complete_step_states() {
        let route = |step, step_count, hash: &str| BrowserScreenRoute {
            hash: hash.to_string(),
            section: 0,
            detail: 0,
            step,
            step_count,
            slide_id: "section-1-main".to_string(),
            role: "main".to_string(),
            generated: None,
        };

        assert!(
            validate_screen_route_inventory(&[
                route(0, 2, "#/0/0"),
                route(1, 2, "#/0/0/1"),
                route(2, 2, "#/0/0/2"),
            ])
            .is_ok()
        );
        assert!(
            validate_screen_route_inventory(&[route(0, 2, "#/0/0"), route(2, 2, "#/0/0/2"),])
                .unwrap_err()
                .contains("expected {0, 1, 2}")
        );
        assert!(
            validate_screen_route_inventory(&[route(0, 1, "#/0/0"), route(1, 1, "#/0/0"),])
                .unwrap_err()
                .contains("duplicate hash")
        );
    }

    #[test]
    fn platform_font_bridge_ignores_stable_author_selectors_during_capture() {
        if discover_chromium_executable().executable.is_none() {
            return;
        }
        let temp = tempdir().unwrap();
        let html_path = temp.path().join("page.html");
        fs::write(
            &html_path,
            r#"<!doctype html>
<style>
html, body { margin: 0; width: 1280px; height: 720px; }
.zpres-print-slide, .zpres-slide-canvas, .zpres-slide-content { box-sizing: border-box; width: 1280px; height: 720px; }
.zpres-block { width: 600px; height: 200px; font-family: monospace; }
[data-zpres-platform-font-slide] { display: none !important; }
[data-zpres-platform-font-candidate], [data-zpres-platform-font-probe] {
  font-family: serif !important;
}
</style>
<body data-zpres-autoscale-ready="true" data-zpres-palette="calibration">
<section class="zpres-print-slide" data-page="1" data-slide-id="test-slide" data-slide-role="main">
  <div class="zpres-slide-canvas"><div class="zpres-slide-content">
    <div class="zpres-block" data-block-type="paragraph"><h1 data-zpres-type-role="display">Measured in Chromium</h1><p data-zpres-type-role="body">Body text.</p></div>
  </div></div>
</section>
</body>"#,
        )
        .unwrap();
        let url = format!("file://{}", html_path.canonicalize().unwrap().display());
        let mut browser = ChromiumSession::launch(ChromiumSessionOptions::default()).unwrap();
        let capture = browser
            .capture_page(
                &url,
                PngViewport {
                    width: 1280,
                    height: 720,
                },
            )
            .unwrap();
        assert_eq!(capture.observation.slide_id, "test-slide");
        assert_eq!(capture.observation.title, "Measured in Chromium");
        assert_eq!(capture.observation.page, Some(1));
        assert!(capture.observation.content_union.is_some());
        assert!(capture.observation.platform_font_probe_count >= 2);
        assert_eq!(
            capture.observation.platform_font_probe_count,
            capture.observation.platform_font_candidate_count
        );
        assert!(!capture.observation.platform_font_probe_truncated);
        assert!(
            capture
                .observation
                .platform_font_probes
                .iter()
                .any(|probe| {
                    (probe.element == "h1" || probe.element == "p")
                        && !probe.text_excerpt.is_empty()
                        && probe.requested_family.contains("monospace")
                        && !probe.fonts.is_empty()
                })
        );
        assert!(
            capture
                .observation
                .platform_fonts
                .iter()
                .any(|font| font.glyph_count > 0),
            "expected actual platform-font evidence: {:?}",
            capture.observation.platform_fonts
        );
        assert_eq!(
            capture.observation.platform_font_evidence.actual_font_use,
            BrowserFontEvidenceStatus::Observed
        );
        let body_measurement = capture
            .observation
            .design_measurements
            .iter()
            .find(|measurement| measurement.element == "p")
            .unwrap();
        assert_eq!(body_measurement.surface, "print");
        assert_eq!(body_measurement.palette, "calibration");
        assert_eq!(body_measurement.slide_id, "test-slide");
        assert_eq!(body_measurement.type_role, "body");
        assert_eq!(body_measurement.line_count, Some(1));
        assert!(body_measurement.contrast_ratio.is_some());
        assert!(!body_measurement.actual_font_families.is_empty());
        assert_eq!(
            capture
                .observation
                .platform_font_evidence
                .missing_glyph_cause,
            BrowserFontEvidenceStatus::Unavailable
        );
        assert_eq!(
            capture
                .observation
                .platform_font_evidence
                .synthesized_face_activation,
            BrowserFontEvidenceStatus::Unavailable
        );
        assert_eq!(
            browser
                .evaluate(COUNT_PLATFORM_FONT_BRIDGE_ATTRIBUTES_JS)
                .unwrap(),
            json!(0)
        );
        let image = image::load_from_memory(&capture.png).unwrap();
        assert_eq!(image.dimensions(), (1280, 720));
    }

    #[test]
    fn semantic_text_suppression_overrides_layered_important_svg_ink_on_screen_and_print() {
        if discover_chromium_executable().executable.is_none() {
            return;
        }
        let temp = tempdir().unwrap();
        let screen_path = temp.path().join("screen.html");
        let print_path = temp.path().join("print.html");
        let style = r#"
<style>
@layer zpres-reset, zpres-theme;
@layer zpres-theme {
  .zpres-chart-axis-label {
    fill: rgb(0, 255, 255) !important;
    stroke: rgb(0, 255, 255) !important;
  }
}
html, body { margin: 0; width: 1280px; height: 720px; background: rgb(5, 12, 24); }
.zpres-slide, .zpres-print-slide, .zpres-slide-canvas, .zpres-slide-content {
  box-sizing: border-box;
  width: 1280px;
  height: 720px;
}
.zpres-slide-body { padding: 120px; }
.zpres-chart-svg { width: 800px; height: 320px; }
</style>
"#;
        fs::write(
            &screen_path,
            format!(
                r#"<!doctype html>{style}
<body data-zpres-autoscale-ready="true">
<main class="reveal"><div class="slides"><section class="zpres-section-stack is-active">
<section class="zpres-slide is-active" data-slide-id="chart-screen" data-slide-role="main">
  <div class="zpres-slide-canvas"><div class="zpres-slide-content">
    <div class="zpres-slide-body"><svg class="zpres-chart-svg" viewBox="0 0 800 320">
      <text class="zpres-chart-axis-label" x="80" y="160" font-size="64">Visible axis label</text>
    </svg></div>
  </div></div>
</section>
</section></div></main>
</body>"#
            ),
        )
        .unwrap();
        fs::write(
            &print_path,
            format!(
                r#"<!doctype html>{style}
<body data-zpres-autoscale-ready="true">
<section class="zpres-print-slide" data-page="1" data-slide-id="chart-print" data-slide-role="main">
  <div class="zpres-slide-canvas"><div class="zpres-slide-content">
    <div class="zpres-slide-body"><svg class="zpres-chart-svg" viewBox="0 0 800 320">
      <text class="zpres-chart-axis-label" x="80" y="160" font-size="64">Visible axis label</text>
    </svg></div>
  </div></div>
</section>
</body>"#
            ),
        )
        .unwrap();

        let viewport = PngViewport::default();
        let mut browser = ChromiumSession::launch(ChromiumSessionOptions::default()).unwrap();
        for (surface, path) in [("screen", &screen_path), ("print", &print_path)] {
            let url = file_url(path);
            if surface == "screen" {
                browser.load_screen_document(&url, viewport).unwrap();
            } else {
                browser.load_print_document(&url, viewport).unwrap();
            }
            browser
                .evaluate(&format!(
                    "window.__zpresVisualObservationMode = '{surface}'; window.__zpresRequestedPrintPageIndex = 0; true"
                ))
                .unwrap();
            assert_eq!(
                browser
                    .evaluate(
                        "getComputedStyle(document.querySelector('.zpres-chart-axis-label')).fill"
                    )
                    .unwrap(),
                json!("rgb(0, 255, 255)"),
                "fixture did not establish layered important SVG ink on {surface}"
            );
            let ordinary = browser.capture_screenshot().unwrap();

            browser.suppress_semantic_text().unwrap();
            assert_eq!(
                browser
                    .evaluate("getComputedStyle(document.querySelector('.zpres-chart-axis-label')).fill === 'rgba(0, 0, 0, 0)' && getComputedStyle(document.querySelector('.zpres-chart-axis-label')).stroke === 'rgba(0, 0, 0, 0)'")
                    .unwrap(),
                json!(true),
                "semantic SVG paint remained visible after suppression on {surface}"
            );
            let suppressed = browser.capture_screenshot().unwrap();
            browser.restore_semantic_text().unwrap();

            assert_ne!(
                ordinary, suppressed,
                "semantic suppression did not change the {surface} raster"
            );
            assert_eq!(
                browser
                    .evaluate(
                        "getComputedStyle(document.querySelector('.zpres-chart-axis-label')).fill"
                    )
                    .unwrap(),
                json!("rgb(0, 255, 255)"),
                "semantic SVG paint was not restored on {surface}"
            );
        }
    }

    #[test]
    fn cdp_observes_custom_font_use_on_screen_and_print_with_bounded_samples() {
        if discover_chromium_executable().executable.is_none() {
            return;
        }
        let temp = tempdir().unwrap();
        fs::write(
            temp.path().join("zpres-test.ttf"),
            base64::engine::general_purpose::STANDARD
                .decode(TEST_FONT_BASE64)
                .unwrap(),
        )
        .unwrap();
        let shared_style = r#"
<style>
@font-face {
  font-family: "Zpres Test Face";
  src: url("zpres-test.ttf") format("truetype");
  font-display: block;
}
html, body { margin: 0; width: 1280px; height: 720px; }
.font-surface, .zpres-slide-canvas, .zpres-slide-content {
  box-sizing: border-box; width: 1280px; height: 720px;
}
.font-surface { font-family: "Zpres Test Face", sans-serif; }
.sample { display: inline-block; width: 24px; height: 24px; }
</style>
"#;
        let samples = (0..32)
            .map(|index| format!("<span class=\"sample\" data-sample=\"{index}\">A</span>"))
            .collect::<String>();
        let screen_path = temp.path().join("screen.html");
        fs::write(
            &screen_path,
            format!(
                r#"<!doctype html>{shared_style}
<body data-zpres-autoscale-ready="true">
<main class="reveal"><div class="slides"><section class="zpres-section-stack is-active">
<section class="zpres-slide font-surface is-active" data-slide-id="font-screen" data-slide-role="main">
  <div class="zpres-slide-canvas"><div class="zpres-slide-content"><h1>AAAA</h1><div>{samples}</div></div></div>
</section></section></div></main>
</body>"#
            ),
        )
        .unwrap();
        let print_path = temp.path().join("print.html");
        fs::write(
            &print_path,
            format!(
                r#"<!doctype html>{shared_style}
<body data-zpres-autoscale-ready="true">
<section class="zpres-print-slide font-surface" data-page="1" data-slide-id="font-print" data-slide-role="main">
  <div class="zpres-slide-canvas"><div class="zpres-slide-content"><h1>AAAA</h1></div></div>
</section>
</body>"#
            ),
        )
        .unwrap();

        let mut browser = ChromiumSession::launch(ChromiumSessionOptions::default()).unwrap();
        browser
            .load_screen_document(&file_url(&screen_path), PngViewport::default())
            .unwrap();
        let screen = browser
            .observe_visual_page(&format!(
                "window.__zpresVisualObservationMode = 'screen';\n{VISUAL_OBSERVATION_JS}"
            ))
            .unwrap();
        assert_eq!(screen.platform_font_probe_count, MAX_PLATFORM_FONT_PROBES);
        assert!(screen.platform_font_candidate_count > screen.platform_font_probe_count);
        assert!(screen.platform_font_probe_truncated);

        browser
            .load_print_document(&file_url(&print_path), PngViewport::default())
            .unwrap();
        let print = browser.observe_print_page(0).unwrap();

        for (surface, observation) in [("screen", screen), ("print", print)] {
            assert_eq!(
                observation.platform_font_evidence.actual_font_use,
                BrowserFontEvidenceStatus::Observed,
                "missing aggregate actual-font evidence on {surface}"
            );
            assert!(
                observation.platform_fonts.iter().any(|font| {
                    font.custom && font.family == "Zpres Test Face" && font.glyph_count > 0
                }),
                "expected the custom Zpres Test Face on {surface}: {:?}",
                observation.platform_fonts
            );
            assert_eq!(
                observation.platform_font_evidence.fallback_activation,
                BrowserFontEvidenceStatus::Unavailable
            );
        }
        assert_eq!(
            browser
                .evaluate(COUNT_PLATFORM_FONT_BRIDGE_ATTRIBUTES_JS)
                .unwrap(),
            json!(0)
        );
    }

    #[test]
    fn platform_font_collection_error_cleans_all_temporary_attributes() {
        if discover_chromium_executable().executable.is_none() {
            return;
        }
        let temp = tempdir().unwrap();
        let html_path = temp.path().join("font-probe-race.html");
        fs::write(
            &html_path,
            r#"<!doctype html>
<style>
html, body, .zpres-slide { margin: 0; width: 1280px; height: 720px; }
</style>
<section class="zpres-slide is-active"><p>Visible text</p></section>
<script>
new MutationObserver((records) => {
  for (const record of records) {
    if ((record.attributeName || "").startsWith("data-zpres-platform-font-probe-")) {
      record.target.removeAttribute(record.attributeName);
    }
  }
}).observe(document, { attributes: true, subtree: true });
</script>"#,
        )
        .unwrap();
        let mut browser = ChromiumSession::launch(ChromiumSessionOptions::default()).unwrap();
        browser
            .load_screen_document(&file_url(&html_path), PngViewport::default())
            .unwrap();
        browser
            .evaluate("window.__zpresVisualObservationMode = 'screen'; true")
            .unwrap();

        let error = browser
            .attach_platform_font_evidence(&mut BrowserPageObservation::default())
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("platform-font probe inventory changed"),
            "unexpected collection error: {error}"
        );
        assert_eq!(
            browser
                .evaluate(COUNT_PLATFORM_FONT_BRIDGE_ATTRIBUTES_JS)
                .unwrap(),
            json!(0)
        );
    }

    #[test]
    fn cdp_observes_logical_pages_and_captures_scaled_output_from_one_print_document() {
        if discover_chromium_executable().executable.is_none() {
            return;
        }
        let temp = tempdir().unwrap();
        let html_path = temp.path().join("print.html");
        fs::write(
            &html_path,
            r#"<!doctype html>
<style>
@page { size: 13.333333in 7.5in; margin: 0; }
html, body { margin: 0; width: 1280px; }
.zpres-print-slide, .zpres-slide-canvas, .zpres-slide-content {
  box-sizing: border-box; width: 1280px; height: 720px;
}
.zpres-print-slide { overflow: hidden; break-after: page; }
.zpres-block { width: 600px; height: 200px; }
#alpha { background: rgb(217, 57, 74) url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='1' height='1'%3E%3Crect width='1' height='1' fill='gold'/%3E%3C/svg%3E"); }
#beta { background: rgb(38, 117, 182) url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='1' height='1'%3E%3Crect width='1' height='1' fill='cyan'/%3E%3C/svg%3E"); }
</style>
<body data-zpres-ready="false" data-zpres-ready-target="pdf" data-zpres-page-count="2">
<section id="alpha" class="zpres-print-slide" data-page="1" data-slide-id="alpha" data-slide-role="main">
  <div class="zpres-slide-canvas"><div class="zpres-slide-content">
    <div class="zpres-slide-body zpres-block" data-block-type="paragraph"><h1>Alpha</h1><p>First page.</p>
      <img alt="red pixel" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='1' height='1'%3E%3Crect width='1' height='1' fill='red'/%3E%3C/svg%3E">
    </div>
  </div></div>
</section>
<section id="beta" class="zpres-print-slide" data-page="2" data-slide-id="beta" data-slide-role="detail">
  <div class="zpres-slide-canvas"><div class="zpres-slide-content">
    <div class="zpres-slide-body zpres-block" data-block-type="paragraph"><h1>Beta</h1><p>Second page.</p>
      <img alt="blue pixel" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='1' height='1'%3E%3Crect width='1' height='1' fill='blue'/%3E%3C/svg%3E">
    </div>
  </div></div>
</section>
<script>
window.zpresTestCaptureResizeCount = 0;
window.addEventListener("resize", () => {
  window.zpresTestCaptureResizeCount += 1;
});
window.zpresStaticReady = new Promise((resolve) => {
  requestAnimationFrame(() => requestAnimationFrame(() => {
    document.body.dataset.zpresAutoscaleReady = "true";
    document.body.dataset.zpresReady = "true";
    resolve({ ready: true, target: "pdf", pageCount: 2, errors: [] });
  }));
});
</script>
</body>"#,
        )
        .unwrap();
        let url = format!("file://{}", html_path.canonicalize().unwrap().display());
        let viewport = PngViewport {
            width: 1280,
            height: 720,
        };
        let mut browser = ChromiumSession::launch(ChromiumSessionOptions::default()).unwrap();

        browser.load_print_document(&url, viewport).unwrap();
        let readiness = browser
            .await_static_readiness(Duration::from_secs(5))
            .unwrap();
        assert!(readiness.promise_present);
        assert_eq!(readiness.promise_status, BrowserPromiseStatus::Fulfilled);
        assert!(readiness.ready);
        assert_eq!(readiness.target.as_deref(), Some("pdf"));
        assert_eq!(readiness.declared_page_count, Some(2));
        let capture_resize_count = browser
            .evaluate("window.zpresTestCaptureResizeCount")
            .unwrap();

        let pages = browser.print_pages().unwrap();
        assert_eq!(
            pages
                .iter()
                .map(|page| (page.index, page.page, page.slide_id.as_str()))
                .collect::<Vec<_>>(),
            vec![(0, Some(1), "alpha"), (1, Some(2), "beta")]
        );

        browser
            .evaluate(
                "window.__zpresVisualObservationMode = 'print'; window.__zpresRequestedPrintPageIndex = 0; true",
            )
            .unwrap();
        browser.suppress_semantic_text().unwrap();
        let suppressed_slide_ids = browser
            .evaluate(
                "Array.from(document.querySelectorAll('[data-zpres-semantic-text-suppressed]')).map((element) => element.id)",
            )
            .unwrap();
        assert_eq!(suppressed_slide_ids, json!(["alpha"]));
        browser.restore_semantic_text().unwrap();

        let output_viewport = PngViewport {
            width: 1920,
            height: 1080,
        };
        let alpha = browser
            .capture_print_page_with_output_viewport(0, output_viewport)
            .unwrap();
        let beta = browser
            .capture_print_page_with_output_viewport(1, output_viewport)
            .unwrap();
        assert_eq!(alpha.observation.slide_id, "alpha");
        assert_eq!(alpha.observation.title, "Alpha");
        let alpha_bounds = alpha.observation.slide_bounds.unwrap();
        assert_eq!((alpha_bounds.width, alpha_bounds.height), (1280.0, 720.0));
        assert_eq!(alpha.observation.images.len(), 1);
        assert!(alpha.observation.images[0].source.contains("red"));
        assert_eq!(alpha.observation.background_images.len(), 1);
        assert!(
            alpha.observation.background_images[0]
                .source
                .contains("gold")
        );
        assert_eq!(beta.observation.slide_id, "beta");
        assert_eq!(beta.observation.title, "Beta");
        assert_eq!(beta.observation.images.len(), 1);
        assert!(beta.observation.images[0].source.contains("blue"));
        assert_eq!(beta.observation.background_images.len(), 1);
        assert!(
            beta.observation.background_images[0]
                .source
                .contains("cyan")
        );
        assert_eq!(
            image::load_from_memory(&alpha.png).unwrap().dimensions(),
            (1920, 1080)
        );
        assert_eq!(
            image::load_from_memory(&beta.png).unwrap().dimensions(),
            (1920, 1080)
        );
        assert_ne!(alpha.png, alpha.semantic_text_suppressed_png);
        assert_ne!(beta.png, beta.semantic_text_suppressed_png);
        assert_eq!(
            browser
                .evaluate("window.zpresTestCaptureResizeCount")
                .unwrap(),
            capture_resize_count,
            "offscreen print capture restarted resize-driven autoscaling"
        );
    }

    #[test]
    fn cdp_traverses_production_screen_routes_and_reports_declared_geometry_violations() {
        if discover_chromium_executable().executable.is_none() {
            return;
        }
        let temp = tempdir().unwrap();
        let html_path = temp.path().join("index.html");
        fs::write(
            &html_path,
            r##"<!doctype html>
<style>
html, body { box-sizing: border-box; width: 100%; height: 100%; margin: 0; overflow: hidden; }
*, *::before, *::after { box-sizing: inherit; }
.reveal, .slides, .zpres-section-stack, .zpres-slide { width: 100%; height: 100%; }
.zpres-section-stack, .zpres-slide { display: none; }
.zpres-section-stack.is-active, .zpres-slide.is-active { display: block; }
.zpres-slide { position: relative; }
.zpres-slide-canvas, .zpres-slide-content { position: relative; width: 100%; height: 100%; overflow: hidden; }
.zpres-block { font: 24px/1.3 system-ui, sans-serif; }
.zpres-slide-footer { position: absolute; left: 0; right: 0; bottom: 0; height: 40px; }
.fragment { visibility: hidden; opacity: 0; transition: opacity 160ms linear; }
.fragment.is-visible { visibility: visible; opacity: 1; }
</style>
<body data-zpres-autoscale-ready="true">
<main class="reveal"><div class="slides">
  <section class="zpres-section-stack is-active" data-section-index="1">
    <section class="zpres-slide is-active" data-slide-id="alpha" data-slide-role="main">
      <div class="zpres-slide-canvas"><div class="zpres-slide-content">
        <div class="zpres-block" data-block-type="paragraph"><h1>Alpha</h1><p>Initial state.</p>
          <span class="fragment" data-step-index="1">Revealed state.</span>
        </div>
      </div></div>
      <footer class="zpres-slide-footer">Alpha footer</footer>
    </section>
  </section>
  <section class="zpres-section-stack" data-section-index="2">
    <section class="zpres-slide" data-slide-id="beta" data-slide-role="detail">
      <div class="zpres-slide-canvas"><div class="zpres-slide-content">
        <header class="zpres-slide-header" style="position:absolute;left:400px;top:300px;width:220px;height:40px">Protected title</header>
        <div class="zpres-slide-body" style="position:absolute;left:400px;top:305px;width:220px;height:40px">Body text remains inside the Slide.</div>
        <div id="tight-block" class="zpres-block" data-block-type="paragraph" style="position:relative;width:200px;height:80px;margin:10px">
          <span id="outside-block" style="position:absolute;left:230px;top:0;width:180px;height:32px">Outside block</span>
        </div>
        <figure class="zpres-block" data-block-type="figure" style="position:absolute;left:10px;top:120px;width:300px;height:160px;margin:0">
          <img alt="fixture" style="position:absolute;left:0;top:0;width:300px;height:130px" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='300' height='130'%3E%3Crect width='300' height='130' fill='navy'/%3E%3C/svg%3E">
          <figcaption style="position:absolute;left:0;top:110px;width:300px;height:40px">Overlapping caption</figcaption>
        </figure>
        <div class="zpres-block-layout" data-layout-kind="overlay">
          <section class="zpres-layout-region" data-region-role="annotation" style="position:absolute;left:20px;top:235px;width:180px;height:30px">Overlay annotation</section>
          <section class="zpres-layout-region" data-region-role="annotation" style="position:absolute;left:410px;top:305px;width:180px;height:30px">Title collision</section>
          <section class="zpres-layout-region" data-region-role="annotation" style="position:absolute;left:350px;top:120px;width:80px;height:30px">Safe area</section>
          <section class="zpres-layout-region" data-region-role="annotation" style="position:absolute;left:400px;bottom:5px;width:180px;height:30px">Footer collision</section>
        </div>
        <div class="zpres-block" data-block-type="paragraph" style="position:absolute;left:10px;bottom:0;width:260px;height:40px"><span>Footer collision</span></div>
      </div></div>
      <footer class="zpres-slide-footer">Beta footer</footer>
    </section>
  </section>
</div></main>
<script>
const screenRoutes = [
  { hash: "#/0/0", section: 0, detail: 0, step: 0, step_count: 1, slide_id: "alpha", role: "main", generated: null },
  { hash: "#/0/0/1", section: 0, detail: 0, step: 1, step_count: 1, slide_id: "alpha", role: "main", generated: null },
  { hash: "#/1/0", section: 1, detail: 0, step: 0, step_count: 0, slide_id: "beta", role: "detail", generated: null },
];
let screenState = { section: 0, detail: 0, step: 0 };
const stacks = Array.from(document.querySelectorAll(".zpres-section-stack"));
function descriptor() {
  return screenRoutes.find((route) => route.section === screenState.section
    && route.detail === screenState.detail && route.step === screenState.step);
}
function nextFrame() { return new Promise((resolve) => requestAnimationFrame(resolve)); }
window.zpresPresentation = Object.freeze({
  ready: Promise.resolve(),
  routes() { return screenRoutes.map((route) => ({ ...route })); },
  async navigate(candidate) {
    const route = screenRoutes.find((value) => value.section === candidate.section
      && value.detail === candidate.detail && value.step === (candidate.step ?? 0));
    if (!route) throw new RangeError("invalid fixture route");
    screenState = { section: route.section, detail: route.detail, step: route.step };
    stacks.forEach((stack, section) => {
      stack.classList.toggle("is-active", section === screenState.section);
      Array.from(stack.querySelectorAll(".zpres-slide")).forEach((slide, detail) => {
        const active = section === screenState.section && detail === screenState.detail;
        slide.classList.toggle("is-active", active);
        slide.querySelectorAll(".fragment").forEach((fragment) => {
          const index = Number(fragment.getAttribute("data-step-index") || "0");
          fragment.classList.toggle("is-visible", active && index <= screenState.step);
        });
      });
    });
    history.replaceState(null, "", route.hash);
    await nextFrame();
    await nextFrame();
    return { ...descriptor() };
  },
});
</script>
</body>"##,
        )
        .unwrap();
        let url = format!("file://{}", html_path.canonicalize().unwrap().display());
        let viewport = PngViewport::default();
        let mut browser = ChromiumSession::launch(ChromiumSessionOptions::default()).unwrap();

        browser.load_screen_document(&url, viewport).unwrap();
        let routes = browser.screen_routes().unwrap();
        assert_eq!(
            routes
                .iter()
                .map(|route| route.hash.as_str())
                .collect::<Vec<_>>(),
            vec!["#/0/0", "#/0/0/1", "#/1/0"]
        );

        let mut mismatched = routes[0].clone();
        mismatched.role = "detail".to_string();
        assert!(matches!(
            browser.capture_screen_route(&mismatched).unwrap_err(),
            ChromiumError::ScreenRouteMismatch { .. }
        ));

        let alpha = browser.capture_screen_route(&routes[1]).unwrap();
        assert_eq!(alpha.observation.slide_id, "alpha");
        assert_eq!(alpha.observation.screen_route.as_deref(), Some("#/0/0/1"));
        assert_eq!(alpha.observation.screen_step, Some(1));
        assert_eq!(alpha.observation.active_stack_count, Some(1));
        assert_eq!(alpha.observation.active_slide_count, Some(1));
        assert_eq!(alpha.observation.screen_bounds.unwrap().width, 1280.0);
        assert_eq!(alpha.observation.root_bounds.unwrap().height, 720.0);
        assert_eq!(
            browser
                .evaluate("getComputedStyle(document.querySelector('.fragment')).opacity")
                .unwrap(),
            json!("1")
        );
        assert_eq!(
            browser
                .evaluate("document.getAnimations().filter((animation) => animation.playState === 'running' || animation.playState === 'pending').length")
                .unwrap(),
            json!(0)
        );
        assert_eq!(
            image::load_from_memory(&alpha.png).unwrap().dimensions(),
            (1280, 720)
        );

        let beta = browser.capture_screen_route(&routes[2]).unwrap();
        let kinds = beta
            .observation
            .geometry_violations
            .iter()
            .map(|violation| violation.kind.as_str())
            .collect::<BTreeSet<_>>();
        assert!(kinds.contains("outside-content-block"));
        assert!(kinds.contains("caption-collision"));
        assert!(kinds.contains("overlay-caption-collision"));
        assert!(kinds.contains("overlay-title-collision"));
        assert!(kinds.contains("overlay-footer-collision"));
        assert!(kinds.contains("overlay-safe-area-collision"));
        assert!(kinds.contains("footer-overlap"));
        assert!(kinds.contains("header-body-overlap"));
        let header_body = beta
            .observation
            .geometry_violations
            .iter()
            .find(|violation| violation.kind == "header-body-overlap")
            .unwrap();
        assert_eq!(header_body.element, "header.zpres-slide-header");
        assert_eq!(header_body.boundary, "div.zpres-slide-body");
        let intersection = header_body.intersection_bounds.unwrap();
        assert!(intersection.width > 0.0);
        assert!(intersection.height > 0.0);
        assert!(beta.observation.geometry_violations.len() <= 64);
        assert_eq!(beta.observation.document_scroll.unwrap().maximum(), 0.0);
    }

    #[test]
    fn retained_slide_runtime_is_directional_inert_step_safe_and_cleans_rapid_navigation() {
        if discover_chromium_executable().executable.is_none() {
            return;
        }
        let temp = tempdir().unwrap();
        let html_path = temp.path().join("retained-slide.html");
        let mut document = String::from(
            r##"<!doctype html><meta charset="utf-8"><style>
html,body,.reveal,.slides,.zpres-section-stack,.zpres-slide{width:100%;height:100%;margin:0}
.slides{position:relative}.zpres-section-stack,.zpres-slide{position:absolute;inset:0;display:none}
.zpres-section-stack:is(.is-active,.is-leaving),.zpres-slide:is(.is-active,.is-leaving){display:grid}
.zpres-slide{--zpres-retain-leaving-slide:1;background:#b8ad9c}.zpres-slide-frame{margin:24px;background:#fff;height:calc(100% - 48px)}
.zpres-slide.is-leaving{z-index:4;background:transparent}.zpres-slide.is-entering{z-index:3}
.zpres-slide.is-leaving .zpres-slide-frame{animation:leave 360ms linear both}.zpres-slide.is-entering .zpres-slide-frame{animation:enter 360ms linear both}
.zpres-slide[data-zpres-transition-direction="backward"].is-leaving .zpres-slide-frame{animation-name:leave-back}
@keyframes leave{to{transform:translate(-1320px,-96px) rotate(-7deg)}}@keyframes leave-back{to{transform:translate(1320px,-96px) rotate(7deg)}}
@keyframes enter{from{transform:translate(12px,9px) rotate(.8deg)}}
@media(prefers-reduced-motion:reduce){.zpres-slide:is(.is-entering,.is-leaving) .zpres-slide-frame{animation:none;transform:none}}
</style><body data-zpres-autoscale-ready="true"><main class="reveal"><div class="slides">
<section class="zpres-section-stack" data-section-index="1"><section class="zpres-slide" data-slide-id="alpha" data-slide-role="main"><div class="zpres-slide-frame"><div class="zpres-slide-content"><h1>Alpha</h1><span class="zpres-step fragment" data-step-index="1">Step</span></div></div></section></section>
<section class="zpres-section-stack" data-section-index="2"><section class="zpres-slide" data-slide-id="beta" data-slide-role="main"><div class="zpres-slide-frame"><div class="zpres-slide-content"><h1>Beta</h1><a href="#target">link</a></div></div></section></section>
</div></main><script>window.zpresAutoscaleAll=()=>Promise.resolve();</script><script>"##,
        );
        document.push_str(crate::html::REVEAL_JS);
        document.push_str("</script></body>");
        fs::write(&html_path, document).unwrap();
        let url = format!("file://{}", html_path.canonicalize().unwrap().display());
        let mut browser = ChromiumSession::launch(ChromiumSessionOptions::default()).unwrap();
        browser
            .load_screen_document(&url, PngViewport::default())
            .unwrap();
        browser
            .evaluate("window.zpresPresentation.ready.then(()=>true)")
            .unwrap();
        let evidence_dir = std::env::var_os("ZPRES_FOLIO_MOTION_EVIDENCE_DIR").map(PathBuf::from);
        if let Some(path) = &evidence_dir {
            fs::create_dir_all(path).unwrap();
        }

        let forward = browser
            .evaluate(
                r#"(() => { window.__sheetNav = window.zpresPresentation.navigate({section:1,detail:0,step:0}); const old=document.querySelector('[data-slide-id="alpha"]'); const next=document.querySelector('[data-slide-id="beta"]'); return { leaving:old.classList.contains('is-leaving'), hidden:old.getAttribute('aria-hidden'), inert:old.hasAttribute('inert'), direction:old.dataset.zpresTransitionDirection, entering:next.classList.contains('is-entering') }; })()"#,
            )
            .unwrap();
        assert_eq!(forward["leaving"], true);
        assert_eq!(forward["hidden"], "true");
        assert_eq!(forward["inert"], true);
        assert_eq!(forward["direction"], "forward");
        assert_eq!(forward["entering"], true);
        if let Some(path) = &evidence_dir {
            fs::write(
                path.join("forward-initial.png"),
                browser.capture_screenshot().unwrap(),
            )
            .unwrap();
            thread::sleep(Duration::from_millis(180));
            fs::write(
                path.join("forward-lift.png"),
                browser.capture_screenshot().unwrap(),
            )
            .unwrap();
        }
        assert_eq!(
            browser.evaluate("window.__sheetNav.then(()=>document.querySelectorAll('.is-leaving,.is-entering').length)").unwrap(),
            json!(0)
        );
        if let Some(path) = &evidence_dir {
            fs::write(
                path.join("forward-settled.png"),
                browser.capture_screenshot().unwrap(),
            )
            .unwrap();
        }

        let backward = browser
            .evaluate(
                r#"(() => { window.__sheetNav = window.zpresPresentation.navigate({section:0,detail:0,step:0}); return document.querySelector('[data-slide-id="beta"]').dataset.zpresTransitionDirection; })()"#,
            )
            .unwrap();
        assert_eq!(backward, "backward");
        if let Some(path) = &evidence_dir {
            thread::sleep(Duration::from_millis(180));
            fs::write(
                path.join("backward-lift.png"),
                browser.capture_screenshot().unwrap(),
            )
            .unwrap();
        }
        browser
            .evaluate("window.__sheetNav.then(()=>true)")
            .unwrap();

        let step = browser
            .evaluate(
                r#"window.zpresPresentation.navigate({section:0,detail:0,step:1}).then(()=>({leaving:document.querySelectorAll('.is-leaving').length,entering:document.querySelectorAll('.is-entering').length,current:document.querySelector('[data-slide-id="alpha"]').dataset.currentStep}))"#,
            )
            .unwrap();
        assert_eq!(step["leaving"], 0);
        assert_eq!(step["entering"], 0);
        assert_eq!(step["current"], "1");
        if let Some(path) = &evidence_dir {
            fs::write(
                path.join("step-no-shuffle.png"),
                browser.capture_screenshot().unwrap(),
            )
            .unwrap();
        }

        let rapid = browser
            .evaluate(
                r#"(async()=>{const first=window.zpresPresentation.navigate({section:1,detail:0,step:0});await new Promise(r=>setTimeout(r,30));const second=window.zpresPresentation.navigate({section:0,detail:0,step:0});let firstResult='resolved';try{await first}catch(error){firstResult=error.name}await second;return{firstResult,leaving:document.querySelectorAll('.is-leaving').length,entering:document.querySelectorAll('.is-entering').length,active:document.querySelectorAll('.zpres-slide.is-active').length,slide:document.querySelector('.zpres-slide.is-active')?.dataset.slideId}})()"#,
            )
            .unwrap();
        assert_eq!(rapid["firstResult"], "AbortError");
        assert_eq!(rapid["leaving"], 0);
        assert_eq!(rapid["entering"], 0);
        assert_eq!(rapid["active"], 1);
        assert_eq!(rapid["slide"], "alpha");
        if let Some(path) = &evidence_dir {
            fs::write(
                path.join("rapid-cleanup.png"),
                browser.capture_screenshot().unwrap(),
            )
            .unwrap();
        }

        browser
            .send_page_command(
                "Emulation.setEmulatedMedia",
                json!({"features":[{"name":"prefers-reduced-motion","value":"reduce"}]}),
            )
            .unwrap();
        let reduced = browser
            .evaluate(
                r#"(()=>{window.__sheetNav=window.zpresPresentation.navigate({section:1,detail:0,step:0});const frame=document.querySelector('[data-slide-id="beta"] .zpres-slide-frame');const style=getComputedStyle(frame);return{animation:style.animationName,transform:style.transform}})()"#,
            )
            .unwrap();
        assert_eq!(reduced["animation"], "none");
        assert_eq!(reduced["transform"], "none");
        assert_eq!(
            browser.evaluate("window.__sheetNav.then(()=>document.querySelectorAll('.is-leaving,.is-entering').length)").unwrap(),
            json!(0)
        );
        if let Some(path) = &evidence_dir {
            fs::write(
                path.join("motion-trace.json"),
                serde_json::to_vec_pretty(&json!({
                    "forward_initial": forward,
                    "backward_direction": backward,
                    "step": step,
                    "rapid_cleanup": rapid,
                    "reduced_motion": reduced,
                    "duration_ms": 360,
                }))
                .unwrap(),
            )
            .unwrap();
        }

        if let (Some(html_path), Some(path)) = (
            std::env::var_os("ZPRES_WEDDING_MOTION_HTML").map(PathBuf::from),
            std::env::var_os("ZPRES_WEDDING_MOTION_EVIDENCE_DIR").map(PathBuf::from),
        ) {
            fs::create_dir_all(&path).unwrap();
            let url = format!("file://{}", html_path.canonicalize().unwrap().display());
            let mut wedding = ChromiumSession::launch(ChromiumSessionOptions::default()).unwrap();
            wedding
                .load_screen_document(&url, PngViewport::default())
                .unwrap();
            wedding
                .evaluate("window.zpresPresentation.ready.then(()=>true)")
                .unwrap();

            fs::write(
                path.join("forward-initial.png"),
                wedding.capture_screenshot().unwrap(),
            )
            .unwrap();
            wedding
                .evaluate(
                    "(()=>{window.__weddingNav=window.zpresPresentation.navigate({section:1,detail:0,step:0});return true})()",
                )
                .unwrap();
            thread::sleep(Duration::from_millis(330));
            let forward_reveal = wedding
                .evaluate(
                    r#"(()=>{const leavingRect=document.querySelector('.zpres-slide.is-leaving .zpres-slide-frame')?.getBoundingClientRect();const enteringRect=document.querySelector('.zpres-slide.is-entering .zpres-slide-frame')?.getBoundingClientRect();const center=document.elementFromPoint(640,360)?.closest('.zpres-slide');return{leaving:leavingRect&&{left:leavingRect.left,right:leavingRect.right},entering:enteringRect&&{left:enteringRect.left,right:enteringRect.right},center:center?.dataset.slideId}})()"#,
                )
                .unwrap();
            assert!(forward_reveal["leaving"]["right"].as_f64().unwrap() < 640.0);
            assert_eq!(forward_reveal["center"], "background-image-splash");
            fs::write(
                path.join("opening-forward-photo.png"),
                wedding.capture_screenshot().unwrap(),
            )
            .unwrap();
            wedding
                .evaluate("window.__weddingNav.then(()=>true)")
                .unwrap();
            fs::write(
                path.join("opening-photo-settled.png"),
                wedding.capture_screenshot().unwrap(),
            )
            .unwrap();

            wedding
                .evaluate(
                    "(()=>{window.__weddingNav=window.zpresPresentation.navigate({section:2,detail:0,step:0});return true})()",
                )
                .unwrap();
            thread::sleep(Duration::from_millis(240));
            let forward_return = wedding
                .evaluate(
                    r#"(()=>{const enteringRect=document.querySelector('.zpres-slide.is-entering .zpres-slide-frame')?.getBoundingClientRect();return{entering:enteringRect&&{left:enteringRect.left,right:enteringRect.right},slide:document.querySelector('.zpres-slide.is-entering')?.dataset.slideId}})()"#,
                )
                .unwrap();
            assert_eq!(forward_return["slide"], "section-2-main");
            assert!(forward_return["entering"]["left"].as_f64().unwrap() < 0.0);
            fs::write(
                path.join("opening-forward-pile-return.png"),
                wedding.capture_screenshot().unwrap(),
            )
            .unwrap();
            wedding
                .evaluate("window.__weddingNav.then(()=>true)")
                .unwrap();
            fs::write(
                path.join("opening-forward-settled.png"),
                wedding.capture_screenshot().unwrap(),
            )
            .unwrap();

            fs::write(
                path.join("opening-backward-initial.png"),
                wedding.capture_screenshot().unwrap(),
            )
            .unwrap();
            wedding
                .evaluate(
                    "(()=>{window.__weddingNav=window.zpresPresentation.navigate({section:1,detail:0,step:0});return true})()",
                )
                .unwrap();
            thread::sleep(Duration::from_millis(330));
            let backward_reveal = wedding
                .evaluate(
                    r#"(()=>{const leavingRect=document.querySelector('.zpres-slide.is-leaving .zpres-slide-frame')?.getBoundingClientRect();const enteringRect=document.querySelector('.zpres-slide.is-entering .zpres-slide-frame')?.getBoundingClientRect();const center=document.elementFromPoint(640,360)?.closest('.zpres-slide');return{leaving:leavingRect&&{left:leavingRect.left,right:leavingRect.right},entering:enteringRect&&{left:enteringRect.left,right:enteringRect.right},center:center?.dataset.slideId}})()"#,
                )
                .unwrap();
            assert!(backward_reveal["leaving"]["right"].as_f64().unwrap() < 640.0);
            assert_eq!(backward_reveal["center"], "background-image-splash");
            fs::write(
                path.join("opening-backward-photo.png"),
                wedding.capture_screenshot().unwrap(),
            )
            .unwrap();
            wedding
                .evaluate("window.__weddingNav.then(()=>true)")
                .unwrap();
            fs::write(
                path.join("opening-backward-photo-settled.png"),
                wedding.capture_screenshot().unwrap(),
            )
            .unwrap();

            wedding
                .evaluate(
                    "(()=>{window.__weddingNav=window.zpresPresentation.navigate({section:0,detail:0,step:0});return true})()",
                )
                .unwrap();
            thread::sleep(Duration::from_millis(240));
            let backward_return = wedding
                .evaluate(
                    r#"(()=>{const enteringRect=document.querySelector('.zpres-slide.is-entering .zpres-slide-frame')?.getBoundingClientRect();return{entering:enteringRect&&{left:enteringRect.left,right:enteringRect.right},slide:document.querySelector('.zpres-slide.is-entering')?.dataset.slideId}})()"#,
                )
                .unwrap();
            assert_eq!(backward_return["slide"], "section-1-main");
            assert!(backward_return["entering"]["left"].as_f64().unwrap() < 0.0);
            fs::write(
                path.join("opening-backward-title-return.png"),
                wedding.capture_screenshot().unwrap(),
            )
            .unwrap();
            wedding
                .evaluate("window.__weddingNav.then(()=>true)")
                .unwrap();
            fs::write(
                path.join("opening-backward-settled.png"),
                wedding.capture_screenshot().unwrap(),
            )
            .unwrap();

            wedding
                .evaluate("window.zpresPresentation.navigate({section:2,detail:0,step:0})")
                .unwrap();
            wedding
                .evaluate(
                    "(()=>{window.__weddingNav=window.zpresPresentation.navigate({section:3,detail:0,step:0});return true})()",
                )
                .unwrap();
            thread::sleep(Duration::from_millis(240));
            let ordinary_forward = wedding
                .evaluate(
                    r#"(()=>{const leavingBackground=document.querySelector('.zpres-slide.is-leaving > .zpres-slide-background');const entering=document.querySelector('.zpres-slide.is-entering .zpres-slide-frame')?.getBoundingClientRect();const center=document.elementFromPoint(640,360)?.closest('.zpres-slide');return{leavingBackgroundOpacity:leavingBackground&&getComputedStyle(leavingBackground).opacity,entering:entering&&{left:entering.left,right:entering.right},center:center?.dataset.slideId}})()"#,
                )
                .unwrap();
            assert_eq!(ordinary_forward["leavingBackgroundOpacity"], "0");
            assert!(ordinary_forward["entering"]["left"].as_f64().unwrap() > 0.0);
            assert!(ordinary_forward["entering"]["right"].as_f64().unwrap() < 1280.0);
            assert_eq!(ordinary_forward["center"], "section-3-main");
            fs::write(
                path.join("ordinary-forward-next-paper-underneath.png"),
                wedding.capture_screenshot().unwrap(),
            )
            .unwrap();
            wedding
                .evaluate("window.__weddingNav.then(()=>true)")
                .unwrap();
            wedding
                .evaluate(
                    "(()=>{window.__weddingNav=window.zpresPresentation.navigate({section:2,detail:0,step:0});return true})()",
                )
                .unwrap();
            thread::sleep(Duration::from_millis(240));
            let ordinary_backward = wedding
                .evaluate(
                    r#"(()=>{const leavingSlide=document.querySelector('.zpres-slide.is-leaving');const enteringSlide=document.querySelector('.zpres-slide.is-entering');const leaving=leavingSlide?.querySelector('.zpres-slide-frame')?.getBoundingClientRect();const entering=enteringSlide?.querySelector('.zpres-slide-frame')?.getBoundingClientRect();const leavingBackground=leavingSlide?.querySelector(':scope > .zpres-slide-background');const enteringBackground=enteringSlide?.querySelector(':scope > .zpres-slide-background');return{leaving:leaving&&{left:leaving.left,right:leaving.right},entering:entering&&{left:entering.left,right:entering.right},leavingBackgroundOpacity:leavingBackground&&getComputedStyle(leavingBackground).opacity,enteringBackgroundOpacity:enteringBackground&&getComputedStyle(enteringBackground).opacity,direction:enteringSlide?.dataset.zpresTransitionDirection}})()"#,
                )
                .unwrap();
            assert_eq!(ordinary_backward["direction"], "backward");
            assert_eq!(ordinary_backward["leavingBackgroundOpacity"], "1");
            assert_eq!(ordinary_backward["enteringBackgroundOpacity"], "0");
            assert!(ordinary_backward["entering"]["left"].as_f64().unwrap() < 0.0);
            assert!(ordinary_backward["leaving"]["left"].as_f64().unwrap() > 0.0);
            fs::write(
                path.join("ordinary-backward-paper-return.png"),
                wedding.capture_screenshot().unwrap(),
            )
            .unwrap();
            wedding
                .evaluate("window.__weddingNav.then(()=>true)")
                .unwrap();

            let step_state = wedding
                .evaluate(
                    r#"(async()=>{const stacks=Array.from(document.querySelectorAll('.zpres-section-stack'));const index=stacks.findIndex(stack=>stack.querySelector('.zpres-slide[data-slide-role="detail"] [data-step-index]'));await window.zpresPresentation.navigate({section:index,detail:1,step:0});await window.zpresPresentation.navigate({section:index,detail:1,step:1});return{index,leaving:document.querySelectorAll('.is-leaving').length,entering:document.querySelectorAll('.is-entering').length}})()"#,
                )
                .unwrap();
            assert_eq!(step_state["leaving"], 0);
            assert_eq!(step_state["entering"], 0);
            fs::write(
                path.join("steps-motionless.png"),
                wedding.capture_screenshot().unwrap(),
            )
            .unwrap();

            let rapid_state = wedding
                .evaluate(
                    r#"(async()=>{const first=window.zpresPresentation.navigate({section:3,detail:0,step:0});await new Promise(resolve=>setTimeout(resolve,30));const second=window.zpresPresentation.navigate({section:2,detail:0,step:0});let firstResult='resolved';try{await first}catch(error){firstResult=error.name}await second;return{firstResult,leaving:document.querySelectorAll('.is-leaving').length,entering:document.querySelectorAll('.is-entering').length,active:document.querySelectorAll('.zpres-slide.is-active').length}})()"#,
                )
                .unwrap();
            assert_eq!(rapid_state["firstResult"], "AbortError");
            assert_eq!(rapid_state["leaving"], 0);
            assert_eq!(rapid_state["entering"], 0);
            assert_eq!(rapid_state["active"], 1);
            fs::write(
                path.join("rapid-navigation-cleanup.png"),
                wedding.capture_screenshot().unwrap(),
            )
            .unwrap();

            let ending_stack_count = wedding
                .evaluate("document.querySelectorAll('.zpres-section-stack').length")
                .unwrap()
                .as_u64()
                .unwrap();
            for (offset, expected_papers) in [(3_u64, 3_u64), (2, 2), (1, 1)] {
                let section = ending_stack_count - offset;
                let paper_count = wedding
                    .evaluate(&format!(
                        r#"window.zpresPresentation.navigate({{section:{section},detail:0,step:0}}).then(()=>{{const slide=document.querySelector('.zpres-slide.is-active');return 1+(getComputedStyle(slide,'::before').display==='none'?0:1)+(getComputedStyle(slide,'::after').display==='none'?0:1)}})"#
                    ))
                    .unwrap();
                assert_eq!(paper_count, expected_papers);
                fs::write(
                    path.join(format!("ending-pile-{expected_papers}-papers.png")),
                    wedding.capture_screenshot().unwrap(),
                )
                .unwrap();
            }

            wedding
                .send_page_command(
                    "Emulation.setEmulatedMedia",
                    json!({"features":[{"name":"prefers-reduced-motion","value":"reduce"}]}),
                )
                .unwrap();
            let reduced_state = wedding
                .evaluate(
                    r#"(()=>{window.__weddingNav=window.zpresPresentation.navigate({section:3,detail:0,step:0});const frames=Array.from(document.querySelectorAll('.zpres-slide:is(.is-entering,.is-leaving) .zpres-slide-frame'));return{animations:frames.map(frame=>getComputedStyle(frame).animationName),transforms:frames.map(frame=>getComputedStyle(frame).transform)}})()"#,
                )
                .unwrap();
            assert!(
                reduced_state["animations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|value| value == "none")
            );
            assert!(
                reduced_state["transforms"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|value| value == "none")
            );
            wedding
                .evaluate("window.__weddingNav.then(()=>true)")
                .unwrap();
            fs::write(
                path.join("reduced-motion-settled.png"),
                wedding.capture_screenshot().unwrap(),
            )
            .unwrap();
            fs::write(
                path.join("motion-trace.json"),
                serde_json::to_vec_pretty(&json!({
                    "duration_ms": 420,
                    "photo_capture_ms": 330,
                    "return_capture_ms": 240,
                    "forward_reveal": forward_reveal,
                    "forward_return": forward_return,
                    "backward_reveal": backward_reveal,
                    "backward_return": backward_return,
                    "ordinary_backward": ordinary_backward,
                    "step_state": step_state,
                    "rapid_state": rapid_state,
                    "reduced_state": reduced_state,
                }))
                .unwrap(),
            )
            .unwrap();
        }
    }

    #[test]
    fn cdp_rejects_hostile_overlapping_print_page_inventory() {
        if discover_chromium_executable().executable.is_none() {
            return;
        }
        let temp = tempdir().unwrap();
        let html_path = temp.path().join("overlapping-print-pages.html");
        fs::write(
            &html_path,
            r#"<!doctype html>
<style>
html, body { margin: 0; width: 1280px; height: 720px; }
.zpres-print-slide {
  position: absolute;
  inset: 0;
  box-sizing: border-box;
  width: 1280px;
  height: 720px;
}
</style>
<body>
<section class="zpres-print-slide" data-page="1" data-slide-id="first"></section>
<section class="zpres-print-slide" data-page="2" data-slide-id="second"></section>
</body>"#,
        )
        .unwrap();
        let url = format!("file://{}", html_path.canonicalize().unwrap().display());
        let mut browser = ChromiumSession::launch(ChromiumSessionOptions::default()).unwrap();
        browser
            .load_print_document(&url, PngViewport::default())
            .unwrap();

        let error = browser.print_pages().unwrap_err();

        assert!(matches!(
            error,
            ChromiumError::InvalidPrintPageLayout { .. }
        ));
        assert!(error.to_string().contains("ordered and non-overlapping"));
        assert!(error.to_string().contains("second"));
    }

    #[test]
    fn cdp_prints_pdf_bytes_from_the_loaded_ready_document() {
        if discover_chromium_executable().executable.is_none() {
            return;
        }
        let temp = tempdir().unwrap();
        let html_path = temp.path().join("print-pdf.html");
        fs::write(
            &html_path,
            r#"<!doctype html>
<style>
@page { size: 13.333in 7.5in; margin: 0; }
html, body { margin: 0; }
.zpres-print-slide { box-sizing: border-box; width: 13.2in; height: 7.4in; overflow: hidden; break-after: page; }
.zpres-print-slide:last-of-type { break-after: auto; }
</style>
<body data-zpres-ready="true" data-zpres-ready-target="pdf" data-zpres-page-count="2">
<section class="zpres-print-slide" data-page="1" data-slide-id="one"><h1>One</h1></section>
<section class="zpres-print-slide" data-page="2" data-slide-id="two"><h1>Two</h1></section>
<script>
window.zpresStaticReadyState = { status: "ready", errors: [] };
window.zpresStaticReady = Promise.resolve(window.zpresStaticReadyState);
</script>
</body>"#,
        )
        .unwrap();
        let url = format!("file://{}", html_path.canonicalize().unwrap().display());
        let mut browser = ChromiumSession::launch(ChromiumSessionOptions::default()).unwrap();
        browser
            .load_print_document(
                &url,
                PngViewport {
                    width: 1280,
                    height: 720,
                },
            )
            .unwrap();
        let readiness = browser
            .await_static_readiness(Duration::from_secs(5))
            .unwrap();
        assert!(readiness.ready);

        let pdf = browser.print_pdf().unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        let document = lopdf::Document::load_mem(&pdf).unwrap();
        assert_eq!(document.get_pages().len(), 2);
    }

    #[test]
    fn cdp_reports_rejected_readiness_state_and_console_diagnostics() {
        if discover_chromium_executable().executable.is_none() {
            return;
        }
        let temp = tempdir().unwrap();
        let html_path = temp.path().join("failed-readiness.html");
        fs::write(
            &html_path,
            r#"<!doctype html>
<body data-zpres-ready="failed" data-zpres-ready-target="pdf" data-zpres-page-count="1">
<section class="zpres-print-slide" data-page="1" data-slide-id="broken"></section>
<script>
const details = {
  status: "failed",
  errors: [{ stage: "images", kind: "load", message: "missing diagram", source: "missing.svg" }],
};
window.zpresStaticReadyState = details;
window.zpresStaticReadyError = details;
console.error("static readiness failed for missing.svg");
const error = new Error("static export readiness failed");
error.name = "ZpresStaticReadinessError";
error.details = details;
window.zpresStaticReady = Promise.reject(error);
</script>
</body>"#,
        )
        .unwrap();
        let url = format!("file://{}", html_path.canonicalize().unwrap().display());
        let mut browser = ChromiumSession::launch(ChromiumSessionOptions::default()).unwrap();
        browser
            .load_print_document(
                &url,
                PngViewport {
                    width: 1280,
                    height: 720,
                },
            )
            .unwrap();

        let readiness = browser
            .await_static_readiness(Duration::from_secs(5))
            .unwrap();
        assert_eq!(readiness.promise_status, BrowserPromiseStatus::Rejected);
        assert!(!readiness.ready);
        assert_eq!(readiness.state["status"], "failed");
        assert!(
            readiness
                .errors
                .iter()
                .any(|message| message.contains("images: load: missing diagram"))
        );
        assert!(browser.diagnostics().iter().any(|diagnostic| {
            diagnostic.kind == "console"
                && diagnostic.level == "error"
                && diagnostic.text.contains("missing.svg")
        }));
    }
}
