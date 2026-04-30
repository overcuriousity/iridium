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

    state.active = Some(ActiveJob {
        spec,
        cancel,
        progress_rx,
        handle: Some(handle),
        progress: ProgressSnapshot::default(),
    });
}

fn run_job(
    spec: JobSpec,
    cancel: Arc<AtomicBool>,
    progress_tx: Sender<ProgressEvent>,
    egui_ctx: egui::Context,
) -> Result<JobOutcome, AppError> {
    // Build the audit log for this job.
    let audit_path = spec.dest_path.with_extension("jsonl");
    let audit = match iridium_audit::Log::open(&audit_path) {
        Ok(log) => Some(Arc::new(log)),
        Err(e) => {
            log::warn!("could not open audit log {audit_path:?}: {e}");
            None
        }
    };

    if spec.recovery_mode {
        let outcome = run_recovery_job(&spec, cancel, progress_tx.clone(), audit.clone())?;
        egui_ctx.request_repaint();

        // seal the audit log
        seal_audit(audit);

        return Ok(outcome);
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
            iridium_acquire::run_ewf(job)?
        }
        _ => iridium_acquire::run(job)?,
    };

    egui_ctx.request_repaint();

    if !result.complete {
        seal_audit(audit);
        return Ok(JobOutcome::Cancelled { bytes_done: result.bytes_processed });
    }

    let verified = if spec.verify_after {
        let image_path = image_file_path(&spec);
        match verify::verify_image(
            &image_path,
            spec.format,
            &spec.algorithms,
            &result.digests,
            |_, _| {},
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
    job.progress_tx = Some(progress_tx);
    job.audit = audit;
    // recovery always writes Raw
    job.format = Some(ImageFormat::Raw);

    let mut opts = RecoveryOptions::default();
    opts.chunk_size = spec.chunk_size;
    opts.mapfile_path = spec.mapfile_path.clone();

    let result = iridium_recovery::run_recovery(job, opts)?;

    let verified = if spec.verify_after && result.complete {
        let image_path = image_file_path(spec);
        match verify::verify_image(
            &image_path,
            ImageFormat::Raw,
            &spec.algorithms,
            &result.digests,
            |_, _| {},
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
    if let Some(arc) = audit {
        if let Ok(log) = Arc::try_unwrap(arc) {
            if let Err(e) = log.seal() {
                log::warn!("audit seal failed: {e}");
            }
        }
    }
}

/// The actual file written on disk (with extension).
fn image_file_path(spec: &JobSpec) -> PathBuf {
    match spec.format {
        ImageFormat::Ewf => spec.dest_path.clone(), // libewf appends .E01 internally
        _ => {
            let mut p = spec.dest_path.clone();
            let name = p
                .file_name()
                .map(|n| format!("{}.img", n.to_string_lossy()))
                .unwrap_or_else(|| "image.img".into());
            p.set_file_name(name);
            p
        }
    }
}
