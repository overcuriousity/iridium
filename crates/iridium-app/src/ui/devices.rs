use egui::Ui;
use egui_extras::{Column, TableBuilder};
use iridium_device::Disk;

use crate::state::{AppState, DeviceCol, InspectorMode, format_bytes};

use super::theme::{self, Palette, icons};

pub fn show(ui: &mut Ui, state: &mut AppState) {
    // Header row with title and controls
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(
            egui::RichText::new("Devices")
                .strong()
                .color(Palette::TEXT_STRONG),
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button(icons::REFRESH)
                .on_hover_text("Refresh device list")
                .clicked()
            {
                state.refresh_devices();
            }
        });
    });

    if let Some(err) = &state.device_error.clone() {
        ui.horizontal(|ui| {
            theme::chip_danger(ui, "ERROR");
            ui.label(egui::RichText::new(err).color(Palette::DANGER).small());
        });
    }

    ui.add_space(2.0);

    let num_devices = state.devices.len();
    if num_devices == 0 {
        ui.add_space(8.0);
        ui.centered_and_justified(|ui| {
            ui.label(egui::RichText::new("No block devices found").color(Palette::TEXT_DIM));
        });
        return;
    }

    // Build sorted index
    let sort_col = state.device_table.sort_col;
    let sort_asc = state.device_table.sort_asc;
    let mut sorted_idx: Vec<usize> = (0..num_devices).collect();
    {
        let devices = &state.devices;
        sorted_idx.sort_by(|&a, &b| {
            let ord = match sort_col {
                DeviceCol::Path => devices[a].path.cmp(&devices[b].path),
                DeviceCol::Model => devices[a].model.cmp(&devices[b].model),
                DeviceCol::Serial => devices[a].serial.cmp(&devices[b].serial),
                DeviceCol::Size => devices[a].size_bytes.cmp(&devices[b].size_bytes),
                DeviceCol::Sector => devices[a].sector_size.cmp(&devices[b].sector_size),
                DeviceCol::Type => devices[a].rotational.cmp(&devices[b].rotational),
                DeviceCol::Flags => {
                    let fa = flag_sort_key(&devices[a]);
                    let fb = flag_sort_key(&devices[b]);
                    fa.cmp(&fb)
                }
            };
            if sort_asc { ord } else { ord.reverse() }
        });
    }

    let selected_idx = state.selected_device_idx;
    let mut new_selected: Option<usize> = None;
    let mut open_job_form: Option<usize> = None;

    // Column sort click accumulators
    let mut sort_clicked: Option<DeviceCol> = None;

    let row_height = 22.0_f32;
    let header_height = 20.0_f32;

    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(85.0).clip(true)) // Path
        .column(Column::remainder().clip(true)) // Model
        .column(Column::initial(90.0).clip(true)) // Size
        .column(Column::initial(24.0)) // Type icon
        .column(Column::initial(70.0)) // Flags
        .header(header_height, |mut header| {
            header.col(|ui| {
                if sort_header(ui, "Path", sort_col == DeviceCol::Path, sort_asc) {
                    sort_clicked = Some(DeviceCol::Path);
                }
            });
            header.col(|ui| {
                if sort_header(ui, "Model / Serial", sort_col == DeviceCol::Model, sort_asc) {
                    sort_clicked = Some(DeviceCol::Model);
                }
            });
            header.col(|ui| {
                if sort_header(ui, "Size", sort_col == DeviceCol::Size, sort_asc) {
                    sort_clicked = Some(DeviceCol::Size);
                }
            });
            header.col(|ui| {
                ui.label("");
            });
            header.col(|ui| {
                ui.label("Flags");
            });
        })
        .body(|body| {
            body.rows(row_height, num_devices, |mut row| {
                let display_idx = row.index();
                let real_idx = sorted_idx[display_idx];

                // Extract row data (immutable borrow of state.devices)
                let (path, model_serial, size_str, type_icon, is_ro, has_hpa, has_dco) = {
                    let disk = &state.devices[real_idx];
                    let path = disk.path.to_string_lossy().into_owned();
                    let model_serial = format!("{} {}", disk.model.trim(), disk.serial.trim());
                    let size_str = format_bytes(disk.size_bytes);
                    let type_icon = device_type_icon(disk);
                    let is_ro = disk.read_only;
                    let has_hpa = disk.hpa_size_bytes.is_some();
                    let has_dco = disk.dco_restricted;
                    (
                        path,
                        model_serial,
                        size_str,
                        type_icon,
                        is_ro,
                        has_hpa,
                        has_dco,
                    )
                };

                let is_selected = selected_idx == Some(real_idx);
                if is_selected {
                    row.set_selected(true);
                }

                row.col(|ui| {
                    let resp = ui.selectable_label(
                        is_selected,
                        egui::RichText::new(&path).font(egui::FontId::monospace(12.0)),
                    );
                    if resp.clicked() {
                        new_selected = Some(real_idx);
                    }
                    resp.context_menu(|ui| {
                        if ui
                            .button(format!("{} Image this device…", icons::PLAY))
                            .clicked()
                        {
                            new_selected = Some(real_idx);
                            open_job_form = Some(real_idx);
                            ui.close();
                        }
                    });
                });
                row.col(|ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&model_serial).color(Palette::TEXT_DIM),
                        )
                        .truncate(),
                    );
                });
                row.col(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(&size_str).font(egui::FontId::monospace(12.0)),
                        );
                    });
                });
                row.col(|ui| {
                    ui.label(type_icon);
                });
                row.col(|ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if is_ro {
                            theme::chip_info(ui, "RO");
                        }
                        if has_hpa {
                            theme::chip_warn(ui, "HPA");
                        }
                        if has_dco {
                            theme::chip_warn(ui, "DCO");
                        }
                    });
                });
            });
        });

    // Apply sort change
    if let Some(col) = sort_clicked {
        if state.device_table.sort_col == col {
            state.device_table.sort_asc = !state.device_table.sort_asc;
        } else {
            state.device_table.sort_col = col;
            state.device_table.sort_asc = true;
        }
    }

    // Apply selection change
    if let Some(idx) = new_selected {
        state.selected_device_idx = Some(idx);
        state.inspector_mode = InspectorMode::DeviceDetail;
    }

    // Context menu "Image…" action
    if let Some(idx) = open_job_form {
        prefill_job_form(state, idx);
    }

    // Bottom action button
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(2.0);
    let can_image = state.selected_device_idx.is_some();
    let btn = egui::Button::new(format!("{} Image…", icons::PLAY))
        .fill(if can_image {
            Palette::ACCENT
        } else {
            Palette::SURFACE_ALT
        })
        .stroke(egui::Stroke::new(
            1.0,
            if can_image {
                Palette::ACCENT
            } else {
                Palette::SEPARATOR
            },
        ));
    let btn_resp = ui.add_enabled(can_image, btn);
    if btn_resp.clicked()
        && let Some(idx) = state.selected_device_idx
    {
        prefill_job_form(state, idx);
    }
}

fn prefill_job_form(state: &mut AppState, idx: usize) {
    let disk = state.devices[idx].clone();
    let dest_path = state
        .config
        .last_output_dir
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("image");
    state.pending_job_form = Some(crate::state::JobSpec {
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

fn sort_header(ui: &mut Ui, label: &str, active: bool, asc: bool) -> bool {
    let text = if active {
        let arrow = if asc { " ↑" } else { " ↓" };
        egui::RichText::new(format!("{label}{arrow}"))
            .strong()
            .color(Palette::ACCENT)
    } else {
        egui::RichText::new(label).color(Palette::TEXT_DIM)
    };
    ui.add(egui::Button::new(text).frame(false)).clicked()
}

fn device_type_icon(disk: &Disk) -> &'static str {
    if disk.removable {
        icons::DEVICE_USB
    } else if disk.rotational {
        icons::DEVICE_HDD
    } else {
        icons::DEVICE_SSD
    }
}

fn flag_sort_key(disk: &Disk) -> u8 {
    let mut k = 0u8;
    if disk.read_only {
        k |= 1;
    }
    if disk.hpa_size_bytes.is_some() {
        k |= 2;
    }
    if disk.dco_restricted {
        k |= 4;
    }
    k
}
