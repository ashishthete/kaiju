# Pet Minimize Render Area Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Shrink the kaiju-pet collapsed panel from 188pt to 132pt, glue the unread badge and status pill to the blob, and make click capture mode-aware — fixing the bug where clicking the unread badge (the only opener of the dialog) falls through to the window behind.

**Architecture:** All changes live in `crates/kaiju-pet-shell`. A new pure function `capture_region_contains(mode, x, y)` in `src/main.rs` decides click capture from panel-local logical points, replacing the normalized ellipse in `update_click_through`; a `PanelMode` atomic replaces the `AT_REST` bool. `ui/index.html` CSS shrinks the pet box and anchors indicators to it. Rust unit tests pin the CSS-derived hit coordinates so layout and hit-testing can't drift apart silently.

**Tech Stack:** Rust (Tauri v2 + tauri-nspanel, macOS-only module), single-file HTML/CSS webview. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-07-30-pet-minimize-render-area-design.md`

## Global Constraints

- Panel sizes (logical pt): collapsed **132×132**, attention **330×216**, expanded **420×540** (unchanged).
- Pet box (`.pet-wrap`): **104×104**, anchored `right:7px; bottom:5px` (offsets unchanged).
- The `.state-label` pill stays `pointer-events:none` — it is deliberately NOT part of the capture region; only blob + badge (+ attention banner) capture clicks.
- Ship gate (see CLAUDE.md + memory): `cargo fmt` + clippy + tests MUST include `-p kaiju-pet-core -p kaiju-pet-shell` — a bare `cargo test` does not cover the pet crates.
- No `unwrap()`/`expect()`/`panic!()` outside tests.

## Layout math (reference for all tasks)

Collapsed panel 132×132; pet box spans x `21..125`, y `23..127` (132−7−104=21, 132−5−104=23).
Badge (`.status-dot`, 27×27 circle at `right:14%; top:12%` of the 104px pet box → offsets 14.56/12.48): centre **(96.94, 48.98)**, radius 13.5.
Pill (`.state-label`, ~25pt tall at `bottom:2px`, centred): centre **(73, 112.5)**.
Capture ellipse: centre = pet-box centre **(73, 75)**, radii **(48, 52)** — derived as `(panel.w − 7 − 52, panel.h − 5 − 52)`, which also gives the attention-mode pet centre **(271, 159)** for a 330×216 panel.
Attention banner (`left:8px; right:8px; top:10px; min-height:92px`): rect **(8, 10)–(322, 102)**.

---

### Task 1: Pure hit-region functions with tests

**Files:**
- Modify: `crates/kaiju-pet-shell/src/main.rs` (inside `mod macos`; add a `#[cfg(test)] mod tests` at the bottom of `mod macos`)

**Interfaces:**
- Produces: `enum PanelMode { Collapsed, Attention, Expanded }` with `fn parse(&str) -> Option<PanelMode>`, `fn size(self) -> (f64, f64)`, `fn as_u8(self) -> u8`, `fn from_u8(u8) -> PanelMode`; `fn capture_region_contains(mode: PanelMode, x: f64, y: f64) -> bool` (x, y = cursor in logical pt relative to the panel's top-left). Task 2 wires these into the window plumbing.

- [ ] **Step 1: Write the failing tests**

Append at the bottom of `mod macos` in `crates/kaiju-pet-shell/src/main.rs` (before the closing brace of the module):

```rust
    #[cfg(test)]
    mod tests {
        use super::*;

        // Badge circle from ui/index.html: .status-dot 27x27 at right:14%/top:12%
        // of the 104pt pet box, which spans (21..125, 23..127) in a 132pt panel.
        const BADGE_CENTER: (f64, f64) = (96.94, 48.98);
        const BADGE_R: f64 = 13.5;

        #[test]
        fn collapsed_captures_badge_center_and_rim() {
            let (cx, cy) = BADGE_CENTER;
            let points = [
                (cx, cy),
                (cx + BADGE_R, cy),
                (cx - BADGE_R, cy),
                (cx, cy + BADGE_R),
                (cx, cy - BADGE_R),
            ];
            for (x, y) in points {
                assert!(
                    capture_region_contains(PanelMode::Collapsed, x, y),
                    "badge point ({x}, {y}) must be clickable"
                );
            }
        }

        #[test]
        fn collapsed_captures_blob_and_pill_center() {
            assert!(capture_region_contains(PanelMode::Collapsed, 73.0, 75.0));
            // .state-label pill centre (visual overlap only; pill ignores pointers)
            assert!(capture_region_contains(PanelMode::Collapsed, 73.0, 112.5));
        }

        #[test]
        fn collapsed_passes_clicks_through_at_corners() {
            for (x, y) in [(2.0, 2.0), (130.0, 2.0), (2.0, 130.0), (130.0, 130.0)] {
                assert!(
                    !capture_region_contains(PanelMode::Collapsed, x, y),
                    "corner ({x}, {y}) must pass through"
                );
            }
        }

        #[test]
        fn attention_captures_banner_and_pet_only() {
            // Banner centre and pet centre capture…
            assert!(capture_region_contains(PanelMode::Attention, 165.0, 56.0));
            assert!(capture_region_contains(PanelMode::Attention, 271.0, 159.0));
            // …but the empty bottom-left of the 330x216 panel passes through.
            assert!(!capture_region_contains(PanelMode::Attention, 20.0, 200.0));
        }

        #[test]
        fn expanded_captures_everywhere() {
            assert!(capture_region_contains(PanelMode::Expanded, 0.0, 0.0));
            assert!(capture_region_contains(PanelMode::Expanded, 419.0, 539.0));
        }

        #[test]
        fn panel_mode_round_trips() {
            for mode in [PanelMode::Collapsed, PanelMode::Attention, PanelMode::Expanded] {
                assert_eq!(PanelMode::from_u8(mode.as_u8()), mode);
            }
            assert_eq!(PanelMode::parse("collapsed"), Some(PanelMode::Collapsed));
            assert_eq!(PanelMode::parse("attention"), Some(PanelMode::Attention));
            assert_eq!(PanelMode::parse("expanded"), Some(PanelMode::Expanded));
            assert_eq!(PanelMode::parse("bogus"), None);
        }
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kaiju-pet-shell`
Expected: COMPILE ERROR — `PanelMode` and `capture_region_contains` not found.

- [ ] **Step 3: Implement the types and region function**

In `crates/kaiju-pet-shell/src/main.rs`, directly below the `IGNORING_CURSOR` static (line ~28), add:

```rust
    /// Current panel mode, written by `set_panel_mode` and read by the
    /// click-through ticker and the position-persistence handler.
    static PANEL_MODE: std::sync::atomic::AtomicU8 =
        std::sync::atomic::AtomicU8::new(0); // PanelMode::Collapsed

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum PanelMode {
        Collapsed,
        Attention,
        Expanded,
    }

    impl PanelMode {
        fn parse(mode: &str) -> Option<Self> {
            match mode {
                "collapsed" => Some(Self::Collapsed),
                "attention" => Some(Self::Attention),
                "expanded" => Some(Self::Expanded),
                _ => None,
            }
        }

        /// Panel size per mode in logical pt.
        fn size(self) -> (f64, f64) {
            match self {
                Self::Collapsed => (132.0, 132.0),
                Self::Attention => (330.0, 216.0),
                Self::Expanded => (420.0, 540.0),
            }
        }

        fn as_u8(self) -> u8 {
            match self {
                Self::Collapsed => 0,
                Self::Attention => 1,
                Self::Expanded => 2,
            }
        }

        fn from_u8(value: u8) -> Self {
            match value {
                1 => Self::Attention,
                2 => Self::Expanded,
                _ => Self::Collapsed,
            }
        }
    }

    fn current_mode() -> PanelMode {
        PanelMode::from_u8(PANEL_MODE.load(Ordering::SeqCst))
    }

    // --- Click-capture geometry -------------------------------------------
    // These constants mirror ui/index.html (.pet-wrap, .status-dot,
    // .attention). If that CSS layout moves, update these together with the
    // tests at the bottom of this module.

    /// Ellipse hugging the blob plus its badge and pill, in logical pt.
    /// The 104pt pet box sits at right:7 bottom:5 of the panel.
    const PET_ELLIPSE_RADII: (f64, f64) = (48.0, 52.0);

    /// Attention banner: left:8 right:8 top:10 min-height:92 in a 330pt panel.
    const ATTENTION_BANNER: (f64, f64, f64, f64) = (8.0, 10.0, 322.0, 102.0);

    /// Centre of the pet box for a given panel size (right:7 bottom:5, 104pt).
    fn pet_ellipse_center(panel: (f64, f64)) -> (f64, f64) {
        (panel.0 - 7.0 - 52.0, panel.1 - 5.0 - 52.0)
    }

    fn inside_pet_ellipse(panel: (f64, f64), x: f64, y: f64) -> bool {
        let (cx, cy) = pet_ellipse_center(panel);
        let ex = (x - cx) / PET_ELLIPSE_RADII.0;
        let ey = (y - cy) / PET_ELLIPSE_RADII.1;
        ex * ex + ey * ey <= 1.0
    }

    /// Whether a cursor at (x, y) logical pt from the panel's top-left is over
    /// interactive UI for the given mode — i.e. the panel must capture clicks.
    fn capture_region_contains(mode: PanelMode, x: f64, y: f64) -> bool {
        match mode {
            PanelMode::Expanded => true,
            PanelMode::Collapsed => inside_pet_ellipse(mode.size(), x, y),
            PanelMode::Attention => {
                let (x0, y0, x1, y1) = ATTENTION_BANNER;
                let over_banner = x >= x0 && x <= x1 && y >= y0 && y <= y1;
                over_banner || inside_pet_ellipse(mode.size(), x, y)
            }
        }
    }
```

Note: `Ordering` is already imported at the top of the module (`use std::sync::atomic::{AtomicBool, Ordering};`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kaiju-pet-shell`
Expected: PASS (6 tests). Dead-code warnings for the not-yet-wired functions are acceptable until Task 2.

- [ ] **Step 5: Commit**

```bash
git add crates/kaiju-pet-shell/src/main.rs
git commit -m "feat(pet-shell): mode-aware click-capture geometry with layout-drift tests"
```

---

### Task 2: Wire PanelMode through the window plumbing

**Files:**
- Modify: `crates/kaiju-pet-shell/src/main.rs` — `set_panel_mode` (~line 133), `update_click_through` (~line 310), the cursor ticker (~line 372), the `WindowEvent::Moved` handler (~line 529), startup sizes (~lines 489 and 572), `restore_position` (~line 423). Delete the `AT_REST` static.

**Interfaces:**
- Consumes: `PanelMode`, `current_mode()`, `capture_region_contains` from Task 1.
- Produces: no new interfaces; behavior change only. After this task `AT_REST` no longer exists.

- [ ] **Step 1: Replace the mode/size handling in `set_panel_mode`**

Replace the body's opening (the `let (width, height) = match … };` block, lines ~135-140) with:

```rust
        let panel_mode =
            PanelMode::parse(&mode).ok_or_else(|| "unknown panel mode".to_string())?;
        let (width, height) = panel_mode.size();
```

Replace the two `AT_REST`-related statements (lines ~151-160) with:

```rust
        // Persist the resting position only while collapsed (see the Moved
        // handler in setup).
        PANEL_MODE.store(panel_mode.as_u8(), Ordering::SeqCst);

        // An expanded panel is interactive UI everywhere, so it must capture
        // clicks at once rather than waiting for the click-through ticker's
        // next poll. Collapsed/attention capture is re-evaluated by the ticker
        // within one 33ms tick.
        if panel_mode == PanelMode::Expanded && IGNORING_CURSOR.swap(false, Ordering::SeqCst) {
            let _ = window.set_ignore_cursor_events(false);
        }
```

- [ ] **Step 2: Rewrite `update_click_through` on the new region function**

Replace the whole `update_click_through` function (doc comment included, lines ~302-334) with:

```rust
    /// Lets clicks fall through the panel's empty area to whatever is behind
    /// it, while keeping the interactive UI clickable: the blob + its unread
    /// badge while collapsed, plus the banner in attention mode. An expanded
    /// panel is interactive everywhere, so it always captures.
    fn update_click_through(
        window: &tauri::WebviewWindow,
        cursor: tauri::LogicalPosition<f64>,
        origin: tauri::LogicalPosition<f64>,
    ) {
        let mode = current_mode();
        let capture = capture_region_contains(mode, cursor.x - origin.x, cursor.y - origin.y);
        let want_ignore = !capture;

        // Only touch the window when the state flips — this runs at 30Hz.
        if IGNORING_CURSOR.swap(want_ignore, Ordering::SeqCst) != want_ignore {
            let _ = window.set_ignore_cursor_events(want_ignore);
        }
    }
```

In the cursor ticker (~line 372) drop the removed parameter:

```rust
            update_click_through(&window, cursor, origin);
```

(The ticker's `size` binding is still used for `center_x`/`center_y`/`span` — keep it.)

- [ ] **Step 3: Replace the remaining AT_REST reader and delete the static**

In the `WindowEvent::Moved` handler (~line 529), replace

```rust
                    if !AT_REST.load(Ordering::SeqCst) {
                        return;
                    }
```

with

```rust
                    if current_mode() != PanelMode::Collapsed {
                        return;
                    }
```

Delete the `AT_REST` static and its comment (lines ~21-24).

- [ ] **Step 4: Update the startup sizes and off-screen restore constants**

Both hard-coded `188.0` window sizes — the `PanelBuilder` `.size(...)` (~line 489) and the post-show reassert (~line 572) — become:

```rust
            let (width, height) = PanelMode::Collapsed.size();
```

with `Size::Logical(tauri::LogicalSize::new(width, height))` at both sites (the reassert site keeps its `set_position` and `eprintln!` lines unchanged).

In `restore_position` (~lines 433-448), the constants derived from the old 188pt panel shrink with it: the two `140.0` visibility margins (≥48pt of a 188pt panel had to remain on-screen) become `100.0` (≥32pt of 132), and the two `208.0` bottom-right fallback offsets (188 + 20 margin) become `152.0` (132 + 20).

Also fix the now-stale comment on the panel-size doc (~line 305 was deleted with the old function) and the ticker's doc comment (~line 339): change "the 188pt panel" to "the collapsed panel".

- [ ] **Step 5: Build, test, lint**

Run: `cargo test -p kaiju-pet-shell && cargo clippy -p kaiju-pet-shell -- -D warnings && cargo fmt --check`
Expected: tests PASS, no clippy warnings (the Task 1 dead-code warnings disappear now that everything is wired), fmt clean. If fmt fails, run `cargo fmt` and re-check.

- [ ] **Step 6: Commit**

```bash
git add crates/kaiju-pet-shell/src/main.rs
git commit -m "feat(pet-shell): shrink panel modes and drive click-through from PanelMode"
```

---

### Task 3: CSS — smaller pet box, indicators glued to the blob

**Files:**
- Modify: `crates/kaiju-pet-shell/ui/index.html` (CSS only; no markup or JS changes)

**Interfaces:**
- Consumes: the Task 1 geometry (badge at `right:14%; top:12%` of a 104px pet box) — these CSS values MUST match the Rust test constants.
- Produces: nothing consumed by other tasks.

- [ ] **Step 1: Apply the CSS edits**

All in the `<style>` block of `ui/index.html`:

1. `.pet-wrap`: `width: 174px; height: 174px;` → `width: 104px; height: 104px;`
2. `.status-dot`: `right:2px; top:6px;` → `right:14%; top:12%;` (matches the Rust badge constants — overlaps the blob's top-right shoulder)
3. `.state-label`: `left:33px; bottom:2px;` → `left:50%; bottom:2px; transform:translateX(-50%);` (centred under the blob, overlapping its shadow; keep `max-width:124px`, keep `pointer-events:none`)
4. `.attention`: `left:8px; top:10px; width:360px;` → `left:8px; right:8px; top:10px;` (fills the 330pt attention panel with 8px gutters)
5. `.attention span`: `max-width:290px;` → `max-width:240px;` (fits the narrower banner)
6. `.bubble`: `inset:8px 8px 128px 8px;` → `inset:8px 8px 116px 8px;` (the reserved pet strip shrinks with the pet: 104 + 5 bottom + 7 gap)
7. Scale the pixel-tuned FX to the 104px box (×0.6 of the old 174px values):
   - `.signal-ring`: `inset:27px;` → `inset:16px;`
   - `.scan-line`: `left:27px; right:27px; top:20px;` → `left:16px; right:16px; top:12px;`
   - `@keyframes event-scan`: `translateY(125px)` → `translateY(75px)`
   - `.spark`: `transform-origin:50% 74px;` → `transform-origin:50% 44px;`
   - `@keyframes event-spark`: `translateY(-74px)` → `translateY(-44px)`
   - `.lottie-pet::after` box-shadow: `104px 34px 0 -1px #fff, 52px 150px 0 -1px …` → `62px 20px 0 -1px #fff, 31px 90px 0 -1px …` (keep the color-mix expression as is)

(`.pet-speech` is already percentage-anchored — no change.)

- [ ] **Step 2: Visual + interaction verification**

The daemon is already running as a launchd service, so a dev pet can be run directly against it after killing the service copy:

```bash
launchctl bootout gui/$(id -u)/dev.kaiju.pet 2>/dev/null; pkill -x kaiju-pet-shell 2>/dev/null; cargo run -p kaiju-pet-shell
```

(`kaiju service` only offers install/uninstall/status/restart, so stop the pet agent via `launchctl bootout` directly; `kaiju service restart` later re-registers it.)

Verify, with a terminal or editor window positioned behind the pet:

1. Pet renders noticeably smaller (~104px blob in a 132pt panel); badge sits ON the blob's top-right shoulder; pill sits centred under the blob.
2. Clicking the unread badge opens the expanded dialog (THE regression fix — needs an agent with unread events; `make e2e` traffic or any waiting agent produces one, or verify at minimum that clicking the badge with `•` opens the bubble).
3. Clicks ~10px outside the blob's silhouette land on the window behind.
4. Poke (click blob), drag, right-click menu still work.
5. Close the bubble (×) → panel collapses back, click-through returns.

Then restore the service copy: quit the dev pet (right-click → Quit Kaiju or Ctrl-C) and run `kaiju service restart` (or `launchctl kickstart -k gui/$(id -u)/dev.kaiju.pet`).

- [ ] **Step 3: Commit**

```bash
git add crates/kaiju-pet-shell/ui/index.html
git commit -m "feat(pet-shell): shrink pet to 104px and glue badge/pill to the blob"
```

---

### Task 4: Ship gate and deploy

**Files:** none (verification + deploy only)

- [ ] **Step 1: Full gate (pet crates are NOT covered by a bare cargo test)**

```bash
make fmt-check && make lint && make test
```

Expected: all green. (The Makefile `lint` and `test` targets already append `-p kaiju-pet-core -p kaiju-pet-shell` — a bare `cargo test` would silently skip the pet crates.)

- [ ] **Step 2: Deploy the new pet to the launchd service**

launchd runs the installed binary, not the repo build:

```bash
cargo install --path crates/kaiju-pet-shell && kaiju service restart
kaiju service status
```

Expected: `pet running pid <new>`, and the smaller pet appears on screen.

- [ ] **Step 3: Merge housekeeping**

Work is on `main` directly (repo convention: small features committed to main). Verify `git log --oneline -4` shows the three feature commits + spec commit, and `git status` is clean.
