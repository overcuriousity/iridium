// iridium-recovery: dd_rescue-style recovery mode for damaged disks.
//
// Implements a four-pass pipeline:
//   1. Forward copying pass — chunk-sized reads, skip bad regions.
//   2. Trimming pass        — sector reads at edges of bad regions.
//   3. Scraping pass        — per-sector retries of remaining bad regions.
//   4. Hash pass            — sequential re-read of the completed image.
//
// Progress is persisted to a GNU ddrescue-compatible mapfile after each pass
// and periodically within passes (default every 30 s).

pub mod hash_pass;
pub mod map;
pub mod passes;
pub mod recovery_file;

use std::path::PathBuf;

use iridium_acquire::{AcquireJob, ProgressEvent};
use iridium_audit::{AuditEvent, DigestRecord, JobMetadata};
use iridium_core::ImageFormat;
use iridium_hash::Digest;
use time::OffsetDateTime;

pub use passes::BlockReader;

// ── Public types ──────────────────────────────────────────────────────────────

/// Tuning parameters for a recovery run.
pub struct RecoveryOptions {
    /// Read chunk size for the forward pass (bytes).  Defaults to 1 MiB.
    pub chunk_size: usize,
    /// Per-sector retry count in the scraping pass.  Defaults to 3.
    pub max_retries: u32,
    /// Explicit mapfile path.  Defaults to `<dest_path>.map`.
    pub mapfile_path: Option<PathBuf>,
    /// Seconds between periodic mapfile flushes within a pass.  Defaults to 30.
    pub mapfile_sync_secs: u64,
}

impl Default for RecoveryOptions {
    fn default() -> Self {
        Self {
            chunk_size: iridium_acquire::DEFAULT_CHUNK_SIZE,
            max_retries: 3,
            mapfile_path: None,
            mapfile_sync_secs: 30,
        }
    }
}

/// Outcome of a completed (or cancelled) recovery run.
#[derive(Debug)]
pub struct RecoveryResult {
    /// One digest per algorithm (post-acquisition hash pass).
    /// Empty when `complete` is `false`.
    pub digests: Vec<Digest>,
    pub total_bytes: u64,
    /// Bytes successfully read from the source device.
    pub finished_bytes: u64,
    /// Bytes that could not be recovered (zero-filled in the output image).
    pub bad_bytes: u64,
    /// Path of the GNU ddrescue-compatible mapfile.
    pub mapfile_path: PathBuf,
    /// `false` if the run was stopped by the cancel flag.
    pub complete: bool,
}

/// Errors that can abort a recovery run.
#[derive(Debug, thiserror::Error)]
pub enum RecoveryError {
    #[error("at least one hash algorithm must be specified")]
    NoAlgorithms,

    #[error("chunk_size must be greater than zero")]
    InvalidChunkSize,

    #[error("failed to open source device {path}: {source}")]
    DeviceOpen {
        path: PathBuf,
        #[source]
        source: iridium_device::DeviceError,
    },

    #[error("failed to create output image {path}: {source}")]
    OutputOpen {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("write failed on {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("mapfile write failed on {path}: {source}")]
    MapfileWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("hash pass failed: {0}")]
    Hash(#[source] std::io::Error),
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Run a full recovery pipeline to completion (or cancellation).
///
/// The function opens the source device named in `job.source`, creates an
/// output image at `<job.dest_path>.img` and a mapfile at
/// `<job.dest_path>.map` (or `opts.mapfile_path` when set), then executes
/// passes 1–4.
///
/// A cancelled recovery still produces a usable partial image and a mapfile
/// reflecting the state at cancellation.  The result has `complete: false`
/// and empty `digests`.
pub fn run_recovery(
    mut job: AcquireJob,
    opts: RecoveryOptions,
) -> Result<RecoveryResult, RecoveryError> {
    if job.algorithms.is_empty() {
        return Err(RecoveryError::NoAlgorithms);
    }
    if opts.chunk_size == 0 {
        return Err(RecoveryError::InvalidChunkSize);
    }

    job.format = Some(ImageFormat::Raw);
    // Keep job.chunk_size in sync so audit metadata matches the actual run.
    job.chunk_size = opts.chunk_size;

    let total_bytes = job.source.size_bytes;
    let sector_size = job.source.logical_sector_size as usize;
    let output_path = job.dest_path.with_extension("img");
    let mapfile_path = opts
        .mapfile_path
        .clone()
        .unwrap_or_else(|| job.dest_path.with_extension("map"));
    let argv: Vec<String> = std::env::args_os()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();

    // ── Open device reader first ──────────────────────────────────────────────
    // Opening the reader before creating output artefacts prevents a failed
    // device open from leaving a truncated image and a misleading audit event.
    let mut reader = job
        .source
        .open_read_only()
        .map_err(|e| RecoveryError::DeviceOpen {
            path: job.source.path.clone(),
            source: e,
        })?;

    // ── Create output file ────────────────────────────────────────────────────
    let recovery_file =
        recovery_file::RecoveryFile::create(&output_path, total_bytes).map_err(|e| {
            RecoveryError::OutputOpen {
                path: output_path.clone(),
                source: e,
            }
        })?;

    // ── Create map state ──────────────────────────────────────────────────────
    let mut map = map::MapState::new(
        total_bytes,
        mapfile_path.clone(),
        env!("CARGO_PKG_VERSION").to_owned(),
        argv.clone(),
    );

    // ── Emit RecoveryStarted ──────────────────────────────────────────────────
    emit_audit(&job, || AuditEvent::RecoveryStarted {
        ts: OffsetDateTime::now_utc(),
        iridium_version: env!("CARGO_PKG_VERSION").to_owned(),
        argv: argv.clone(),
        job: job_metadata(&job),
        mapfile_path: mapfile_path.clone(),
    });

    // ── Pass 1: forward ───────────────────────────────────────────────────────
    start_pass(&job, &mut map, "forward", 1);
    let ok = passes::forward_pass(&mut reader, &mut map, &recovery_file, &job, &opts)?;
    flush_map(&mut map, &job, &opts)?;
    if !ok {
        return cancelled_result(&job, map, mapfile_path);
    }

    // ── Pass 2: trim ──────────────────────────────────────────────────────────
    if map.has_status(map::Status::NonTrimmed) {
        start_pass(&job, &mut map, "trim", 2);
        let ok = passes::trim_pass(
            &mut reader,
            &mut map,
            &recovery_file,
            &job,
            &opts,
            sector_size,
        )?;
        flush_map(&mut map, &job, &opts)?;
        if !ok {
            return cancelled_result(&job, map, mapfile_path);
        }
    }

    // ── Pass 3: scrape ────────────────────────────────────────────────────────
    if map.has_status(map::Status::NonScraped) {
        start_pass(&job, &mut map, "scrape", 3);
        let ok = passes::scrape_pass(
            &mut reader,
            &mut map,
            &recovery_file,
            &job,
            &opts,
            sector_size,
        )?;
        flush_map(&mut map, &job, &opts)?;
        if !ok {
            return cancelled_result(&job, map, mapfile_path);
        }
    }

    // ── Pass 4: hash ──────────────────────────────────────────────────────────
    start_pass(&job, &mut map, "hash", 4);
    let digests =
        hash_pass::hash_pass(&output_path, &job.algorithms).map_err(RecoveryError::Hash)?;

    // Final mapfile flush.
    flush_map(&mut map, &job, &opts)?;

    // ── Emit RecoveryCompleted ────────────────────────────────────────────────
    emit_audit(&job, || AuditEvent::RecoveryCompleted {
        ts: OffsetDateTime::now_utc(),
        total_bytes,
        finished_bytes: map.finished_bytes(),
        bad_bytes: map.bad_bytes(),
        digests: digests
            .iter()
            .map(|d| DigestRecord {
                algorithm: d.algorithm,
                hex: d.hex.clone(),
            })
            .collect(),
    });

    Ok(RecoveryResult {
        digests,
        total_bytes,
        finished_bytes: map.finished_bytes(),
        bad_bytes: map.bad_bytes(),
        mapfile_path,
        complete: true,
    })
}

// ── Private helpers ───────────────────────────────────────────────────────────

fn start_pass(job: &AcquireJob, map: &mut map::MapState, pass: &str, pass_num: u8) {
    map.current_pass = pass_num;
    emit_audit(job, || AuditEvent::RecoveryPassStarted {
        ts: OffsetDateTime::now_utc(),
        pass: pass.to_owned(),
    });
    if let Some(tx) = &job.progress_tx {
        let _ = tx.try_send(ProgressEvent::RecoveryPassStarted {
            pass: pass.to_owned(),
        });
    }
}

fn flush_map(
    map: &mut map::MapState,
    job: &AcquireJob,
    _opts: &RecoveryOptions,
) -> Result<(), RecoveryError> {
    map.flush().map_err(|e| RecoveryError::MapfileWrite {
        path: map.mapfile_path.clone(),
        source: e,
    })?;
    emit_audit(job, || AuditEvent::MapfileFlushed {
        ts: OffsetDateTime::now_utc(),
        mapfile_path: map.mapfile_path.clone(),
        finished_bytes: map.finished_bytes(),
        bad_bytes: map.bad_bytes(),
    });
    Ok(())
}

fn cancelled_result(
    job: &AcquireJob,
    map: map::MapState,
    mapfile_path: PathBuf,
) -> Result<RecoveryResult, RecoveryError> {
    emit_audit(job, || AuditEvent::RecoveryCancelled {
        ts: OffsetDateTime::now_utc(),
        total_bytes: map.total_bytes,
        finished_bytes: map.finished_bytes(),
        bad_bytes: map.bad_bytes(),
    });
    Ok(RecoveryResult {
        digests: vec![],
        total_bytes: map.total_bytes,
        finished_bytes: map.finished_bytes(),
        bad_bytes: map.bad_bytes(),
        mapfile_path,
        complete: false,
    })
}

fn emit_audit(job: &AcquireJob, make_event: impl FnOnce() -> AuditEvent) {
    if let Some(log) = &job.audit
        && let Err(e) = log.append(&make_event())
    {
        log::warn!("iridium-recovery: failed to append audit event: {e}");
    }
}

fn job_metadata(job: &AcquireJob) -> JobMetadata {
    JobMetadata {
        source_path: job.source.path.clone(),
        model: job.source.model.clone(),
        serial: job.source.serial.clone(),
        size_bytes: job.source.size_bytes,
        logical_sector_size: job.source.logical_sector_size,
        sector_size: job.source.sector_size,
        hpa_size_bytes: job.source.hpa_size_bytes,
        dco_restricted: job.source.dco_restricted,
        removable: job.source.removable,
        rotational: job.source.rotational,
        dest_path: job.dest_path.clone(),
        format: job.format,
        algorithms: job.algorithms.clone(),
        chunk_size: job.chunk_size,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use iridium_acquire::AcquireJob;
    use iridium_core::HashAlg;
    use iridium_device::Disk;
    use iridium_hash::new_hasher;

    use super::*;

    fn make_job(data: &[u8]) -> (AcquireJob, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.img");
        std::fs::write(&src, data).unwrap();
        let meta = std::fs::metadata(&src).unwrap();
        let disk = Disk {
            path: src,
            model: String::new(),
            serial: String::new(),
            size_bytes: meta.len(),
            logical_sector_size: 512,
            sector_size: 512,
            hpa_size_bytes: None,
            dco_restricted: false,
            removable: false,
            rotational: false,
            read_only: true,
            partition_of: None,
        };
        let dest = dir.path().join("output");
        let job = AcquireJob::new(disk, dest, vec![HashAlg::Sha256]);
        (job, dir)
    }

    fn default_opts() -> RecoveryOptions {
        RecoveryOptions {
            chunk_size: 512,
            max_retries: 3,
            mapfile_path: None,
            mapfile_sync_secs: u64::MAX,
        }
    }

    // ── Validation ───────────────────────────────────────────────────────────

    #[test]
    fn no_algorithms_returns_error() {
        let (mut job, _dir) = make_job(b"x");
        job.algorithms.clear();
        assert!(matches!(
            run_recovery(job, default_opts()),
            Err(RecoveryError::NoAlgorithms)
        ));
    }

    #[test]
    fn zero_chunk_size_returns_error() {
        let (job, _dir) = make_job(b"x");
        let mut opts = default_opts();
        opts.chunk_size = 0;
        assert!(matches!(
            run_recovery(job, opts),
            Err(RecoveryError::InvalidChunkSize)
        ));
    }

    // ── Happy path ────────────────────────────────────────────────────────────

    #[test]
    fn healthy_source_produces_correct_digests() {
        let data: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
        let (job, _dir) = make_job(&data);

        let result = run_recovery(job, default_opts()).unwrap();

        assert!(result.complete);
        assert_eq!(result.total_bytes, data.len() as u64);
        assert_eq!(result.finished_bytes, data.len() as u64);
        assert_eq!(result.bad_bytes, 0);
        assert_eq!(result.digests.len(), 1);

        let mut h = new_hasher(HashAlg::Sha256);
        h.update(&data);
        assert_eq!(result.digests[0].hex, h.finish().hex);
    }

    #[test]
    fn output_image_matches_source() {
        let data: Vec<u8> = (0u8..=127).cycle().take(2048).collect();
        let (job, dir) = make_job(&data);
        let output = dir.path().join("output.img");

        run_recovery(job, default_opts()).unwrap();

        let written = std::fs::read(&output).unwrap();
        assert_eq!(written, data);
    }

    #[test]
    fn mapfile_is_created_and_valid_ddrescue_format() {
        let data = vec![0u8; 1024];
        let (job, dir) = make_job(&data);
        let mapfile = dir.path().join("output.map");

        run_recovery(job, default_opts()).unwrap();

        assert!(mapfile.exists());
        let content = std::fs::read_to_string(&mapfile).unwrap();

        // Must have the header comment
        assert!(
            content.contains("iridium-recovery"),
            "mapfile missing header"
        );

        // All regions should be '+' (finished) for a healthy source.
        let data_lines: Vec<&str> = content
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .skip(1) // skip current_pos line
            .collect();
        assert!(!data_lines.is_empty());
        for line in data_lines {
            assert!(
                line.ends_with('+'),
                "expected all regions Finished, got: {line}"
            );
        }
    }

    // ── Cancellation ─────────────────────────────────────────────────────────

    #[test]
    fn cancel_before_first_chunk_returns_incomplete() {
        let data = vec![0u8; 1024];
        let (mut job, _dir) = make_job(&data);
        job.cancel = Arc::new(std::sync::atomic::AtomicBool::new(true));

        let result = run_recovery(job, default_opts()).unwrap();
        assert!(!result.complete);
        assert!(result.digests.is_empty());
    }

    // ── Audit log ─────────────────────────────────────────────────────────────

    #[test]
    fn audit_log_contains_required_events() {
        let data: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
        let (mut job, dir) = make_job(&data);

        let log_path = dir.path().join("audit.jsonl");
        let log = Arc::new(iridium_audit::Log::open(&log_path).unwrap());
        job.audit = Some(Arc::clone(&log));

        run_recovery(job, default_opts()).unwrap();

        Arc::try_unwrap(log).ok().unwrap().seal().unwrap();

        let events: Vec<serde_json::Value> = std::fs::read_to_string(&log_path)
            .unwrap()
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        assert!(events.iter().any(|e| e["event"] == "recovery_started"));
        assert!(events.iter().any(|e| e["event"] == "recovery_pass_started"));
        assert!(events.iter().any(|e| e["event"] == "recovery_completed"));
        assert_eq!(events.last().unwrap()["event"], "sealed");
    }

    // ── Progress events ───────────────────────────────────────────────────────

    #[test]
    fn progress_channel_receives_pass_events() {
        let data = vec![0u8; 512];
        let (mut job, _dir) = make_job(&data);

        let (tx, rx) = crossbeam_channel::unbounded();
        job.progress_tx = Some(tx);

        run_recovery(job, default_opts()).unwrap();

        let events: Vec<_> = rx.try_iter().collect();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ProgressEvent::RecoveryPassStarted { .. })),
            "must have at least one RecoveryPassStarted event"
        );
    }
}
