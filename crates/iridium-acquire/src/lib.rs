// iridium-acquire: acquisition pipeline — DeviceReader → HashFanOut → ImageWriter.

mod pipeline;
pub mod writer;

pub use writer::{ImageWriter, RawWriter};

use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

use iridium_core::HashAlg;
use iridium_device::Disk;
use iridium_hash::Digest;

// ── Public types ──────────────────────────────────────────────────────────────

/// Parameters for a single acquisition run.
pub struct AcquireJob {
    /// Source device to image.
    pub source: Disk,
    /// Destination path without extension; [`RawWriter`] appends `.img`,
    /// and future writers will append their own extension.
    pub dest_path: PathBuf,
    /// Hash algorithms to compute in parallel with the read.  Must be non-empty.
    pub algorithms: Vec<HashAlg>,
    /// Read chunk size in bytes.  Defaults to [`DEFAULT_CHUNK_SIZE`].
    pub chunk_size: usize,
    /// Set this to `true` from any thread to request a clean abort between chunks.
    /// An aborted acquisition is forensically invalid; the result will have
    /// `complete: false`.
    pub cancel: Arc<AtomicBool>,
    /// Optional channel for progress events.  Terminal events (`Started`,
    /// `Completed`, `Cancelled`) are sent with a blocking `send`; high-frequency
    /// `Chunk` events use `try_send` and may be dropped under backpressure.
    pub progress_tx: Option<crossbeam_channel::Sender<ProgressEvent>>,
}

/// 1 MiB — matches Guymager's default chunk size.
pub const DEFAULT_CHUNK_SIZE: usize = 1024 * 1024;

impl AcquireJob {
    /// Construct a job with sensible defaults (1 MiB chunks, no cancel, no progress).
    pub fn new(source: Disk, dest_path: PathBuf, algorithms: Vec<HashAlg>) -> Self {
        Self {
            source,
            dest_path,
            algorithms,
            chunk_size: DEFAULT_CHUNK_SIZE,
            cancel: Arc::new(AtomicBool::new(false)),
            progress_tx: None,
        }
    }
}

/// Events emitted by the pipeline via [`AcquireJob::progress_tx`].
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// Emitted once before the first read.
    Started { total_bytes: u64 },
    /// Emitted after each chunk is written.
    Chunk { bytes_done: u64, bad_sectors: u64 },
    /// Emitted when the pipeline finishes successfully.
    Completed { result: AcquireResult },
    /// Emitted when the pipeline is stopped by the cancel flag.
    Cancelled { bytes_done: u64 },
}

/// Outcome of a completed (or cancelled) acquisition.
#[derive(Debug, Clone)]
pub struct AcquireResult {
    /// One digest per algorithm, in the same order as [`AcquireJob::algorithms`].
    /// Always empty when `complete` is `false` (cancelled acquisition).
    pub digests: Vec<Digest>,
    /// Total bytes processed (read or zero-filled on error) from the device.
    pub bytes_processed: u64,
    /// Number of chunks that produced a read error and were zero-filled.
    pub bad_sectors: u64,
    /// `false` if the acquisition was stopped by the cancel flag.
    pub complete: bool,
}

/// Errors that can abort the pipeline.
#[derive(Debug, thiserror::Error)]
pub enum AcquireError {
    #[error("at least one hash algorithm must be specified")]
    NoAlgorithms,

    #[error("chunk_size must be greater than zero")]
    InvalidChunkSize,

    #[error("failed to open device {path}: {source}")]
    DeviceOpen {
        path: PathBuf,
        #[source]
        source: iridium_device::DeviceError,
    },

    #[error("failed to open output file {path}: {source}")]
    WriterOpen {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("device read error at offset {offset}: {source}")]
    DeviceRead {
        offset: u64,
        #[source]
        source: iridium_device::DeviceError,
    },

    #[error("write failed on {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Run an acquisition to completion.
///
/// Creates a [`RawWriter`] for `job.dest_path` and runs the
/// read → hash → write pipeline.  To use a different output format (e.g. EWF
/// in Phase 4), call [`run_with_writer`] directly.
pub fn run(job: AcquireJob) -> Result<AcquireResult, AcquireError> {
    let writer = Box::new(RawWriter::create(&job.dest_path)?);
    pipeline::run(&job, writer)
}

/// Run an acquisition with a caller-supplied [`ImageWriter`].
///
/// This is the extension point for Phase 4: pass an `EwfWriter` here to
/// produce EWF output without changing the pipeline.
pub fn run_with_writer(
    job: AcquireJob,
    writer: Box<dyn ImageWriter>,
) -> Result<AcquireResult, AcquireError> {
    pipeline::run(&job, writer)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use iridium_core::HashAlg;
    use iridium_hash::new_hasher;

    use super::*;

    /// A minimal in-memory ImageWriter for testing the pipeline without real
    /// devices or files.
    struct MemWriter(Vec<u8>);

    impl ImageWriter for MemWriter {
        fn write_chunk(&mut self, data: &[u8]) -> Result<(), AcquireError> {
            self.0.extend_from_slice(data);
            Ok(())
        }
        fn finalize(self: Box<Self>) -> Result<(), AcquireError> {
            Ok(())
        }
    }

    /// Build an AcquireJob backed by a loopback file instead of a real device.
    /// Returns the job and the `TempDir` guard; the caller must hold the guard
    /// for the duration of the test so the directory is not deleted early.
    fn make_job(data: &[u8], algorithms: Vec<HashAlg>) -> (AcquireJob, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let src_path = dir.path().join("source.img");
        std::fs::write(&src_path, data).unwrap();

        // Build a synthetic Disk pointing at the temp file.
        let meta = std::fs::metadata(&src_path).unwrap();
        let disk = iridium_device::Disk {
            path: src_path,
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
        let job = AcquireJob::new(disk, dest, algorithms);
        (job, dir)
    }

    #[test]
    fn pipeline_produces_correct_digests() {
        let data = b"the quick brown fox jumps over the lazy dog";
        let algs = vec![HashAlg::Md5, HashAlg::Sha1, HashAlg::Sha256];
        let (job, _dir) = make_job(data, algs.clone());

        let writer = Box::new(MemWriter(Vec::new()));
        let result = run_with_writer(job, writer).unwrap();

        assert!(result.complete);
        assert_eq!(result.bytes_processed, data.len() as u64);
        assert_eq!(result.bad_sectors, 0);
        assert_eq!(result.digests.len(), 3);

        // Cross-check each digest against iridium-hash known-answer vectors.
        for (alg, digest) in algs.iter().zip(&result.digests) {
            assert_eq!(digest.algorithm, *alg);
            let mut h = new_hasher(*alg);
            h.update(data);
            let expected = h.finish();
            assert_eq!(digest.hex, expected.hex, "algorithm {alg:?}");
        }
    }

    #[test]
    fn pipeline_written_bytes_match_source() {
        let data: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
        let (job, _dir) = make_job(&data, vec![HashAlg::Md5]);
        let result = run_with_writer(job, Box::new(MemWriter(Vec::new()))).unwrap();
        assert_eq!(result.bytes_processed, data.len() as u64);
    }

    #[test]
    fn cancel_before_first_chunk_returns_incomplete() {
        let data = b"should not be read";
        let (job, _dir) = make_job(data, vec![HashAlg::Md5]);

        job.cancel.store(true, Ordering::Relaxed);
        let writer = Box::new(MemWriter(Vec::new()));
        let result = run_with_writer(job, writer).unwrap();

        assert!(!result.complete);
        assert!(result.digests.is_empty());
    }

    #[test]
    fn no_algorithms_returns_error() {
        let data = b"x";
        let (job, _dir) = make_job(data, vec![]);

        let writer = Box::new(MemWriter(Vec::new()));
        assert!(matches!(
            run_with_writer(job, writer),
            Err(AcquireError::NoAlgorithms)
        ));
    }

    #[test]
    fn progress_events_emitted() {
        let data: Vec<u8> = vec![0u8; 512];
        let (mut job, _dest) = make_job(&data, vec![HashAlg::Sha256]);

        let (tx, rx) = crossbeam_channel::unbounded();
        job.progress_tx = Some(tx);

        let writer = Box::new(MemWriter(Vec::new()));
        let result = run_with_writer(job, writer).unwrap();
        assert!(result.complete);

        let events: Vec<_> = rx.try_iter().collect();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ProgressEvent::Started { .. }))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ProgressEvent::Completed { .. }))
        );
    }
}
