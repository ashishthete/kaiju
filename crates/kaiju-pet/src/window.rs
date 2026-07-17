/// Bundled attention cue, written to a temp file and played once via afplay.
const CUE_WAV: &[u8] = include_bytes!("../assets/attention.wav");

/// Bundled mascot sprite (placeholder terminal-robot; swap this PNG for final art).
const PET_PNG: &[u8] = include_bytes!("../assets/pet.png");

/// Decode the bundled sprite and upload it as a GPU texture once. On a decode
/// failure returns a 1×1 transparent texture so the UI still renders.
pub fn load_sprite(ctx: &egui::Context) -> egui::TextureHandle {
    let color = match image::load_from_memory(PET_PNG) {
        Ok(dynamic) => {
            let rgba = dynamic.to_rgba8();
            let (w, h) = rgba.dimensions();
            egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba.as_raw())
        }
        Err(_) => egui::ColorImage::from_rgba_unmultiplied([1, 1], &[0, 0, 0, 0]),
    };
    ctx.load_texture("pet-sprite", color, egui::TextureOptions::LINEAR)
}

pub fn viewport(pos: crate::prefs::WindowPos) -> egui::ViewportBuilder {
    egui::ViewportBuilder::default()
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_taskbar(false)
        .with_resizable(false)
        .with_inner_size([240.0, 340.0])
        .with_position([pos.x as f32, pos.y as f32])
}

#[cfg(target_os = "macos")]
pub fn play_cue() {
    let path = std::env::temp_dir().join("kaiju-pet-cue.wav");
    if std::fs::write(&path, CUE_WAV).is_ok() {
        let _ = std::process::Command::new("afplay").arg(&path).spawn();
    }
}

#[cfg(not(target_os = "macos"))]
pub fn play_cue() {
    // Sound is supplemental; other platforms are a window-adapter follow-up.
    let _ = CUE_WAV;
}

/// Make every app window float over all Spaces and over full-screen apps — the
/// native behavior eframe does not expose. Must run on the main thread; safe to
/// call once the window exists. No-op off macOS.
#[cfg(target_os = "macos")]
pub fn configure_all_spaces() {
    use objc2_app_kit::{NSApplication, NSScreenSaverWindowLevel, NSWindowCollectionBehavior};
    use objc2_foundation::MainThreadMarker;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let behavior = NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::FullScreenAuxiliary
        | NSWindowCollectionBehavior::Stationary;
    for window in app.windows().iter() {
        window.setCollectionBehavior(behavior);
        // Floating level (3) sits *below* a full-screen app's content; the
        // screen-saver level (1000) draws above it so the pet stays visible.
        window.setLevel(NSScreenSaverWindowLevel);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn configure_all_spaces() {}
