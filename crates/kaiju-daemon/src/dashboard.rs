//! The live fleet dashboard — a self-contained HTML page served at `/`.
//!
//! It polls the daemon's own API in the browser, so it always reflects current
//! state with no server-side rendering. Clicking an agent opens a detail panel
//! with its live log tail and a reply box, all driven by the existing endpoints
//! (`/agents`, `/agents/:id/logs`, `/agents/:id/input`, `/interrupt`, `/stop`,
//! `/diff`). Kept as one static string to avoid a templating dependency.

use axum::http::header;
use axum::response::IntoResponse;

/// Dashboard scripts, vendored alongside the page. `dashboard-utils.js` holds
/// the pure (DOM-free) helpers — the same file `dashboard-utils.test.js` unit
/// tests under node — and loads before the controller so its functions are in
/// scope.
const DASHBOARD_JS: &str = include_str!("../assets/dashboard.js");
const DASHBOARD_UTILS_JS: &str = include_str!("../assets/dashboard-utils.js");

/// `GET /assets/dashboard.js` — the dashboard controller (public, no auth).
pub async fn dashboard_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        DASHBOARD_JS,
    )
}

/// `GET /assets/dashboard-utils.js` — pure dashboard helpers (public, no auth).
pub async fn dashboard_utils_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        DASHBOARD_UTILS_JS,
    )
}

/// The dashboard page. Served verbatim from `GET /`.
pub const PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Kaiju</title>
<link rel="stylesheet" href="/assets/xterm.css">
<script src="/assets/xterm.js"></script>
<style>
  /* ── Design tokens ─────────────────────────────────────────────────────── */
  :root {
    --bg:           #07090d;
    --surface:      #0d1117;
    --surface-2:    #11161f;
    --rail:         #0a0d12;
    --border:       #1b2230;
    --text:         #dfe7ef;
    --muted:        #7d8aa0;
    --accent:       #34e2a0;
    --accent-2:     #14b8a6;
    --accent-weak:  #34e2a026;
    --accent-glow:  #34e2a018;
    --accent-fg:    #07090d;
    --amber:        #f5a623;
    --amber-weak:   #f5a62320;
    --red:          #ff5470;
    --red-weak:     #ff547020;
    --blue:         #4c8dff;
    --blue-weak:    #4c8dff20;
    --term-bg:      #0d1117;
    --radius:       10px;
    --shadow:       0 1px 3px #00000050, 0 8px 32px #00000060;
    --top-h:        3.25rem;
  }

  /* ── Reset & base ───────────────────────────────────────────────────────── */
  *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }

  body {
    font-family: system-ui, -apple-system, "Segoe UI", sans-serif;
    background:
      radial-gradient(900px circle at 12% 8%,  #34e2a014, transparent 60%),
      radial-gradient(800px circle at 88% 92%, #14b8a610, transparent 60%),
      #07090d;
    background-attachment: fixed;
    color: var(--text);
    -webkit-font-smoothing: antialiased;
    height: 100vh;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  /* Subtle grain overlay */
  body::before {
    content: "";
    position: fixed;
    inset: 0;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='200' height='200'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.75' numOctaves='4' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='200' height='200' filter='url(%23n)' opacity='0.03'/%3E%3C/svg%3E");
    pointer-events: none;
    z-index: 0;
    opacity: 0.4;
  }

  /* ── Typography helpers ────────────────────────────────────────────────── */
  .mono {
    font-family: "SF Mono","JetBrains Mono","Cascadia Code",ui-monospace,Menlo,monospace;
  }
  .label {
    font-size: .68rem;
    text-transform: uppercase;
    letter-spacing: .06em;
    font-weight: 600;
    color: var(--muted);
  }

  /* ── Top bar ───────────────────────────────────────────────────────────── */
  #topbar {
    position: relative;
    z-index: 10;
    height: var(--top-h);
    flex: 0 0 var(--top-h);
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    display: flex;
    align-items: center;
    gap: .6rem;
    padding: 0 1rem;
    box-shadow: 0 1px 0 #34e2a010;
  }

  #brand {
    font-family: "SF Mono","JetBrains Mono","Cascadia Code",ui-monospace,Menlo,monospace;
    font-size: 1.05rem;
    font-weight: 700;
    letter-spacing: .18em;
    color: var(--accent);
    text-shadow: 0 0 18px #34e2a055;
    flex: 0 0 auto;
    margin-right: .25rem;
    user-select: none;
  }

  /* toolbar divider */
  .tb-sep { width: 1px; height: 1.2rem; background: var(--border); flex: 0 0 auto; }

  .tb-actions { display: flex; align-items: center; gap: .35rem; }

  #filter-text {
    flex: 0 1 14rem;
    min-width: 8rem;
    font-size: .82rem;
  }
  #filter-status { font-size: .82rem; }

  #updated {
    font-size: .75rem;
    color: var(--muted);
    white-space: nowrap;
    flex: 0 0 auto;
  }

  /* spacer */
  .grow { flex: 1; }

  /* ── Shell grid ────────────────────────────────────────────────────────── */
  #shell {
    flex: 1;
    display: grid;
    grid-template-columns: minmax(300px, 360px) 1fr;
    overflow: hidden;
    position: relative;
    z-index: 1;
  }

  /* ── Left rail ─────────────────────────────────────────────────────────── */
  #rail {
    background: var(--rail);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  #rail-header {
    padding: .6rem .75rem .4rem;
    border-bottom: 1px solid var(--border);
    display: flex;
    align-items: center;
    gap: .5rem;
    flex-shrink: 0;
  }

  .counts { display: flex; gap: .4rem; flex-wrap: wrap; }
  .pill {
    padding: .2rem .6rem;
    border-radius: 999px;
    font-size: .72rem;
    font-weight: 600;
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--muted);
    font-family: "SF Mono","JetBrains Mono","Cascadia Code",ui-monospace,Menlo,monospace;
  }

  #agent-list {
    flex: 1;
    overflow-y: auto;
    padding: .5rem .45rem;
    scrollbar-width: thin;
    scrollbar-color: var(--border) transparent;
  }
  #agent-list::-webkit-scrollbar { width: 5px; }
  #agent-list::-webkit-scrollbar-track { background: transparent; }
  #agent-list::-webkit-scrollbar-thumb { background: var(--border); border-radius: 3px; }

  /* ── Agent card ────────────────────────────────────────────────────────── */
  @keyframes cardReveal {
    from { opacity: 0; transform: translateY(8px); }
    to   { opacity: 1; transform: translateY(0); }
  }

  .agent-card {
    position: relative;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 9px;
    margin-bottom: .4rem;
    cursor: pointer;
    overflow: hidden;
    transition:
      transform .15s ease,
      border-color .15s ease,
      box-shadow .15s ease;
  }
  /* Reveal animation only plays on first paint (the `enter` class), so the
     2-second poll re-render doesn't replay it and flicker the list. */
  .agent-card.enter { animation: cardReveal .32s ease both; }
  .agent-card:hover {
    transform: translateY(-1px);
    border-color: #34e2a040;
    box-shadow: 0 0 0 1px #34e2a018, 0 4px 16px #00000060;
  }
  .agent-card.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent), inset 0 0 24px #34e2a012;
  }
  .agent-card.attention {
    border-color: var(--amber);
    box-shadow: 0 0 0 1px var(--amber-weak), 0 0 18px #f5a62318;
    animation: attentionPulse 2.4s ease-in-out 1s infinite;
  }
  .agent-card.enter.attention {
    animation: cardReveal .32s ease both, attentionPulse 2.4s ease-in-out 1s infinite;
  }
  @keyframes attentionPulse {
    0%,100% { box-shadow: 0 0 0 1px var(--amber-weak), 0 0 18px #f5a62318; }
    50%      { box-shadow: 0 0 0 1px #f5a62355, 0 0 30px #f5a62328; }
  }
  /* Selection must stay unmistakable even while the card is in an attention
     state: keep the accent ring dominant (with a faint amber halo as a hint),
     and stop the pulse so "selected" reads clearly. */
  .agent-card.attention.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent), 0 0 20px #f5a62322;
    animation: none;
  }

  /* Left accent bar */
  .agent-card::before {
    content: "";
    position: absolute;
    left: 0; top: 0; bottom: 0;
    width: 3px;
    border-radius: 9px 0 0 9px;
    background: var(--border);
    transition: background .15s ease;
  }
  .agent-card.s-waitingforinput::before,
  .agent-card.s-limited::before,
  .agent-card.attention::before   { background: var(--amber); }
  .agent-card.s-stuck::before,
  .agent-card.s-error::before     { background: var(--red); }
  .agent-card.s-running::before   { background: var(--accent); }
  .agent-card.s-starting::before  { background: var(--blue); }
  /* Selected bar comes last so it wins the tie against the status bars above —
     the selected card always shows the accent bar, whatever its status. */
  .agent-card.selected::before    { background: var(--accent); width: 4px; }

  .card-body {
    padding: .65rem .75rem .55rem .9rem;
  }

  .card-top {
    display: flex;
    align-items: center;
    gap: .5rem;
    margin-bottom: .35rem;
  }

  /* Status dot */
  .dot {
    width: .5rem;
    height: .5rem;
    border-radius: 50%;
    flex: 0 0 auto;
    background: var(--muted);
    transition: background .2s;
  }
  .dot.s-running       { background: var(--accent);  animation: dotPulse 1.6s ease-in-out infinite; }
  .dot.s-starting      { background: var(--blue);    animation: dotPulse 1.2s ease-in-out infinite; }
  .dot.s-waitingforinput { background: var(--amber); animation: dotPulse 2s ease-in-out infinite; }
  .dot.s-limited       { background: var(--amber);    animation: dotPulse 2s ease-in-out infinite; }
  .dot.s-stuck         { background: var(--red);     animation: dotPulse 2.2s ease-in-out infinite; }
  .dot.s-error         { background: var(--red); }
  .dot.s-completed     { background: #4b6080; }
  .dot.s-stopped       { background: #3a4558; }
  @keyframes dotPulse {
    0%,100% { opacity: 1; }
    50%     { opacity: .35; }
  }

  .card-id {
    font-family: "SF Mono","JetBrains Mono","Cascadia Code",ui-monospace,Menlo,monospace;
    font-size: .8rem;
    font-weight: 600;
    color: var(--text);
    letter-spacing: .02em;
  }
  .agent-card.selected .card-id { color: var(--accent); }

  /* Type chip */
  .chip {
    font-size: .68rem;
    font-weight: 600;
    padding: .1rem .45rem;
    border-radius: 999px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--muted);
    letter-spacing: .02em;
    flex: 0 0 auto;
  }

  /* Status badge */
  .status {
    font-weight: 600;
    padding: .15rem .5rem;
    border-radius: 999px;
    font-size: .72rem;
    white-space: nowrap;
    display: inline-flex;
    align-items: center;
    gap: .3rem;
    flex: 0 0 auto;
  }
  .s-waitingforinput { background: var(--amber-weak); color: var(--amber); }
  .s-limited           { background: var(--amber-weak); color: var(--amber); }
  .s-stuck, .s-error { background: var(--red-weak);   color: var(--red);   }
  .s-running         { background: var(--accent-weak); color: var(--accent);}
  .s-starting        { background: var(--blue-weak);   color: var(--blue);  }
  .s-completed, .s-stopped { background: #2a3344; color: #5a6a80; }

  .card-task {
    font-size: .78rem;
    color: var(--muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    margin-bottom: .35rem;
    line-height: 1.35;
  }

  .card-metrics {
    display: flex;
    gap: .75rem;
    font-size: .7rem;
    color: var(--muted);
    font-family: "SF Mono","JetBrains Mono","Cascadia Code",ui-monospace,Menlo,monospace;
    flex-wrap: wrap;
  }
  .card-metrics span { white-space: nowrap; }
  .card-metrics .metric-val { color: #9eafcc; }

  /* Hover action strip */
  .card-actions {
    display: flex;
    align-items: center;
    gap: .25rem;
    padding: .35rem .75rem .35rem .9rem;
    border-top: 1px solid var(--border);
    background: var(--surface-2);
    opacity: 0;
    transition: opacity .15s ease;
  }
  .agent-card:hover .card-actions,
  .agent-card.selected .card-actions {
    opacity: 1;
  }

  /* ── Right stage ────────────────────────────────────────────────────────── */
  #stage {
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--bg);
  }

  /* Empty welcome state */
  #welcome {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 1rem;
    color: var(--muted);
    user-select: none;
  }
  #welcome .welcome-glyph {
    font-family: "SF Mono","JetBrains Mono","Cascadia Code",ui-monospace,Menlo,monospace;
    font-size: 4.5rem;
    font-weight: 800;
    letter-spacing: .18em;
    color: transparent;
    -webkit-text-stroke: 1px #34e2a018;
    line-height: 1;
  }
  #welcome p {
    font-size: .88rem;
    color: var(--muted);
    text-align: center;
  }

  /* Empty agent list */
  .empty {
    color: var(--muted);
    padding: 2rem 1rem;
    text-align: center;
    font-size: .85rem;
  }

  /* ── Detail panel (right stage) ─────────────────────────────────────────── */
  #detail {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  #detail[hidden] { display: none; }

  .detail-head {
    display: flex;
    align-items: center;
    gap: .6rem;
    flex-wrap: wrap;
    padding: .7rem 1rem;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    box-shadow: 0 1px 0 #34e2a00a;
  }
  .detail-head .id {
    font-family: "SF Mono","JetBrains Mono","Cascadia Code",ui-monospace,Menlo,monospace;
    font-weight: 700;
    font-size: 1rem;
    color: var(--accent);
    letter-spacing: .04em;
  }
  .path {
    font-family: "SF Mono","JetBrains Mono","Cascadia Code",ui-monospace,Menlo,monospace;
    font-size: .75rem;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 38ch;
  }
  .activity {
    font-size: .75rem;
    color: var(--muted);
    white-space: nowrap;
  }
  .detail-head .port {
    font-family: "SF Mono","JetBrains Mono","Cascadia Code",ui-monospace,Menlo,monospace;
    font-size: .75rem;
    white-space: nowrap;
  }
  .detail-head .port a {
    color: var(--accent);
    text-decoration: none;
    border-bottom: 1px dotted color-mix(in srgb, var(--accent) 50%, transparent);
  }
  .detail-head .port a:hover { border-bottom-style: solid; }

  .detail-body {
    flex: 1;
    overflow: auto;
    padding: .85rem 1rem;
    scrollbar-width: thin;
    scrollbar-color: var(--border) transparent;
  }

  /* Tabs */
  .tabs {
    display: flex;
    gap: .25rem;
    margin-bottom: .75rem;
    border-bottom: 1px solid var(--border);
    padding-bottom: 0;
  }
  .tab {
    font-size: .8rem;
    padding: .35rem .85rem;
    border-radius: 7px 7px 0 0;
    border-bottom: 2px solid transparent;
    background: transparent;
    border-top: 1px solid transparent;
    border-left: 1px solid transparent;
    border-right: 1px solid transparent;
    color: var(--muted);
    position: relative;
    bottom: -1px;
  }
  .tab.active {
    background: var(--surface);
    border-color: var(--border) var(--border) var(--surface) var(--border);
    color: var(--accent);
    font-weight: 600;
  }
  .tab.active::after {
    content: "";
    position: absolute;
    bottom: -2px;
    left: .5rem;
    right: .5rem;
    height: 2px;
    background: var(--accent);
    border-radius: 1px;
  }

  pre.logs {
    background: var(--term-bg);
    color: #c9d1d9;
    padding: .85rem;
    border-radius: 8px;
    max-height: calc(100vh - 16rem);
    overflow: auto;
    font-size: .82rem;
    white-space: pre-wrap;
    word-break: break-word;
    border: 1px solid var(--border);
    font-family: "SF Mono","JetBrains Mono","Cascadia Code",ui-monospace,Menlo,monospace;
  }
  pre.logs .d-add  { color: #3fb950; }
  pre.logs .d-del  { color: #f85149; }
  pre.logs .d-hunk { color: #39c5cf; }
  pre.logs .d-file { color: #d2a8ff; font-weight: 600; }

  /* Terminal viewport */
  #d-term {
    width: 100%;
    height: calc(100vh - 16rem);
    min-height: 22rem;
    background: var(--term-bg);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: .6rem;
    overflow: hidden;
    box-shadow: inset 0 0 32px #00000040;
  }
  #d-term[hidden] { display: none; }
  #d-term .xterm-viewport { background: transparent !important; }

  /* Reply + note */
  .reply { display: flex; gap: .5rem; margin-top: .85rem; }
  .reply input { flex: 1; }
  .note {
    color: var(--muted);
    font-size: .78rem;
    margin-top: .5rem;
    min-height: 1rem;
    font-family: "SF Mono","JetBrains Mono","Cascadia Code",ui-monospace,Menlo,monospace;
  }

  /* ── Compare panel ──────────────────────────────────────────────────────── */
  #compare-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  #compare-panel[hidden] { display: none; }

  .compare-body {
    flex: 1;
    overflow: auto;
    padding: .85rem 1rem;
  }

  .cmp-cols { display: flex; gap: 12px; overflow-x: auto; padding: 4px 0; }
  .cmp-col {
    flex: 1 0 320px;
    min-width: 320px;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: 8px;
    overflow: hidden;
    background: var(--surface);
  }
  .cmp-col-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    background: var(--surface-2);
    font-weight: 600;
    font-size: .85rem;
    border-bottom: 1px solid var(--border);
  }
  .cmp-col-head .grow { flex: 1; }
  .cmp-diff {
    margin: 0;
    padding: 8px 10px;
    max-height: 60vh;
    overflow: auto;
    font-family: "SF Mono","JetBrains Mono","Cascadia Code",ui-monospace,Menlo,monospace;
    font-size: 12px;
    white-space: pre;
    color: #c9d1d9;
  }
  #cmp-verdict {
    margin: 0 0 12px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-2);
  }
  #cmp-verdict[hidden] { display: none; }
  .cmp-legend { font-size: 12px; color: var(--muted); margin-bottom: 6px; }
  .cmp-verdict-text { margin: 0; white-space: pre-wrap; font-size: 13px; }

  /* ── Leaderboard panel ──────────────────────────────────────────────────── */
  #leaderboard-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  #leaderboard-panel[hidden] { display: none; }

  .lb-body {
    flex: 1;
    overflow: auto;
    padding: .85rem 1rem;
  }

  /* ── Shared panel styles ────────────────────────────────────────────────── */
  /* Leaderboard + compare table inside stage */
  #lb-body table, .compare-body table {
    width: 100%;
    border-collapse: collapse;
    font-size: .88rem;
  }
  #lb-body th, .compare-body th {
    text-align: left;
    padding: .55rem .75rem;
    font-size: .7rem;
    text-transform: uppercase;
    letter-spacing: .05em;
    font-weight: 600;
    color: var(--muted);
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
  }
  #lb-body td, .compare-body td {
    text-align: left;
    padding: .55rem .75rem;
    border-bottom: 1px solid var(--border);
    font-family: "SF Mono","JetBrains Mono","Cascadia Code",ui-monospace,Menlo,monospace;
    font-size: .83rem;
  }
  #lb-body tr:last-child td { border-bottom: none; }

  /* ── Buttons ────────────────────────────────────────────────────────────── */
  button {
    font: inherit;
    font-size: .82rem;
    padding: .38rem .75rem;
    border-radius: 7px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    cursor: pointer;
    transition: background .12s ease, border-color .12s ease, box-shadow .12s ease;
    white-space: nowrap;
  }
  button:hover {
    background: #1e2838;
    border-color: #2e3f55;
  }
  button.primary {
    background: linear-gradient(135deg, var(--accent) 0%, var(--accent-2) 100%);
    border-color: var(--accent);
    color: var(--accent-fg);
    font-weight: 700;
    box-shadow: 0 0 12px #34e2a030;
  }
  button.primary:hover {
    filter: brightness(1.1);
    box-shadow: 0 0 20px #34e2a045;
  }
  button.danger {
    color: var(--red);
    border-color: #ff547040;
  }
  button.danger:hover {
    background: #ff547018;
    border-color: var(--red);
  }
  button.icon {
    padding: .3rem .6rem;
    font-size: 1rem;
    line-height: 1;
    border-radius: 8px;
  }

  /* Small action buttons in cards and actions strips */
  button.sm {
    padding: .2rem .5rem;
    font-size: .75rem;
    border-radius: 6px;
  }

  /* ── Inputs ─────────────────────────────────────────────────────────────── */
  input, select, textarea {
    font: inherit;
    padding: .4rem .65rem;
    border-radius: 7px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    font-size: .85rem;
  }
  input:focus, select:focus, textarea:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-weak);
  }
  select option { background: var(--surface); }

  /* ── Spinner ─────────────────────────────────────────────────────────────── */
  .spinner {
    display: inline-block;
    width: .8em; height: .8em;
    vertical-align: -2px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin .6s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }

  /* ── Modals ──────────────────────────────────────────────────────────────── */
  dialog.modal {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--text);
    padding: 0;
    width: min(560px, 92vw);
    /* Restore centering: the global `* { margin: 0 }` reset overrides the UA's
       `dialog:modal { margin: auto }`, which would otherwise pin it top-left. */
    margin: auto;
    box-shadow: 0 0 0 1px #34e2a015, 0 24px 64px #00000080;
  }
  dialog.modal::backdrop {
    background: #000000b0;
    backdrop-filter: blur(4px);
  }
  dialog.modal form {
    padding: 1.25rem 1.4rem 1.4rem;
    display: flex;
    flex-direction: column;
    gap: .9rem;
  }
  .modal-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .modal-head h2 {
    margin: 0;
    font-size: 1.05rem;
    letter-spacing: -.01em;
    color: var(--text);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: .3rem;
    font-size: .8rem;
    color: var(--muted);
  }
  .field span em { color: var(--accent); font-style: normal; }
  .field input, .field select, .field textarea { width: 100%; }
  textarea {
    resize: vertical;
    min-height: 4rem;
    line-height: 1.5;
  }
  details.advanced {
    border-top: 1px solid var(--border);
    padding-top: .7rem;
  }
  details.advanced summary {
    cursor: pointer;
    color: var(--muted);
    font-size: .82rem;
  }
  details.advanced > .field,
  details.advanced > .check { margin-top: .7rem; }
  .check {
    display: flex;
    align-items: center;
    gap: .45rem;
    font-size: .85rem;
    color: var(--muted);
  }
  .check input { width: auto; flex: none; }
  .cmp-types { display: flex; flex-wrap: wrap; gap: 1.1rem; }
  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: .5rem;
    margin-top: .3rem;
  }

  /* ── Settings popover ────────────────────────────────────────────────────── */
  .popover {
    position: fixed;
    inset: auto;
    top: calc(var(--top-h) + .4rem);
    right: .75rem;
    margin: 0;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: 0 0 0 1px #34e2a012, 0 16px 48px #00000070;
    padding: .9rem 1rem;
    min-width: 18rem;
    max-width: 24rem;
    max-height: 85vh;
    overflow: auto;
    backdrop-filter: blur(12px);
  }
  .popover:not(:popover-open) { display: none; }
  .pop-title {
    font-weight: 700;
    font-size: .9rem;
    margin-bottom: .7rem;
    color: var(--text);
    letter-spacing: .02em;
  }
  .popover .check { margin-bottom: .7rem; }
  .pop-section {
    font-size: .68rem;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--muted);
    font-weight: 600;
    margin: .9rem 0 .5rem;
    border-top: 1px solid var(--border);
    padding-top: .8rem;
  }
  .pfield {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: .6rem;
    font-size: .82rem;
    color: var(--muted);
    margin-bottom: .55rem;
  }
  .pfield > span { flex: 0 0 7.5rem; }
  .pfield input, .pfield select { flex: 1; min-width: 0; }
  .pop-actions {
    display: flex;
    justify-content: flex-end;
    margin-top: .4rem;
  }
  .pop-status {
    font-size: .78rem;
    color: var(--muted);
    margin-top: .5rem;
    min-height: 1rem;
    font-family: "SF Mono","JetBrains Mono","Cascadia Code",ui-monospace,Menlo,monospace;
  }
  .pop-hint {
    font-size: .75rem;
    color: var(--muted);
    margin-top: .6rem;
    line-height: 1.45;
    border-top: 1px solid var(--border);
    padding-top: .6rem;
  }
  .pop-hint code {
    font-family: "SF Mono","JetBrains Mono","Cascadia Code",ui-monospace,Menlo,monospace;
    background: var(--surface-2);
    padding: .05rem .3rem;
    border-radius: 4px;
    font-size: .88em;
    color: var(--accent);
  }
  .device-list { display: flex; flex-direction: column; gap: 6px; margin: 6px 0; }
  .device-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }
  #pair-qr svg { width: 200px; height: 200px; }
</style>
</head>
<body>

  <!-- ── Top bar ─────────────────────────────────────────────────────────── -->
  <div id="topbar">
    <span id="brand">KAIJU</span>
    <div class="tb-sep"></div>
    <div class="tb-actions">
      <button class="primary sm" onclick="toggleNew()">+ New agent</button>
      <button class="sm" onclick="openAdopt()">Adopt</button>
      <button class="sm" onclick="openCompare()">Compare</button>
      <button class="sm" onclick="openLeaderboard()">Leaderboard</button>
      <button class="sm" onclick="launchPet(this)" title="Launch the desktop pet companion">🐾 Pet</button>
    </div>
    <div class="tb-sep"></div>
    <input id="filter-text" placeholder="Search agents…" oninput="applyFilter()" autocomplete="off">
    <select id="filter-status" onchange="applyFilter()" title="Filter by status">
      <option value="all">All statuses</option>
      <option value="waitingforinput">Waiting for input</option>
      <option value="limited">Limited</option>
      <option value="stuck">Stuck</option>
      <option value="error">Error</option>
      <option value="running">Running</option>
      <option value="starting">Starting</option>
      <option value="completed">Completed</option>
      <option value="stopped">Stopped</option>
    </select>
    <button id="pause-btn" class="sm" onclick="togglePause()" title="Pause / resume live updates">⏸ Pause</button>
    <div class="grow"></div>
    <span id="updated"></span>
    <button class="icon" id="settings-btn" popovertarget="settings-pop"
            title="Preferences" aria-label="Preferences">⚙</button>
  </div>

  <!-- ── Settings popover ────────────────────────────────────────────────── -->
  <div id="settings-pop" popover class="popover" aria-label="Preferences">
    <div class="pop-title">Preferences</div>
    <label class="check">
      <input type="checkbox" id="notify-toggle" onchange="toggleNotify()">
      Notify when an agent needs input
    </label>
    <div class="pop-hint">Browser notifications are per-browser and need this tab open (and macOS Focus/Do&nbsp;Not&nbsp;Disturb off).</div>

    <div class="pop-section">Defaults for new agents</div>
    <label class="pfield"><span>Default agent</span>
      <select id="pref-type">
        <option value="">—</option>
        <option value="claude">claude</option>
        <option value="codex">codex</option>
        <option value="gemini">gemini</option>
      </select>
    </label>
    <label class="pfield"><span>Default model</span>
      <input id="pref-model" list="model-options" placeholder="e.g. claude-opus-4-8"></label>
    <datalist id="model-options">
      <option value="claude-opus-4-8"></option>
      <option value="claude-opus-4-7"></option>
      <option value="claude-opus-4-6"></option>
      <option value="claude-sonnet-4-6"></option>
      <option value="claude-haiku-4-5"></option>
    </datalist>
    <label class="pfield"><span>Default args</span>
      <input id="pref-args" placeholder="--permission-mode acceptEdits"></label>
    <label class="check"><input type="checkbox" id="pref-isolate"> Isolate new agents in a git worktree</label>
    <label class="check"><input type="checkbox" id="pref-continue"> Auto-continue new agents past usage limits</label>
    <label class="pfield"><span>Max tokens</span>
      <input id="pref-maxtok" type="number" min="0" placeholder="no cap"></label>
    <label class="pfield"><span>Max cost ($)</span>
      <input id="pref-maxcost" type="number" min="0" step="0.01" placeholder="no cap"></label>

    <div class="pop-actions"><button class="primary" onclick="savePrefs()">Save</button></div>
    <div class="pop-status" id="pref-status" aria-live="polite"></div>
    <div class="pop-hint">Defaults apply to agents created <strong>after</strong> saving — running agents keep their settings. Token pricing for cost lives in <code>~/.kaiju/pricing.json</code>.</div>

    <div class="pop-section">Devices</div>
    <div class="pop-hint">Pair another device on your network. The host machine is always trusted.</div>
    <div id="device-list" class="device-list"></div>
    <div class="pop-actions"><button onclick="startPairing()">Pair a device</button></div>
    <div id="pair-box" hidden style="text-align:center;margin-top:10px">
      <div id="pair-qr"></div>
      <div class="pop-hint">Scan this, or open <code id="pair-url"></code> and enter
        <strong id="pair-code"></strong>. Valid for 10 minutes.</div>
    </div>
  </div>

  <!-- ── Modals ──────────────────────────────────────────────────────────── -->
  <dialog id="newmodal" class="modal" onclick="if(event.target===this)closeNew()">
    <form onsubmit="createAgent(event)">
      <div class="modal-head">
        <h2>New agent</h2>
        <button type="button" class="icon" onclick="closeNew()" title="Close">&times;</button>
      </div>
      <label class="field">
        <span>Workspace path <em>*</em></span>
        <input id="n-ws" placeholder="/path/to/repo" required autocomplete="off">
      </label>
      <label class="field">
        <span>Agent</span>
        <select id="n-type">
          <option value="claude">claude</option>
          <option value="codex">codex</option>
          <option value="gemini">gemini</option>
        </select>
      </label>
      <label class="field">
        <span>Prompt</span>
        <textarea id="n-prompt" rows="3" placeholder="What should the agent do?"></textarea>
      </label>
      <details class="advanced">
        <summary>Advanced</summary>
        <label class="field">
          <span>Model</span>
          <input id="n-model" placeholder="optional — defaults to the agent's own">
        </label>
        <label class="check"><input type="checkbox" id="n-isolate"> Run in an isolated git worktree</label>
        <label class="check"><input type="checkbox" id="n-auto"> Auto-approve (run unattended — skips approval prompts)</label>
        <label class="check"><input type="checkbox" id="n-continue"> Auto-continue past usage limits (resume at reset)</label>
      </details>
      <div class="modal-actions">
        <button type="button" onclick="closeNew()">Cancel</button>
        <button type="submit" class="primary">Start agent</button>
      </div>
    </form>
  </dialog>

  <dialog id="adoptmodal" class="modal" onclick="if(event.target===this)closeAdopt()">
    <form onsubmit="return false">
    <div class="modal-head">
      <h2>Adopt a session</h2>
      <button type="button" class="icon" onclick="closeAdopt()" title="Close">&times;</button>
    </div>
    <label class="field">
      <span>Workspace path <em>*</em></span>
      <input id="ad-ws" placeholder="/path/to/repo" autocomplete="off" onchange="loadSessions()">
    </label>
    <label class="field">
      <span>Agent</span>
      <select id="ad-type" onchange="loadSessions()">
        <option value="claude">claude</option>
      </select>
    </label>
    <div id="ad-sessions" class="device-list"></div>
    <div class="note">Close the original session first so two clients do not drive one conversation.</div>
    <div class="modal-actions">
      <button type="button" onclick="closeAdopt()">Cancel</button>
    </div>
    </form>
  </dialog>

  <dialog id="comparemodal" class="modal" onclick="if(event.target===this)closeCompare()">
    <form onsubmit="event.preventDefault();submitCompare()">
    <div class="modal-head">
      <h2>Compare task across CLIs</h2>
      <button type="button" class="icon" onclick="closeCompare()" title="Close">&times;</button>
    </div>
    <label class="field">
      <span>Workspace path <em>*</em></span>
      <input id="cmp-ws" placeholder="/path/to/repo (git)" autocomplete="off">
    </label>
    <label class="field">
      <span>Prompt <em>*</em></span>
      <textarea id="cmp-prompt" rows="3" placeholder="What should each agent do?"></textarea>
    </label>
    <div class="field">
      <span>Run on</span>
      <div class="cmp-types">
        <label class="check"><input type="checkbox" class="cmp-type" value="claude" checked> claude</label>
        <label class="check"><input type="checkbox" class="cmp-type" value="codex"> codex</label>
        <label class="check"><input type="checkbox" class="cmp-type" value="gemini"> gemini</label>
      </div>
    </div>
    <label class="check"><input type="checkbox" id="cmp-auto" checked> Auto-approve runs (finish unattended, then judge)</label>
    <div class="note">Each CLI runs the same prompt in its own isolated git worktree, so they don't clobber each other.</div>
    <div class="modal-actions">
      <button type="button" onclick="closeCompare()">Cancel</button>
      <button type="submit" class="primary">Run comparison</button>
    </div>
    </form>
  </dialog>

  <!-- ── Main shell: rail + stage ─────────────────────────────────────────── -->
  <div id="shell">

    <!-- Left rail: agent list -->
    <div id="rail">
      <div id="rail-header">
        <div class="counts" id="counts"></div>
      </div>
      <div id="agent-list">
        <div id="rows"></div>
        <div class="empty" id="empty" hidden>No agents yet.</div>
      </div>
    </div>

    <!-- Right stage: welcome / detail / compare / leaderboard -->
    <div id="stage">

      <!-- Welcome empty state -->
      <div id="welcome">
        <div class="welcome-glyph">KAIJU</div>
        <p>Select an agent to open its terminal, or start a bake-off.</p>
      </div>

      <!-- Agent detail -->
      <div id="detail" hidden>
        <div class="detail-head">
          <span class="id" id="d-id"></span>
          <button class="sm" title="Copy full ID" onclick="copyId(selected)">⧉ Copy ID</button>
          <span class="status" id="d-status"></span>
          <span class="path" id="d-workspace" title=""></span>
          <span class="activity" id="d-activity"></span>
          <span class="port" id="d-port"></span>
          <div class="grow"></div>
          <button class="sm" id="d-autocont" onclick="toggleAutoContinue()" title="Auto-continue past usage limits" hidden></button>
          <button class="sm" id="d-interrupt" onclick="act('interrupt')">Interrupt</button>
          <button class="sm danger" id="d-stop" onclick="act('stop')">Stop</button>
          <button class="sm primary" id="d-resume" onclick="act('resume')" hidden>Resume</button>
          <button class="sm" onclick="closeDetail()">Close</button>
        </div>
        <div class="detail-body">
          <div class="tabs">
            <button id="tab-logs" class="tab" onclick="showTab('logs')">Logs</button>
            <button id="tab-term" class="tab active" onclick="showTab('term')">Terminal</button>
            <button id="tab-diff" class="tab" onclick="showTab('diff')">Diff</button>
          </div>
          <pre class="logs" id="d-logs" hidden>Loading…</pre>
          <div id="d-term"></div>
          <div class="reply">
            <input id="d-reply" placeholder="Reply or approve (Enter to send)…" onkeydown="if(event.key==='Enter')sendReply()">
            <button class="primary" onclick="sendReply()">Send</button>
          </div>
          <div class="note" id="d-note"></div>
        </div>
      </div>

      <!-- Compare panel -->
      <div id="compare-panel" hidden>
        <div class="detail-head">
          <span class="id">Comparison</span>
          <span class="path" id="cmp-prompt-label" title=""></span>
          <div class="grow"></div>
          <input id="cmp-test-cmd" placeholder="test cmd (optional, e.g. cargo test)"
                 title="Run this in each worktree and feed pass/fail to the judge"
                 style="flex:0 1 18rem">
          <button id="cmp-judge-btn" onclick="judgeComparison()">Judge</button>
          <button onclick="closeComparison()">Close</button>
        </div>
        <div class="compare-body">
          <div id="cmp-verdict" hidden></div>
          <div id="cmp-cols" class="cmp-cols"></div>
        </div>
      </div>

      <!-- Leaderboard panel -->
      <div id="leaderboard-panel" hidden>
        <div class="detail-head">
          <span class="id">Leaderboard</span>
          <input id="lb-ws" placeholder="/path/to/repo" autocomplete="off"
                 style="flex:0 1 22rem" onchange="loadLeaderboard()">
          <div class="grow"></div>
          <button onclick="closeLeaderboard()">Close</button>
        </div>
        <div class="lb-body" id="lb-body"></div>
      </div>

    </div><!-- #stage -->
  </div><!-- #shell -->

<script src="/assets/dashboard-utils.js"></script>
<script src="/assets/dashboard.js"></script>
</body>
</html>"#;

/// The pairing claim page served at `GET /pair`. A scanned QR lands here with
/// `?code=...`; it redeems the code and saves the returned token under the same
/// `kaiju_token` localStorage key the dashboard reads, then redirects to `/`.
pub const PAIR_PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Pair this device — Kaiju</title>
<style>
  body { font: 16px system-ui, sans-serif; margin: 0; display: grid; place-items: center;
         min-height: 100vh; background: #0b0c0f; color: #e6e6e6; }
  .card { max-width: 360px; padding: 28px; text-align: center; }
  h1 { font-size: 20px; margin: 0 0 12px; }
  input { width: 100%; padding: 10px; margin: 10px 0; box-sizing: border-box;
          border-radius: 8px; border: 1px solid #333; background: #15171c; color: #e6e6e6; }
  button { padding: 10px 18px; border-radius: 8px; border: 0; background: #5b8cff;
           color: #fff; font-weight: 600; cursor: pointer; }
  .msg { margin-top: 12px; min-height: 1.4em; color: #ff8080; }
</style>
</head>
<body>
  <div class="card">
    <h1>Pair this device</h1>
    <p>Name this device, then pair to access the Kaiju dashboard.</p>
    <input id="name" placeholder="e.g. My phone" autocomplete="off">
    <button onclick="claim()">Pair</button>
    <div class="msg" id="msg"></div>
  </div>
<script>
  const params = new URLSearchParams(location.search);
  const code = params.get("code") || "";
  document.getElementById("name").value =
    /iphone|android|ipad|mobile/i.test(navigator.userAgent) ? "Phone" : "";
  async function claim() {
    const name = document.getElementById("name").value || "device";
    const msg = document.getElementById("msg");
    if (!code) { msg.textContent = "Missing pairing code in the link."; return; }
    msg.textContent = "Pairing…";
    try {
      const res = await fetch("/pair/claim", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ code, name }),
      });
      if (!res.ok) { msg.textContent = "Code invalid or expired. Ask for a new one."; return; }
      const data = await res.json();
      localStorage.setItem("kaiju_token", data.token);
      location.href = "/";
    } catch (e) {
      msg.textContent = "Network error. Are you on the same network?";
    }
  }
</script>
</body>
</html>"#;
