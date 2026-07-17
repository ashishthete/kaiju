use kaiju_pet::{app::PetApp, client::Client, poller, prefs, window};

fn main() -> eframe::Result<()> {
    let mut prefs = prefs::load();
    // `kaiju pet` forwards the CLI's --url as KAIJU_URL; let it override the
    // stored preference so the launcher's URL is actually honored.
    if let Ok(url) = std::env::var("KAIJU_URL") {
        if !url.is_empty() {
            prefs.daemon_url = url;
        }
    }
    let client = Client::new(prefs.daemon_url.clone(), None);
    let options = eframe::NativeOptions {
        viewport: window::viewport(prefs.window.clone()),
        ..Default::default()
    };
    eframe::run_native(
        "Kaiju Pet",
        options,
        Box::new(move |cc| {
            let egui_ctx = cc.egui_ctx.clone();
            let (cmd_tx, msg_rx) = poller::spawn_worker(client, move || egui_ctx.request_repaint());
            Ok(Box::new(PetApp::new(cmd_tx, msg_rx, prefs)))
        }),
    )
}
