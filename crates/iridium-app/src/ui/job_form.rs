use iridium_core::{HashAlg, ImageFormat};

use crate::state::AppState;

/// Shows the job-configuration window. Closes on Submit (enqueue) or Cancel.
/// Must be called from within `eframe::App::update()`.
pub fn show(ctx: &egui::Context, state: &mut AppState) {
    let Some(spec) = state.pending_job_form.as_mut() else {
        return;
    };

    let mut submit = false;
    let mut cancel_form = false;

    egui::Window::new("New imaging job")
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(format!("Source: {}", spec.source.path.display()));
            ui.separator();

            // Destination path
            ui.horizontal(|ui| {
                ui.label("Output path (no extension):");
                let mut path_str = spec.dest_path.to_string_lossy().into_owned();
                if ui.text_edit_singleline(&mut path_str).changed() {
                    spec.dest_path = std::path::PathBuf::from(&path_str);
                }
            });

            // Output format (Raw | EWF; AFF hidden per ADR 0003)
            ui.horizontal(|ui| {
                ui.label("Format:");
                egui::ComboBox::from_id_salt("format_select")
                    .selected_text(format_label(spec.format))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut spec.format, ImageFormat::Raw, "Raw (.img)");
                        ui.selectable_value(&mut spec.format, ImageFormat::Ewf, "EWF (.E01)");
                    });
            });

            // Hash algorithms
            ui.label("Hash algorithms:");
            ui.horizontal(|ui| {
                for alg in [HashAlg::Md5, HashAlg::Sha1, HashAlg::Sha256] {
                    let mut enabled = spec.algorithms.contains(&alg);
                    if ui.checkbox(&mut enabled, alg_label(alg)).changed() {
                        if enabled {
                            if !spec.algorithms.contains(&alg) {
                                spec.algorithms.push(alg);
                            }
                        } else {
                            spec.algorithms.retain(|a| *a != alg);
                        }
                    }
                }
            });
            if spec.algorithms.is_empty() {
                ui.colored_label(egui::Color32::RED, "Select at least one algorithm.");
            }

            // Chunk size
            ui.horizontal(|ui| {
                ui.label("Chunk size (bytes):");
                let mut chunk_str = spec.chunk_size.to_string();
                if ui.text_edit_singleline(&mut chunk_str).changed() {
                    if let Ok(v) = chunk_str.parse::<usize>() {
                        if v > 0 {
                            spec.chunk_size = v;
                        }
                    }
                }
            });

            // Recovery mode toggle
            ui.checkbox(&mut spec.recovery_mode, "Recovery mode (dd_rescue-style)");
            if spec.recovery_mode {
                ui.indent("mapfile_row", |ui| {
                    ui.label("Mapfile path (blank = <dest>.map):");
                    let mut mp = spec
                        .mapfile_path
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if ui.text_edit_singleline(&mut mp).changed() {
                        spec.mapfile_path = if mp.is_empty() {
                            None
                        } else {
                            Some(std::path::PathBuf::from(mp))
                        };
                    }
                });
            }

            // Verify after acquire
            ui.checkbox(&mut spec.verify_after, "Verify after acquisition");

            let dest_ok = !spec.dest_path.as_os_str().is_empty()
                && spec.dest_path.parent().is_some();
            if !dest_ok {
                ui.colored_label(egui::Color32::RED, "Output path is required.");
            }

            ui.separator();
            ui.horizontal(|ui| {
                let ready = !spec.algorithms.is_empty() && dest_ok;
                if ui.add_enabled(ready, egui::Button::new("Start")).clicked() {
                    submit = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel_form = true;
                }
            });
        });

    if submit {
        let spec = state.pending_job_form.take().unwrap();
        if let Some(parent) = spec.dest_path.parent() {
            state.config.last_output_dir = Some(parent.to_path_buf());
            let _ = crate::config::save(&state.config);
        }
        state.pending.push_back(spec);
        state.show_job_form = false;
        if state.active.is_none() {
            crate::worker::start_next(state, ctx);
        }
    } else if cancel_form {
        state.pending_job_form = None;
        state.show_job_form = false;
    }
}

fn format_label(f: ImageFormat) -> &'static str {
    match f {
        ImageFormat::Raw => "Raw (.img)",
        ImageFormat::Ewf => "EWF (.E01)",
        ImageFormat::Aff => "AFF (unsupported)",
    }
}

fn alg_label(a: HashAlg) -> &'static str {
    match a {
        HashAlg::Md5 => "MD5",
        HashAlg::Sha1 => "SHA-1",
        HashAlg::Sha256 => "SHA-256",
    }
}
