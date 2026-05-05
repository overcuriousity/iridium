use std::time::Duration;

use crate::{config, state::AppState, ui};

pub struct IridiumApp {
    state: AppState,
}

impl IridiumApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        ui::theme::apply(&cc.egui_ctx);
        let cfg = config::load();
        Self {
            state: AppState::new(cfg),
        }
    }
}

impl eframe::App for IridiumApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ── Persist window size so on_exit saves the current dimensions ───────
        if let Some(rect) = ctx.input(|i| i.viewport().inner_rect) {
            self.state.config.window_width = rect.width();
            self.state.config.window_height = rect.height();
        }

        // ── Progress polling ──────────────────────────────────────────────────
        let finished = self.state.poll_progress();
        if finished {
            self.state.collect_finished(ctx);
        }
        if self.state.active.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // ── File dialog result ────────────────────────────────────────────────
        if let Ok(mut guard) = self.state.file_dialog_slot.try_lock()
            && let Some(path) = guard.take()
        {
            self.state.file_dialog_open = false;
            if let Some(spec) = self.state.pending_job_form.as_mut() {
                spec.dest_path = path;
            }
        }

        // ── Job submit ────────────────────────────────────────────────────────
        if self.state.job_submit_requested {
            self.state.job_submit_requested = false;
            if let Some(spec) = self.state.pending_job_form.take() {
                if let Some(parent) = spec.dest_path.parent() {
                    self.state.config.last_output_dir = Some(parent.to_path_buf());
                }
                self.state.config.default_format = spec.format;
                self.state.config.default_hash_algs = spec.algorithms.clone();
                let _ = config::save(&self.state.config);
                self.state.pending.push_back(spec);
                self.state.show_job_form = false;
                self.state.inspector_mode = crate::state::InspectorMode::DeviceDetail;
                if self.state.active.is_none() {
                    crate::worker::start_next(&mut self.state, ctx);
                    self.state.central_tab = crate::state::CentralTab::Active;
                } else {
                    self.state.central_tab = crate::state::CentralTab::Queue;
                }
            }
        }

        // ── Top menu bar ──────────────────────────────────────────────────────
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

        // ── Status bar (absolute bottom) ──────────────────────────────────────
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(22.0)
            .show(ctx, |ui| {
                ui::status_bar::show(ui, &mut self.state);
            });

        // ── Audit dock (above status bar) ─────────────────────────────────────
        if self.state.show_audit {
            egui::TopBottomPanel::bottom("audit_panel")
                .resizable(true)
                .min_height(80.0)
                .default_height(140.0)
                .show(ctx, |ui| {
                    ui::audit::show(ui, &mut self.state);
                });
        }

        // ── Left panel: device list ───────────────────────────────────────────
        egui::SidePanel::left("devices_panel")
            .resizable(true)
            .min_width(240.0)
            .default_width(300.0)
            .show(ctx, |ui| {
                ui::devices::show(ui, &mut self.state);
            });

        // ── Right panel: inspector (device detail / job wizard) ───────────────
        egui::SidePanel::right("inspector_panel")
            .resizable(true)
            .min_width(280.0)
            .default_width(340.0)
            .show(ctx, |ui| {
                ui::inspector::show(ui, &mut self.state);
            });

        // ── Central panel: tabbed queue / active / completed ──────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            ui::central_tabs::show(ui, &mut self.state);
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Persist config first so a hung worker thread cannot prevent it.
        let _ = config::save(&self.state.config);

        // Cancel and join the active worker so libewf finalizes / closes the
        // segment cleanly before the process exits. Without this the EWF
        // writer's drop guard can run mid-flush and leave a truncated .E01.
        if let Some(mut active) = self.state.active.take() {
            active
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
            if let Some(handle) = active.handle.take() {
                let _ = handle.join();
            }
        }
    }
}
