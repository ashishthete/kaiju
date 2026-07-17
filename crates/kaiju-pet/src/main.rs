use kaiju_pet::{app::PetApp, client::Client, poller, prefs, window};

fn main() -> eframe::Result<()> {
    let prefs = prefs::load();
    let client = Client::new(prefs.daemon_url.clone(), None);
    let options = eframe::NativeOptions {
        viewport: window::viewport(prefs.window.clone()),
        ..Default::default()
    };
    eframe::run_native(
        "Kaiju Pet",
        options,
        Box::new(move |cc| {
            let (cmd_tx, msg_rx) = poller::spawn_worker(client, cc.egui_ctx.clone());
            Ok(Box::new(PetApp::new(cmd_tx, msg_rx, prefs)))
        }),
    )
}
