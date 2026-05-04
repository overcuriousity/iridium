use egui::Ui;

use crate::state::{AppState, CentralTab};

use super::theme::Palette;

pub fn show(ui: &mut Ui, state: &mut AppState) {
    // Tab strip
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        for (label, tab) in [
            ("Queue", CentralTab::Queue),
            ("Active", CentralTab::Active),
            ("Completed", CentralTab::Completed),
        ] {
            let selected = state.central_tab == tab;
            let text = egui::RichText::new(label).strong();
            let text = if selected {
                text.color(Palette::ACCENT)
            } else {
                text.color(Palette::TEXT_DIM)
            };
            let btn = egui::Button::new(text)
                .frame(false)
                .stroke(if selected {
                    egui::Stroke::new(0.0, Palette::ACCENT)
                } else {
                    egui::Stroke::NONE
                });
            if ui.add(btn).clicked() {
                state.central_tab = tab;
            }
            if selected {
                // Underline indicator
                let r = ui.min_rect();
                // Draw a thin line just below the last response rect
                ui.painter().hline(
                    r.min.x..=r.max.x,
                    r.max.y + 1.0,
                    egui::Stroke::new(2.0, Palette::ACCENT),
                );
            }
        }
    });

    ui.add_space(2.0);
    ui.separator();
    ui.add_space(4.0);

    match state.central_tab {
        CentralTab::Queue => super::queue::show(ui, state),
        CentralTab::Active => super::progress::show_active_tab(ui, state),
        CentralTab::Completed => super::completed::show(ui, state),
    }
}
