use egui::Ui;

use crate::state::AppState;

use super::progress;

/// Center panel: pending queue, active job, and completed history.
pub fn show(ui: &mut Ui, state: &mut AppState) {
    ui.heading("Queue");

    // Active job
    if let Some(active) = &state.active {
        let cancelled = ui.group(|ui| progress::show_active(ui, active)).inner;
        if cancelled {
            // Wake the event loop immediately so poll_progress picks up the
            // Cancelled event within the same frame rather than after 100 ms.
            ui.ctx().request_repaint();
        }
        ui.separator();
    }

    // Pending jobs
    if !state.pending.is_empty() {
        ui.label(format!("Pending: {} job(s)", state.pending.len()));
        egui::ScrollArea::vertical()
            .id_salt("pending_scroll")
            .max_height(120.0)
            .show(ui, |ui| {
                for (i, spec) in state.pending.iter().enumerate() {
                    ui.label(format!(
                        "{}. {} → {} ({:?})",
                        i + 1,
                        spec.source.path.display(),
                        spec.dest_path.display(),
                        spec.format,
                    ));
                }
            });
        ui.separator();
    }

    // Completed jobs — always expanded so cancelled/failed results are visible immediately
    if !state.completed.is_empty() {
        ui.separator();
        ui.heading(format!("Completed ({})", state.completed.len()));
        egui::ScrollArea::vertical()
            .id_salt("completed_scroll")
            .max_height(300.0)
            .show(ui, |ui| {
                for job in state.completed.iter().rev() {
                    progress::show_completed(ui, job);
                    ui.separator();
                }
            });
    }
}
