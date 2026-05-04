use egui::Ui;
use egui_extras::{Column, TableBuilder};

use crate::state::{AppState, AuditLevel};

use super::theme::{self, Palette, icons};

pub fn show(ui: &mut Ui, state: &mut AppState) {
    // ── Toolbar ───────────────────────────────────────────────────────────────
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(icons::AUDIT).color(Palette::TEXT_DIM));
        ui.label(egui::RichText::new("Audit log").strong().color(Palette::TEXT_STRONG));

        if let Some(path) = &state.audit_path.clone() {
            ui.label(
                egui::RichText::new(format!("({})", path.display()))
                    .small()
                    .color(Palette::TEXT_DIM),
            );
            if ui.small_button("Open folder").clicked() {
                if let Some(parent) = path.parent() {
                    let _ = std::process::Command::new("xdg-open").arg(parent).spawn();
                }
            }
        } else {
            ui.label(egui::RichText::new("no active log").small().color(Palette::TEXT_DIM));
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Follow tail toggle
            let tail_label = if state.audit_filter.follow_tail { "↓ Follow" } else { "⏸ Paused" };
            if ui.selectable_label(state.audit_filter.follow_tail, tail_label).clicked() {
                state.audit_filter.follow_tail = !state.audit_filter.follow_tail;
            }

            ui.add_space(4.0);

            // Level filter
            let level_text = match state.audit_filter.min_level {
                AuditLevel::Debug => "ALL",
                AuditLevel::Info => "INFO+",
                AuditLevel::Warn => "WARN+",
                AuditLevel::Error => "ERROR",
            };
            egui::ComboBox::from_id_salt("audit_level_filter")
                .selected_text(level_text)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut state.audit_filter.min_level, AuditLevel::Debug, "All");
                    ui.selectable_value(&mut state.audit_filter.min_level, AuditLevel::Info, "INFO+");
                    ui.selectable_value(&mut state.audit_filter.min_level, AuditLevel::Warn, "WARN+");
                    ui.selectable_value(&mut state.audit_filter.min_level, AuditLevel::Error, "ERROR");
                });

            ui.add_space(4.0);

            // Filter text box
            ui.add(
                egui::TextEdit::singleline(&mut state.audit_filter.text)
                    .hint_text(format!("{} filter…", icons::MAGNIFYING_GLASS))
                    .desired_width(140.0),
            );
        });
    });

    ui.add_space(2.0);

    // ── Table ─────────────────────────────────────────────────────────────────
    let filter_text = state.audit_filter.text.to_lowercase();
    let min_level = state.audit_filter.min_level;

    let visible: Vec<_> = state
        .audit_views
        .iter()
        .filter(|v| {
            v.level >= min_level
                && (filter_text.is_empty()
                    || v.event.to_lowercase().contains(&filter_text)
                    || v.detail.to_lowercase().contains(&filter_text)
                    || v.ts.contains(&filter_text))
        })
        .collect();

    let n = visible.len();
    let follow = state.audit_filter.follow_tail;

    let mut table = TableBuilder::new(ui)
        .striped(true)
        .resizable(false)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(72.0).clip(true))   // Timestamp
        .column(Column::initial(52.0))              // Level
        .column(Column::initial(140.0).clip(true))  // Event
        .column(Column::remainder().clip(true));    // Detail

    if follow && n > 0 {
        table = table.scroll_to_row(n.saturating_sub(1), Some(egui::Align::BOTTOM));
    }

    table
        .header(18.0, |mut header| {
            for label in ["Timestamp", "Level", "Event", "Detail"] {
                header.col(|ui| {
                    ui.label(egui::RichText::new(label).small().color(Palette::TEXT_DIM));
                });
            }
        })
        .body(|mut body| {
            body.rows(18.0, n, |mut row| {
                let view = visible[row.index()];
                row.col(|ui| {
                    // Trim timestamp to HH:MM:SS if RFC3339
                    let ts_short = view.ts.get(11..19).unwrap_or(&view.ts);
                    ui.add(egui::Label::new(
                        egui::RichText::new(ts_short)
                            .font(egui::FontId::monospace(10.0))
                            .color(Palette::TEXT_DIM),
                    ).truncate());
                });
                row.col(|ui| {
                    let (fg, bg) = level_colors(view.level);
                    theme::chip(ui, view.level.label(), fg, bg);
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(
                        egui::RichText::new(&view.event)
                            .font(egui::FontId::monospace(11.0))
                            .color(Palette::TEXT_STRONG),
                    ).truncate());
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(
                        egui::RichText::new(&view.detail)
                            .font(egui::FontId::monospace(11.0))
                            .color(Palette::TEXT_DIM),
                    ).truncate());
                });
            });
        });
}

fn level_colors(level: AuditLevel) -> (egui::Color32, egui::Color32) {
    match level {
        AuditLevel::Debug => (Palette::TEXT_DIM, Palette::SURFACE_ALT),
        AuditLevel::Info => (Palette::ACCENT, Palette::ACCENT_DIM),
        AuditLevel::Warn => (Palette::WARN, Palette::WARN_BG),
        AuditLevel::Error => (Palette::DANGER, Palette::DANGER_BG),
    }
}
