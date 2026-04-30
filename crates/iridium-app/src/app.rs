use std::time::Duration;

use crate::{config, state::AppState, ui};

pub struct IridiumApp {
    state: AppState,
}

impl IridiumApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let cfg = config::load();
        Self {
            state: AppState::new(cfg),
        }
    }
}

impl eframe::App for IridiumApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll progress from worker thread.
        let finished = self.state.poll_progress();
        if finished {
            self.state.collect_finished(ctx);
        }

        // Keep repainting while a job is running.
        if self.state.active.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // Top menu bar
        egui::TopBottomPanel::top("menubar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("View", |ui| {
                    ui.checkbox(&mut self.state.show_audit, "Audit log");
                });
            });
        });

        // Audit log dock at bottom
        if self.state.show_audit {
            egui::TopBottomPanel::bottom("audit_panel")
                .resizable(true)
                .min_height(80.0)
                .show(ctx, |ui| {
                    ui::audit::show(ui, &mut self.state);
                });
        }

        // Left panel: device list
        egui::SidePanel::left("devices_panel")
            .resizable(true)
            .min_width(260.0)
            .show(ctx, |ui| {
                ui::devices::show(ui, &mut self.state);
            });

        // Central panel: queue + progress
        egui::CentralPanel::default().show(ctx, |ui| {
            ui::queue::show(ui, &mut self.state);
        });

        // Job form modal window — rendered last so it floats above panels
        if self.state.show_job_form {
            ui::job_form::show(ctx, &mut self.state);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = config::save(&self.state.config);
    }
}
