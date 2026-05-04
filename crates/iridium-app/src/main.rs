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
            .with_inner_size([cfg.window_width.max(1100.0), cfg.window_height.max(700.0)])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "iridium",
        options,
        Box::new(|cc| Ok(Box::new(app::IridiumApp::new(cc)))),
    )
}
