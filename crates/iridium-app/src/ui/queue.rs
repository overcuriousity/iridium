use egui::Ui;
use egui_extras::{Column, TableBuilder};

use crate::state::{AppState, format_bytes};

use super::theme::{Palette, icons};

pub fn show(ui: &mut Ui, state: &mut AppState) {
    if state.pending.is_empty() {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("No pending jobs. Select a device and click Image…")
                .color(Palette::TEXT_DIM),
        );
        return;
    }

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("Pending: {} job(s)", state.pending.len()))
                .strong()
                .color(Palette::TEXT_STRONG),
        );
    });

    ui.add_space(4.0);

    let n = state.pending.len();

    // Collect interactions before borrowing state mutably
    let mut action_move_up: Option<usize> = None;
    let mut action_move_down: Option<usize> = None;
    let mut action_remove: Option<usize> = None;

    TableBuilder::new(ui)
        .striped(true)
        .resizable(false)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::exact(24.0))     // #
        .column(Column::initial(90.0).clip(true))  // Source
        .column(Column::remainder().clip(true))    // Dest
        .column(Column::initial(60.0))   // Format
        .column(Column::initial(72.0))   // Actions
        .header(20.0, |mut header| {
            header.col(|ui| { ui.label(egui::RichText::new("#").color(Palette::TEXT_DIM).small()); });
            header.col(|ui| { ui.label(egui::RichText::new("Source").color(Palette::TEXT_DIM).small()); });
            header.col(|ui| { ui.label(egui::RichText::new("Destination").color(Palette::TEXT_DIM).small()); });
            header.col(|ui| { ui.label(egui::RichText::new("Format").color(Palette::TEXT_DIM).small()); });
            header.col(|ui| { ui.label(""); });
        })
        .body(|mut body| {
            body.rows(22.0, n, |mut row| {
                let i = row.index();
                let spec = &state.pending[i];
                let src = spec.source.path.to_string_lossy().into_owned();
                let dst = spec.dest_path.to_string_lossy().into_owned();
                let fmt = format!("{:?}", spec.format);
                let size = format_bytes(spec.source.size_bytes);

                row.col(|ui| {
                    ui.label(egui::RichText::new(format!("{}", i + 1)).color(Palette::TEXT_DIM).small());
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(
                        egui::RichText::new(&src).font(egui::FontId::monospace(11.0)),
                    ).truncate())
                    .on_hover_text(format!("{src}\n{size}"));
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(
                        egui::RichText::new(&dst).font(egui::FontId::monospace(11.0)).color(Palette::TEXT_DIM),
                    ).truncate());
                });
                row.col(|ui| {
                    ui.label(egui::RichText::new(&fmt).small().color(Palette::TEXT_DIM));
                });
                row.col(|ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let can_up = i > 0;
                        let can_down = i + 1 < n;
                        if ui.add_enabled(can_up,
                            egui::Button::new(egui::RichText::new(icons::ARROW_UP).size(12.0)).frame(false)
                        ).on_hover_text("Move up").clicked() {
                            action_move_up = Some(i);
                        }
                        if ui.add_enabled(can_down,
                            egui::Button::new(egui::RichText::new(icons::ARROW_DOWN).size(12.0)).frame(false)
                        ).on_hover_text("Move down").clicked() {
                            action_move_down = Some(i);
                        }
                        if ui.add(
                            egui::Button::new(egui::RichText::new(icons::X).size(12.0).color(Palette::DANGER)).frame(false)
                        ).on_hover_text("Remove").clicked() {
                            action_remove = Some(i);
                        }
                    });
                });
            });
        });

    // Apply actions
    if let Some(i) = action_move_up {
        if i > 0 {
            state.pending.swap(i, i - 1);
        }
    }
    if let Some(i) = action_move_down {
        if i + 1 < n {
            state.pending.swap(i, i + 1);
        }
    }
    if let Some(i) = action_remove {
        state.pending.remove(i);
    }
}
