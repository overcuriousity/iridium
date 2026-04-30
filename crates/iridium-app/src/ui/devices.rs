use egui::Ui;
use iridium_device::Disk;

use crate::state::AppState;

pub fn show(ui: &mut Ui, state: &mut AppState) {
    ui.heading("Devices");
    ui.horizontal(|ui| {
        if ui.button("Refresh").clicked() {
            state.refresh_devices();
        }
    });

    if let Some(err) = &state.device_error {
        ui.colored_label(egui::Color32::RED, format!("Enumeration error: {err}"));
    }

    egui::ScrollArea::vertical()
        .id_salt("device_scroll")
        .show(ui, |ui| {
            for (idx, disk) in state.devices.iter().enumerate() {
                let selected = state.selected_device_idx == Some(idx);
                let label = format!(
                    "{} — {} {} ({})",
                    disk.path.display(),
                    disk.model.trim(),
                    disk.serial.trim(),
                    format_bytes(disk.size_bytes),
                );
                if ui.selectable_label(selected, &label).clicked() {
                    state.selected_device_idx = Some(idx);
                }
                if selected {
                    ui.indent("disk_details", |ui| {
                        show_disk_details(ui, disk);
                    });
                }
            }
        });

    ui.separator();

    let can_image = state.selected_device_idx.is_some() && !state.show_job_form;
    if ui
        .add_enabled(can_image, egui::Button::new("Image…"))
        .clicked()
    {
        if let Some(idx) = state.selected_device_idx {
            let disk = state.devices[idx].clone();
            let config = &state.config;
            state.pending_job_form = Some(crate::state::JobSpec {
                source: disk,
                dest_path: config
                    .last_output_dir
                    .clone()
                    .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                    .join("image"),
                format: config.default_format,
                algorithms: config.default_hash_algs.clone(),
                chunk_size: iridium_acquire::DEFAULT_CHUNK_SIZE,
                recovery_mode: false,
                mapfile_path: None,
                verify_after: false,
            });
            state.show_job_form = true;
        }
    }
}

fn show_disk_details(ui: &mut Ui, disk: &Disk) {
    ui.label(format!("Size:       {}", format_bytes(disk.size_bytes)));
    ui.label(format!(
        "Sector:     {} B logical / {} B physical",
        disk.logical_sector_size, disk.sector_size
    ));
    ui.label(format!(
        "Type:       {}{}",
        if disk.rotational { "HDD" } else { "SSD/NVMe" },
        if disk.removable { " (removable)" } else { "" }
    ));
    if disk.read_only {
        ui.colored_label(egui::Color32::YELLOW, "Read-only (write-blocker?)");
    }
    if let Some(hpa) = disk.hpa_size_bytes {
        ui.colored_label(
            egui::Color32::from_rgb(255, 165, 0),
            format!("HPA detected — restricted size: {}", format_bytes(hpa)),
        );
    }
    if disk.dco_restricted {
        ui.colored_label(
            egui::Color32::from_rgb(255, 165, 0),
            "DCO active — device configuration overlay restricts capacity/features",
        );
    }
    if let Some(parent) = &disk.partition_of {
        ui.label(format!("Partition of: {}", parent.display()));
    }
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
