/// Bundled attention cue, written to a temp file and played once via afplay.
const CUE_WAV: &[u8] = include_bytes!("../assets/attention.wav");

pub fn viewport(pos: crate::prefs::WindowPos) -> egui::ViewportBuilder {
    egui::ViewportBuilder::default()
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_taskbar(false)
        .with_resizable(false)
        .with_inner_size([220.0, 160.0])
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
