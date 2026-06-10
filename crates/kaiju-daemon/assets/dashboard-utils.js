// Pure helpers for the Kaiju dashboard — no DOM or network access, so they can
// be unit-tested under node (see dashboard-utils.test.js). In the browser this
// loads as a plain script before dashboard.js, so these become globals; in
// tests it's required as a CommonJS module.

// Sort weight per status (lower sorts first — most urgent at the top).
const ORDER = { waitingforinput: 0, stuck: 1, error: 2, starting: 3, running: 4, completed: 5, stopped: 6 };

// Statuses that mean an agent needs the operator.
const ATTENTION = new Set(["waitingforinput", "stuck"]);

// Statuses an agent can't act from (offer Resume, hide Interrupt/Stop).
const TERMINAL = new Set(["stopped", "completed", "error"]);

// Human-readable duration from seconds: "45s", "3m 5s", "2h 10m".
function fmtDuration(s) {
  if (s < 60) return s + "s";
  if (s < 3600) return Math.floor(s / 60) + "m " + (s % 60) + "s";
  return Math.floor(s / 3600) + "h " + Math.floor((s % 3600) / 60) + "m";
}

// Escape HTML-significant characters for safe interpolation into innerHTML.
function esc(s) {
  return (s || "").replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

// Colorize a unified diff (added/removed/hunk/file lines) as HTML spans.
function renderDiff(diff) {
  return diff.split("\n").map((line) => {
    const text = esc(line);
    if (line.startsWith("+") && !line.startsWith("+++")) return '<span class="d-add">' + text + "</span>";
    if (line.startsWith("-") && !line.startsWith("---")) return '<span class="d-del">' + text + "</span>";
    if (line.startsWith("@@")) return '<span class="d-hunk">' + text + "</span>";
    if (line.startsWith("diff ") || line.startsWith("+++") || line.startsWith("---") ||
        line.startsWith("index ") || line.startsWith("?? ") || line.startsWith('#'))
      return '<span class="d-file">' + text + "</span>";
    return text;
  }).join("\n");
}

// Trim a path from the FRONT so the meaningful tail stays visible, e.g.
// "/Users/a/work/projects/x/webapp/esg" -> "…/x/webapp/esg". Keeps whole path
// segments and never drops below the last one.
function shortPath(path, max) {
  max = max || 44;
  if (!path || path.length <= max) return path || "";
  const parts = path.split("/").filter(Boolean);
  let acc = "";
  for (let i = parts.length - 1; i >= 0; i--) {
    const next = "/" + parts[i] + acc;
    if (acc && ("…" + next).length > max) break;
    acc = next;
  }
  return "…" + acc;
}

// Export for node tests; the guard is a no-op in the browser (module undefined).
if (typeof module !== "undefined" && module.exports) {
  module.exports = { ORDER, ATTENTION, TERMINAL, fmtDuration, esc, renderDiff, shortPath };
}
