// Background worker: spawns one thread per imaging job.

use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

use crossbeam_channel::Sender;
use iridium_acquire::{AcquireJob, ProgressEvent};
use iridium_core::ImageFormat;
use iridium_recovery::RecoveryOptions;

use crate::{
    error::AppError,
    state::{ActiveJob, AppState, JobOutcome, JobSpec, ProgressSnapshot},
    verify,
};

/// Dequeue the next pending job and start its worker thread.
/// Must only be called when `state.active` is `None`.
pub fn start_next(state: &mut AppState, egui_ctx: &egui::Context) {
    let Some(spec) = state.pending.pop_front() else {
        return;
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let (progress_tx, progress_rx) = crossbeam_channel::unbounded::<ProgressEvent>();
    let ctx = egui_ctx.clone();

    let worker_spec = spec.clone();
    let worker_cancel = Arc::clone(&cancel);

    let handle = std::thread::spawn(move || run_job(worker_spec, worker_cancel, progress_tx, ctx));

    // Point the audit dock at the log file this job will write.
    state.audit_path = Some(audit_path_for(&spec));

    state.active = Some(ActiveJob {
        spec,
        cancel,
        progress_rx,
        handle: Some(handle),
        progress: ProgressSnapshot::default(),
        throughput_samples: std::collections::VecDeque::new(),
        ewma_bps: 0.0,
        started_at: std::time::Instant::now(),
    });
}

fn run_job(
    spec: JobSpec,
    cancel: Arc<AtomicBool>,
    progress_tx: Sender<ProgressEvent>,
    egui_ctx: egui::Context,
) -> Result<JobOutcome, AppError> {
    // Build the audit log for this job.
    let audit_path = audit_path_for(&spec);
    let audit = match iridium_audit::Log::open(&audit_path) {
        Ok(log) => Some(Arc::new(log)),
        Err(e) => {
            log::warn!("could not open audit log {audit_path:?}: {e}");
            None
        }
    };

    if spec.recovery_mode {
        let outcome = run_recovery_job(&spec, cancel, progress_tx.clone(), audit.clone());
        egui_ctx.request_repaint();
        seal_audit(audit);
        return outcome;
    }

    // Normal acquisition.
    let mut job = AcquireJob::new(
        spec.source.clone(),
        spec.dest_path.clone(),
        spec.algorithms.clone(),
    );
    job.chunk_size = spec.chunk_size;
    job.cancel = cancel;
    job.progress_tx = Some(progress_tx.clone());
    job.audit = audit.clone();

    let result = match spec.format {
        ImageFormat::Ewf => {
            job.format = Some(ImageFormat::Ewf);
            iridium_acquire::run_ewf(job)
        }
        _ => iridium_acquire::run(job),
    };

    egui_ctx.request_repaint();

    let result = match result {
        Ok(r) => r,
        Err(e) => {
            seal_audit(audit);
            return Err(e.into());
        }
    };

    if !result.complete {
        seal_audit(audit);
        return Ok(JobOutcome::Cancelled {
            bytes_done: result.bytes_processed,
        });
    }

    let verified = if spec.verify_after {
        let image_path = image_file_path(&spec);
        let tx = progress_tx.clone();
        match verify::verify_image(
            &image_path,
            spec.format,
            &spec.algorithms,
            &result.digests,
            move |done, total| {
                let _ = tx.send(ProgressEvent::VerifyProgress {
                    bytes_done: done,
                    total_bytes: total,
                });
            },
        ) {
            Ok(()) => true,
            Err(e) => {
                log::error!("verify failed: {e}");
                seal_audit(audit);
                return Err(AppError::Verify(e));
            }
        }
    } else {
        false
    };

    seal_audit(audit);

    Ok(JobOutcome::Completed {
        digests: result.digests,
        bytes_processed: result.bytes_processed,
        bad_chunks: result.bad_chunks,
        verified,
    })
}

fn run_recovery_job(
    spec: &JobSpec,
    cancel: Arc<AtomicBool>,
    progress_tx: Sender<ProgressEvent>,
    audit: Option<Arc<iridium_audit::Log>>,
) -> Result<JobOutcome, AppError> {
    let mut job = AcquireJob::new(
        spec.source.clone(),
        spec.dest_path.clone(),
        spec.algorithms.clone(),
    );
    job.chunk_size = spec.chunk_size;
    job.cancel = cancel;
    job.progress_tx = Some(progress_tx.clone());
    job.audit = audit;
    // recovery always writes Raw
    job.format = Some(ImageFormat::Raw);

    let mut opts = RecoveryOptions::default();
    opts.chunk_size = spec.chunk_size;
    opts.mapfile_path = spec.mapfile_path.clone();

    let result = iridium_recovery::run_recovery(job, opts)?;

    let verified = if spec.verify_after && result.complete {
        let image_path = image_file_path(spec);
        let tx = progress_tx.clone();
        match verify::verify_image(
            &image_path,
            ImageFormat::Raw,
            &spec.algorithms,
            &result.digests,
            move |done, total| {
                let _ = tx.send(ProgressEvent::VerifyProgress {
                    bytes_done: done,
                    total_bytes: total,
                });
            },
        ) {
            Ok(()) => true,
            Err(e) => {
                log::error!("verify failed after recovery: {e}");
                return Err(AppError::Verify(e));
            }
        }
    } else {
        false
    };

    Ok(JobOutcome::Recovery { result, verified })
}

fn seal_audit(audit: Option<Arc<iridium_audit::Log>>) {
    let Some(arc) = audit else { return };
    match Arc::into_inner(arc) {
        Some(log) => {
            if let Err(e) = log.seal() {
                log::warn!("audit seal failed: {e}");
            }
        }
        None => {
            // Another clone outlived the worker — should not happen with the
            // current pipeline contract (pipeline drops its `job.audit` before
            // returning). Log so a regression here is visible.
            log::warn!("audit seal skipped: outstanding Arc references prevent unwrap");
        }
    }
}

/// JSONL audit log path for a job (sibling of dest_path).
fn audit_path_for(spec: &JobSpec) -> PathBuf {
    spec.dest_path.with_extension("jsonl")
}

/// The actual file written on disk (with extension). Must mirror what
/// `iridium_acquire::writer::RawWriter::create` and the EWF writer produce.
pub(crate) fn image_file_path(spec: &JobSpec) -> PathBuf {
    match spec.format {
        // libewf appends `.E01` to `dest_path` (which has no extension).
        ImageFormat::Ewf => spec.dest_path.with_extension("E01"),
        // RawWriter does `dest_path.with_extension("img")`; mirror that exactly.
        _ => spec.dest_path.with_extension("img"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use iridium_core::{HashAlg, ImageFormat};
    use iridium_device::Disk;

    use super::*;

    fn spec_with(dest: &str, format: ImageFormat) -> JobSpec {
        JobSpec {
            source: Disk {
                path: PathBuf::from("/dev/loop0"),
                model: String::new(),
                serial: String::new(),
                size_bytes: 0,
                logical_sector_size: 512,
                sector_size: 512,
                hpa_size_bytes: None,
                dco_restricted: false,
                removable: false,
                rotational: false,
                read_only: true,
                partition_of: None,
            },
            dest_path: PathBuf::from(dest),
            format,
            algorithms: vec![HashAlg::Sha256],
            chunk_size: 1024 * 1024,
            recovery_mode: false,
            mapfile_path: None,
            verify_after: false,
        }
    }

    #[test]
    fn raw_image_path_appends_img() {
        let s = spec_with("/case/evidence/image", ImageFormat::Raw);
        assert_eq!(image_file_path(&s), PathBuf::from("/case/evidence/image.img"));
    }

    #[test]
    fn ewf_image_path_appends_e01() {
        let s = spec_with("/case/evidence/image", ImageFormat::Ewf);
        assert_eq!(image_file_path(&s), PathBuf::from("/case/evidence/image.E01"));
    }

    #[test]
    fn audit_path_replaces_extension() {
        let s = spec_with("/case/evidence/image", ImageFormat::Raw);
        assert_eq!(audit_path_for(&s), PathBuf::from("/case/evidence/image.jsonl"));
    }
}
