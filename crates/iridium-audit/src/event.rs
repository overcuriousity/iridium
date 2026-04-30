use std::path::PathBuf;

use iridium_core::{HashAlg, ImageFormat};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// One digest entry as recorded in the audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DigestRecord {
    pub algorithm: HashAlg,
    pub hex: String,
}

/// Device and job parameters captured at acquisition start.
///
/// All fields use owned primitive types so this struct has no lifetime
/// dependencies on the acquisition crate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobMetadata {
    pub source_path: PathBuf,
    pub model: String,
    pub serial: String,
    pub size_bytes: u64,
    pub logical_sector_size: u32,
    pub sector_size: u32,
    pub hpa_size_bytes: Option<u64>,
    pub dco_restricted: bool,
    pub removable: bool,
    pub rotational: bool,
    pub dest_path: PathBuf,
    /// `None` when the caller used `run_with_writer` without setting a format.
    pub format: Option<ImageFormat>,
    pub algorithms: Vec<HashAlg>,
    pub chunk_size: usize,
}

/// A single record appended to the audit log.
///
/// Serialises as a flat JSON object with an `"event"` discriminant field,
/// e.g. `{"event":"start","ts":"2026-04-30T...","job":{...}}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AuditEvent {
    /// Emitted once, before the first device read.
    Start {
        #[serde(with = "time::serde::rfc3339")]
        ts: OffsetDateTime,
        iridium_version: String,
        libewf_version: String,
        /// Process argv at the time of the call.  Phase 7 will replace this
        /// with the resolved clap configuration once the CLI shell exists.
        argv: Vec<String>,
        job: JobMetadata,
    },
    /// Emitted once per zero-filled chunk (spec §2.3).
    ReadError {
        #[serde(with = "time::serde::rfc3339")]
        ts: OffsetDateTime,
        /// Byte offset of the failing read.
        offset: u64,
        /// Number of bytes zero-filled to cover the error.
        length: u64,
        /// `Display` representation of the underlying error.
        error: String,
        /// Running total of bad chunks after this error.
        bad_chunks_total: u64,
    },
    /// Emitted when the pipeline is stopped by the cancel flag.
    Cancelled {
        #[serde(with = "time::serde::rfc3339")]
        ts: OffsetDateTime,
        bytes_processed: u64,
        bad_chunks: u64,
    },
    /// Emitted after a successful finalization.
    Completed {
        #[serde(with = "time::serde::rfc3339")]
        ts: OffsetDateTime,
        bytes_processed: u64,
        bad_chunks: u64,
        digests: Vec<DigestRecord>,
    },
    /// Written by [`Log::seal`]; marks the log as closed.
    Sealed {
        #[serde(with = "time::serde::rfc3339")]
        ts: OffsetDateTime,
    },

    // ── Recovery-mode events (Phase 6) ────────────────────────────────────────

    /// Emitted once at the start of a recovery run.
    RecoveryStarted {
        #[serde(with = "time::serde::rfc3339")]
        ts: OffsetDateTime,
        iridium_version: String,
        argv: Vec<String>,
        job: JobMetadata,
        mapfile_path: std::path::PathBuf,
    },
    /// Emitted when a recovery pass begins.
    ///
    /// `pass` is one of `"forward"`, `"trim"`, `"scrape"`, or `"hash"`.
    RecoveryPassStarted {
        #[serde(with = "time::serde::rfc3339")]
        ts: OffsetDateTime,
        pass: String,
    },
    /// Emitted for each sector that could not be read during recovery.
    ///
    /// `map_status` is the ddrescue status character assigned after the
    /// failure: `"*"` (non-trimmed), `"/"` (non-scraped), or `"-"` (bad-sector).
    RecoveryReadError {
        #[serde(with = "time::serde::rfc3339")]
        ts: OffsetDateTime,
        offset: u64,
        length: u64,
        error: String,
        map_status: String,
    },
    /// Emitted after each atomic mapfile rewrite.
    MapfileFlushed {
        #[serde(with = "time::serde::rfc3339")]
        ts: OffsetDateTime,
        mapfile_path: std::path::PathBuf,
        finished_bytes: u64,
        bad_bytes: u64,
    },
    /// Emitted after the hash pass completes (or immediately before sealing
    /// when the run was cancelled before hashing).
    RecoveryCompleted {
        #[serde(with = "time::serde::rfc3339")]
        ts: OffsetDateTime,
        total_bytes: u64,
        finished_bytes: u64,
        bad_bytes: u64,
        digests: Vec<DigestRecord>,
    },
}
