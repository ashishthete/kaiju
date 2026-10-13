#!/usr/bin/env bash
#
# API contract smoke test.
#
# Verifies the daemon's HTTP behavior end to end WITHOUT launching real agent
# CLIs (no tmux, no API keys). Starts a throwaway daemon on a test port with a
# temp state file, runs assertions with curl, and tears everything down.
#
# Usage:  ./scripts/smoke.sh        (run from the repo root)
#
set -euo pipefail

# Deliberately not KAIJU_PORT: that is the real daemon's port.
PORT="${KAIJU_SMOKE_PORT:-7811}"
URL="http://127.0.0.1:${PORT}"
TMP="$(mktemp -d)"
STATE="$TMP/state.json"
LOG="$TMP/daemon.log"
PASS=0
FAIL=0

cleanup() {
  [[ -n "${DAEMON_PID:-}" ]] && kill "$DAEMON_PID" 2>/dev/null || true
  rm -rf "$TMP"
}
trap cleanup EXIT

# expect_status METHOD PATH EXPECTED_CODE [JSON_BODY]
expect_status() {
  local method="$1" path="$2" want="$3" body="${4:-}" code
  if [[ -n "$body" ]]; then
    code=$(curl -s -o /dev/null -w '%{http_code}' -X "$method" "$URL$path" \
      -H 'content-type: application/json' -d "$body")
  else
    code=$(curl -s -o /dev/null -w '%{http_code}' -X "$method" "$URL$path")
  fi
  printf '%-50s' "$method $path -> $want"
  if [[ "$code" == "$want" ]]; then
    echo "PASS"; PASS=$((PASS + 1))
  else
    echo "FAIL (got $code)"; FAIL=$((FAIL + 1))
  fi
}

echo "Starting daemon on :$PORT ..."
if curl -sf "$URL/health" >/dev/null 2>&1; then
  echo "something already answers on :$PORT — refusing to test against it" >&2
  exit 1
fi
# Point every store at the temp dir so the real ~/.kaiju is never touched.
KAIJU_PORT="$PORT" KAIJU_STATE="$STATE" KAIJU_TASKS="$TMP/tasks.json" \
  KAIJU_WORKTREES="$TMP/worktrees" KAIJU_CONFIG="$TMP/config.json" \
  KAIJU_DEVICES="$TMP/devices.json" KAIJU_LOGS="$TMP/logs" cargo run -q -p kaiju-daemon >"$LOG" 2>&1 &
DAEMON_PID=$!
for _ in $(seq 1 60); do
  curl -sf "$URL/health" >/dev/null 2>&1 && break || sleep 1
done

expect_status GET    /health                       200
expect_status GET    /agents                       200
expect_status POST   /agents                       201 '{"agent_type":"claude","workspace":"/tmp","auto_start":false}'
expect_status POST   /agents                       201 '{"agent_type":"aider","workspace":"/tmp","auto_start":false}'
expect_status POST   /agents                       400 '{"agent_type":"  ","workspace":"/tmp","auto_start":false}'
expect_status GET    /agents/does-not-exist        404
expect_status POST   /agents/does-not-exist/input  404 '{"text":"hi"}'
expect_status DELETE /agents/does-not-exist        404

# Create -> stop -> input must be 409 (no tmux needed for any of these).
ID=$(curl -s -X POST "$URL/agents" -H 'content-type: application/json' \
  -d '{"agent_type":"claude","workspace":"/tmp","auto_start":false}' \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["id"])')
curl -s -o /dev/null -X POST "$URL/agents/$ID/stop"
expect_status POST   "/agents/$ID/input"           409 '{"text":"hi"}'
expect_status DELETE "/agents/$ID"                  204

echo
echo "Passed: $PASS   Failed: $FAIL"
[[ "$FAIL" -eq 0 ]]
