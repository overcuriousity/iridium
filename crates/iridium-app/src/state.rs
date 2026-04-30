use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    thread::JoinHandle,
};

use crossbeam_channel::Receiver;
use iridium_acquire::ProgressEvent;
use iridium_core::{HashAlg, ImageFormat};
use iridium_device::Disk;
use iridium_hash::Digest;
use iridium_recovery::RecoveryResult;

use crate::{config::Config, error::AppError};

// ── Job specification ─────────────────────────────────────────────────────────

/// All user-supplied parameters for one imaging job.
#[derive(Debug, Clone)]
pub struct JobSpec {
    pub source: Disk,
    /// Destination path without extension.
    pub dest_path: PathBuf,
    pub format: ImageFormat,
    pub algorithms: Vec<HashAlg>,
    pub chunk_size: usize,
    /// When true, run `iridium_recovery::run_recovery` instead of the normal pipeline.
    pub recovery_mode: bool,
    pub mapfile_path: Option<PathBuf>,
    /// When true, run a verify pass after a successful acquire.
    pub verify_after: bool,
}

// ── Active job ────────────────────────────────────────────────────────────────

/// Snapshot of progress for the currently running job.
#[derive(Debug, Default, Clone)]
pub struct ProgressSnapshot {
    pub total_bytes: u64,
    pub bytes_done: u64,
    pub bad_chunks: u64,
    /// Non-empty during recovery passes.
    pub recovery_pass: Option<&'static str>,
    pub recovery_bad_bytes: u64,
    /// Set to Some(...) when the verify pass is running.
    pub verify_bytes_done: u64,
    pub verifying: bool,
}

pub struct ActiveJob {
    pub spec: JobSpec,
    pub cancel: Arc<AtomicBool>,
    pub progress_rx: Receiver<ProgressEvent>,
    pub handle: Option<JoinHandle<Result<JobOutcome, AppError>>>,
    pub progress: ProgressSnapshot,
}

// ── Job outcome ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum JobOutcome {
    Completed {
        digests: Vec<Digest>,
        bytes_processed: u64,
        bad_chunks: u64,
        verified: bool,
    },
    Cancelled {
        bytes_done: u64,
    },
    Recovery {
        result: RecoveryResult,
        verified: bool,
    },
}

// ── Completed job record ──────────────────────────────────────────────────────

#[derive(Debug)]
pub struct CompletedJob {
    pub spec: JobSpec,
    pub outcome: Result<JobOutcome, AppError>,
}

// ── App state ─────────────────────────────────────────────────────────────────

/// Central state owned by `eframe::App`.
pub struct AppState {
    pub config: Config,
    pub devices: Vec<Disk>,
    pub device_error: Option<String>,
    pub pending: VecDeque<JobSpec>,
    pub active: Option<ActiveJob>,
    pub completed: Vec<CompletedJob>,
    pub audit_path: Option<PathBuf>,
    pub audit_lines: Vec<String>,
    pub show_audit: bool,
    pub show_job_form: bool,
    pub pending_job_form: Option<JobSpec>,
    pub selected_device_idx: Option<usize>,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let (devices, device_error) = match iridium_device::Disk::enumerate() {
            Ok(d) => (d, None),
            Err(e) => (vec![], Some(e.to_string())),
        };
        Self {
            config,
            devices,
            device_error,
            pending: VecDeque::new(),
            active: None,
            completed: vec![],
            audit_path: None,
            audit_lines: vec![],
            show_audit: false,
            show_job_form: false,
            pending_job_form: None,
            selected_device_idx: None,
        }
    }

    pub fn refresh_devices(&mut self) {
        match iridium_device::Disk::enumerate() {
            Ok(d) => {
                self.devices = d;
                self.device_error = None;
                self.selected_device_idx = None;
            }
            Err(e) => {
                self.device_error = Some(e.to_string());
            }
        }
    }

    /// Drain the progress channel and update `active.progress`.
    /// Returns `true` if the job has finished (Completed or Cancelled event received).
    pub fn poll_progress(&mut self) -> bool {
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        let mut finished = false;
        for event in active.progress_rx.try_iter() {
            match event {
                ProgressEvent::Started { total_bytes } => {
                    active.progress.total_bytes = total_bytes;
                }
                ProgressEvent::Chunk { bytes_done, bad_chunks } => {
                    active.progress.bytes_done = bytes_done;
                    active.progress.bad_chunks = bad_chunks;
                    active.progress.recovery_pass = None;
                }
                ProgressEvent::Completed { .. } | ProgressEvent::Cancelled { .. } => {
                    finished = true;
                }
                ProgressEvent::RecoveryPassStarted { pass } => {
                    active.progress.recovery_pass = Some(pass);
                }
                ProgressEvent::RecoveryProgress { pass, finished_bytes, bad_bytes } => {
                    active.progress.recovery_pass = Some(pass);
                    active.progress.bytes_done = finished_bytes;
                    active.progress.recovery_bad_bytes = bad_bytes;
                }
                _ => {}
            }
        }
        finished
    }

    /// Join the finished worker thread and move the result into `completed`.
    /// Starts the next pending job if any.
    pub fn collect_finished(&mut self, egui_ctx: &egui::Context) {
        let Some(mut active) = self.active.take() else {
            return;
        };
        let spec = active.spec.clone();
        let outcome = active
            .handle
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_else(|| Err(AppError::Config("worker thread panicked".into())));

        // tail audit log
        if let Some(path) = &self.audit_path {
            self.audit_lines = tail_file(path, 200);
        }

        self.completed.push(CompletedJob { spec, outcome });

        if !self.pending.is_empty() {
            crate::worker::start_next(self, egui_ctx);
        }
    }
}

fn tail_file(path: &PathBuf, max_lines: usize) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return vec![];
    };
    content
        .lines()
        .filter(|l| !l.is_empty())
        .rev()
        .take(max_lines)
        .map(String::from)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

// ── Queue-transition tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use iridium_core::{HashAlg, ImageFormat};
    use iridium_device::Disk;

    use super::*;

    fn dummy_disk() -> Disk {
        Disk {
            path: PathBuf::from("/dev/loop0"),
            model: String::new(),
            serial: String::new(),
            size_bytes: 1024 * 1024,
            logical_sector_size: 512,
            sector_size: 512,
            hpa_size_bytes: None,
            dco_restricted: false,
            removable: false,
            rotational: false,
            read_only: true,
            partition_of: None,
        }
    }

    fn dummy_spec() -> JobSpec {
        JobSpec {
            source: dummy_disk(),
            dest_path: PathBuf::from("/tmp/test"),
            format: ImageFormat::Raw,
            algorithms: vec![HashAlg::Sha256],
            chunk_size: iridium_acquire::DEFAULT_CHUNK_SIZE,
            recovery_mode: false,
            mapfile_path: None,
            verify_after: false,
        }
    }

    #[test]
    fn new_state_starts_empty() {
        // enumerate() may fail in test env — that's fine
        let state = AppState::new(Config::default());
        assert!(state.pending.is_empty());
        assert!(state.active.is_none());
        assert!(state.completed.is_empty());
    }

    #[test]
    fn enqueue_increases_pending() {
        let mut state = AppState::new(Config::default());
        state.pending.push_back(dummy_spec());
        state.pending.push_back(dummy_spec());
        assert_eq!(state.pending.len(), 2);
    }

    #[test]
    fn poll_progress_no_active_returns_false() {
        let mut state = AppState::new(Config::default());
        assert!(!state.poll_progress());
    }
}
