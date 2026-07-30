# Kaiju Pet native macOS shell

This is the Tauri/WebKit replacement for the eframe window in `kaiju-pet`.
The daemon client, reducer, validation, preferences, ANSI cleanup, and poller
live in the GUI-free `kaiju-pet-core` crate and are shared with the old egui
frontend.

## Run

Start the Kaiju daemon, then from the workspace root run:

```sh
cargo run -p kaiju-pet-shell
```

The shell defaults to `http://127.0.0.1:7800`. Override it with `KAIJU_URL`;
set `KAIJU_TOKEN` when the daemon requires a bearer token.

- Drag the character to move the panel; it reacts while dragging.
- Blob is the only character for now and changes motion for idle, working,
  waiting, stuck, limited, error, completed, and offline states.
- Click the character to poke it, double-click to spin it, or click rapidly to
  trigger an annoyed reaction.
- Click the status badge to open the agent bubble.
- Right-click for dashboard and Quit.
- Click the reply field for temporary keyboard focus. Send, Escape, or blur
  returns the panel to passive non-keyable mode.

## Programmatic character control

The daemon drives the pet automatically. The webview also exposes a small API
for previews and future integrations:

```js
KaijuPet.setStatus("working");       // also accepts running, waiting_for_input, completed, etc.
KaijuPet.react("celebrate");
KaijuPet.speak("Build completed!");  // speech bubble + system voice
KaijuPet.setMood("angry", 2000);
KaijuPet.clearMood();
KaijuPet.setCharacter("blob");
KaijuPet.getState();
KaijuPet.getCapabilities();
```

A status can also be sent as a DOM event:

```js
window.dispatchEvent(new CustomEvent("kaiju-pet:set-status", { detail: "limited" }));
```

## Why it floats over full-screen apps

`tauri-nspanel` converts the hidden Tauri `NSWindow` to a real `NSPanel` before
it is shown. The shell then applies these AppKit properties:

```text
NSApplicationActivationPolicyAccessory
NSWindowStyleMaskNonactivatingPanel                 0x80
NSWindowCollectionBehaviorCanJoinAllSpaces
NSWindowCollectionBehaviorCanJoinAllApplications
NSWindowCollectionBehaviorFullScreenAuxiliary
NSWindowCollectionBehaviorStationary
NSWindowCollectionBehaviorIgnoresCycle              combined: 0x40151
NSScreenSaverWindowLevel                            1000
hidesOnDeactivate                                   false
floatingPanel                                       true
canBecomeMainWindow                                 false
canBecomeKeyWindow                                  false (passive mode)
```

`CanJoinAllApplications` is significant on macOS 26 in addition to the usual
all-Spaces/full-screen-auxiliary combination. A plain high-level `NSWindow`
was not reliable for this behavior.

The app never calls `NSApplication.activate`. For the one-line reply only, an
atomic reply-mode flag temporarily makes the non-activating panel eligible to
be key, calls `makeKeyWindow`, then resigns it on Send/Escape/blur. The
previous application therefore remains the frontmost application.

Window mode changes must go through Tauri's `WebviewWindow::set_size`, not
directly through `NSPanel::setContentSize`. The latter resized the native
panel but left WKWebView with zero layout bounds on macOS 26.

At startup the binary logs the effective native values. A healthy launch
includes:

```text
KAIJU_PANEL level=1000 style=0x80 behavior=0x40151 panel=true keyable=false mainable=false
```

## Verification

```sh
cargo test -p kaiju-pet-core
cargo check -p kaiju-pet-shell -p kaiju-pet
```

The full-screen proof and final shell use the same panel class and flags. The
proof was manually verified visible/clickable over a full-screen Space and
across Spaces without activating the app. The final shell was also exercised
for native dragging, 420x540 expansion, daemon data/logs, and reply-field
focus while the prior app remained frontmost.
