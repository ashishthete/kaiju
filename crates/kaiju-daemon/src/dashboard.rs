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
  :root {
    color-scheme: light dark;
    --bg: #f6f8fa; --surface: #ffffff; --surface-2: #eef1f5;
    --border: #d6dbe1; --text: #1f2328; --muted: #6b7280;
    --accent: #3b82f6; --accent-weak: #3b82f61f; --accent-fg: #fff;
    --term-bg: #0d1117; --radius: 10px;
    --shadow: 0 1px 2px #0000000d, 0 4px 16px #0000000a;
  }
  @media (prefers-color-scheme: dark) {
    :root {
      --bg: #0b0e14; --surface: #11161f; --surface-2: #1a2130;
      --border: #232a36; --text: #e6edf3; --muted: #8b949e;
      --accent: #4c8dff; --accent-weak: #4c8dff26;
      --shadow: 0 1px 2px #0000003d, 0 8px 24px #00000040;
    }
  }
  * { box-sizing: border-box; }
  body { font-family: system-ui, -apple-system, sans-serif; margin: 0; padding: 1.75rem;
         background: var(--bg); color: var(--text); -webkit-font-smoothing: antialiased; }
  h1 { font-size: 1.35rem; margin: 0 0 .15rem; letter-spacing: -.01em; }
  .sub { color: var(--muted); font-size: .85rem; margin-bottom: 1.25rem; }

  .card { background: var(--surface); border: 1px solid var(--border);
          border-radius: var(--radius); box-shadow: var(--shadow); }
  .toolbar { padding: 1rem 1.1rem; margin-bottom: 1.25rem; }

  .counts { display: flex; gap: .5rem; flex-wrap: wrap; margin-bottom: 1rem; }
  .pill { padding: .25rem .7rem; border-radius: 999px; font-size: .78rem; font-weight: 500;
          background: var(--surface-2); border: 1px solid var(--border); color: var(--muted); }

  table { width: 100%; border-collapse: collapse; font-size: .9rem; }
  thead { background: var(--surface-2); }
  th, td { text-align: left; padding: .6rem .75rem; border-bottom: 1px solid var(--border); }
  th:first-child { border-top-left-radius: var(--radius); }
  th:last-child { border-top-right-radius: var(--radius); }
  th { font-weight: 600; color: var(--muted); font-size: .72rem; text-transform: uppercase; letter-spacing: .04em; }
  tbody tr { cursor: pointer; transition: background .1s ease; }
  tbody tr:hover { background: var(--surface-2); }
  tbody tr:last-child td { border-bottom: none; }
  tr.selected { background: var(--accent-weak); box-shadow: inset 3px 0 0 var(--accent); }
  td.id { font-family: ui-monospace, monospace; font-weight: 500; }

  .status { font-weight: 600; padding: .2rem .55rem; border-radius: 999px; font-size: .76rem;
            white-space: nowrap; display: inline-flex; align-items: center; gap: .35rem; }
  .status::before { content: ""; width: .45rem; height: .45rem; border-radius: 999px; background: currentColor; }
  .s-waitingforinput { background: #f59e0b22; color: #d97706; }
  .s-stuck, .s-error { background: #ef444422; color: #ef4444; }
  .s-running { background: #22c55e22; color: #22c55e; }
  .s-starting { background: #3b82f622; color: #4c8dff; }
  .s-completed, .s-stopped { background: #88888822; color: #9aa4b2; }
  .prompt { color: var(--muted); max-width: 24rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .empty { color: var(--muted); padding: 2.5rem 0; text-align: center; }
  .attention td { background: #f59e0b12; }

  #detail { margin-top: 1.5rem; padding: 1.1rem 1.25rem; }
  #detail[hidden] { display: none; }
  .detail-head { display: flex; align-items: center; gap: .75rem; margin-bottom: .85rem; }
  .detail-head .id { font-family: ui-monospace, monospace; font-weight: 600; font-size: 1.05rem; }
  .path { font-family: ui-monospace, monospace; font-size: .78rem; color: var(--muted);
          overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 46ch; }
  .grow { flex: 1; }

  button { font: inherit; font-size: .85rem; padding: .4rem .8rem; border-radius: 7px;
           border: 1px solid var(--border); background: var(--surface-2); color: var(--text);
           cursor: pointer; transition: background .12s ease, border-color .12s ease; }
  button:hover { background: var(--border); }
  button.primary { background: var(--accent); border-color: var(--accent); color: var(--accent-fg); font-weight: 600; }
  button.primary:hover { filter: brightness(1.08); }

  input, select { font: inherit; padding: .45rem .65rem; border-radius: 7px;
                  border: 1px solid var(--border); background: var(--bg); color: var(--text); }
  input:focus, select:focus { outline: none; border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-weak); }

  pre.logs { background: var(--term-bg); color: #c9d1d9; padding: .85rem; border-radius: 8px;
             max-height: 60vh; overflow: auto; font-size: .82rem; white-space: pre-wrap; word-break: break-word; }
  pre.logs .d-add { color: #3fb950; }
  pre.logs .d-del { color: #f85149; }
  pre.logs .d-hunk { color: #39c5cf; }
  pre.logs .d-file { color: #d2a8ff; font-weight: 600; }

  .spinner { display: inline-block; width: .8em; height: .8em; vertical-align: -2px;
             border: 2px solid var(--border); border-top-color: var(--accent);
             border-radius: 50%; animation: spin .6s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .reply { display: flex; gap: .5rem; margin-top: .85rem; }
  .reply input { flex: 1; }
  .note { color: var(--muted); font-size: .8rem; margin-top: .45rem; min-height: 1rem; }

  .tabs { display: flex; gap: .35rem; margin-bottom: .65rem; }
  .tab { font-size: .8rem; padding: .3rem .85rem; border-radius: 7px; }
  .tab.active { background: var(--accent-weak); border-color: var(--accent); color: var(--accent); font-weight: 600; }

  #d-term { width: 100%; height: 62vh; min-height: 22rem; background: var(--term-bg);
            border: 1px solid var(--border); border-radius: 8px; padding: .6rem; overflow: hidden; }
  #d-term[hidden] { display: none; }
  #d-term .xterm-viewport { background: transparent !important; }

  td.actions { white-space: nowrap; }
  td.actions button { padding: .25rem .5rem; font-size: .8rem; margin-left: .3rem; line-height: 1; }

  dialog.modal { border: 1px solid var(--border); border-radius: var(--radius);
                 background: var(--surface); color: var(--text); padding: 0;
                 width: min(560px, 92vw); box-shadow: var(--shadow); }
  dialog.modal::backdrop { background: #0008; backdrop-filter: blur(2px); }
  dialog.modal form { padding: 1.25rem 1.4rem 1.4rem; display: flex; flex-direction: column; gap: .9rem; }
  .modal-head { display: flex; align-items: center; justify-content: space-between; }
  .modal-head h2 { margin: 0; font-size: 1.1rem; letter-spacing: -.01em; }
  .field { display: flex; flex-direction: column; gap: .3rem; font-size: .8rem; color: var(--muted); }
  .field span em { color: var(--accent); font-style: normal; }
  .field input, .field select, .field textarea { width: 100%; }
  textarea { font: inherit; padding: .45rem .65rem; border-radius: 7px; resize: vertical;
             border: 1px solid var(--border); background: var(--bg); color: var(--text); }
  textarea:focus { outline: none; border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-weak); }
  details.advanced { border-top: 1px solid var(--border); padding-top: .7rem; }
  details.advanced summary { cursor: pointer; color: var(--muted); font-size: .82rem; }
  details.advanced > .field, details.advanced > .check { margin-top: .7rem; }
  .check { display: flex; align-items: center; gap: .45rem; font-size: .85rem; color: var(--muted); }
  .modal-actions { display: flex; justify-content: flex-end; gap: .5rem; margin-top: .3rem; }
  .icon { padding: .15rem .5rem; font-size: 1.1rem; line-height: 1; }

  .popover { position: fixed; inset: auto; top: 4.6rem; right: 1.75rem; margin: 0;
             background: var(--surface); color: var(--text); border: 1px solid var(--border);
             border-radius: var(--radius); box-shadow: var(--shadow); padding: .9rem 1rem;
             min-width: 17rem; max-width: 24rem; }
  .popover:not(:popover-open) { display: none; }
  .pop-title { font-weight: 600; font-size: .9rem; margin-bottom: .7rem; }
  .popover .check { margin-bottom: .7rem; }
  .pop-status { font-size: .78rem; color: var(--muted); margin-top: .6rem; min-height: 1rem; }
  .pop-hint { font-size: .75rem; color: var(--muted); margin-top: .6rem; line-height: 1.4;
              border-top: 1px solid var(--border); padding-top: .6rem; }
  .pop-hint code { font-family: ui-monospace, monospace; background: var(--surface-2);
                   padding: .05rem .3rem; border-radius: 4px; }
</style>
</head>
<body>
  <h1>Kaiju</h1>
  <div class="sub">Live fleet &middot; refreshing every 2s &middot; <span id="updated"></span></div>
  <div class="card toolbar" style="margin-bottom:1.25rem; display:flex; align-items:center; gap:1rem; flex-wrap:wrap">
    <button class="primary" onclick="toggleNew()">+ New agent</button>
    <button class="icon" id="settings-btn" popovertarget="settings-pop" style="margin-left:auto"
            title="Settings" aria-label="Settings">⚙</button>
  </div>

  <div id="settings-pop" popover class="popover" aria-label="Settings">
    <div class="pop-title">Settings</div>
    <label class="check">
      <input type="checkbox" id="notify-toggle" onchange="toggleNotify()">
      Notify when an agent needs input
    </label>
    <button onclick="testNotify()">Send test notification</button>
    <div class="pop-status" id="settings-status" aria-live="polite"></div>
    <div class="pop-hint">Per-browser, and needs this tab open. For alerts without the browser, run the daemon with <code>KAIJU_DESKTOP_NOTIFY=1</code>.</div>
  </div>

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
      </details>
      <div class="modal-actions">
        <button type="button" onclick="closeNew()">Cancel</button>
        <button type="submit" class="primary">Start agent</button>
      </div>
    </form>
  </dialog>
  <div class="counts" id="counts"></div>
  <div class="card" style="overflow:hidden">
  <table>
    <thead><tr>
      <th>ID</th><th>Type</th><th>Model</th><th>Status</th>
      <th>Runtime</th><th>Tokens</th><th>Cost</th><th>Task</th><th>Actions</th>
    </tr></thead>
    <tbody id="rows"></tbody>
  </table>
  </div>
  <div class="empty" id="empty" hidden>No agents yet.</div>

  <div id="detail" class="card" hidden>
    <div class="detail-head">
      <span class="id" id="d-id"></span>
      <button title="Copy full ID" onclick="copyId(selected)">⧉ copy id</button>
      <span class="status" id="d-status"></span>
      <span class="path" id="d-workspace" title=""></span>
      <span class="grow"></span>
      <button id="d-interrupt" onclick="act('interrupt')">Interrupt</button>
      <button id="d-stop" onclick="act('stop')">Stop</button>
      <button id="d-resume" class="primary" onclick="act('resume')" hidden>Resume</button>
      <button onclick="closeDetail()">Close</button>
    </div>
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

<script src="/assets/dashboard-utils.js"></script>
<script src="/assets/dashboard.js"></script>
</body>
</html>"#;
