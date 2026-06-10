// Kaiju dashboard controller. Pure helpers (ORDER, ATTENTION, TERMINAL, esc,
// fmtDuration, renderDiff) come from dashboard-utils.js, loaded first.

let selected = null;
let lastStatus = {};
let lastWorkspace = {};   // agent id -> workspace path, from the latest poll
let lastUpdated = {};     // agent id -> updated_at (ISO), for "last activity"
let lastAgents = [];      // most recent fetch, so filtering re-renders without refetch
let token = localStorage.getItem("kaiju_token") || "";
let term = null, ws = null, activeTab = "term";
let paused = false, pollTimer = null;

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
  document.getElementById("tab-logs").classList.toggle("active", which === "logs");
  document.getElementById("tab-term").classList.toggle("active", which === "term");
  document.getElementById("tab-diff").classList.toggle("active", which === "diff");
  // The terminal has its own element; Logs and Diff share the <pre>.
  document.getElementById("d-term").hidden = which !== "term";
  document.getElementById("d-logs").hidden = which === "term";
  if (which === "term") { openTerminal(); return; }
  closeTerminal();
  if (which === "diff") { lastDiff = null; loadDiff(); } else { refreshDetail(); }
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

// Cell size in CSS px. Prefer xterm's *actual* rendered cell (so rows/cols match
// exactly and nothing is clipped); before the terminal exists, fall back to a
// measured estimate rounded UP — a slightly small grid leaves a thin margin,
// never a clipped bottom row.
function cellSize() {
  try {
    const c = term._core._renderService.dimensions.css.cell;
    if (c && c.width && c.height) return { w: c.width, h: c.height };
  } catch (e) { /* not rendered yet — estimate below */ }
  const span = document.createElement("span");
  span.style.cssText = "position:absolute;visibility:hidden;white-space:pre;font-family:" +
    TERM_FONT + ";font-size:" + TERM_FONT_SIZE + "px;letter-spacing:" + TERM_LETTER + "px;";
  span.textContent = "0".repeat(100);
  document.body.appendChild(span);
  const w = span.getBoundingClientRect().width / 100;
  document.body.removeChild(span);
  return { w: Math.ceil(w), h: Math.ceil(TERM_FONT_SIZE * TERM_LINE_HEIGHT) };
}

function fitDims(host) {
  const { w, h } = cellSize();
  return {
    cols: Math.max(20, Math.floor((host.clientWidth - TERM_PAD * 2) / w)),
    rows: Math.max(6, Math.floor((host.clientHeight - TERM_PAD * 2) / h)),
  };
}

// Resize xterm (and the tmux pane) to fit the panel using the real cell size.
function reFit() {
  if (!term) return;
  const d = fitDims(document.getElementById("d-term"));
  if (d.cols !== term.cols || d.rows !== term.rows) {
    try { term.resize(d.cols, d.rows); } catch (e) {}
    syncBackendSize(d.cols, d.rows);
  }
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
  const { cols, rows } = fitDims(host);   // estimate (terminal not created yet)
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
  // Now that xterm has rendered, re-fit to its real cell size so the bottom rows
  // aren't clipped (a deferred pass catches late layout).
  reFit();
  setTimeout(reFit, 60);

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
  termResizeTimer = setTimeout(reFit, 200);
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

function note(msg) { document.getElementById("d-note").textContent = msg; }
function noteBusy(msg) {
  document.getElementById("d-note").innerHTML = '<span class="spinner"></span> ' + esc(msg);
}

function render(agents) {
  lastAgents = agents;
  document.getElementById("empty").hidden = agents.length > 0;
  const prev = lastStatus;            // previous poll's statuses, to detect transitions
  lastStatus = {};
  for (const a of agents) {
    lastStatus[a.id] = a.status;
    lastWorkspace[a.id] = a.workspace;
    lastUpdated[a.id] = a.updated_at;
    notifyTransition(prev[a.id], a);
  }

  // Status summary reflects the whole fleet (not the filtered view).
  const counts = {};
  for (const a of agents) counts[a.status] = (counts[a.status] || 0) + 1;
  document.getElementById("counts").innerHTML =
    Object.keys(counts).sort((x, y) => (ORDER[x]??9) - (ORDER[y]??9))
      .map(s => `<span class="pill">${counts[s]} ${esc(statusLabel(s))}</span>`).join("");

  // Apply the fleet filter (search text + status) to the displayed rows.
  const ft = (document.getElementById("filter-text") || {}).value || "";
  const fs = (document.getElementById("filter-status") || {}).value || "all";
  const rows = agents
    .filter(a => matchesFilter(a, ft) && (fs === "all" || a.status === fs))
    .sort((a, b) => (ORDER[a.status]??9) - (ORDER[b.status]??9));

  document.getElementById("rows").innerHTML = rows.map(a => {
    const m = a.metrics || {};
    const attn = ATTENTION.has(a.status) ? "attention" : "";
    const sel = a.id === selected ? " selected" : "";
    const cost = m.estimated_cost_usd != null ? "$" + m.estimated_cost_usd.toFixed(2) : "-";
    const toks = m.tokens_used != null ? m.tokens_used.toLocaleString() : "-";
    return `<tr class="${attn}${sel}" title="${esc(a.workspace)}" onclick="select('${a.id}')">
      <td class="id" title="${a.id}">${a.id.slice(0,10)}</td>
      <td>${esc(a.agent_type)}</td>
      <td>${esc(a.model) || "-"}</td>
      <td><span class="status s-${a.status}">${esc(statusLabel(a.status))}</span></td>
      <td>${fmtDuration(m.runtime_secs || 0)}</td>
      <td>${toks}</td>
      <td>${cost}</td>
      <td class="prompt">${esc(a.prompt) || "-"}</td>
      <td class="actions" onclick="event.stopPropagation()">
        <button title="Copy ID" onclick="copyId('${a.id}')">⧉</button>
        <button title="Interrupt" onclick="rowAct('${a.id}','interrupt')">⎋</button>
        <button title="Stop" onclick="rowAct('${a.id}','stop')">■</button>
        <button title="Remove" onclick="removeAgent('${a.id}')">✕</button>
      </td>
    </tr>`;
  }).join("");
}

// Re-render the current fleet with the active filters (no refetch).
function applyFilter() { render(lastAgents); }

// Pause/resume the 2s live poll.
function schedulePoll() { pollTimer = setInterval(refresh, 2000); }
function togglePause() {
  paused = !paused;
  const btn = document.getElementById("pause-btn");
  if (paused) {
    if (pollTimer) { clearInterval(pollTimer); pollTimer = null; }
    btn.textContent = "▶ Resume";
    document.getElementById("updated").textContent = "paused";
  } else {
    btn.textContent = "⏸ Pause";
    refresh();
    schedulePoll();
  }
}

function select(id) {
  selected = id;
  lastDiff = null;
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
  badge.textContent = statusLabel(st) || "?";
  badge.className = "status s-" + (st || "");
  const ws = lastWorkspace[selected] || "";
  const wsEl = document.getElementById("d-workspace");
  wsEl.textContent = shortPath(ws);   // trim from the front; full path in the tooltip
  wsEl.title = ws;
  const upd = lastUpdated[selected];
  document.getElementById("d-activity").textContent = upd ? "active " + timeAgo(upd) : "";
  // Offer Resume on a finished agent; Interrupt/Stop only while it's active.
  const terminal = TERMINAL.has(st);
  document.getElementById("d-resume").hidden = !terminal;
  document.getElementById("d-interrupt").hidden = terminal;
  document.getElementById("d-stop").hidden = terminal;
  // Keep the Diff tab current too (cheap: only re-renders on change).
  if (activeTab === "diff") { loadDiff(); return; }
  // Only the Logs tab pulls logs; the terminal manages itself.
  if (activeTab !== "logs") return;
  try {
    const res = await api("/agents/" + selected + "/logs");
    if (res.ok) {
      document.getElementById("d-logs").textContent = (await res.json()).logs || "(no output)";
    } else {
      document.getElementById("d-logs").textContent = "(logs unavailable — session may have ended)";
    }
  } catch (e) { /* leave previous logs */ }
}

// Load the diff into the shared pane. Called on entering the Diff tab and on
// each poll while it's open; only re-renders when the diff actually changed, so
// the scroll position is preserved between unchanged refreshes.
let lastDiff = null;
async function loadDiff() {
  if (!selected) return;
  const pane = document.getElementById("d-logs");
  if (lastDiff === null) pane.innerHTML = '<span class="spinner"></span> loading diff…';
  try {
    const res = await api("/agents/" + selected + "/diff");
    const body = await res.json();
    if (!res.ok) { pane.textContent = body.error || "diff failed"; lastDiff = null; return; }
    const diff = (body.diff || "").replace(/\s+$/, "");
    if (diff !== lastDiff) {
      lastDiff = diff;
      pane.innerHTML = diff ? renderDiff(diff) : "(no changes)";
    }
  } catch (e) { if (lastDiff === null) pane.textContent = "diff failed"; }
}

async function sendReply() {
  if (!selected) return;
  const input = document.getElementById("d-reply");
  const text = input.value;
  if (!text) return;
  noteBusy("sending…");
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
  noteBusy(path + "…");
  try {
    const res = await api("/agents/" + selected + "/" + path, { method: "POST" });
    note(res.ok ? (path + " sent") : ((await res.json()).error || (path + " failed")));
  } catch (e) { note(path + " failed"); }
}

function toggleNew() {
  const m = document.getElementById("newmodal");
  if (typeof m.showModal === "function") m.showModal(); else m.setAttribute("open", "");
  document.getElementById("n-ws").focus();
}
function closeNew() {
  const m = document.getElementById("newmodal");
  if (typeof m.close === "function") m.close(); else m.removeAttribute("open");
}

async function createAgent(ev) {
  ev.preventDefault();
  const workspace = document.getElementById("n-ws").value.trim();
  if (!workspace) return;   // required; native validation also guards this
  const body = {
    agent_type: document.getElementById("n-type").value,
    workspace,
    prompt: document.getElementById("n-prompt").value || null,
    isolate: document.getElementById("n-isolate").checked,
    auto_start: true,
  };
  const model = document.getElementById("n-model").value.trim();
  if (model) body.model = model;
  try {
    const res = await api("/agents", {
      method: "POST", headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    if (res.ok) { ev.target.reset(); closeNew(); refresh(); }
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
schedulePoll();
