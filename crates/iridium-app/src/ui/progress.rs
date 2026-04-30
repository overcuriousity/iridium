use egui::Ui;

use crate::state::{ActiveJob, CompletedJob, JobOutcome};

/// Shows the active job's live progress.
/// Returns `true` if the Cancel button was clicked this frame.
pub fn show_active(ui: &mut Ui, active: &ActiveJob) -> bool {
    ui.heading("Active job");
    ui.label(format!("Source: {}", active.spec.source.path.display()));
    ui.label(format!("Output: {}", active.spec.dest_path.display()));

    let p = &active.progress;
    if p.total_bytes > 0 {
        let frac = p.bytes_done as f32 / p.total_bytes as f32;
        let label = format!(
            "{} / {} ({:.1}%)",
            format_bytes(p.bytes_done),
            format_bytes(p.total_bytes),
            frac * 100.0,
        );
        ui.add(egui::ProgressBar::new(frac).text(label));
    } else {
        ui.add(egui::ProgressBar::new(0.0).text("Waiting…").animate(true));
    }

    if p.bad_chunks > 0 {
        ui.colored_label(
            egui::Color32::from_rgb(255, 165, 0),
            format!("Bad chunks (zero-filled): {}", p.bad_chunks),
        );
    }

    if let Some(pass) = p.recovery_pass {
        ui.label(format!("Recovery pass: {pass}"));
        if p.bytes_done > 0 && p.total_bytes > 0 {
            let frac = p.bytes_done as f32 / p.total_bytes as f32;
            ui.add(egui::ProgressBar::new(frac).text(format!(
                "Recovered: {} bad: {}",
                format_bytes(p.bytes_done),
                format_bytes(p.recovery_bad_bytes),
            )));
        }
    }

    if p.verifying {
        let frac = if p.total_bytes > 0 {
            p.verify_bytes_done as f32 / p.total_bytes as f32
        } else {
            0.0
        };
        ui.add(
            egui::ProgressBar::new(frac)
                .text(format!("Verifying: {:.1}%", frac * 100.0))
                .animate(true),
        );
    }

    let cancelled = ui.button("Cancel").clicked();
    if cancelled {
        active.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    cancelled
}

/// Shows a single completed job entry.
pub fn show_completed(ui: &mut Ui, job: &CompletedJob) {
    let header = format!("{} → {}", job.spec.source.path.display(), job.spec.dest_path.display());
    egui::CollapsingHeader::new(&header)
        .default_open(false)
        .show(ui, |ui| match &job.outcome {
            Err(e) => {
                ui.colored_label(egui::Color32::RED, format!("Error: {e}"));
            }
            Ok(JobOutcome::Cancelled { bytes_done }) => {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    format!("Cancelled after {}", format_bytes(*bytes_done)),
                );
            }
            Ok(JobOutcome::Completed { digests, bytes_processed, bad_chunks, verified }) => {
                ui.colored_label(egui::Color32::GREEN, "Completed");
                ui.label(format!("Bytes imaged: {}", format_bytes(*bytes_processed)));
                if *bad_chunks > 0 {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 165, 0),
                        format!("Bad chunks: {bad_chunks}"),
                    );
                }
                for d in digests {
                    ui.monospace(format!("{:?}: {}", d.algorithm, d.hex));
                }
                if *verified {
                    ui.colored_label(egui::Color32::GREEN, "Verified ✓");
                }
            }
            Ok(JobOutcome::Recovery { result, verified }) => {
                let color = if result.complete {
                    egui::Color32::GREEN
                } else {
                    egui::Color32::YELLOW
                };
                ui.colored_label(
                    color,
                    if result.complete { "Recovery complete" } else { "Recovery cancelled" },
                );
                ui.label(format!(
                    "Finished: {}  Bad: {}",
                    format_bytes(result.finished_bytes),
                    format_bytes(result.bad_bytes),
                ));
                ui.label(format!("Mapfile: {}", result.mapfile_path.display()));
                for d in &result.digests {
                    ui.monospace(format!("{:?}: {}", d.algorithm, d.hex));
                }
                if *verified {
                    ui.colored_label(egui::Color32::GREEN, "Verified ✓");
                }
            }
        });
}

fn format_bytes(bytes: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;
    const KIB: u64 = 1024;
    if bytes >= GIB {
        format!("{:.2} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.2} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.2} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}
