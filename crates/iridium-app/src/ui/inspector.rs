use std::path::PathBuf;
use std::sync::Arc;

use egui::Ui;
use iridium_core::{HashAlg, ImageFormat};

use crate::state::{AppState, ChunkUnit, InspectorMode, JobSpec, format_bytes};

use super::theme::{self, Palette, icons, section_heading};

pub fn show(ui: &mut Ui, state: &mut AppState) {
    match state.inspector_mode {
        InspectorMode::DeviceDetail => show_device_detail(ui, state),
        InspectorMode::NewJob => show_job_form(ui, state),
    }
}

// ── Device detail view ────────────────────────────────────────────────────────

fn show_device_detail(ui: &mut Ui, state: &mut AppState) {
    let Some(idx) = state.selected_device_idx else {
        ui.add_space(16.0);
        ui.label(egui::RichText::new("Select a device from the list").color(Palette::TEXT_DIM));
        return;
    };

    let disk = state.devices[idx].clone();

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(icons::DEVICE_HDD).size(18.0));
        ui.add(
            egui::Label::new(
                egui::RichText::new(disk.path.to_string_lossy().as_ref())
                    .font(egui::FontId::monospace(13.0))
                    .strong()
                    .color(Palette::TEXT_STRONG),
            )
            .truncate(),
        );
    });

    ui.add_space(2.0);

    if !disk.model.trim().is_empty() {
        ui.label(egui::RichText::new(disk.model.trim()).color(Palette::TEXT_DIM));
    }
    if !disk.serial.trim().is_empty() {
        ui.add(egui::Label::new(
            egui::RichText::new(format!("S/N: {}", disk.serial.trim()))
                .font(egui::FontId::monospace(11.0))
                .color(Palette::TEXT_DIM),
        ));
    }

    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);

    egui::Grid::new("disk_detail_grid")
        .num_columns(2)
        .spacing([8.0, 3.0])
        .show(ui, |ui| {
            kv(ui, "Size", &format_bytes(disk.size_bytes));
            kv(
                ui,
                "Sectors",
                &format!(
                    "{} B logical / {} B physical",
                    disk.logical_sector_size, disk.sector_size
                ),
            );
            kv(
                ui,
                "Type",
                if disk.rotational {
                    "HDD (rotational)"
                } else if disk.removable {
                    "Removable / Flash"
                } else {
                    "SSD / NVMe"
                },
            );
            kv(ui, "Removable", if disk.removable { "yes" } else { "no" });
            kv(
                ui,
                "Read-only",
                if disk.read_only {
                    "yes (write-blocker?)"
                } else {
                    "no"
                },
            );

            if let Some(hpa) = disk.hpa_size_bytes {
                ui.label(egui::RichText::new("HPA").color(Palette::WARN).strong());
                ui.horizontal(|ui| {
                    theme::chip_warn(ui, "DETECTED");
                    ui.label(
                        egui::RichText::new(format!("restricted to {}", format_bytes(hpa)))
                            .color(Palette::WARN)
                            .small(),
                    );
                });
                ui.end_row();
            }

            if disk.dco_restricted {
                ui.label(egui::RichText::new("DCO").color(Palette::WARN).strong());
                ui.horizontal(|ui| {
                    theme::chip_warn(ui, "ACTIVE");
                    ui.label(
                        egui::RichText::new("device configuration overlay")
                            .color(Palette::WARN)
                            .small(),
                    );
                });
                ui.end_row();
            }

            if let Some(parent) = &disk.partition_of {
                kv(ui, "Partition of", &parent.to_string_lossy());
            }
        });

    ui.add_space(8.0);

    let btn = egui::Button::new(
        egui::RichText::new(format!("{} Image this device…", icons::PLAY))
            .color(Palette::ACCENT_TEXT),
    )
    .fill(Palette::ACCENT)
    .stroke(egui::Stroke::new(1.0, Palette::ACCENT));
    if ui.add(btn).clicked() {
        let dest_path = state
            .config
            .last_output_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("image");
        state.pending_job_form = Some(JobSpec {
            source: disk,
            dest_path,
            format: state.config.default_format,
            algorithms: state.config.default_hash_algs.clone(),
            chunk_size: iridium_acquire::DEFAULT_CHUNK_SIZE,
            recovery_mode: false,
            mapfile_path: None,
            verify_after: false,
        });
        state.show_job_form = true;
        state.inspector_mode = InspectorMode::NewJob;
    }
}

fn kv(ui: &mut Ui, key: &str, value: &str) {
    ui.label(egui::RichText::new(key).color(Palette::TEXT_DIM).small());
    ui.add(
        egui::Label::new(
            egui::RichText::new(value)
                .font(egui::FontId::monospace(12.0))
                .color(Palette::TEXT_STRONG),
        )
        .truncate(),
    );
    ui.end_row();
}

// ── New job wizard ────────────────────────────────────────────────────────────

fn show_job_form(ui: &mut Ui, state: &mut AppState) {
    if state.pending_job_form.is_none() {
        state.inspector_mode = InspectorMode::DeviceDetail;
        return;
    }

    // Clone spec so we can freely access other parts of state inside egui closures.
    // Written back at end of this function.
    let mut spec = state.pending_job_form.as_ref().unwrap().clone();
    let mut cancel_form = false;
    let mut submit = false;

    ui.horizontal(|ui| {
        ui.add(egui::Label::new(
            egui::RichText::new("New imaging job")
                .strong()
                .color(Palette::TEXT_STRONG),
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button(egui::RichText::new(icons::X).color(Palette::TEXT_DIM))
                .on_hover_text("Cancel")
                .clicked()
            {
                cancel_form = true;
            }
        });
    });

    ui.add_space(2.0);
    ui.separator();

    egui::ScrollArea::vertical()
        .id_salt("job_form_scroll")
        .show(ui, |ui| {
            // ── Source ──────────────────────────────────────────────────────
            section_heading(ui, "SOURCE");
            egui::Frame::new()
                .fill(Palette::SURFACE_ALT)
                .corner_radius(4.0)
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(icons::DEVICE_HDD));
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(spec.source.path.to_string_lossy().as_ref())
                                    .font(egui::FontId::monospace(12.0))
                                    .strong(),
                            )
                            .truncate(),
                        );
                    });
                    ui.label(
                        egui::RichText::new(format!(
                            "{} — {}",
                            spec.source.model.trim(),
                            format_bytes(spec.source.size_bytes)
                        ))
                        .small()
                        .color(Palette::TEXT_DIM),
                    );
                });

            // ── Destination ─────────────────────────────────────────────────
            section_heading(ui, "DESTINATION");

            // Check if file dialog just delivered a result
            if let Ok(mut guard) = state.file_dialog_slot.try_lock()
                && let Some(path) = guard.take()
            {
                state.file_dialog_open = false;
                spec.dest_path = path;
            }

            let file_dialog_open = state.file_dialog_open;
            let browse_btn = egui::Button::new(egui::RichText::new(format!(
                "{} Browse…",
                icons::FOLDER_OPEN
            )))
            .fill(Palette::SURFACE)
            .stroke(egui::Stroke::new(1.0, Palette::SEPARATOR));

            if ui.add(browse_btn).clicked() && !file_dialog_open {
                state.file_dialog_open = true;
                let slot = Arc::clone(&state.file_dialog_slot);
                let ctx = ui.ctx().clone();
                let initial_dir = spec.dest_path.parent().map(|p| p.to_path_buf());
                let stem = spec
                    .dest_path
                    .file_name()
                    .map(|n| n.to_os_string())
                    .unwrap_or_else(|| std::ffi::OsString::from("image"));
                let current_dest_path = spec.dest_path.clone();
                std::thread::spawn(move || {
                    // rfd with xdg-portal+tokio features requires a Tokio reactor.
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("tokio rt");
                    let chosen = rt.block_on(async {
                        let mut dialog =
                            rfd::AsyncFileDialog::new().set_title("Select output folder");
                        if let Some(dir) = initial_dir {
                            dialog = dialog.set_directory(dir);
                        }
                        dialog
                            .pick_folder()
                            .await
                            .map(|fh| fh.path().join(&stem))
                            .unwrap_or(current_dest_path)
                    });
                    *slot.lock().unwrap() = Some(chosen);
                    ctx.request_repaint();
                });
            }

            let mut path_str = spec.dest_path.to_string_lossy().into_owned();
            if ui
                .add(
                    egui::TextEdit::singleline(&mut path_str)
                        .font(egui::FontId::monospace(12.0))
                        .hint_text("/case/evidence/image")
                        .desired_width(f32::INFINITY),
                )
                .changed()
            {
                spec.dest_path = PathBuf::from(&path_str);
            }

            let dest_ok =
                !spec.dest_path.as_os_str().is_empty() && spec.dest_path.parent().is_some();
            if !dest_ok {
                ui.horizontal(|ui| {
                    theme::chip_danger(ui, "REQUIRED");
                    ui.label(
                        egui::RichText::new("Output path is required")
                            .small()
                            .color(Palette::DANGER),
                    );
                });
            }

            // ── Format ──────────────────────────────────────────────────────
            section_heading(ui, "FORMAT");
            if spec.recovery_mode {
                // Recovery always writes raw output; override any stale selection.
                spec.format = ImageFormat::Raw;
                ui.label(
                    egui::RichText::new("Raw (recovery always writes raw)")
                        .small()
                        .color(Palette::TEXT_DIM),
                );
            } else {
                theme::segmented(
                    ui,
                    &mut spec.format,
                    &[("Raw", ImageFormat::Raw), ("EWF/E01", ImageFormat::Ewf)],
                );
            }

            // ── Hashing ─────────────────────────────────────────────────────
            section_heading(ui, "HASHING");
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for alg in [HashAlg::Md5, HashAlg::Sha1, HashAlg::Sha256] {
                    let label = match alg {
                        HashAlg::Md5 => "MD5",
                        HashAlg::Sha1 => "SHA-1",
                        HashAlg::Sha256 => "SHA-256",
                    };
                    let mut enabled = spec.algorithms.contains(&alg);
                    theme::hash_chip(ui, label, &mut enabled);
                    if enabled && !spec.algorithms.contains(&alg) {
                        spec.algorithms.push(alg);
                    } else if !enabled {
                        spec.algorithms.retain(|a| *a != alg);
                    }
                }
            });
            if spec.algorithms.is_empty() {
                ui.horizontal(|ui| {
                    theme::chip_danger(ui, "REQUIRED");
                    ui.label(
                        egui::RichText::new("Select at least one algorithm")
                            .small()
                            .color(Palette::DANGER),
                    );
                });
            }

            // Chunk size
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Chunk size")
                        .small()
                        .color(Palette::TEXT_DIM),
                );
                ui.add_space(4.0);
                let mib = 1024 * 1024_usize;
                let kib = 1024_usize;
                let (divisor, unit_label) = match state.chunk_unit {
                    ChunkUnit::Kib => (kib, "KiB"),
                    ChunkUnit::Mib => (mib, "MiB"),
                };
                // Round to nearest unit so a chunk_size of 1.5 MiB shown as KiB
                // (1536) and switched back to MiB does not snap silently to 1.
                let mut display_val =
                    ((spec.chunk_size as f64 / divisor as f64).round() as u32).max(1);
                if ui
                    .add(
                        egui::DragValue::new(&mut display_val)
                            .range(1..=1024)
                            .suffix(format!(" {unit_label}")),
                    )
                    .changed()
                {
                    spec.chunk_size = display_val as usize * divisor;
                }
                egui::ComboBox::from_id_salt("chunk_unit")
                    .selected_text(unit_label)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut state.chunk_unit, ChunkUnit::Kib, "KiB");
                        ui.selectable_value(&mut state.chunk_unit, ChunkUnit::Mib, "MiB");
                    });
            });

            // ── Recovery ────────────────────────────────────────────────────
            section_heading(ui, "RECOVERY");
            ui.checkbox(&mut spec.recovery_mode, "Recovery mode (dd_rescue-style)");
            if spec.recovery_mode {
                ui.indent("mapfile_indent", |ui| {
                    ui.label(
                        egui::RichText::new("Mapfile path (blank = <dest>.map)")
                            .small()
                            .color(Palette::TEXT_DIM),
                    );
                    let mut mp = spec
                        .mapfile_path
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if ui.text_edit_singleline(&mut mp).changed() {
                        spec.mapfile_path = if mp.is_empty() {
                            None
                        } else {
                            Some(PathBuf::from(mp))
                        };
                    }
                });
            }

            // ── Verify ──────────────────────────────────────────────────────
            section_heading(ui, "VERIFY");
            ui.checkbox(&mut spec.verify_after, "Verify after acquisition");

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);

            // ── Actions ─────────────────────────────────────────────────────
            let ready = !spec.algorithms.is_empty() && dest_ok;
            ui.horizontal(|ui| {
                let start_btn = egui::Button::new(
                    egui::RichText::new(format!("{} Queue job", icons::PLAY)).color(if ready {
                        Palette::ACCENT_TEXT
                    } else {
                        Palette::TEXT_DIM
                    }),
                )
                .fill(if ready {
                    Palette::ACCENT
                } else {
                    Palette::SURFACE_ALT
                })
                .stroke(egui::Stroke::new(
                    1.0,
                    if ready {
                        Palette::ACCENT
                    } else {
                        Palette::SEPARATOR
                    },
                ));
                if ui.add_enabled(ready, start_btn).clicked() {
                    submit = true;
                }
                let reset_btn = egui::Button::new("Reset")
                    .fill(Palette::SURFACE)
                    .stroke(egui::Stroke::new(1.0, Palette::SEPARATOR));
                if ui.add(reset_btn).clicked() {
                    cancel_form = true;
                }
            });
        });

    // ── Apply mutations after all rendering is done ───────────────────────────
    if cancel_form {
        state.pending_job_form = None;
        state.show_job_form = false;
        state.inspector_mode = InspectorMode::DeviceDetail;
    } else if submit {
        state.pending_job_form = Some(spec);
        state.job_submit_requested = true;
    } else {
        // Write cloned spec back (captures edits made inside closures)
        state.pending_job_form = Some(spec);
    }
}
