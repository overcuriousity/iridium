use std::time::{Duration, Instant};

use egui::Ui;

use crate::state::{AppState, format_bytes};

use super::theme::{Palette, icons};

pub fn show(ui: &mut Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;

        // Device count
        ui.label(egui::RichText::new(icons::DATABASE).color(Palette::TEXT_DIM));
        ui.label(
            egui::RichText::new(format!("{} device(s)", state.devices.len()))
                .small()
                .color(Palette::TEXT_DIM),
        );

        ui.separator();

        // Live throughput
        if let Some(active) = &state.active {
            let mbps = active.ewma_bps / 1_048_576.0;
            if mbps > 0.1 {
                ui.label(
                    egui::RichText::new(format!("{mbps:.1} MB/s"))
                        .small()
                        .font(egui::FontId::monospace(11.0))
                        .color(Palette::ACCENT),
                );
                ui.separator();
            }
            let elapsed = active.started_at.elapsed();
            ui.label(
                egui::RichText::new(format_elapsed(elapsed))
                    .small()
                    .font(egui::FontId::monospace(11.0))
                    .color(Palette::TEXT_DIM),
            );
            ui.separator();
        }

        // Target free space (cached 1s)
        if let Some(dest_dir) = dest_dir(state) {
            let now = Instant::now();
            let stale = state
                .target_free_cache
                .as_ref()
                .map_or(true, |(p, _, t)| p != &dest_dir || now.duration_since(*t) > Duration::from_secs(1));
            if stale {
                if let Ok(free) = free_bytes(&dest_dir) {
                    state.target_free_cache = Some((dest_dir, free, now));
                }
            }
            if let Some((_, free, _)) = &state.target_free_cache {
                ui.label(
                    egui::RichText::new(format!("{} free", format_bytes(*free)))
                        .small()
                        .color(Palette::TEXT_DIM),
                );
                ui.separator();
            }
        }

        // Version (right-aligned)
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(concat!("iridium v", env!("CARGO_PKG_VERSION")))
                    .small()
                    .color(Palette::TEXT_DIM),
            );
        });
    });
}

fn dest_dir(state: &AppState) -> Option<std::path::PathBuf> {
    if let Some(spec) = &state.pending_job_form {
        return spec.dest_path.parent().map(|p| p.to_path_buf());
    }
    if let Some(active) = &state.active {
        return active.spec.dest_path.parent().map(|p| p.to_path_buf());
    }
    state.config.last_output_dir.clone()
}

fn free_bytes(path: &std::path::Path) -> std::io::Result<u64> {
    use std::mem::MaybeUninit;
    let path_c = std::ffi::CString::new(path.to_string_lossy().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let mut stat: MaybeUninit<libc::statvfs64> = MaybeUninit::uninit();
    // SAFETY: path_c is a valid C string, stat is allocated, and statvfs64 is safe to call.
    let ret = unsafe { libc::statvfs64(path_c.as_ptr(), stat.as_mut_ptr()) };
    if ret != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let stat = unsafe { stat.assume_init() };
    Ok(stat.f_bavail * stat.f_frsize)
}

fn format_elapsed(d: Duration) -> String {
    let s = d.as_secs();
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m:02}:{sec:02}")
    }
}
