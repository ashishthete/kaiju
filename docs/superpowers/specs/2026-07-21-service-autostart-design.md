# Kaiju Service — Autostart Daemon and Pet at Login

**Date:** 2026-07-21
**Status:** Design approved; ready to plan implementation
**Scope:** macOS only. Adds a `kaiju service` subcommand that registers the
daemon and the pet as launchd user agents.

## Why

Today nothing starts automatically. The daemon is foreground-only
(`crates/kaiju-daemon/src/main.rs`) with no lifecycle CLI, no pidfile, and no
self-daemonize — it is backgrounded by hand with `&`. The pet is launched
ad hoc, three different ways: `kaiju pet` (which spawns the *legacy* egui
binary), the dashboard's `POST /pet/launch`, or `cargo run -p kaiju-pet-shell`.
The only install path is `make install`, which installs the CLI and nothing
else.

The goal: log in, and both the daemon and the pet are already running.

## The constraint that shapes the design

The daemon spawns helper binaries **by bare name** — `tmux`
(`crates/kaiju-daemon/src/tmux.rs`), `sh` (`batch.rs:34`, `judge.rs:153`), and
the agent CLIs `claude` / `codex` / `gemini` (`judge.rs:122`).

launchd gives user agents a minimal `PATH` of
`/usr/bin:/bin:/usr/sbin:/sbin`. That excludes `/opt/homebrew/bin`, where
`tmux` and the agent CLIs live. A naive plist therefore produces a daemon that
**boots cleanly and then fails every agent spawn** — the worst kind of failure,
because the service reports healthy.

Resolution: `kaiju service install` snapshots the installing shell's `PATH` and
bakes it into the plist's `EnvironmentVariables`. The interactive `PATH` is
known to work, so we capture it rather than guess at homebrew prefixes.
Re-running `install` refreshes the snapshot.

## Decisions

| Question | Decision | Rationale |
|---|---|---|
| Registration mechanism | launchd LaunchAgents | `RunAtLoad`, `KeepAlive`, and log redirection — the last is required since the daemon only logs to stdout. Scriptable and uninstallable. |
| Binary location | Installed copies (`~/.cargo/bin`, then `~/.local/bin`) | Survives `cargo clean`, branch switches, and debug-only builds. Pointing at `target/release` would silently break login startup. |
| Which pet | `kaiju-pet-shell` only | All recent work targets it; it has the port-52789 singleton guard; it runs as an accessory app with no Dock icon. The legacy egui `kaiju-pet` is left alone. |
| Command surface | `kaiju service` subcommand in `kaiju-cli` | Testable Rust, ships with the binary the user already has, reuses existing path/config knowledge instead of duplicating it in shell. |
| Pet `KeepAlive` | Off | Quitting a desktop pet should mean quitting. Also avoids a launchd respawn-loop racing the singleton guard. |

## Architecture

New module `crates/kaiju-cli/src/service/`, split by responsibility:

| Module | Responsibility | Depends on |
|---|---|---|
| `plist.rs` | Pure: `ServiceSpec` → plist XML `String` | nothing (no I/O) |
| `launchctl.rs` | Shell out to `launchctl` bootstrap / bootout / kickstart / print | `std::process` |
| `mod.rs` | Orchestrate install / uninstall / status / restart | the other two |

The split exists so plist generation is unit-testable without touching the
filesystem or launchd. That is the only part with logic worth testing; the rest
is orchestration and process invocation.

`ServiceSpec` is a plain struct — label, program path, `run_at_load`,
`keep_alive`, optional stdout/stderr paths, and an environment map. Both agents
are described by the same type; they differ only in field values.

## The two agents

Labels `dev.kaiju.daemon` and `dev.kaiju.pet`, written to
`~/Library/LaunchAgents/`.

```
dev.kaiju.daemon.plist                dev.kaiju.pet.plist
  Program   <bin>/kaiju-daemon          Program   <bin>/kaiju-pet-shell
  RunAtLoad          true               RunAtLoad          true
  KeepAlive          true               KeepAlive          false
  StandardOutPath    ~/.kaiju/logs/daemon.out.log
  StandardErrorPath  ~/.kaiju/logs/daemon.err.log
  EnvironmentVariables:
    PATH      <snapshot from installing shell>
    KAIJU_PORT, KAIJU_HOST   (emitted only when set)
```

The daemon gets `KeepAlive` and log files: it only logs to stdout, so without
redirection launchd discards everything and a crash leaves no trace. The pet
gets neither.

`~/.kaiju/logs/` already exists as the daemon's log directory (`KAIJU_LOGS`);
`install` creates it if absent.

Note the pet reads its prefs from `~/.config/kaiju/pet.json`, a different root
from the daemon's `~/.kaiju`. This design does not change that.

## Commands

- **`install`** — resolve binary paths, snapshot `PATH`, create
  `~/.kaiju/logs/`, write both plists, then `launchctl bootstrap gui/$UID`.
  Idempotent: bootout-then-bootstrap, so re-running upgrades cleanly rather
  than erroring on an already-loaded label.
- **`uninstall`** — `launchctl bootout` both labels, delete both plists.
  Tolerates already-absent plists and already-unloaded labels.
- **`status`** — `launchctl print gui/$UID/<label>` per agent; prints
  `running pid N`, `stopped`, or `not installed`.
- **`restart [daemon|pet]`** — `launchctl kickstart -k`. With no argument,
  restarts both.

`install` does **not** build. It verifies each binary exists at the resolved
path and, if not, errors with the exact `cargo install --path crates/<crate>`
command to run. Building is not the service command's responsibility, and a
multi-minute silent `cargo install` inside `service install` is a bad surprise.

Binary resolution order: `~/.cargo/bin/<name>`, then `~/.local/bin/<name>`.
First hit wins. If neither exists, that is a `BinaryNotFound` error carrying
both the searched paths and the install hint.

## Error handling

A `thiserror` enum in `service/mod.rs`:

- `BinaryNotFound { name, searched: Vec<PathBuf>, hint: String }`
- `LaunchctlFailed { command: String, code: i32, stderr: String }`
- `PlistWrite { path: PathBuf, source: io::Error }`
- `LogDirCreate { path: PathBuf, source: io::Error }`
- `NotMacos`

No `unwrap` / `expect` / `panic!` outside tests.

On non-macOS targets the subcommand still compiles and parses, returning
`NotMacos` at execution. It is not `#[cfg]`-ed out, so the CLI surface and its
`--help` stay uniform across platforms.

Partial-failure policy for `install`: if the daemon plist writes but the pet
plist fails, report the failure and leave the daemon registered. Do not roll
back — a half-installed service the user is told about is better than a silent
revert, and `uninstall` cleans up either state.

## Testing

Unit tests on `plist.rs`, which is pure and needs no fixtures:

- generated XML parses and contains the expected keys
- `PATH` from the spec's environment map is present in the output
- `KeepAlive` is `true` for the daemon spec and `false` for the pet spec
- log paths appear for the daemon spec and are absent for the pet spec
- paths containing spaces are XML-escaped correctly

Unit tests on binary resolution, using a `resolve_in(dirs: &[PathBuf], name)`
helper that takes its search roots as an argument so tests can point it at a
tempdir instead of `$HOME`.

`launchctl.rs` is a thin process wrapper with no branching logic; it is
verified by the manual check below rather than mocked.

Manual verification: `kaiju service install` → `kaiju service status` shows both
running → log out and back in → both running again → `kaiju service uninstall`
→ neither present and both plists gone.

## Out of scope

- A `.app` bundle for the pet (`tauri.conf.json` keeps `bundle.active = false`).
- System Settings login-item registration via `SMAppService`.
- Repointing the `kaiju pet` subcommand at `kaiju-pet-shell`. The divergence
  with the dashboard's `POST /pet/launch` is real and worth fixing, but it is a
  separate change from autostart.
- Linux systemd user units. The module boundary leaves room for it later; this
  spec does not build it.
- A daemon lifecycle CLI (`kaiju daemon start/stop`). launchd owns lifecycle
  here; adding a parallel mechanism would give two sources of truth.
