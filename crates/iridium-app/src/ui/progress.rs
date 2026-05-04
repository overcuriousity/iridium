use std::time::Instant;

use egui::Ui;
use egui_plot::{Line, Plot, PlotPoints};

use crate::state::{ActiveJob, format_bytes, format_duration};

use super::theme::{self, Palette, icons};

pub fn show_active_tab(ui: &mut Ui, state: &mut crate::state::AppState) {
    if let Some(active) = &state.active {
        // We need to split the borrow carefully: pass active immutably,
        // handle cancel button response afterwards.
        let cancelled = show_active_inner(ui, active);
        if cancelled {
            state.active.as_ref().unwrap().cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            ui.ctx().request_repaint();
        }
    } else if !state.completed.is_empty() {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("No active job. See the Completed tab.")
                .color(Palette::TEXT_DIM),
        );
    } else {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("No job running. Select a device and click Image…")
                .color(Palette::TEXT_DIM),
        );
    }
}

/// Returns true if Cancel was clicked.
fn show_active_inner(ui: &mut Ui, active: &ActiveJob) -> bool {
    let p = &active.progress;

    // ── Header row ────────────────────────────────────────────────────────────
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(
            egui::RichText::new(format!(
                "{} {} → {}",
                icons::DEVICE_HDD,
                active.spec.source.path.display(),
                active.spec.dest_path.display(),
            ))
            .font(egui::FontId::monospace(12.0))
            .color(Palette::TEXT_STRONG),
        ).truncate());
    });

    ui.add_space(4.0);

    // ── Progress bar ──────────────────────────────────────────────────────────
    if p.verifying {
        let frac = if p.total_bytes > 0 {
            p.verify_bytes_done as f32 / p.total_bytes as f32
        } else {
            0.0
        };
        ui.add(
            egui::ProgressBar::new(frac)
                .text(format!("Verifying: {:.1}%", frac * 100.0))
                .animate(true)
                .fill(Palette::ACCENT),
        );
    } else if p.total_bytes > 0 {
        let frac = p.bytes_done as f32 / p.total_bytes as f32;
        ui.add(
            egui::ProgressBar::new(frac)
                .text(format!("{:.1}%", frac * 100.0))
                .fill(Palette::ACCENT),
        );
    } else {
        ui.add(
            egui::ProgressBar::new(0.0)
                .text("Starting…")
                .animate(true)
                .fill(Palette::ACCENT),
        );
    }

    ui.add_space(4.0);

    // ── Stats grid ────────────────────────────────────────────────────────────
    let mbps = active.ewma_bps / 1_048_576.0;
    let eta_secs = if active.ewma_bps > 1.0 && p.total_bytes > p.bytes_done {
        (p.total_bytes - p.bytes_done) as f64 / active.ewma_bps
    } else {
        f64::INFINITY
    };
    let elapsed = active.started_at.elapsed().as_secs_f64();

    egui::Grid::new("progress_stats")
        .num_columns(4)
        .spacing([16.0, 2.0])
        .show(ui, |ui| {
            // Labels row
            for label in ["Bytes", "Throughput", "ETA", "Elapsed"] {
                ui.label(egui::RichText::new(label).small().color(Palette::TEXT_DIM));
            }
            ui.end_row();
            // Values row
            ui.add(egui::Label::new(
                egui::RichText::new(format!(
                    "{} / {}",
                    format_bytes(p.bytes_done),
                    format_bytes(p.total_bytes)
                ))
                .font(egui::FontId::monospace(12.0))
                .color(Palette::TEXT_STRONG),
            ));
            ui.add(egui::Label::new(
                egui::RichText::new(format!("{mbps:.1} MB/s"))
                    .font(egui::FontId::monospace(12.0))
                    .color(if mbps > 0.1 { Palette::ACCENT } else { Palette::TEXT_DIM }),
            ));
            ui.add(egui::Label::new(
                egui::RichText::new(format_duration(eta_secs))
                    .font(egui::FontId::monospace(12.0))
                    .color(Palette::TEXT_STRONG),
            ));
            ui.add(egui::Label::new(
                egui::RichText::new(format_duration(elapsed))
                    .font(egui::FontId::monospace(12.0))
                    .color(Palette::TEXT_STRONG),
            ));
            ui.end_row();
        });

    ui.add_space(4.0);

    // ── Hash chips ────────────────────────────────────────────────────────────
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(icons::HASH).color(Palette::TEXT_DIM));
        for alg in &active.spec.algorithms {
            let label = match alg {
                iridium_core::HashAlg::Md5 => "MD5",
                iridium_core::HashAlg::Sha1 => "SHA-1",
                iridium_core::HashAlg::Sha256 => "SHA-256",
            };
            if p.verifying || p.bytes_done == p.total_bytes {
                theme::chip_success(ui, label);
            } else if p.bytes_done > 0 {
                theme::chip_info(ui, label);
            } else {
                theme::chip(ui, label, Palette::TEXT_DIM, Palette::SURFACE_ALT);
            }
        }
        if p.bad_chunks > 0 {
            ui.add_space(4.0);
            theme::chip_warn(ui, &format!("{} bad chunks", p.bad_chunks));
        }
    });

    ui.add_space(4.0);

    // ── Throughput sparkline ──────────────────────────────────────────────────
    if active.throughput_samples.len() >= 2 {
        let job_start = active.started_at;
        let samples: Vec<_> = active.throughput_samples.iter().collect();
        let points: Vec<[f64; 2]> = samples
            .windows(2)
            .map(|w| {
                let dt = w[1].0.duration_since(w[0].0).as_secs_f64().max(0.001);
                let bytes = w[1].1.saturating_sub(w[0].1);
                let mbps = bytes as f64 / dt / 1_048_576.0;
                let t = w[1].0.duration_since(job_start).as_secs_f64();
                [t, mbps]
            })
            .collect();

        if !points.is_empty() {
            Plot::new("throughput_plot")
                .height(55.0)
                .allow_zoom(false)
                .allow_drag(false)
                .allow_scroll(false)
                .show_axes([false, true])
                .show_grid([false, true])
                .label_formatter(|_, v| format!("{:.1} MB/s", v.y))
                .show(ui, |plot_ui| {
                    plot_ui.line(
                        Line::new("MB/s", PlotPoints::new(points))
                            .color(Palette::ACCENT),
                    );
                });
        }
    }

    // ── Recovery passes ───────────────────────────────────────────────────────
    if let Some(pass) = p.recovery_pass {
        ui.collapsing("Recovery passes", |ui| {
            if p.total_bytes > 0 {
                let frac = p.bytes_done as f32 / p.total_bytes as f32;
                ui.label(egui::RichText::new(format!("Pass: {pass}")).small().color(Palette::TEXT_DIM));
                ui.add(
                    egui::ProgressBar::new(frac)
                        .desired_height(8.0)
                        .text(format!(
                            "Done: {}  Bad: {}",
                            format_bytes(p.bytes_done),
                            format_bytes(p.recovery_bad_bytes),
                        )),
                );
            }
        });
    }

    ui.add_space(6.0);

    // ── Cancel button (right-aligned) ─────────────────────────────────────────
    let mut cancelled = false;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
        let btn = egui::Button::new(
            egui::RichText::new(format!("{} Cancel", icons::CANCEL)).color(Palette::DANGER),
        )
        .stroke(egui::Stroke::new(1.0, Palette::DANGER))
        .fill(Palette::SURFACE);
        if ui.add(btn).clicked() {
            cancelled = true;
        }
    });
    cancelled
}
