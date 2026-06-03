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
<style>
  :root { color-scheme: light dark; }
  body { font-family: system-ui, sans-serif; margin: 0; padding: 1.5rem; }
  h1 { font-size: 1.25rem; margin: 0 0 .25rem; }
  .sub { color: #888; font-size: .85rem; margin-bottom: 1rem; }
  .counts { display: flex; gap: .5rem; flex-wrap: wrap; margin-bottom: 1rem; }
  .pill { padding: .2rem .6rem; border-radius: 999px; font-size: .8rem; border: 1px solid #8884; }
  table { width: 100%; border-collapse: collapse; font-size: .9rem; }
  th, td { text-align: left; padding: .5rem .6rem; border-bottom: 1px solid #8883; }
  th { font-weight: 600; color: #888; font-size: .78rem; text-transform: uppercase; letter-spacing: .03em; }
  tbody tr { cursor: pointer; }
  tbody tr:hover { background: #8881; }
  tr.selected { outline: 2px solid #3b82f6aa; }
  td.id { font-family: ui-monospace, monospace; }
  .status { font-weight: 600; padding: .15rem .5rem; border-radius: 4px; font-size: .8rem; white-space: nowrap; }
  .s-waitingforinput { background: #f59e0b22; color: #b45309; }
  .s-stuck, .s-error { background: #ef444422; color: #b91c1c; }
  .s-running { background: #22c55e22; color: #15803d; }
  .s-starting { background: #3b82f622; color: #1d4ed8; }
  .s-completed, .s-stopped { background: #88888822; color: #6b7280; }
  .prompt { color: #888; max-width: 24rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .empty { color: #888; padding: 2rem 0; }
  .attention td { background: #f59e0b0d; }

  #detail { margin-top: 1.5rem; border: 1px solid #8884; border-radius: 8px; padding: 1rem; }
  #detail[hidden] { display: none; }
  .detail-head { display: flex; align-items: center; gap: .75rem; margin-bottom: .75rem; }
  .detail-head .id { font-family: ui-monospace, monospace; font-weight: 600; }
  .grow { flex: 1; }
  button { font: inherit; padding: .35rem .7rem; border-radius: 5px; border: 1px solid #8886; background: #8881; cursor: pointer; }
  button:hover { background: #8883; }
  pre.logs { background: #0001; padding: .75rem; border-radius: 6px; max-height: 22rem; overflow: auto; font-size: .82rem; white-space: pre-wrap; word-break: break-word; }
  .reply { display: flex; gap: .5rem; margin-top: .75rem; }
  .reply input { flex: 1; font: inherit; padding: .4rem .6rem; border-radius: 5px; border: 1px solid #8886; background: transparent; }
  .note { color: #888; font-size: .8rem; margin-top: .4rem; min-height: 1rem; }
</style>
</head>
<body>
  <h1>Kaiju</h1>
  <div class="sub">Live fleet &middot; refreshing every 2s &middot; <span id="updated"></span></div>
  <div class="counts" id="counts"></div>
  <table>
    <thead><tr>
      <th>ID</th><th>Type</th><th>Model</th><th>Status</th>
      <th>Runtime</th><th>Tokens</th><th>Cost</th><th>Task</th>
    </tr></thead>
    <tbody id="rows"></tbody>
  </table>
  <div class="empty" id="empty" hidden>No agents yet.</div>

  <div id="detail" hidden>
    <div class="detail-head">
      <span class="id" id="d-id"></span>
      <span class="status" id="d-status"></span>
      <span class="grow"></span>
      <button onclick="loadDiff()">Diff</button>
      <button onclick="act('interrupt')">Interrupt</button>
      <button onclick="act('stop')">Stop</button>
      <button onclick="closeDetail()">Close</button>
    </div>
    <pre class="logs" id="d-logs">Loading…</pre>
    <div class="reply">
      <input id="d-reply" placeholder="Reply or approve (Enter to send)…" onkeydown="if(event.key==='Enter')sendReply()">
      <button onclick="sendReply()">Send</button>
    </div>
    <div class="note" id="d-note"></div>
  </div>

<script>
const ORDER = { waitingforinput: 0, stuck: 1, error: 2, starting: 3, running: 4, completed: 5, stopped: 6 };
let selected = null;
let lastStatus = {};
let token = localStorage.getItem("kaiju_token") || "";

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
  lastStatus = {};
  for (const a of agents) lastStatus[a.id] = a.status;

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
      <td class="id">${a.id.slice(0,10)}</td>
      <td>${esc(a.agent_type)}</td>
      <td>${esc(a.model) || "-"}</td>
      <td><span class="status s-${a.status}">${esc(a.status)}</span></td>
      <td>${fmtDuration(m.runtime_secs || 0)}</td>
      <td>${toks}</td>
      <td>${cost}</td>
      <td class="prompt">${esc(a.prompt) || "-"}</td>
    </tr>`;
  }).join("");
}

function select(id) {
  selected = id;
  document.getElementById("detail").hidden = false;
  document.getElementById("d-id").textContent = id.slice(0, 10);
  document.getElementById("d-logs").textContent = "Loading…";
  note("");
  refreshDetail();
}
function closeDetail() {
  selected = null;
  document.getElementById("detail").hidden = true;
}

async function refreshDetail() {
  if (!selected) return;
  const st = lastStatus[selected];
  const badge = document.getElementById("d-status");
  badge.textContent = st || "?";
  badge.className = "status s-" + (st || "");
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

refresh();
setInterval(refresh, 2000);
</script>
</body>
</html>"#;
