# Pet: minimize render area, hug indicators, fix click capture

Date: 2026-07-30
Status: approved (design discussed in session; user emphasized the unread badge must sit close to the blob)

## Problem

Three symptoms, one root cause — the collapsed pet panel is much larger than the
visible blob, and the click-capture region matches neither the blob nor the
interactive elements:

1. **Render area too big.** The collapsed panel is a 188×188 pt transparent
   square; the blob renders 174×174 px inside it. Attention mode grows to
   380×270, expanded to 420×540.
2. **Clicks eaten near the pet.** `update_click_through` (main.rs) captures the
   cursor inside an ellipse of radii 0.44w/0.47h centred at (0.50, 0.55) —
   ~88%×94% of the panel, far larger than the visible blob. In attention mode
   the whole 380×270 surface captures everything (`AT_REST == false`).
3. **Dialog does not open.** The unread badge (`.status-dot`, `right:2px;
   top:6px` — the panel's top-right corner) is the *only* clickable opener of
   the expanded dialog while collapsed (`.state-label` is `pointer-events:none`).
   The badge lies **outside** the capture ellipse: at its centre
   (lx≈0.92, ly≈0.10), (ex²+ey²)≈1.8 > 1, so the ticker enables
   click-through and badge clicks fall to the window behind. Regression
   introduced by commit 3bc5796.

Additionally the badge and pill float far from the blob (screenshots from
user), looking disconnected.

## Design

### 1. Smaller panel, smaller blob

`set_panel_mode` sizes (logical pt):

| mode      | before   | after                   |
|-----------|----------|-------------------------|
| collapsed | 188×188  | **132×132**             |
| attention | 380×270  | **330×210**             |
| expanded  | 420×540  | unchanged (420×540)     |

`.pet-wrap` shrinks 174×174 → **104×104** (anchored bottom-right as today).
Constants elsewhere derived from the old 188/174 sizes (`restore_position`'s
208.0 offsets and visibility margins, the expanded `.bubble` bottom inset of
128px reserving pet space, attention banner width 360) are re-derived from the
new sizes during implementation — grep for 188/174/208/360/128.

### 2. Indicators hug the blob

Anchor both indicators to `.pet-wrap` (not `#app`), so they stay glued to the
blob at any panel size:

- `.status-dot` (unread badge): overlaps the blob's top-right shoulder
  (approximately `top:-2px; right:-2px` of the pet box). Stays clickable;
  still opens the dialog.
- `.state-label` (status pill): centred directly under the blob, overlapping
  its drop shadow (`bottom:-4px`, horizontally centred). Stays
  `pointer-events:none`.

### 3. Mode-aware click capture

Replace the `AT_REST: AtomicBool` with an atomic panel-mode value
(`AtomicU8`: collapsed / attention / expanded) written by `set_panel_mode`.
`update_click_through` captures per mode:

- **collapsed** — a single ellipse sized to hug blob + badge + pill (all three
  now overlap the blob, so one ellipse covers them with minimal dead margin).
- **attention** — banner rect ∪ blob ellipse; the rest of the 330×210 panel
  passes clicks through (today: captures everything).
- **expanded** — capture everywhere (interactive UI; unchanged), and
  `set_panel_mode` still disables click-through immediately on expand.

The geometry test moves into a pure function
`capture_region_contains(mode, lx, ly) -> bool` (normalized 0..1 panel
coordinates) so it is unit-testable without a window.

### 4. Tests (the drift guard)

Rust unit tests assert, using the same normalized constants that mirror the
CSS layout:

- badge centre and its four corners are captured in collapsed mode;
- pill centre is captured in collapsed mode;
- panel corners (0.02,0.02)…(0.98,0.98) are NOT captured in collapsed mode;
- attention banner centre is captured, attention panel bottom-left corner is
  not;
- expanded captures everything.

The CSS constants are duplicated as named Rust constants with a comment
pointing at `ui/index.html`; the tests exist precisely because the two can
drift.

## Error handling

No new failure paths: `set_panel_mode` keeps its existing error returns; the
ticker keeps ignoring windows/geometry it cannot read.

## Out of scope

- Expanded dialog size/content changes (no complaint; only the bubble's
  reserved pet inset is re-derived).
- User-configurable pet size (YAGNI).
- Webview-reported hit regions over IPC (rejected: extra IPC + state sync for
  no gain at this scale).
