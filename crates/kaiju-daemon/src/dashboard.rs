//! The live fleet dashboard — a self-contained HTML page served at `/`.
//!
//! It polls the daemon's own API in the browser, so it always reflects current
//! state with no server-side rendering. Clicking an agent opens a detail panel
//! with its live log tail and a reply box, all driven by the existing endpoints
//! (`/agents`, `/agents/:id/logs`, `/agents/:id/input`, `/interrupt`, `/stop`,
//! `/diff`). Kept as one static string to avoid a templating dependency.

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
</style>
</head>
<body>
  <h1>Kaiju</h1>
  <div class="sub">Live fleet &middot; refreshing every 2s &middot; <span id="updated"></span></div>
  <div class="card toolbar" style="margin-bottom:1.25rem; display:flex; align-items:center; gap:1rem; flex-wrap:wrap">
    <button class="primary" onclick="toggleNew()">+ New agent</button>
    <label style="font-size:.85rem; color:var(--muted); margin-left:auto">
      <input type="checkbox" id="notify-toggle" onchange="toggleNotify()"> notify when an agent needs input
    </label>
    <form id="newform" hidden onsubmit="createAgent(event)"
          style="margin-top:.75rem; display:flex; gap:.5rem; flex-wrap:wrap; align-items:center">
      <select id="n-type">
        <option value="claude">claude</option>
        <option value="codex">codex</option>
        <option value="gemini">gemini</option>
      </select>
      <input id="n-ws" placeholder="workspace path" value="." style="flex:1; min-width:14rem">
      <input id="n-model" placeholder="model (optional)">
      <input id="n-prompt" placeholder="prompt" style="flex:2; min-width:18rem">
      <label style="font-size:.85rem; color:var(--muted)"><input type="checkbox" id="n-isolate"> isolate</label>
      <button type="submit" class="primary">Start</button>
    </form>
  </div>
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
      <span class="grow"></span>
      <button onclick="loadDiff()">Diff</button>
      <button onclick="act('interrupt')">Interrupt</button>
      <button onclick="act('stop')">Stop</button>
      <button onclick="closeDetail()">Close</button>
    </div>
    <div class="tabs">
      <button id="tab-logs" class="tab" onclick="showTab('logs')">Logs</button>
      <button id="tab-term" class="tab active" onclick="showTab('term')">Terminal</button>
    </div>
    <pre class="logs" id="d-logs" hidden>Loading…</pre>
    <div id="d-term"></div>
    <div class="reply">
      <input id="d-reply" placeholder="Reply or approve (Enter to send)…" onkeydown="if(event.key==='Enter')sendReply()">
      <button class="primary" onclick="sendReply()">Send</button>
    </div>
    <div class="note" id="d-note"></div>
  </div>

<script>
const ORDER = { waitingforinput: 0, stuck: 1, error: 2, starting: 3, running: 4, completed: 5, stopped: 6 };
let selected = null;
let lastStatus = {};
let token = localStorage.getItem("kaiju_token") || "";
let term = null, ws = null, activeTab = "term";

// Statuses that mean an agent needs the operator. A transition *into* one of
// these fires a desktop notification (opt-in, persisted).
const ATTENTION = new Set(["waitingforinput", "stuck"]);
let notifyOn = localStorage.getItem("kaiju_notify") === "1";

function initNotify() {
  const box = document.getElementById("notify-toggle");
  if (box) box.checked = notifyOn;
  if (notifyOn && "Notification" in window && Notification.permission === "default") {
    Notification.requestPermission();
  }
}

function toggleNotify() {
  notifyOn = document.getElementById("notify-toggle").checked;
  localStorage.setItem("kaiju_notify", notifyOn ? "1" : "0");
  if (notifyOn && "Notification" in window && Notification.permission === "default") {
    Notification.requestPermission().then((p) => {
      if (p !== "granted") note("notifications are blocked in browser settings");
    });
  }
}

// Fire a desktop toast when an agent *newly* enters an attention state. Skips
// the first sighting (prev === undefined) so opening the page doesn't spam one
// per already-waiting agent; the tag coalesces repeats for the same agent.
function notifyTransition(prev, agent) {
  if (!notifyOn || !("Notification" in window) || Notification.permission !== "granted") return;
  if (prev === undefined || prev === agent.status || !ATTENTION.has(agent.status)) return;
  const n = new Notification("Kaiju · " + agent.id.slice(0, 10) + " " + agent.status, {
    body: agent.prompt || agent.agent_type,
    tag: agent.id,
  });
  n.onclick = () => { window.focus(); select(agent.id); n.close(); };
}

function showTab(which) {
  activeTab = which;
  const onTerm = which === "term";
  document.getElementById("tab-logs").classList.toggle("active", !onTerm);
  document.getElementById("tab-term").classList.toggle("active", onTerm);
  document.getElementById("d-logs").hidden = onTerm;
  document.getElementById("d-term").hidden = !onTerm;
  if (onTerm) openTerminal(); else { closeTerminal(); refreshDetail(); }
}

// Terminal appearance — kept in one place so the measuring span used to fit the
// pane to the panel matches exactly what xterm renders.
const TERM_FONT = "'JetBrains Mono','SF Mono','Cascadia Code',Menlo,Monaco,'DejaVu Sans Mono',ui-monospace,monospace";
const TERM_FONT_SIZE = 13, TERM_LINE_HEIGHT = 1.25, TERM_LETTER = 0.2, TERM_PAD = 10;
const TERM_THEME = {
  background: '#0d1117', foreground: '#c9d1d9', cursor: '#c9d1d9',
  selectionBackground: '#3b82f655',
  black: '#484f58', red: '#ff7b72', green: '#3fb950', yellow: '#d29922',
  blue: '#58a6ff', magenta: '#bc8cff', cyan: '#39c5cf', white: '#b1bac4',
  brightBlack: '#6e7681', brightRed: '#ffa198', brightGreen: '#56d364',
  brightYellow: '#e3b341', brightBlue: '#79c0ff', brightMagenta: '#d2a8ff',
  brightCyan: '#56d4dd', brightWhite: '#f0f6fc',
};

// Measure one monospace cell as xterm will draw it, then derive how many
// cols/rows fit the panel. Avoids depending on xterm internals or a fit addon.
function fitTermSize(host) {
  const span = document.createElement("span");
  span.style.cssText = "position:absolute;visibility:hidden;white-space:pre;font-family:" +
    TERM_FONT + ";font-size:" + TERM_FONT_SIZE + "px;letter-spacing:" + TERM_LETTER + "px;";
  span.textContent = "0".repeat(100);
  document.body.appendChild(span);
  const cellW = span.getBoundingClientRect().width / 100;
  document.body.removeChild(span);
  const cellH = TERM_FONT_SIZE * TERM_LINE_HEIGHT;
  const cols = Math.max(20, Math.floor((host.clientWidth - TERM_PAD * 2) / cellW));
  const rows = Math.max(6, Math.floor((host.clientHeight - TERM_PAD * 2) / cellH));
  return { cols, rows };
}

// Resize the tmux pane to match the browser viewport so the capture fills the
// panel and wraps at the right column. Best-effort.
async function syncBackendSize(cols, rows) {
  try {
    await api("/agents/" + selected + "/terminal/size",
      { method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ cols, rows }) });
  } catch (e) { /* best effort */ }
}

async function openTerminal() {
  closeTerminal();
  if (!selected || !window.Terminal) return;
  const host = document.getElementById("d-term");
  const { cols, rows } = fitTermSize(host);
  await syncBackendSize(cols, rows);
  // convertEol: tmux `capture-pane` separates rows with a bare LF (no CR), so
  // each line must return to column 0 — without this every partial line starts
  // where the previous one ended and the screen staircases to the right.
  term = new Terminal({ cols, rows, cursorBlink: true,
                        convertEol: true, scrollback: 500,
                        fontFamily: TERM_FONT, fontSize: TERM_FONT_SIZE,
                        fontWeight: 400, fontWeightBold: 600,
                        lineHeight: TERM_LINE_HEIGHT, letterSpacing: TERM_LETTER,
                        rightClickSelectsWord: true, macOptionClickForcesSelection: true,
                        theme: TERM_THEME });
  term.open(host);

  // The pane is a full repaint every tick, which would wipe a text selection and
  // yank the view to the bottom. So hold incoming frames while the user is busy —
  // selecting text or scrolled up into history — then apply the latest once they
  // return to the live bottom. Keeps select/copy and 500-line scrollback usable.
  let pendingFrame = null;
  const isPinned = () => term && (term.hasSelection() ||
    term.buffer.active.viewportY < term.buffer.active.baseY);
  const flushPending = () => {
    if (term && !isPinned() && pendingFrame != null) {
      term.write(pendingFrame); pendingFrame = null;
    }
  };
  term.onSelectionChange(flushPending);
  term.onScroll(flushPending);
  // Cmd/Ctrl+C copies the selection (like a real terminal) instead of being sent
  // to the agent as SIGINT; with no selection it falls through to the app.
  term.attachCustomKeyEventHandler((e) => {
    const mod = e.metaKey || e.ctrlKey;
    if (e.type === "keydown" && mod && (e.key === "c" || e.key === "C") && term.hasSelection()) {
      if (navigator.clipboard) navigator.clipboard.writeText(term.getSelection());
      note("copied selection");
      return false;
    }
    return true;
  });

  const proto = location.protocol === "https:" ? "wss" : "ws";
  const q = token ? ("?token=" + encodeURIComponent(token)) : "";
  ws = new WebSocket(proto + "://" + location.host +
                     "/agents/" + selected + "/terminal/ws" + q);
  ws.onmessage = (e) => {
    if (!term) return;
    if (isPinned()) { pendingFrame = e.data; return; }
    term.write(e.data);
  };
  ws.onclose = () => { if (term) term.write("\r\n[disconnected]\r\n"); };
  term.onData((d) => { if (ws && ws.readyState === 1) ws.send(d); });
}

// Re-fit the pane when the window changes size (debounced).
let termResizeTimer = null;
window.addEventListener("resize", () => {
  if (!term || activeTab !== "term") return;
  clearTimeout(termResizeTimer);
  termResizeTimer = setTimeout(() => {
    if (!term) return;
    const { cols, rows } = fitTermSize(document.getElementById("d-term"));
    try { term.resize(cols, rows); } catch (e) {}
    syncBackendSize(cols, rows);
  }, 200);
});

function closeTerminal() {
  if (ws) { try { ws.close(); } catch (e) {} ws = null; }
  if (term) { try { term.dispose(); } catch (e) {} term = null; }
  const el = document.getElementById("d-term");
  if (el) el.innerHTML = "";
}

// fetch wrapper that attaches the bearer token and, on 401, prompts for one.
async function api(url, opts) {
  opts = opts || {};
  const headers = Object.assign({}, opts.headers || {});
  if (token) headers["Authorization"] = "Bearer " + token;
  const res = await window.fetch(url, Object.assign({}, opts, { headers }));
  if (res.status === 401) {
    const t = prompt("Daemon API token:");
    if (t) { token = t; localStorage.setItem("kaiju_token", t); return api(url, opts); }
  }
  return res;
}

function fmtDuration(s) {
  if (s < 60) return s + "s";
  if (s < 3600) return Math.floor(s/60) + "m " + (s%60) + "s";
  return Math.floor(s/3600) + "h " + Math.floor((s%3600)/60) + "m";
}
function esc(s) { return (s || "").replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;'); }
function note(msg) { document.getElementById("d-note").textContent = msg; }

function render(agents) {
  document.getElementById("empty").hidden = agents.length > 0;
  const prev = lastStatus;            // previous poll's statuses, to detect transitions
  lastStatus = {};
  for (const a of agents) {
    lastStatus[a.id] = a.status;
    notifyTransition(prev[a.id], a);
  }

  const counts = {};
  for (const a of agents) counts[a.status] = (counts[a.status] || 0) + 1;
  document.getElementById("counts").innerHTML =
    Object.keys(counts).sort((x, y) => (ORDER[x]??9) - (ORDER[y]??9))
      .map(s => `<span class="pill">${counts[s]} ${esc(s)}</span>`).join("");

  agents.sort((a, b) => (ORDER[a.status]??9) - (ORDER[b.status]??9));

  document.getElementById("rows").innerHTML = agents.map(a => {
    const m = a.metrics || {};
    const attn = (a.status === "waitingforinput" || a.status === "stuck") ? "attention" : "";
    const sel = a.id === selected ? " selected" : "";
    const cost = m.estimated_cost_usd != null ? "$" + m.estimated_cost_usd.toFixed(2) : "-";
    const toks = m.tokens_used != null ? m.tokens_used.toLocaleString() : "-";
    return `<tr class="${attn}${sel}" onclick="select('${a.id}')">
      <td class="id" title="${a.id}">${a.id.slice(0,10)}</td>
      <td>${esc(a.agent_type)}</td>
      <td>${esc(a.model) || "-"}</td>
      <td><span class="status s-${a.status}">${esc(a.status)}</span></td>
      <td>${fmtDuration(m.runtime_secs || 0)}</td>
      <td>${toks}</td>
      <td>${cost}</td>
      <td class="prompt">${esc(a.prompt) || "-"}</td>
      <td class="actions" onclick="event.stopPropagation()">
        <button title="Copy full ID" onclick="copyId('${a.id}')">⧉</button>
        <button title="Interrupt" onclick="rowAct('${a.id}','interrupt')">⎋</button>
        <button title="Stop" onclick="rowAct('${a.id}','stop')">■</button>
        <button title="Remove" onclick="removeAgent('${a.id}')">✕</button>
      </td>
    </tr>`;
  }).join("");
}

function select(id) {
  selected = id;
  document.getElementById("detail").hidden = false;
  document.getElementById("d-id").textContent = id.slice(0, 10);
  document.getElementById("d-logs").textContent = "Loading…";
  note("");
  showTab("term");   // open on the live terminal by default
}
function closeDetail() {
  closeTerminal();
  selected = null;
  document.getElementById("detail").hidden = true;
}

async function refreshDetail() {
  if (!selected) return;
  // Keep the header status in sync with the table on every poll, regardless of
  // which tab is open.
  const st = lastStatus[selected];
  const badge = document.getElementById("d-status");
  badge.textContent = st || "?";
  badge.className = "status s-" + (st || "");
  // Logs are hidden while the terminal is open — skip fetching them, but the
  // status above is already up to date.
  if (activeTab === "term") return;
  try {
    const res = await api("/agents/" + selected + "/logs");
    if (res.ok) {
      document.getElementById("d-logs").textContent = (await res.json()).logs || "(no output)";
    } else {
      document.getElementById("d-logs").textContent = "(logs unavailable — session may have ended)";
    }
  } catch (e) { /* leave previous logs */ }
}

async function loadDiff() {
  if (!selected) return;
  note("loading diff…");
  try {
    const res = await api("/agents/" + selected + "/diff");
    const body = await res.json();
    document.getElementById("d-logs").textContent = res.ok ? (body.diff || "(no changes)") : (body.error || "diff failed");
    note("showing diff");
  } catch (e) { note("diff failed"); }
}

async function sendReply() {
  if (!selected) return;
  const input = document.getElementById("d-reply");
  const text = input.value;
  if (!text) return;
  try {
    const res = await api("/agents/" + selected + "/input", {
      method: "POST", headers: { "content-type": "application/json" },
      body: JSON.stringify({ text }),
    });
    if (res.ok) { input.value = ""; note("sent"); setTimeout(refreshDetail, 400); }
    else { note((await res.json()).error || "send failed"); }
  } catch (e) { note("send failed"); }
}

async function act(path) {
  if (!selected) return;
  try {
    const res = await api("/agents/" + selected + "/" + path, { method: "POST" });
    note(res.ok ? (path + " sent") : ((await res.json()).error || (path + " failed")));
  } catch (e) { note(path + " failed"); }
}

function toggleNew() {
  const f = document.getElementById("newform");
  f.hidden = !f.hidden;
}

async function createAgent(ev) {
  ev.preventDefault();
  const body = {
    agent_type: document.getElementById("n-type").value,
    workspace: document.getElementById("n-ws").value,
    prompt: document.getElementById("n-prompt").value || null,
    isolate: document.getElementById("n-isolate").checked,
    auto_start: true,
  };
  const model = document.getElementById("n-model").value;
  if (model) body.model = model;
  try {
    const res = await api("/agents", {
      method: "POST", headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    if (res.ok) { document.getElementById("newform").hidden = true; refresh(); }
    else { alert((await res.json()).error || "create failed"); }
  } catch (e) { alert("create failed"); }
}

async function rowAct(id, path) {
  try { await api("/agents/" + id + "/" + path, { method: "POST" }); refresh(); }
  catch (e) { /* ignore */ }
}

async function removeAgent(id) {
  if (!confirm("Remove agent " + id.slice(0, 10) + "? (stops it if running)")) return;
  try {
    await api("/agents/" + id, { method: "DELETE" });
    if (id === selected) closeDetail();
    refresh();
  } catch (e) { /* ignore */ }
}

function copyId(id) {
  navigator.clipboard.writeText(id).then(
    () => { /* copied */ },
    () => { window.prompt("Copy agent ID:", id); }
  );
}

async function refresh() {
  try {
    const res = await api("/agents");
    render(await res.json());
    document.getElementById("updated").textContent = "updated " + new Date().toLocaleTimeString();
    refreshDetail();
  } catch (e) {
    document.getElementById("updated").textContent = "daemon unreachable";
  }
}

// Drag-and-drop a file onto the terminal: upload it into the agent's working
// dir, then type the returned path into the live session.
(function setupTerminalDrop() {
  const el = document.getElementById("d-term");
  if (!el) return;
  el.addEventListener("dragover", (e) => { e.preventDefault(); el.style.outline = "2px dashed #3b82f6"; });
  el.addEventListener("dragleave", () => { el.style.outline = ""; });
  el.addEventListener("drop", async (e) => {
    e.preventDefault();
    el.style.outline = "";
    if (!selected || !ws || ws.readyState !== 1) {
      note("open the Terminal tab before dropping a file");
      return;
    }
    for (const file of e.dataTransfer.files) {
      note("⬆ uploading " + file.name + "…");
      try {
        const buf = await file.arrayBuffer();
        const res = await api("/agents/" + selected + "/files?name=" + encodeURIComponent(file.name), {
          method: "POST",
          headers: { "content-type": "application/octet-stream" },
          body: buf,
        });
        if (res.ok) {
          const p = (await res.json()).path;
          ws.send(p + " ");
          if (term) term.focus();
          note("📎 uploaded " + file.name + " → " + p);
        } else {
          note("upload failed (" + res.status + "): " + file.name);
        }
      } catch (err) {
        note("upload failed: " + file.name);
      }
    }
  });
})();

initNotify();
refresh();
setInterval(refresh, 2000);
</script>
</body>
</html>"#;
