mod app;
mod config;
mod error;
mod state;
mod ui;
mod verify;
mod worker;

fn main() -> eframe::Result<()> {
    let cfg = config::load();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("iridium — forensic disk imaging")
            .with_inner_size([cfg.window_width, cfg.window_height])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "iridium",
        options,
        Box::new(|cc| Ok(Box::new(app::IridiumApp::new(cc)))),
    )
}
