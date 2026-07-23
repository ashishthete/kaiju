#[cfg(target_os = "macos")]
mod macos {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::Sender;
    use std::sync::{Arc, Mutex};

    use kaiju_pet_core::client::Client;
    use kaiju_pet_core::poller::{self, UiCmd, WorkerMsg};
    use kaiju_pet_core::prefs::{self, Prefs};
    use kaiju_pet_core::reducer::ReducerOutput;
    use kaiju_pet_core::validate::{validate_reply, ReplyError};
    use serde::Serialize;
    use tauri::{Emitter, Manager, Position, Size, WebviewUrl, WindowEvent};
    use tauri_nspanel::objc2_app_kit::NSWindowCollectionBehavior;
    use tauri_nspanel::{
        tauri_panel, CollectionBehavior, ManagerExt, PanelBuilder, PanelLevel, StyleMask,
    };

    const PANEL_LABEL: &str = "kaiju-pet";
    static REPLY_MODE: AtomicBool = AtomicBool::new(false);
    // True only while the pet is collapsed (its resting size). We persist the
    // window position only at rest, so opening/closing the panel never overwrites
    // the remembered spot with a transient panel origin.
    static AT_REST: AtomicBool = AtomicBool::new(true);
    // Whether the panel is currently ignoring cursor events (clicks passing
    // through to whatever is behind it). Tracked so the 30Hz ticker only calls
    // into the window when the state actually changes.
    static IGNORING_CURSOR: AtomicBool = AtomicBool::new(false);

    tauri_panel! {
        panel!(KaijuPanel {
            config: {
                // A passive pet is deliberately non-keyable. We enable this
                // only while its one-line reply editor is in use, without
                // activating NSApplication.
                can_become_key_window: REPLY_MODE.load(Ordering::SeqCst),
                can_become_main_window: false
            }
        })
    }

    #[derive(Clone)]
    struct ShellState {
        commands: Sender<UiCmd>,
        latest: Arc<Mutex<Option<ReducerOutput>>>,
        prefs: Arc<Mutex<Prefs>>,
    }

    #[derive(Clone, Serialize)]
    struct LogPayload {
        id: String,
        lines: Vec<String>,
        error: Option<String>,
    }

    /// Cursor direction relative to the panel centre, each component -1..1.
    #[derive(Clone, Serialize)]
    struct CursorTick {
        dx: f64,
        dy: f64,
    }

    #[derive(Clone, Serialize)]
    struct SendPayload {
        id: String,
        ok: bool,
        error: Option<String>,
    }

    fn command_error(error: std::sync::mpsc::SendError<UiCmd>) -> String {
        format!("pet worker stopped: {error}")
    }

    #[tauri::command]
    fn get_snapshot(state: tauri::State<'_, ShellState>) -> Option<ReducerOutput> {
        state.latest.lock().ok().and_then(|value| value.clone())
    }

    #[tauri::command]
    fn fetch_logs(id: String, state: tauri::State<'_, ShellState>) -> Result<(), String> {
        state
            .commands
            .send(UiCmd::FetchLogs(id))
            .map_err(command_error)
    }

    #[tauri::command]
    fn mark_read(id: String, state: tauri::State<'_, ShellState>) -> Result<(), String> {
        state
            .commands
            .send(UiCmd::MarkRead(id))
            .map_err(command_error)
    }

    #[tauri::command]
    fn retry_now(state: tauri::State<'_, ShellState>) -> Result<(), String> {
        state.commands.send(UiCmd::RetryNow).map_err(command_error)
    }

    #[tauri::command]
    fn send_reply(
        id: String,
        text: String,
        state: tauri::State<'_, ShellState>,
    ) -> Result<(), String> {
        let text = validate_reply(&text).map_err(|error| match error {
            ReplyError::Empty => "Reply cannot be empty".to_string(),
            ReplyError::TooLarge => "Reply must be at most 4 KiB".to_string(),
            ReplyError::ControlChar => "Reply must be a single line".to_string(),
        })?;

        let latest = state
            .latest
            .lock()
            .map_err(|_| "pet state is unavailable".to_string())?;
        let agent = latest
            .as_ref()
            .and_then(|snapshot| snapshot.agents.iter().find(|agent| agent.id == id))
            .ok_or_else(|| "Agent no longer exists".to_string())?;
        if agent.status.is_terminal() {
            return Err("Agent is no longer accepting replies".to_string());
        }
        drop(latest);

        // Deliberately enqueue exactly once. The core worker never retries a
        // POST, even when transport fails.
        state
            .commands
            .send(UiCmd::SendReply { id, text })
            .map_err(command_error)
    }

    #[tauri::command]
    fn set_panel_mode(mode: String, app: tauri::AppHandle) -> Result<(), String> {
        let (width, height) = match mode.as_str() {
            "collapsed" => (188.0, 188.0),
            "attention" => (380.0, 270.0),
            "expanded" => (420.0, 540.0),
            _ => return Err("unknown panel mode".to_string()),
        };
        let window = app
            .get_webview_window(PANEL_LABEL)
            .ok_or_else(|| "pet webview is unavailable".to_string())?;

        // The pet sits at the window's bottom-right corner (CSS right/bottom), so
        // pin that corner while resizing: the panel opens up-and-left from the pet
        // instead of shoving the whole window off the bottom-right of the screen.
        let scale = window.scale_factor().unwrap_or(1.0);
        let origin = new_origin_anchoring_pet(&window, scale, width, height);

        // Persist the resting position only while collapsed (see AT_REST).
        let collapsed = mode == "collapsed";
        AT_REST.store(collapsed, Ordering::SeqCst);

        // An expanded panel is interactive UI everywhere, so it must capture
        // clicks at once rather than waiting for the click-through ticker's next
        // poll. (The ticker resumes pass-through once collapsed again.)
        if !collapsed && IGNORING_CURSOR.swap(false, Ordering::SeqCst) {
            let _ = window.set_ignore_cursor_events(false);
        }

        // Resize through Tauri so WKWebView's autoresizing machinery updates
        // its layout viewport. Calling NSPanel::setContentSize directly leaves
        // WebKit at zero layout bounds on macOS 26.
        window
            .set_size(Size::Logical(tauri::LogicalSize::new(width, height)))
            .map_err(|error| error.to_string())?;
        if let Some(origin) = origin {
            window
                .set_position(Position::Logical(origin))
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    // Origin (top-left) that keeps the pet's bottom-right corner fixed at the new
    // size, clamped to the monitor under the pet so the panel stays fully visible.
    // Returns None if the geometry can't be read (leave the window where it is).
    fn new_origin_anchoring_pet(
        window: &tauri::WebviewWindow,
        scale: f64,
        width: f64,
        height: f64,
    ) -> Option<tauri::LogicalPosition<f64>> {
        let pos = window.outer_position().ok()?.to_logical::<f64>(scale);
        let size = window.outer_size().ok()?.to_logical::<f64>(scale);
        let mut x = pos.x + size.width - width;
        let mut y = pos.y + size.height - height;
        if let Some(monitor) = window.current_monitor().ok().flatten() {
            let m_pos = monitor.position().to_logical::<f64>(scale);
            let m_size = monitor.size().to_logical::<f64>(scale);
            let max_x = (m_pos.x + m_size.width - width).max(m_pos.x);
            let max_y = (m_pos.y + m_size.height - height).max(m_pos.y);
            x = x.clamp(m_pos.x, max_x);
            y = y.clamp(m_pos.y, max_y);
        }
        Some(tauri::LogicalPosition::new(x, y))
    }

    #[tauri::command]
    fn begin_reply(app: tauri::AppHandle) -> Result<(), String> {
        let panel = app
            .get_webview_panel(PANEL_LABEL)
            .map_err(|_| "pet panel is unavailable".to_string())?;
        REPLY_MODE.store(true, Ordering::SeqCst);
        panel.set_becomes_key_only_if_needed(false);
        panel.make_key_window();
        Ok(())
    }

    #[tauri::command]
    fn end_reply(app: tauri::AppHandle) -> Result<(), String> {
        let panel = app
            .get_webview_panel(PANEL_LABEL)
            .map_err(|_| "pet panel is unavailable".to_string())?;
        panel.resign_key_window();
        panel.set_becomes_key_only_if_needed(true);
        REPLY_MODE.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// Surfaces webview errors in the process log.
    ///
    /// The panel cannot become key and has no menu bar, so there is no way to
    /// open an inspector in a release build. Without this, a JS failure is
    /// completely silent from outside the webview.
    #[tauri::command]
    fn log_js(level: String, message: String) {
        eprintln!("KAIJU_JS [{level}] {message}");
    }

    #[tauri::command]
    fn open_dashboard(state: tauri::State<'_, ShellState>) -> Result<(), String> {
        let url = state
            .prefs
            .lock()
            .map_err(|_| "preferences are unavailable".to_string())?
            .daemon_url
            .clone();
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("could not open dashboard: {error}"))
    }

    #[tauri::command]
    fn quit(app: tauri::AppHandle, state: tauri::State<'_, ShellState>) {
        let _ = state.commands.send(UiCmd::Shutdown);
        app.exit(0);
    }

    fn start_bridge(
        app: tauri::AppHandle,
        state: ShellState,
        messages: std::sync::mpsc::Receiver<WorkerMsg>,
    ) {
        std::thread::spawn(move || {
            while let Ok(message) = messages.recv() {
                match message {
                    WorkerMsg::Snapshot(snapshot) => {
                        if let Ok(mut latest) = state.latest.lock() {
                            *latest = Some(snapshot.clone());
                        }
                        let _ = app.emit("kaiju:snapshot", snapshot);
                    }
                    WorkerMsg::LogTail { id, lines } => {
                        let payload = match lines {
                            Ok(lines) => LogPayload {
                                id,
                                lines,
                                error: None,
                            },
                            Err(error) => LogPayload {
                                id,
                                lines: Vec::new(),
                                error: Some(error.to_string()),
                            },
                        };
                        let _ = app.emit("kaiju:logs", payload);
                    }
                    WorkerMsg::SendResult { id, result } => {
                        let payload = match result {
                            Ok(()) => SendPayload {
                                id,
                                ok: true,
                                error: None,
                            },
                            Err(error) => SendPayload {
                                id,
                                ok: false,
                                error: Some(error.to_string()),
                            },
                        };
                        let _ = app.emit("kaiju:send-result", payload);
                    }
                }
            }
        });
    }

    /// Lets clicks fall through the panel's empty area to whatever is behind it,
    /// while keeping the character itself clickable (poke, drag, reply button).
    ///
    /// The panel is a 188pt transparent square but the character only fills a
    /// centred ellipse of it; without this, the transparent corners still eat
    /// every click, which is maddening for an always-on-top window. While the
    /// panel is expanded (a reply/attention UI, not at rest) the whole surface
    /// must stay clickable, so we only pass clicks through when collapsed.
    fn update_click_through(
        window: &tauri::WebviewWindow,
        cursor: tauri::LogicalPosition<f64>,
        origin: tauri::LogicalPosition<f64>,
        size: tauri::LogicalSize<f64>,
    ) {
        // Cursor position within the panel, 0..1 on each axis.
        let lx = (cursor.x - origin.x) / size.width.max(1.0);
        let ly = (cursor.y - origin.y) / size.height.max(1.0);

        // Ellipse hugging the character: centred slightly low, freeing the
        // corners where the panel is always transparent.
        let ex = (lx - 0.5) / 0.44;
        let ey = (ly - 0.55) / 0.47;
        let over_character = ex * ex + ey * ey <= 1.0;

        // Capture everything while expanded; otherwise only over the character.
        let capture = !AT_REST.load(Ordering::SeqCst) || over_character;
        let want_ignore = !capture;

        // Only touch the window when the state flips — this runs at 30Hz.
        if IGNORING_CURSOR.swap(want_ignore, Ordering::SeqCst) != want_ignore {
            let _ = window.set_ignore_cursor_events(want_ignore);
        }
    }

    /// Drives the pet's animation clock and cursor tracking from Rust.
    ///
    /// Two reasons this cannot live in the webview:
    ///   1. `pointermove` only fires while the cursor is INSIDE the 188pt panel,
    ///      so a webview-side listener can never track the cursor across the
    ///      desktop — which is the whole point of a pet that looks at you.
    ///   2. WebKit throttles `requestAnimationFrame` in a non-activating, non-key
    ///      NSPanel. A Rust timer is not subject to that, so the pet keeps moving
    ///      while it sits unfocused over another app.
    ///
    /// Emits the cursor direction already normalised to the panel's centre, so
    /// the webview needs no screen geometry of its own.
    fn start_cursor_ticker(app: tauri::AppHandle) {
        const TICK: std::time::Duration = std::time::Duration::from_millis(33); // ~30Hz

        std::thread::spawn(move || loop {
            std::thread::sleep(TICK);

            let Some(window) = app.get_webview_window(PANEL_LABEL) else {
                continue;
            };
            let (Ok(cursor), Ok(origin), Ok(size), Ok(scale)) = (
                app.cursor_position(),
                window.outer_position(),
                window.inner_size(),
                window.scale_factor(),
            ) else {
                continue;
            };

            let cursor = cursor.to_logical::<f64>(scale);
            let origin = origin.to_logical::<f64>(scale);
            let size = size.to_logical::<f64>(scale);
            let center_x = origin.x + size.width / 2.0;
            let center_y = origin.y + size.height / 2.0;

            update_click_through(&window, cursor, origin, size);

            // Normalise over a few panel-widths so the head turns for cursors
            // near the pet and saturates for ones far away.
            let span = (size.width.max(size.height) * 3.0).max(1.0);
            let payload = CursorTick {
                dx: ((cursor.x - center_x) / span).clamp(-1.0, 1.0),
                dy: ((cursor.y - center_y) / span).clamp(-1.0, 1.0),
            };
            let _ = app.emit("kaiju:tick", payload);
        });
    }

    /// Watches the panel and puts it back if the system orders it out.
    ///
    /// Nothing in this app hides the panel, yet it can vanish while the process
    /// stays alive and healthy. A panel at ScreenSaver level can be ordered out
    /// by the window server — screen lock, display sleep, a screensaver, or a
    /// Space transition — and it is not always restored afterwards.
    ///
    /// Logs every transition, so the log says exactly when and why the pet went
    /// missing rather than leaving us to guess.
    fn start_visibility_watchdog(app: tauri::AppHandle) {
        const POLL: std::time::Duration = std::time::Duration::from_secs(2);

        std::thread::spawn(move || {
            let mut was_visible = true;
            loop {
                std::thread::sleep(POLL);

                let Some(window) = app.get_webview_window(PANEL_LABEL) else {
                    continue;
                };
                let visible = window.is_visible().unwrap_or(true);

                if !visible {
                    if was_visible {
                        eprintln!(
                            "KAIJU_WATCHDOG panel went invisible (position={:?}); restoring",
                            window.outer_position()
                        );
                    }
                    let _ = window.show();
                } else if !was_visible {
                    eprintln!("KAIJU_WATCHDOG panel visible again");
                }
                was_visible = visible;
            }
        });
    }

    fn restore_position(app: &tauri::AppHandle, x: i32, y: i32) -> tauri::LogicalPosition<f64> {
        let monitors = app.available_monitors().unwrap_or_default();
        let still_visible = monitors.iter().any(|monitor| {
            let origin = monitor.position();
            let size = monitor.size();
            let scale = monitor.scale_factor();
            let left = origin.x as f64 / scale;
            let top = origin.y as f64 / scale;
            let width = size.width as f64 / scale;
            let height = size.height as f64 / scale;
            x as f64 >= left - 140.0
                && (x as f64) < left + width - 30.0
                && y as f64 >= top - 140.0
                && (y as f64) < top + height - 30.0
        });
        if still_visible {
            return tauri::LogicalPosition::new(x as f64, y as f64);
        }

        if let Ok(Some(primary)) = app.primary_monitor() {
            let origin = primary.position();
            let size = primary.size();
            let scale = primary.scale_factor();
            return tauri::LogicalPosition::new(
                (origin.x + size.width as i32) as f64 / scale - 208.0,
                (origin.y + size.height as i32) as f64 / scale - 208.0,
            );
        }
        tauri::LogicalPosition::new(80.0, 80.0)
    }

    pub fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
        app.handle()
            .set_activation_policy(tauri::ActivationPolicy::Accessory)?;

        let mut loaded_prefs = prefs::load();
        if let Ok(url) = std::env::var("KAIJU_URL") {
            loaded_prefs.daemon_url = url;
        }
        let initial_position =
            restore_position(app.handle(), loaded_prefs.window.x, loaded_prefs.window.y);
        loaded_prefs.window.x = initial_position.x.round() as i32;
        loaded_prefs.window.y = initial_position.y.round() as i32;
        let client = Client::new(
            loaded_prefs.daemon_url.clone(),
            std::env::var("KAIJU_TOKEN").ok(),
        );
        let (commands, messages) = poller::spawn_worker(client, || {});
        let state = ShellState {
            commands,
            latest: Arc::new(Mutex::new(None)),
            prefs: Arc::new(Mutex::new(loaded_prefs)),
        };
        app.manage(state.clone());

        let behavior = CollectionBehavior::from_raw(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::CanJoinAllApplications
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );

        let panel = PanelBuilder::<_, KaijuPanel>::new(app.handle(), PANEL_LABEL)
            .url(WebviewUrl::App("index.html".into()))
            .title("Kaiju Pet")
            .size(Size::Logical(tauri::LogicalSize::new(188.0, 188.0)))
            .no_activate(true)
            .style_mask(StyleMask::empty().nonactivating_panel())
            .collection_behavior(behavior)
            .level(PanelLevel::ScreenSaver)
            .floating(true)
            .hides_on_deactivate(false)
            .becomes_key_only_if_needed(true)
            .accepts_mouse_moved_events(true)
            .movable_by_window_background(true)
            .transparent(true)
            .has_shadow(false)
            .with_window(|window| {
                window
                    .visible(false)
                    .decorations(false)
                    .transparent(true)
                    .resizable(false)
                    .skip_taskbar(true)
            })
            .build()?;

        let native = panel.as_panel();
        eprintln!(
            "KAIJU_PANEL level={} style=0x{:x} behavior=0x{:x} panel={} keyable={} mainable={}",
            native.level(),
            native.styleMask().bits(),
            native.collectionBehavior().bits(),
            native.isFloatingPanel(),
            native.canBecomeKeyWindow(),
            native.canBecomeMainWindow(),
        );

        if let Some(window) = app.get_webview_window(PANEL_LABEL) {
            let preferences = state.prefs.clone();
            let scale_window = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::Moved(position) = event {
                    // Persist only the resting position; skip the programmatic moves
                    // that opening/closing the panel produces.
                    if !AT_REST.load(Ordering::SeqCst) {
                        return;
                    }
                    let scale = scale_window.scale_factor().unwrap_or(1.0);
                    let position = position.to_logical::<f64>(scale);
                    if let Ok(mut preferences) = preferences.lock() {
                        preferences.window.x = position.x.round() as i32;
                        preferences.window.y = position.y.round() as i32;
                        let _ = prefs::save(&preferences);
                    }
                }
            });
        }

        start_bridge(app.handle().clone(), state, messages);
        start_cursor_ticker(app.handle().clone());
        start_visibility_watchdog(app.handle().clone());

        // Launch-time character override, for testing a character without
        // driving the right-click menu. Emitted after a short delay so the
        // webview has registered its listener.
        if let Ok(id) = std::env::var("KAIJU_PET_CHARACTER") {
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(800));
                let _ = handle.emit("kaiju:force-character", id);
            });
        }

        // The panel has no menu bar and cannot become key on demand, so there is
        // no way to reach the inspector by hand. Open it automatically in debug
        // builds; release builds ship without it.
        #[cfg(debug_assertions)]
        if let Some(window) = app.get_webview_window(PANEL_LABEL) {
            window.open_devtools();
        }

        panel.show();
        app.manage(panel.clone());
        // Converting the hidden Tauri window changes its Objective-C class.
        // Reassert the webview content size after it is ordered front so
        // WebKit receives a non-zero layout viewport on macOS 26.
        if let Some(window) = app.get_webview_window(PANEL_LABEL) {
            window.set_size(Size::Logical(tauri::LogicalSize::new(188.0, 188.0)))?;
            window.set_position(Position::Logical(initial_position))?;
            eprintln!(
                "KAIJU_WINDOW visible={} position={:?} size={:?}",
                panel.is_visible(),
                window.outer_position(),
                window.outer_size(),
            );
        }
        Ok(())
    }

    // A passive pet is a singleton. Hold a fixed loopback port for the whole
    // process lifetime: a second launch fails to bind and bows out, so the
    // dashboard button (or `cargo run`) can never stack windows. The OS frees
    // the port on exit or crash, so there is no stale lock file to clean up.
    const SINGLETON_PORT: u16 = 52789;

    fn acquire_single_instance() -> Option<std::net::TcpListener> {
        std::net::TcpListener::bind(("127.0.0.1", SINGLETON_PORT)).ok()
    }

    pub fn run() {
        let Some(_singleton) = acquire_single_instance() else {
            eprintln!("KAIJU_PET already running (port {SINGLETON_PORT} held); exiting.");
            return;
        };

        tauri::Builder::default()
            .plugin(tauri_nspanel::init())
            .invoke_handler(tauri::generate_handler![
                get_snapshot,
                fetch_logs,
                mark_read,
                retry_now,
                send_reply,
                set_panel_mode,
                begin_reply,
                end_reply,
                open_dashboard,
                log_js,
                quit,
            ])
            .setup(setup)
            .run(tauri::generate_context!())
            .expect("failed to run Kaiju pet shell");
    }
}

#[cfg(target_os = "macos")]
fn main() {
    macos::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("kaiju-pet-shell is currently available only on macOS");
}
