// Unit tests for the dashboard's pure helpers. Run with:
//   node --test crates/kaiju-daemon/assets/
const { test } = require("node:test");
const assert = require("node:assert/strict");
const { ORDER, ATTENTION, TERMINAL, fmtDuration, esc, renderDiff, shortPath } = require("./dashboard-utils.js");

test("fmtDuration formats seconds, minutes, and hours", () => {
  assert.equal(fmtDuration(0), "0s");
  assert.equal(fmtDuration(45), "45s");
  assert.equal(fmtDuration(125), "2m 5s");
  assert.equal(fmtDuration(3661), "1h 1m");
});

test("esc escapes HTML-significant characters", () => {
  assert.equal(esc("<b>&'</b>"), "&lt;b&gt;&amp;'&lt;/b&gt;");
  assert.equal(esc(null), "");
  assert.equal(esc(undefined), "");
  assert.equal(esc("plain"), "plain");
});

test("ORDER ranks attention statuses ahead of finished ones", () => {
  assert.ok(ORDER.waitingforinput < ORDER.running);
  assert.ok(ORDER.stuck < ORDER.completed);
  assert.ok(ORDER.running < ORDER.stopped);
});

test("ATTENTION and TERMINAL classify statuses", () => {
  assert.ok(ATTENTION.has("waitingforinput"));
  assert.ok(ATTENTION.has("stuck"));
  assert.ok(!ATTENTION.has("running"));
  assert.ok(TERMINAL.has("stopped") && TERMINAL.has("completed") && TERMINAL.has("error"));
  assert.ok(!TERMINAL.has("running"));
});

test("renderDiff colorizes lines and escapes their content", () => {
  const out = renderDiff("+added\n-removed\n@@ hunk @@\n context\ndiff --git a b");
  assert.match(out, /<span class="d-add">\+added<\/span>/);
  assert.match(out, /<span class="d-del">-removed<\/span>/);
  assert.match(out, /<span class="d-hunk">@@ hunk @@<\/span>/);
  assert.match(out, /<span class="d-file">diff --git a b<\/span>/);
  // A context line is left plain.
  assert.ok(out.includes("\n context\n"));
});

test("renderDiff does not mistake +++/--- file markers for add/del lines", () => {
  const out = renderDiff("+++ b/file\n--- a/file");
  assert.match(out, /<span class="d-file">\+\+\+ b\/file<\/span>/);
  assert.match(out, /<span class="d-file">--- a\/file<\/span>/);
});

test("shortPath trims from the front, keeping whole trailing segments", () => {
  const p = "/Users/a/work/projects/asrs/credibl-esg/webapp/esg";
  const out = shortPath(p, 44);
  assert.ok(out.startsWith("…/"), out);
  assert.ok(out.endsWith("/webapp/esg"), out);
  assert.ok(out.length <= 44, "length " + out.length);
});

test("shortPath leaves short paths untouched and handles empties", () => {
  assert.equal(shortPath("/a/b", 44), "/a/b");
  assert.equal(shortPath("", 44), "");
  assert.equal(shortPath(null, 44), "");
});

test("shortPath always keeps at least the last segment", () => {
  const out = shortPath("/very/deep/" + "x".repeat(80), 20);
  assert.ok(out.includes("x".repeat(80)), "keeps the final segment whole");
});

test("renderDiff escapes HTML in diff content", () => {
  const out = renderDiff("+<script>x</script>");
  assert.ok(out.includes("&lt;script&gt;"));
  assert.ok(!out.includes("<script>x"));
});
