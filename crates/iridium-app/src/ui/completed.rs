use egui::Ui;
use egui_extras::{Column, TableBuilder};

use crate::state::{AppState, CompletedJob, JobOutcome, format_bytes};

use super::theme::{self, Palette, icons};

pub fn show(ui: &mut Ui, state: &mut AppState) {
    if state.completed.is_empty() {
        ui.add_space(8.0);
        ui.label(egui::RichText::new("No completed jobs yet.").color(Palette::TEXT_DIM));
        return;
    }

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("Completed: {} job(s)", state.completed.len()))
                .strong()
                .color(Palette::TEXT_STRONG),
        );
    });
    ui.add_space(4.0);

    let n = state.completed.len();

    TableBuilder::new(ui)
        .striped(true)
        .resizable(false)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(160.0).clip(true)) // When
        .column(Column::initial(90.0).clip(true)) // Source
        .column(Column::remainder().clip(true)) // Dest
        .column(Column::initial(80.0)) // Bytes
        .column(Column::initial(90.0)) // Result
        .header(20.0, |mut header| {
            for label in ["When", "Source", "Destination", "Bytes", "Result"] {
                header.col(|ui| {
                    ui.label(egui::RichText::new(label).small().color(Palette::TEXT_DIM));
                });
            }
        })
        .body(|mut body| {
            // Show most recent first
            for (display_i, real_i) in (0..n).rev().enumerate() {
                let _ = display_i;
                body.row(22.0, |mut row| {
                    let job = &state.completed[real_i];
                    let when = format_when(&job.finished_at);
                    let src = job.spec.source.path.to_string_lossy().into_owned();
                    let dst = job.spec.dest_path.to_string_lossy().into_owned();
                    let bytes = match &job.outcome {
                        Ok(JobOutcome::Completed {
                            bytes_processed, ..
                        }) => format_bytes(*bytes_processed),
                        Ok(JobOutcome::Cancelled { bytes_done }) => {
                            format!("~{}", format_bytes(*bytes_done))
                        }
                        Ok(JobOutcome::Recovery { result, .. }) => {
                            format_bytes(result.finished_bytes)
                        }
                        Err(_) => "—".to_owned(),
                    };

                    row.col(|ui| {
                        ui.label(egui::RichText::new(&when).small().color(Palette::TEXT_DIM));
                    });
                    row.col(|ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&src).font(egui::FontId::monospace(11.0)),
                            )
                            .truncate(),
                        );
                    });
                    row.col(|ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&dst)
                                    .font(egui::FontId::monospace(11.0))
                                    .color(Palette::TEXT_DIM),
                            )
                            .truncate(),
                        );
                    });
                    row.col(|ui| {
                        ui.label(egui::RichText::new(&bytes).font(egui::FontId::monospace(11.0)));
                    });
                    row.col(|ui| {
                        result_chip(ui, job);
                        if ui
                            .interact(
                                ui.min_rect(),
                                egui::Id::new(("completed_row", real_i)),
                                egui::Sense::click(),
                            )
                            .double_clicked()
                        {
                            state.completed_detail_idx = Some(real_i);
                        }
                    });
                });
            }
        });

    // Persistent detail window; stays open until the user closes it.
    if let Some(idx) = state.completed_detail_idx {
        if idx < state.completed.len() {
            let mut open = true;
            egui::Window::new(format!(
                "Job detail — {}",
                state.completed[idx].spec.source.path.display()
            ))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .show(ui.ctx(), |ui| {
                show_job_detail(ui, &state.completed[idx]);
            });
            if !open {
                state.completed_detail_idx = None;
            }
        } else {
            state.completed_detail_idx = None;
        }
    }
}

fn result_chip(ui: &mut Ui, job: &CompletedJob) {
    match &job.outcome {
        Err(e) => {
            theme::chip_danger(ui, "ERROR");
            ui.label(
                egui::RichText::new(e.to_string())
                    .small()
                    .color(Palette::DANGER),
            )
            .on_hover_text(e.to_string());
        }
        Ok(JobOutcome::Cancelled { .. }) => {
            theme::chip_warn(ui, "CANCELLED");
        }
        Ok(JobOutcome::Completed {
            verified,
            bad_chunks,
            ..
        }) => {
            if *bad_chunks > 0 {
                theme::chip_warn(ui, "PARTIAL");
            } else {
                theme::chip_success(ui, "OK");
            }
            if *verified {
                ui.label(icons::CHECK).on_hover_text("Verified ✓");
            }
        }
        Ok(JobOutcome::Recovery { result, verified }) => {
            if result.complete {
                theme::chip_success(ui, "RECOVERED");
            } else {
                theme::chip_warn(ui, "PARTIAL");
            }
            if *verified {
                ui.label(icons::CHECK).on_hover_text("Verified ✓");
            }
        }
    }
}

fn show_job_detail(ui: &mut Ui, job: &CompletedJob) {
    egui::Grid::new("job_detail_grid")
        .num_columns(2)
        .spacing([8.0, 3.0])
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new("Source")
                    .color(Palette::TEXT_DIM)
                    .small(),
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new(job.spec.source.path.to_string_lossy().as_ref())
                        .font(egui::FontId::monospace(12.0)),
                )
                .truncate(),
            );
            ui.end_row();

            ui.label(egui::RichText::new("Dest").color(Palette::TEXT_DIM).small());
            ui.add(
                egui::Label::new(
                    egui::RichText::new(job.spec.dest_path.to_string_lossy().as_ref())
                        .font(egui::FontId::monospace(12.0)),
                )
                .truncate(),
            );
            ui.end_row();
        });

    ui.add_space(4.0);

    match &job.outcome {
        Err(e) => {
            theme::chip_danger(ui, "ERROR");
            ui.label(egui::RichText::new(e.to_string()).color(Palette::DANGER));
        }
        Ok(JobOutcome::Cancelled { bytes_done }) => {
            theme::chip_warn(ui, "CANCELLED");
            ui.label(format!("Stopped after {}", format_bytes(*bytes_done)));
        }
        Ok(JobOutcome::Completed {
            digests,
            bytes_processed,
            bad_chunks,
            verified,
        }) => {
            theme::chip_success(ui, "COMPLETED");
            ui.label(format!("{} imaged", format_bytes(*bytes_processed)));
            if *bad_chunks > 0 {
                theme::chip_warn(ui, &format!("{bad_chunks} bad chunks"));
            }
            ui.add_space(4.0);
            for d in digests {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{:?}", d.algorithm))
                            .small()
                            .color(Palette::TEXT_DIM),
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&d.hex).font(egui::FontId::monospace(11.0)),
                        )
                        .truncate(),
                    );
                });
            }
            if *verified {
                ui.horizontal(|ui| {
                    theme::chip_success(ui, "VERIFIED");
                });
            }
        }
        Ok(JobOutcome::Recovery { result, verified }) => {
            if result.complete {
                theme::chip_success(ui, "RECOVERY COMPLETE");
            } else {
                theme::chip_warn(ui, "RECOVERY PARTIAL");
            }
            ui.label(format!(
                "Done: {}  Bad: {}",
                format_bytes(result.finished_bytes),
                format_bytes(result.bad_bytes),
            ));
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("Mapfile: {}", result.mapfile_path.display()))
                        .font(egui::FontId::monospace(11.0))
                        .color(Palette::TEXT_DIM),
                )
                .truncate(),
            );
            ui.add_space(4.0);
            for d in &result.digests {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{:?}", d.algorithm))
                            .small()
                            .color(Palette::TEXT_DIM),
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&d.hex).font(egui::FontId::monospace(11.0)),
                        )
                        .truncate(),
                    );
                });
            }
            if *verified {
                theme::chip_success(ui, "VERIFIED");
            }
        }
    }
}

fn format_when(t: &std::time::SystemTime) -> String {
    // UTC date+time so jobs from different days remain distinguishable. The
    // `time` crate is already in the workspace; falling back to a numeric
    // unix-seconds rendering keeps the column populated if formatting ever
    // fails (which it shouldn't for well-known descriptions).
    let dt: time::OffsetDateTime = (*t).into();
    let fmt =
        match time::format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second]Z") {
            Ok(f) => f,
            Err(_) => return "—".to_owned(),
        };
    dt.format(&fmt).unwrap_or_else(|_| "—".to_owned())
}
