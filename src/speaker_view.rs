#[derive(Debug, Clone)]
pub(crate) struct SpeakerViewConfig {
    pub(crate) presentation_url: String,
    pub(crate) rendered_generation: u64,
}

pub(crate) fn render(config: &SpeakerViewConfig) -> String {
    let presentation_url = escape_attr(&config.presentation_url);
    let preview_url = "/presentation.html?zpres-preview=1";
    r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>zpres speaker view</title>
<style>
:root { color-scheme:dark; font-family:Inter,ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif; background:#070b12; color:#f8fafc; }
* { box-sizing:border-box; }
body { margin:0; min-height:100vh; background:radial-gradient(circle at 18% -12%,#172554 0,transparent 34rem),#070b12; }
button,a { font:inherit; }
.speaker-shell { height:100vh; min-height:0; overflow:hidden; display:grid; grid-template-rows:auto minmax(0,1fr) auto; gap:14px; padding:16px; }
.speaker-header { display:grid; grid-template-columns:minmax(0,1fr) auto auto; align-items:center; gap:20px; padding:10px 14px; background:#0d1420e8; border:1px solid #263244; border-radius:12px; box-shadow:0 12px 40px #0007; }
.speaker-brand { display:flex; align-items:center; gap:12px; min-width:0; }
.speaker-mark { display:grid; place-items:center; width:34px; height:34px; border-radius:9px; background:#2563eb; color:white; font-weight:900; letter-spacing:-.04em; }
.speaker-brand strong { display:block; font-size:14px; }
.speaker-status { display:flex; gap:8px; align-items:center; color:#94a3b8; font-size:12px; }
.speaker-status-dot { width:8px; height:8px; border-radius:50%; background:#f59e0b; box-shadow:0 0 0 4px #f59e0b22; }
.speaker-status[data-connected="true"] .speaker-status-dot { background:#22c55e; box-shadow:0 0 0 4px #22c55e22; }
.speaker-clock { color:#cbd5e1; font-size:18px; font-variant-numeric:tabular-nums; }
.speaker-timer { min-width:116px; text-align:right; font-size:30px; font-weight:750; line-height:1; font-variant-numeric:tabular-nums; letter-spacing:-.04em; }
.speaker-main { min-height:0; overflow:hidden; display:grid; grid-template-columns:minmax(0,1.65fr) minmax(320px,.85fr); grid-template-rows:auto minmax(0,1fr); gap:14px; }
.preview-column { display:contents; }
.preview-column > .speaker-panel:first-child { grid-column:1; grid-row:1 / 3; }
.preview-column > .speaker-panel:nth-child(2) { grid-column:2; grid-row:1; aspect-ratio:16 / 10; }
.speaker-panel { min-width:0; min-height:0; display:grid; grid-template-rows:auto minmax(0,1fr); overflow:hidden; background:#0d1420e8; border:1px solid #263244; border-radius:12px; box-shadow:0 12px 40px #0005; }
.panel-label { display:flex; align-items:center; justify-content:space-between; min-height:38px; padding:8px 12px; color:#94a3b8; font-size:11px; font-weight:800; letter-spacing:.11em; text-transform:uppercase; border-bottom:1px solid #263244; }
.panel-route { color:#64748b; font-variant-numeric:tabular-nums; letter-spacing:0; text-transform:none; }
.preview-frame-wrap { position:relative; min-height:0; display:grid; place-items:center; overflow:hidden; background:#030712; }
.preview-frame { width:100%; height:100%; border:0; pointer-events:none; }
.preview-empty { position:absolute; inset:0; display:grid; place-items:center; color:#64748b; font-size:18px; background:#030712; }
.preview-empty[hidden] { display:none; }
.notes-panel { grid-column:2; grid-row:2; grid-template-rows:auto minmax(0,1fr) auto; }
.notes-body { overflow:auto; padding:20px 22px 32px; color:#e2e8f0; font-size:clamp(18px,1.55vw,26px); line-height:1.55; scrollbar-color:#475569 transparent; }
.notes-body p { margin:0; }
.notes-body code { padding:.08em .28em; color:#dbeafe; background:#172554; border:1px solid #1e3a8a; border-radius:.25em; font:600 .84em/1.3 ui-monospace,SFMono-Regular,Menlo,monospace; }
.notes-body a { color:#93c5fd; text-underline-offset:.16em; }
.notes-empty { color:#64748b; font-style:italic; }
.zpres-speaker-note-click { display:inline; color:#94a3b8; transition:color .15s ease; }
.zpres-speaker-note-click.is-active { color:#fbbf24; font-weight:750; }
.zpres-speaker-note-click.is-complete { color:#64748b; }
.notes-meta { display:flex; justify-content:space-between; gap:10px; padding:10px 14px; color:#64748b; border-top:1px solid #263244; font-size:12px; }
.speaker-controls { display:flex; align-items:center; justify-content:center; flex-wrap:wrap; gap:9px; padding:4px; }
.speaker-controls button,.speaker-open { min-height:40px; padding:0 15px; color:#e2e8f0; background:#111c2c; border:1px solid #334155; border-radius:9px; cursor:pointer; text-decoration:none; font-size:13px; font-weight:700; }
.speaker-controls button:hover,.speaker-controls button:focus-visible,.speaker-open:hover,.speaker-open:focus-visible { border-color:#60a5fa; outline:2px solid #2563eb55; outline-offset:1px; }
.speaker-controls .primary { min-width:112px; color:white; background:#2563eb; border-color:#3b82f6; }
.speaker-controls .timer-control { color:#cbd5e1; background:#0d1420; }
.speaker-diagnostic { color:#fbbf24; }
@media (max-width:900px) {
  .speaker-shell { height:auto; min-height:100vh; overflow:visible; padding:10px; gap:10px; }
  .speaker-header { grid-template-columns:1fr auto; gap:10px; }
  .speaker-clock { display:none; }
  .speaker-main { overflow:visible; grid-template-columns:1fr; grid-template-rows:auto auto; }
  .preview-column { min-height:0; display:grid; grid-column:1; grid-row:1; grid-template-rows:auto auto; gap:10px; }
  .preview-column > .speaker-panel:first-child,
  .preview-column > .speaker-panel:nth-child(2) { grid-column:auto; grid-row:auto; aspect-ratio:16 / 10; }
  .notes-panel { grid-column:1; grid-row:2; min-height:260px; }
  .notes-body { font-size:18px; }
}
</style>
</head>
<body>
<div class="speaker-shell">
  <header class="speaker-header">
    <div class="speaker-brand"><span class="speaker-mark">z</span><div><strong>Speaker view</strong><span class="speaker-status" id="connection-status"><span class="speaker-status-dot"></span><span data-status-text>Connecting</span></span></div></div>
    <time class="speaker-clock" id="wall-clock">--:--:--</time>
    <div class="speaker-timer" id="elapsed-timer">00:00</div>
  </header>
  <main class="speaker-main">
    <div class="preview-column">
      <section class="speaker-panel"><div class="panel-label"><span>Current</span><span class="panel-route" id="current-route">Waiting</span></div><div class="preview-frame-wrap"><iframe class="preview-frame" id="current-frame" title="Current presentation state" sandbox="allow-scripts" tabindex="-1" referrerpolicy="no-referrer" allow="autoplay 'none'; camera 'none'; microphone 'none'; display-capture 'none'; fullscreen 'none'"></iframe></div></section>
      <section class="speaker-panel"><div class="panel-label"><span>Next</span><span class="panel-route" id="next-route">Waiting</span></div><div class="preview-frame-wrap"><iframe class="preview-frame" id="next-frame" title="Next presentation state" sandbox="allow-scripts" tabindex="-1" referrerpolicy="no-referrer" allow="autoplay 'none'; camera 'none'; microphone 'none'; display-capture 'none'; fullscreen 'none'"></iframe><div class="preview-empty" id="next-empty" hidden>End of presentation</div></div></section>
    </div>
    <section class="speaker-panel notes-panel"><div class="panel-label"><span>Speaker notes</span><span class="speaker-diagnostic" id="diagnostic-status"></span></div><div class="notes-body" id="speaker-notes"><span class="notes-empty">Waiting for the Deck.</span></div><div class="notes-meta"><span id="slide-title">Untitled slide</span><span id="step-status"></span></div></section>
  </main>
  <nav class="speaker-controls" aria-label="Presentation controls">
    <button data-command="detail-previous" title="Previous Detail (Arrow Up)">Detail ↑</button>
    <button data-command="retreat" title="Previous Step or Main slide (Arrow Left)">← Previous</button>
    <button class="primary" data-command="advance" title="Next Step or Main slide (Arrow Right or Space)">Next →</button>
    <button data-command="detail-next" title="Next Detail (Arrow Down)">Detail ↓</button>
    <button class="timer-control" data-command="timer-start">Start</button>
    <button class="timer-control" data-command="timer-pause">Pause</button>
    <button class="timer-control" data-command="timer-reset">Reset</button>
    <a class="speaker-open" href="__PRESENTATION_URL__" target="_blank" rel="noopener">Open presentation</a>
  </nav>
</div>
<script>
(function () {
  let renderedGeneration = __RENDERED_GENERATION__;
  window.zpresRenderedGeneration = renderedGeneration;
  const previewBaseUrl = "__PREVIEW_URL__";
  const frames = {
    current: { element:document.querySelector("#current-frame"), role:"current", epoch:0, nonce:"", ready:false, route:null, routeKey:"" },
    next: { element:document.querySelector("#next-frame"), role:"next", epoch:0, nonce:"", ready:false, route:null, routeKey:"" },
  };
  const status = document.querySelector("#connection-status");
  const statusText = status.querySelector("[data-status-text]");
  const timer = document.querySelector("#elapsed-timer");
  let latest = null;
  let timerObservedAt = performance.now();
  function routeHash(route) { return "#/" + route.section + "/" + route.detail + (route.step ? "/" + route.step : ""); }
  function routeLabel(route) {
    if (!route) return "End";
    const detail = route.detail ? " · Detail " + route.detail : "";
    const step = route.step_count ? " · Step " + route.step + "/" + route.step_count : "";
    return "Section " + (route.section + 1) + detail + step;
  }
  function randomNonce() {
    const bytes = new Uint8Array(16);
    crypto.getRandomValues(bytes);
    return Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");
  }
  function envelope(frame, type, extra) {
    return Object.assign({ type, protocol_version:1, nonce:frame.nonce, epoch:frame.epoch, role:frame.role }, extra || {});
  }
  function previewRouteKey(route) {
    if (!route || !Number.isSafeInteger(route.section) || route.section < 0
        || !Number.isSafeInteger(route.detail) || route.detail < 0
        || !Number.isSafeInteger(route.step) || route.step < 0) return "";
    return route.section + ":" + route.detail + ":" + route.step;
  }
  function reloadPreview(frame, generation) {
    if (!frame.route) return;
    frame.epoch += 1;
    frame.nonce = randomNonce();
    frame.ready = false;
    const url = new URL(previewBaseUrl, window.location.href);
    url.searchParams.set("zpres-preview", "1");
    url.searchParams.set("zpres-preview-epoch", String(frame.epoch));
    url.searchParams.set("zpres-preview-role", frame.role);
    url.searchParams.set("zpres-parent-origin", window.location.origin);
    url.searchParams.set("zpres-generation", String(generation));
    url.searchParams.set("zpres-preview-section", String(frame.route.section));
    url.searchParams.set("zpres-preview-detail", String(frame.route.detail));
    url.searchParams.set("zpres-preview-step", String(frame.route.step));
    url.hash = routeHash(frame.route);
    const previous = frame.element;
    const replacement = previous.cloneNode(false);
    replacement.removeAttribute("src");
    delete replacement.dataset.previewReady;
    delete replacement.dataset.previewRoute;
    delete replacement.dataset.previewEvidence;
    replacement.src = url.pathname + url.search + url.hash;
    previous.replaceWith(replacement);
    frame.element = replacement;
  }
  function sendPreview(frame, route) {
    const nextRouteKey = previewRouteKey(route);
    frame.route = route || null;
    frame.routeKey = nextRouteKey;
    if (!route) return;
    if (!frame.epoch) {
      reloadPreview(frame, renderedGeneration);
      return;
    }
    if (!frame.ready) return;
    frame.element.contentWindow?.postMessage(envelope(frame, "zpres-preview-route", { route }), "*");
  }
  function updateNotes(route) {
    const notes = document.querySelector("#speaker-notes");
    if (!route?.notes_html) notes.innerHTML = '<span class="notes-empty">No speaker notes.</span>';
    else notes.innerHTML = route.notes_html;
    notes.querySelectorAll("[data-note-click-index]").forEach(function (marker) {
      const index = Number(marker.dataset.noteClickIndex || 0);
      marker.classList.toggle("is-pending", index > route.step);
      marker.classList.toggle("is-active", index === route.step);
      marker.classList.toggle("is-complete", index > 0 && index < route.step);
    });
    document.querySelector("#slide-title").textContent = route?.title || route?.slide_id || "Untitled slide";
    document.querySelector("#step-status").textContent = route?.step_count ? "Step " + route.step + " of " + route.step_count : "";
  }
  function renderSession(session) {
    if (!session || (latest && session.revision < latest.revision)) return;
    latest = session;
    timerObservedAt = performance.now();
    document.querySelector("#current-route").textContent = routeLabel(session.current);
    document.querySelector("#next-route").textContent = routeLabel(session.next);
    document.querySelector("#next-empty").hidden = Boolean(session.next);
    frames.next.element.hidden = !session.next;
    updateNotes(session.current);
    sendPreview(frames.current, session.current);
    sendPreview(frames.next, session.next);
  }
  async function control(command) {
    const response = await fetch("/zpres/control", { method:"POST", headers:{"Content-Type":"application/json"}, body:JSON.stringify({ command }) });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || "control failed");
    renderSession(payload);
  }
  document.querySelectorAll("[data-command]").forEach(function (button) {
    button.addEventListener("click", function () { control(button.dataset.command).catch(console.error); });
  });
  document.addEventListener("keydown", function (event) {
    if (["INPUT","TEXTAREA","SELECT","BUTTON"].includes(event.target?.tagName)) return;
    const command = event.key === "ArrowRight" || event.key === "PageDown" || event.key === " " ? "advance"
      : event.key === "ArrowLeft" || event.key === "PageUp" ? "retreat"
      : event.key === "ArrowDown" ? "detail-next"
      : event.key === "ArrowUp" ? "detail-previous" : null;
    if (command) { event.preventDefault(); control(command).catch(console.error); }
  });
  window.addEventListener("message", function (event) {
    if (event.origin !== "null" || !event.data || event.data.protocol_version !== 1) return;
    const frame = Object.values(frames).find(candidate => event.source === candidate.element.contentWindow);
    if (!frame || event.data.epoch !== frame.epoch || event.data.role !== frame.role) return;
    if (event.data.type === "zpres-preview-ready") {
      frame.element.contentWindow?.postMessage(envelope(frame, "zpres-preview-init"), "*");
      return;
    }
    if (event.data.nonce !== frame.nonce) return;
    if (event.data.type === "zpres-preview-ack" && event.data.acknowledged === "init") {
      frame.ready = true;
      frame.element.dataset.previewReady = "true";
      sendPreview(frame, frame.route);
    } else if (event.data.type === "zpres-preview-ack" && event.data.acknowledged === "route") {
      const acknowledgedRouteKey = previewRouteKey(event.data.route);
      if (event.data.error || !acknowledgedRouteKey || acknowledgedRouteKey !== frame.routeKey) return;
      frame.element.dataset.previewRoute = JSON.stringify(event.data.route || null);
      frame.element.dataset.previewEvidence = JSON.stringify(event.data.evidence || null);
    }
  });
  const events = new EventSource("/events");
  events.onopen = function () { latest = null; status.dataset.connected = "true"; statusText.textContent = "Connected"; };
  events.onerror = function () { status.dataset.connected = "false"; statusText.textContent = "Reconnecting"; };
  events.addEventListener("session", function (event) { renderSession(JSON.parse(event.data)); });
  events.addEventListener("diagnostics", function (event) {
    const payload = JSON.parse(event.data);
    const parsed = new DOMParser().parseFromString(payload.html || "", "text/html");
    const count = parsed.querySelectorAll("li").length;
    const hasErrors = Boolean(parsed.querySelector('[data-severity="error"]'));
    document.querySelector("#diagnostic-status").textContent = count
      ? (hasErrors ? "Last good · " : "") + count + " diagnostic" + (count === 1 ? "" : "s")
      : "";
  });
  function refreshPreviews(generation) {
    Object.values(frames).forEach(function (frame) { reloadPreview(frame, generation); });
  }
  function applyRenderNotice(event) {
    const payload = JSON.parse(event.data);
    const generation = Number(payload.generation);
    if (!Number.isSafeInteger(generation) || generation <= renderedGeneration) return;
    renderedGeneration = generation;
    window.zpresRenderedGeneration = generation;
    refreshPreviews(generation);
  }
  events.addEventListener("render-state", function (event) {
    const payload = JSON.parse(event.data);
    const generation = Number(payload.generation);
    if (!Number.isSafeInteger(generation) || generation === renderedGeneration) return;
    renderedGeneration = generation;
    window.zpresRenderedGeneration = generation;
    refreshPreviews(generation);
  });
  events.addEventListener("reload", applyRenderNotice);
  events.addEventListener("theme-css", applyRenderNotice);
  events.addEventListener("assets", applyRenderNotice);
  setInterval(function () {
    document.querySelector("#wall-clock").textContent = new Intl.DateTimeFormat([], { hour:"2-digit", minute:"2-digit", second:"2-digit" }).format(new Date());
    if (!latest) return;
    let elapsed = latest.timer.elapsed_ms;
    if (latest.timer.status === "running") elapsed += performance.now() - timerObservedAt;
    const totalSeconds = Math.max(0, Math.floor(elapsed / 1000));
    const hours = Math.floor(totalSeconds / 3600);
    const minutes = Math.floor((totalSeconds % 3600) / 60);
    const seconds = totalSeconds % 60;
    timer.textContent = (hours ? String(hours).padStart(2,"0") + ":" : "") + String(minutes).padStart(2,"0") + ":" + String(seconds).padStart(2,"0");
  }, 250);
})();
</script>
</body>
</html>"##
        .replace("__PREVIEW_URL__", preview_url)
        .replace("__PRESENTATION_URL__", &presentation_url)
        .replace(
            "__RENDERED_GENERATION__",
            &config.rendered_generation.to_string(),
        )
}

fn escape_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_sandboxes_renderer_owned_previews() {
        let speaker = render(&SpeakerViewConfig {
            presentation_url: "http://127.0.0.1:3000".to_string(),
            rendered_generation: 7,
        });
        assert!(speaker.contains("Current"));
        assert!(speaker.contains("Speaker notes"));
        assert!(speaker.contains("timer-start"));
        assert!(speaker.contains("/presentation.html?zpres-preview=1"));
        assert_eq!(speaker.matches("sandbox=\"allow-scripts\"").count(), 2);
        assert_eq!(speaker.matches("tabindex=\"-1\"").count(), 2);
        assert!(!speaker.contains("allow-same-origin"));
        assert!(speaker.contains("event.origin !== \"null\""));
        assert!(speaker.contains("event.source === candidate.element.contentWindow"));
        assert!(speaker.contains("let renderedGeneration = 7;"));
    }
}
