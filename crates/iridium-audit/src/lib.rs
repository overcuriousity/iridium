// iridium-audit: append-only JSONL audit log.
//
// Each call to `Log::append` writes one JSON object followed by a newline and
// calls `sync_data` so events survive a crash. `Log::seal` appends a terminal
// `Sealed` record and consumes the log handle.

mod event;

pub use event::{AuditEvent, DigestRecord, JobMetadata};

use std::{
    fs::{File, OpenOptions},
    io::{self, Write as _},
    path::{Path, PathBuf},
    sync::Mutex,
};

use thiserror::Error;

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum AuditError {
    #[error("failed to open audit log {path}: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to write audit log {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to encode audit event: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

// ── Log ───────────────────────────────────────────────────────────────────────

/// An append-only, crash-durable JSONL audit log.
///
/// Opens (or creates) the file in append mode; existing content is preserved,
/// allowing multiple acquisitions to share a single log file.  Each event is
/// flushed to the OS and `sync_data`'d before the call returns.
///
/// `Log` is `Send`: the inner file descriptor is protected by a `Mutex` so
/// multiple threads can safely call `append` concurrently.
pub struct Log {
    file: Mutex<File>,
    path: PathBuf,
}

impl Log {
    /// Open (or create) the log file at `path` in append mode.
    pub fn open(path: &Path) -> Result<Self, AuditError> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| AuditError::Open {
                path: path.to_path_buf(),
                source: e,
            })?;
        Ok(Self {
            file: Mutex::new(file),
            path: path.to_path_buf(),
        })
    }

    /// Serialise `event` as a single JSON line and flush + sync to disk.
    ///
    /// On failure the log remains open and further appends may be attempted.
    pub fn append(&self, event: &AuditEvent) -> Result<(), AuditError> {
        let line = serde_json::to_string(event).map_err(|e| AuditError::Encode { source: e })?;

        // Build the full JSONL record before taking the mutex so the JSON bytes
        // and trailing newline are written in one write_all while the lock is held.
        // This prevents interleaving between threads sharing this Log instance,
        // but does not guarantee cross-process atomicity (write_all may still issue
        // multiple syscalls on partial writes).
        let mut line = line;
        line.push('\n');

        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());

        file.write_all(line.as_bytes())
            .map_err(|e| AuditError::Write {
                path: self.path.clone(),
                source: e,
            })?;
        file.sync_data().map_err(|e| AuditError::Write {
            path: self.path.clone(),
            source: e,
        })
    }

    /// Append a [`AuditEvent::Sealed`] record and close the log.
    ///
    /// Consuming `self` prevents further appends after sealing.
    pub fn seal(self) -> Result<(), AuditError> {
        self.append(&AuditEvent::Sealed {
            ts: time::OffsetDateTime::now_utc(),
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use iridium_core::{HashAlg, ImageFormat};
    use time::OffsetDateTime;

    use super::*;

    fn now() -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }

    fn sample_read_error(offset: u64) -> AuditEvent {
        AuditEvent::ReadError {
            ts: now(),
            offset,
            length: 512,
            error: "test error".into(),
            bad_chunks_total: offset / 512 + 1,
        }
    }

    #[test]
    fn open_creates_file_with_append_semantics() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");

        let log = Log::open(&path).unwrap();
        log.append(&sample_read_error(0)).unwrap();
        drop(log);

        let log2 = Log::open(&path).unwrap();
        log2.append(&sample_read_error(512)).unwrap();
        drop(log2);

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.lines().filter(|l| !l.is_empty()).count(), 2);
    }

    #[test]
    fn append_writes_one_json_object_per_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let log = Log::open(&path).unwrap();

        log.append(&sample_read_error(0)).unwrap();
        log.append(&sample_read_error(512)).unwrap();
        log.append(&AuditEvent::Cancelled {
            ts: now(),
            bytes_processed: 1024,
            bad_chunks: 2,
        })
        .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        for line in content.lines().filter(|l| !l.is_empty()) {
            serde_json::from_str::<AuditEvent>(line).expect("each line must be valid JSON");
        }
    }

    #[test]
    fn seal_appends_sealed_event() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("seal.jsonl");
        {
            let log = Log::open(&path).unwrap();
            log.append(&AuditEvent::Cancelled {
                ts: now(),
                bytes_processed: 0,
                bad_chunks: 0,
            })
            .unwrap();
            log.seal().unwrap();
        }

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(
            lines.len(),
            2,
            "must have exactly 2 lines (cancelled + sealed)"
        );

        let last: AuditEvent = serde_json::from_str(lines.last().unwrap()).unwrap();
        assert!(
            matches!(last, AuditEvent::Sealed { .. }),
            "last line must be a Sealed event"
        );
    }

    #[test]
    fn start_event_captures_version_and_argv() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("start.jsonl");
        let log = Log::open(&path).unwrap();

        log.append(&AuditEvent::Start {
            ts: now(),
            iridium_version: env!("CARGO_PKG_VERSION").to_owned(),
            libewf_version: "20240506".to_owned(),
            argv: std::env::args().collect(),
            job: JobMetadata {
                source_path: "/dev/sda".into(),
                model: "SAMSUNG".into(),
                serial: "S1ABC".into(),
                size_bytes: 512 * 1024 * 1024,
                logical_sector_size: 512,
                sector_size: 512,
                hpa_size_bytes: None,
                dco_restricted: false,
                removable: false,
                rotational: true,
                dest_path: "/evidence/out".into(),
                format: Some(ImageFormat::Ewf),
                algorithms: vec![HashAlg::Sha256],
                chunk_size: 1024 * 1024,
            },
        })
        .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(content.lines().next().unwrap()).unwrap();

        assert_eq!(parsed["event"], "start");
        assert_eq!(parsed["iridium_version"], env!("CARGO_PKG_VERSION"));
        assert!(
            parsed["ts"].as_str().is_some_and(|s| s.contains('T')),
            "ts must be an RFC3339 string"
        );
    }

    #[test]
    fn read_error_event_round_trips() {
        let event = AuditEvent::ReadError {
            ts: now(),
            offset: 1_048_576,
            length: 4096,
            error: "Input/output error (os error 5)".into(),
            bad_chunks_total: 3,
        };

        let json = serde_json::to_string(&event).unwrap();
        let parsed: AuditEvent = serde_json::from_str(&json).unwrap();

        match parsed {
            AuditEvent::ReadError {
                offset,
                length,
                bad_chunks_total,
                ..
            } => {
                assert_eq!(offset, 1_048_576);
                assert_eq!(length, 4096);
                assert_eq!(bad_chunks_total, 3);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn concurrent_appends_are_atomic() {
        use std::sync::Arc;
        use std::thread;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("concurrent.jsonl");
        let log = Arc::new(Log::open(&path).unwrap());

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let log = Arc::clone(&log);
                thread::spawn(move || {
                    for i in 0u64..100 {
                        log.append(&sample_read_error(i * 512)).unwrap();
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 400, "must have exactly 400 lines");
        for line in &lines {
            serde_json::from_str::<AuditEvent>(line)
                .expect("every line must be a valid JSON audit event");
        }
    }
}
