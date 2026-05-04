use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex, atomic::AtomicBool},
    thread::JoinHandle,
    time::Instant,
};

use crossbeam_channel::Receiver;
use iridium_acquire::ProgressEvent;
use iridium_core::{HashAlg, ImageFormat};
use iridium_device::Disk;
use iridium_hash::Digest;
use iridium_recovery::RecoveryResult;

use crate::{config::Config, error::AppError};

// ── Job specification ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct JobSpec {
    pub source: Disk,
    pub dest_path: PathBuf,
    pub format: ImageFormat,
    pub algorithms: Vec<HashAlg>,
    pub chunk_size: usize,
    pub recovery_mode: bool,
    pub mapfile_path: Option<PathBuf>,
    pub verify_after: bool,
}

// ── Active job ────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
pub struct ProgressSnapshot {
    pub total_bytes: u64,
    pub bytes_done: u64,
    pub bad_chunks: u64,
    pub recovery_pass: Option<&'static str>,
    pub recovery_bad_bytes: u64,
    pub verify_bytes_done: u64,
    pub verifying: bool,
}

pub struct ActiveJob {
    pub spec: JobSpec,
    pub cancel: Arc<AtomicBool>,
    pub progress_rx: Receiver<ProgressEvent>,
    pub handle: Option<JoinHandle<Result<JobOutcome, AppError>>>,
    pub progress: ProgressSnapshot,
    /// Cumulative bytes_done samples for throughput sparkline. Each entry is
    /// (sample_instant, cumulative_bytes_done). Capped at ~120 samples (≈60 s).
    pub throughput_samples: VecDeque<(Instant, u64)>,
    /// EWMA of instantaneous bytes/s. Used for ETA and status bar.
    pub ewma_bps: f64,
    pub started_at: Instant,
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
    pub finished_at: std::time::SystemTime,
}

// ── View state ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InspectorMode {
    #[default]
    DeviceDetail,
    NewJob,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CentralTab {
    Queue,
    #[default]
    Active,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeviceCol {
    #[default]
    Path,
    Model,
    Serial,
    Size,
    Sector,
    Type,
    Flags,
}

#[derive(Debug, Default, Clone)]
pub struct TableViewState {
    pub sort_col: DeviceCol,
    pub sort_asc: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum AuditLevel {
    #[default]
    Debug,
    Info,
    Warn,
    Error,
}

impl AuditLevel {
    pub fn label(self) -> &'static str {
        match self {
            AuditLevel::Debug => "DEBUG",
            AuditLevel::Info => "INFO",
            AuditLevel::Warn => "WARN",
            AuditLevel::Error => "ERROR",
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct AuditFilter {
    pub text: String,
    pub min_level: AuditLevel,
    pub follow_tail: bool,
}

#[derive(Debug, Clone)]
pub struct AuditView {
    pub ts: String,
    pub level: AuditLevel,
    pub event: String,
    pub detail: String,
}

// ── Chunk-unit selector for job form ─────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChunkUnit {
    Kib,
    #[default]
    Mib,
}

// ── App state ─────────────────────────────────────────────────────────────────

pub struct AppState {
    pub config: Config,
    pub devices: Vec<Disk>,
    pub device_error: Option<String>,
    pub pending: VecDeque<JobSpec>,
    pub active: Option<ActiveJob>,
    pub completed: Vec<CompletedJob>,
    pub audit_path: Option<PathBuf>,
    pub audit_lines: Vec<String>,
    pub audit_views: Vec<AuditView>,
    pub show_audit: bool,
    pub show_job_form: bool,
    pub pending_job_form: Option<JobSpec>,
    pub selected_device_idx: Option<usize>,
    // New view state
    pub inspector_mode: InspectorMode,
    pub central_tab: CentralTab,
    pub device_table: TableViewState,
    pub audit_filter: AuditFilter,
    pub target_free_cache: Option<(PathBuf, u64, Instant)>,
    /// Receives file path from background rfd dialog thread.
    pub file_dialog_slot: Arc<Mutex<Option<PathBuf>>>,
    pub file_dialog_open: bool,
    /// Signals app.rs that the inspector's "Queue job" was clicked.
    pub job_submit_requested: bool,
    /// Chunk unit selector for the job form.
    pub chunk_unit: ChunkUnit,
    /// Which completed job is shown in the detail window (persists across frames).
    pub completed_detail_idx: Option<usize>,
    /// Last observed byte-size of the audit JSONL file; used to skip re-reads.
    pub audit_file_size: u64,
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
            audit_views: vec![],
            show_audit: false,
            show_job_form: false,
            pending_job_form: None,
            selected_device_idx: None,
            inspector_mode: InspectorMode::DeviceDetail,
            central_tab: CentralTab::Active,
            device_table: TableViewState { sort_col: DeviceCol::Path, sort_asc: true },
            audit_filter: AuditFilter { text: String::new(), min_level: AuditLevel::Debug, follow_tail: true },
            target_free_cache: None,
            file_dialog_slot: Arc::new(Mutex::new(None)),
            file_dialog_open: false,
            job_submit_requested: false,
            chunk_unit: ChunkUnit::Mib,
            completed_detail_idx: None,
            audit_file_size: 0,
        }
    }

    pub fn refresh_devices(&mut self) {
        match iridium_device::Disk::enumerate() {
            Ok(d) => {
                self.devices = d;
                self.device_error = None;
                self.selected_device_idx = None;
                self.device_table = TableViewState::default();
            }
            Err(e) => {
                self.device_error = Some(e.to_string());
            }
        }
    }

    /// Drain the progress channel and update `active.progress` and throughput state.
    /// Returns `true` when the worker thread has fully exited.
    pub fn poll_progress(&mut self) -> bool {
        let finished = {
            let Some(active) = self.active.as_mut() else {
                return false;
            };
            for event in active.progress_rx.try_iter() {
                match event {
                    ProgressEvent::Started { total_bytes } => {
                        active.progress.total_bytes = total_bytes;
                    }
                    ProgressEvent::Chunk { bytes_done, bad_chunks } => {
                        active.progress.bytes_done = bytes_done;
                        active.progress.bad_chunks = bad_chunks;
                        active.progress.recovery_pass = None;

                        let now = Instant::now();
                        active.throughput_samples.push_back((now, bytes_done));
                        while let Some(&(t, _)) = active.throughput_samples.front() {
                            if now.duration_since(t).as_secs() > 60 {
                                active.throughput_samples.pop_front();
                            } else {
                                break;
                            }
                        }
                        let inst_bps = window_rate(&active.throughput_samples, 2);
                        active.ewma_bps = 0.2 * inst_bps + 0.8 * active.ewma_bps;
                    }
                    ProgressEvent::VerifyProgress { bytes_done, total_bytes, .. } => {
                        active.progress.verifying = true;
                        active.progress.total_bytes = total_bytes;
                        active.progress.verify_bytes_done = bytes_done;
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
            active.handle.as_ref().map_or(false, |h| h.is_finished())
        }; // release borrow on self.active before refresh
        self.refresh_audit_if_changed();
        finished
    }

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

        self.audit_file_size = 0; // force a re-read on job completion
        self.refresh_audit_if_changed();

        self.completed.push(CompletedJob {
            spec,
            outcome,
            finished_at: std::time::SystemTime::now(),
        });
        self.central_tab = CentralTab::Completed;

        if !self.pending.is_empty() {
            crate::worker::start_next(self, egui_ctx);
        }
    }

    fn refresh_audit_if_changed(&mut self) {
        let Some(path) = &self.audit_path else { return };
        let current_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if current_size <= self.audit_file_size {
            return;
        }
        self.audit_file_size = current_size;
        let path = path.clone();
        let lines = tail_file(&path, 400);
        self.audit_views = parse_audit_views(&lines);
        self.audit_lines = lines;
    }
}

/// Compute instantaneous bytes/s over the last `window_secs` of samples.
fn window_rate(samples: &VecDeque<(Instant, u64)>, window_secs: u64) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    let now = Instant::now();
    let cutoff = now - std::time::Duration::from_secs(window_secs);
    let recent: Vec<_> = samples.iter().filter(|(t, _)| *t >= cutoff).collect();
    if recent.len() < 2 {
        return 0.0;
    }
    let first = recent.first().unwrap();
    let last = recent.last().unwrap();
    let dt = (last.0 - first.0).as_secs_f64();
    if dt < 0.001 {
        return 0.0;
    }
    last.1.saturating_sub(first.1) as f64 / dt
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

fn parse_audit_views(lines: &[String]) -> Vec<AuditView> {
    lines
        .iter()
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).ok()?;
            let event = v.get("event")?.as_str().unwrap_or("unknown").to_owned();
            let ts = v
                .get("ts")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_owned();
            let level = match event.as_str() {
                "read_error" | "recovery_read_error" => AuditLevel::Warn,
                "cancelled" | "recovery_cancelled" => AuditLevel::Warn,
                "start" | "recovery_started" | "completed" | "recovery_completed" | "sealed" => {
                    AuditLevel::Info
                }
                _ => AuditLevel::Debug,
            };
            let detail = match event.as_str() {
                "start" | "recovery_started" => v
                    .get("job")
                    .and_then(|j| j.get("source_path"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_owned(),
                "completed" | "recovery_completed" => {
                    let bytes = v
                        .get("bytes_processed")
                        .or_else(|| v.get("finished_bytes"))
                        .and_then(|b| b.as_u64())
                        .unwrap_or(0);
                    format_bytes(bytes)
                }
                "read_error" | "recovery_read_error" => v
                    .get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("")
                    .to_owned(),
                _ => String::new(),
            };
            Some(AuditView { ts, level, event, detail })
        })
        .collect()
}

pub fn format_bytes(bytes: u64) -> String {
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

pub fn format_duration(secs: f64) -> String {
    if secs < 0.0 || secs.is_infinite() || secs.is_nan() {
        return "—".to_owned();
    }
    let s = secs as u64;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m:02}:{sec:02}")
    }
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
