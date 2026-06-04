#!/usr/bin/env bash
#
# Full-pipeline end-to-end test using a FAKE `claude` binary, so no API keys or
# real agent CLI are needed. Exercises the whole control plane:
#
#   interactive launch -> status transitions -> waiting-for-input detection ->
#   operator alert -> send-input -> completion -> git-worktree isolation ->
#   persistence across a daemon restart -> worktree cleanup on remove.
#
# Requires: tmux, git, and a built workspace. For speed, install the CLI first:
#   cargo install --path crates/kaiju-cli
#
# Usage:  ./scripts/e2e.sh           (run from the repo root)
#
set -euo pipefail

PORT="${KAIJU_PORT:-7812}"
URL="http://127.0.0.1:${PORT}"
ROOT="$(mktemp -d)"
BIN="$ROOT/bin"
WS="$ROOT/repo"
STATE="$ROOT/state.json"
WT="$ROOT/worktrees"
LOG="$ROOT/daemon.log"
mkdir -p "$BIN" "$WS"

cleanup() {
  [[ -n "${DAEMON_PID:-}" ]] && kill "$DAEMON_PID" 2>/dev/null || true
  rm -rf "$ROOT"
}
trap cleanup EXIT

# Use the installed CLI if present (fast), else fall back to cargo run.
cli() {
  if command -v kaiju >/dev/null 2>&1; then
    kaiju --url "$URL" "$@"
  else
    cargo run -q -p kaiju-cli -- --url "$URL" "$@"
  fi
}

start_daemon() {
  # KAIJU_CLAUDE_BIN pins the fake agent by absolute path. PATH alone is not
  # enough: tmux spawns a login shell that re-sources the user's profile and can
  # reorder PATH ahead of our temp dir, launching the real `claude` instead.
  KAIJU_PORT="$PORT" KAIJU_STATE="$STATE" KAIJU_WORKTREES="$WT" \
    KAIJU_CLAUDE_BIN="$BIN/claude" \
    cargo run -q -p kaiju-daemon >>"$LOG" 2>&1 &
  DAEMON_PID=$!
  for _ in $(seq 1 60); do
    curl -sf "$URL/health" >/dev/null 2>&1 && return 0 || sleep 1
  done
  echo "daemon did not come up"; cat "$LOG"; exit 1
}

wait_status() { # wait_status <substring> <timeout-seconds>
  local want="$1" timeout="${2:-25}"
  for _ in $(seq 1 "$timeout"); do
    cli status "$ID" 2>/dev/null | grep -qi "$want" && return 0
    sleep 1
  done
  echo "TIMEOUT waiting for status '$want'; last logs:"; cli logs "$ID" || true
  exit 1
}

# --- fake interactive agent placed first on PATH ---
cat > "$BIN/claude" <<'SH'
#!/usr/bin/env bash
# Ignores --model and the positional prompt; simulates an interactive agent.
echo "Working on the task..."
sleep 0.5
echo "Edited 1 file"
echo "Do you want to apply these changes?"
echo "❯ 1. Yes"
echo "  2. No"
while read -r line; do
  case "$line" in
    *yes*|1) echo "Applying..."; echo "Task completed successfully."; break ;;
    *)       echo "Please answer." ;;
  esac
done
sleep 3
SH
chmod +x "$BIN/claude"
export PATH="$BIN:$PATH"

# --- git workspace so isolation has something to branch from ---
git -C "$WS" init -q
git -C "$WS" config user.email t@e.com
git -C "$WS" config user.name t
echo hi > "$WS/README.md"
git -C "$WS" add -A
git -C "$WS" commit -qm init

echo "Starting daemon ..."
start_daemon

echo "1) start an isolated agent"
ID=$(cli start --agent-type claude --workspace "$WS" --isolate --prompt "do it" \
      | awk '/ID:/ {print $2; exit}')
echo "   id=$ID"
[[ -n "$ID" ]] || { echo "no id parsed"; exit 1; }

echo "2) status reaches waiting-for-input (question above ❯ menu)"
wait_status waitingforinput 25
echo "   OK"

echo "3) git worktree was created"
git -C "$WS" worktree list | grep -q "kaiju/" && echo "   OK" || { echo "   MISSING"; exit 1; }

echo "4) operator was alerted"
grep -q "waiting for your input" "$LOG" && echo "   OK" || echo "   WARNING: alert not found in log"

echo "5) reply, expect completion"
cli send "$ID" "yes"
wait_status completed 25
echo "   OK"

echo "6) state survives a daemon restart"
kill "$DAEMON_PID"; sleep 1
start_daemon
cli list | grep -q "${ID:0:10}" && echo "   OK" || { echo "   state LOST"; exit 1; }

echo "7) remove cleans up the worktree"
cli remove "$ID"
if git -C "$WS" worktree list | grep -q "kaiju/"; then
  echo "   worktree NOT cleaned"; exit 1
else
  echo "   OK"
fi

echo
echo "E2E PASSED"
