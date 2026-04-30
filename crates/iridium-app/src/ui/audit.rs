use egui::Ui;

use crate::state::AppState;

/// Bottom dock: last N lines of the current audit log.
pub fn show(ui: &mut Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        ui.heading("Audit log");
        if let Some(path) = &state.audit_path {
            ui.label(format!("({})", path.display()));
            if ui.button("Open folder").clicked() {
                if let Some(parent) = path.parent() {
                    let _ = open_in_file_manager(parent);
                }
            }
        } else {
            ui.label("(no active log)");
        }
    });

    egui::ScrollArea::vertical()
        .id_salt("audit_scroll")
        .max_height(150.0)
        .stick_to_bottom(true)
        .show(ui, |ui| {
            for line in &state.audit_lines {
                ui.monospace(line);
            }
        });
}

fn open_in_file_manager(path: &std::path::Path) -> std::io::Result<()> {
    std::process::Command::new("xdg-open").arg(path).spawn().map(|_| ())
}
