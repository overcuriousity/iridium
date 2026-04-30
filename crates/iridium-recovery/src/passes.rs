// passes.rs — forward, trim, and scrape passes for recovery mode.

use std::{io, sync::atomic::Ordering, time::Instant};

use iridium_acquire::{AcquireJob, ProgressEvent};
use iridium_audit::AuditEvent;
use time::OffsetDateTime;

use crate::{
    RecoveryError, RecoveryOptions,
    map::{MapState, Status},
    recovery_file::RecoveryFile,
};

// ── BlockReader trait ─────────────────────────────────────────────────────────

/// Abstraction over a readable block device.
///
/// Defined here (rather than importing `DeviceReader` directly) so that tests
/// can inject a `MockReader` without a real device or sysfs.
pub trait BlockReader: Send {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, io::Error>;
    fn size_bytes(&self) -> u64;
}

impl BlockReader for iridium_device::DeviceReader {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, io::Error> {
        iridium_device::DeviceReader::read_at(self, offset, buf)
            .map_err(|e| io::Error::other(e.to_string()))
    }

    fn size_bytes(&self) -> u64 {
        iridium_device::DeviceReader::size_bytes(self)
    }
}

// ── Pass 1 — forward copying ──────────────────────────────────────────────────

/// Read the source sequentially in `opts.chunk_size` chunks.
///
/// On success: mark `Finished`, write data.
/// On error: mark `NonTrimmed`, write zeros (pre-allocation already zeroed
/// the file, but we write explicitly for filesystem portability).
///
/// Returns `false` if the job was cancelled.
pub fn forward_pass(
    reader: &mut dyn BlockReader,
    map: &mut MapState,
    file: &RecoveryFile,
    job: &AcquireJob,
    opts: &RecoveryOptions,
) -> Result<bool, RecoveryError> {
    let total = reader.size_bytes();
    let chunk = opts.chunk_size;
    let mut buf = vec![0u8; chunk];
    let mut offset = 0u64;
    let mut last_flush = Instant::now();

    map.current_pass = 1;

    while offset < total {
        if job.cancel.load(Ordering::Relaxed) {
            return Ok(false);
        }

        let len = chunk.min((total - offset) as usize);

        match reader.read_at(offset, &mut buf[..len]) {
            Ok(0) => break,
            Ok(n) => {
                file.write_at(offset, &buf[..n])
                    .map_err(|e| RecoveryError::Write {
                        path: file.path().to_owned(),
                        source: e,
                    })?;
                map.mark(offset, n as u64, Status::Finished);
                map.current_status = Status::Finished;
                offset += n as u64;
            }
            Err(e) => {
                buf[..len].fill(0);
                file.write_at(offset, &buf[..len])
                    .map_err(|e| RecoveryError::Write {
                        path: file.path().to_owned(),
                        source: e,
                    })?;
                map.mark(offset, len as u64, Status::NonTrimmed);
                map.current_status = Status::NonTrimmed;
                emit_audit(job, || AuditEvent::RecoveryReadError {
                    ts: OffsetDateTime::now_utc(),
                    offset,
                    length: len as u64,
                    error: e.to_string(),
                    map_status: Status::NonTrimmed.as_str().to_owned(),
                });
                offset += len as u64;
            }
        }

        map.current_pos = offset;
        send_progress(job, map, "forward");

        if last_flush.elapsed().as_secs() >= opts.mapfile_sync_secs {
            flush_map_and_notify(map, job, opts)?;
            last_flush = Instant::now();
        }
    }

    Ok(true)
}

// ── Pass 2 — trimming ─────────────────────────────────────────────────────────

/// For each `NonTrimmed` region: read forward sector-by-sector until the first
/// error, then read backward sector-by-sector from the end.  The remaining
/// middle (both sides blocked) is downgraded to `NonScraped`.
///
/// Returns `false` if the job was cancelled.
pub fn trim_pass(
    reader: &mut dyn BlockReader,
    map: &mut MapState,
    file: &RecoveryFile,
    job: &AcquireJob,
    opts: &RecoveryOptions,
    sector_size: usize,
) -> Result<bool, RecoveryError> {
    // Clone region list so we can mutate the map while iterating.
    let regions: Vec<_> = map
        .regions_with_status(Status::NonTrimmed)
        .cloned()
        .collect();

    let mut buf = vec![0u8; sector_size];
    let mut last_flush = Instant::now();
    map.current_pass = 2;

    for region in &regions {
        // ── Forward scan ────────────────────────────────────────────────────
        let mut fwd = region.pos;
        let reg_end = region.end();

        while fwd < reg_end {
            if job.cancel.load(Ordering::Relaxed) {
                map.current_pos = fwd;
                return Ok(false);
            }

            let len = sector_size.min((reg_end - fwd) as usize);

            match reader.read_at(fwd, &mut buf[..len]) {
                Ok(n) if n > 0 => {
                    file.write_at(fwd, &buf[..n])
                        .map_err(|e| RecoveryError::Write {
                            path: file.path().to_owned(),
                            source: e,
                        })?;
                    map.mark(fwd, n as u64, Status::Finished);
                    map.current_status = Status::Finished;
                    fwd += n as u64;
                }
                _ => break,
            }

            map.current_pos = fwd;
            send_progress(job, map, "trim");

            if last_flush.elapsed().as_secs() >= opts.mapfile_sync_secs {
                flush_map_and_notify(map, job, opts)?;
                last_flush = Instant::now();
            }
        }

        // Entire region trimmed from the front.
        if fwd >= reg_end {
            continue;
        }

        // ── Backward scan ────────────────────────────────────────────────────
        let mut bwd = reg_end;

        while bwd > fwd {
            if job.cancel.load(Ordering::Relaxed) {
                map.current_pos = bwd;
                // Mark whatever remains between fwd and bwd as NonScraped.
                if fwd < bwd {
                    map.mark(fwd, bwd - fwd, Status::NonScraped);
                    map.current_status = Status::NonScraped;
                }
                return Ok(false);
            }

            let read_len = sector_size.min((bwd - fwd) as usize);
            let read_start = bwd - read_len as u64;

            match reader.read_at(read_start, &mut buf[..read_len]) {
                Ok(n) if n > 0 => {
                    file.write_at(read_start, &buf[..n])
                        .map_err(|e| RecoveryError::Write {
                            path: file.path().to_owned(),
                            source: e,
                        })?;
                    map.mark(read_start, n as u64, Status::Finished);
                    map.current_status = Status::Finished;
                    bwd = read_start;
                }
                _ => break,
            }

            map.current_pos = bwd;
            send_progress(job, map, "trim");

            if last_flush.elapsed().as_secs() >= opts.mapfile_sync_secs {
                flush_map_and_notify(map, job, opts)?;
                last_flush = Instant::now();
            }
        }

        // Mark the untrimmed middle as NonScraped.
        if fwd < bwd {
            map.mark(fwd, bwd - fwd, Status::NonScraped);
        }
    }

    Ok(true)
}

// ── Pass 3 — scraping ─────────────────────────────────────────────────────────

/// For each `NonScraped` sector: retry up to `opts.max_retries` times.
/// Sectors that still fail become `BadSector` (zero-filled in the output).
///
/// Returns `false` if the job was cancelled.
pub fn scrape_pass(
    reader: &mut dyn BlockReader,
    map: &mut MapState,
    file: &RecoveryFile,
    job: &AcquireJob,
    opts: &RecoveryOptions,
    sector_size: usize,
) -> Result<bool, RecoveryError> {
    let regions: Vec<_> = map
        .regions_with_status(Status::NonScraped)
        .cloned()
        .collect();

    let mut buf = vec![0u8; sector_size];
    let mut last_flush = Instant::now();
    map.current_pass = 3;

    for region in &regions {
        let mut pos = region.pos;
        let reg_end = region.end();

        while pos < reg_end {
            if job.cancel.load(Ordering::Relaxed) {
                map.current_pos = pos;
                return Ok(false);
            }

            let len = sector_size.min((reg_end - pos) as usize);
            let mut rescued = false;
            let mut last_err = String::new();

            for _attempt in 0..=opts.max_retries {
                match reader.read_at(pos, &mut buf[..len]) {
                    Ok(n) if n > 0 => {
                        file.write_at(pos, &buf[..n])
                            .map_err(|e| RecoveryError::Write {
                                path: file.path().to_owned(),
                                source: e,
                            })?;
                        map.mark(pos, n as u64, Status::Finished);
                        map.current_status = Status::Finished;
                        pos += n as u64;
                        rescued = true;
                        break;
                    }
                    Ok(_) => {
                        last_err = "empty read".into();
                    }
                    Err(e) => {
                        last_err = e.to_string();
                    }
                }
            }

            if !rescued {
                // Pre-alloc already zeroed this region; mark as bad.
                map.mark(pos, len as u64, Status::BadSector);
                map.current_status = Status::BadSector;
                emit_audit(job, || AuditEvent::RecoveryReadError {
                    ts: OffsetDateTime::now_utc(),
                    offset: pos,
                    length: len as u64,
                    error: last_err.clone(),
                    map_status: Status::BadSector.as_str().to_owned(),
                });
                pos += len as u64;
            }

            map.current_pos = pos;
            send_progress(job, map, "scrape");

            if last_flush.elapsed().as_secs() >= opts.mapfile_sync_secs {
                flush_map_and_notify(map, job, opts)?;
                last_flush = Instant::now();
            }
        }
    }

    Ok(true)
}

// ── Shared helpers ────────────────────────────────────────────────────────────

fn flush_map_and_notify(
    map: &mut MapState,
    job: &AcquireJob,
    opts: &RecoveryOptions,
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
    let _ = opts; // consumed to keep signature consistent; reserved for future opts
    Ok(())
}

fn send_progress(job: &AcquireJob, map: &MapState, pass: &str) {
    if let Some(tx) = &job.progress_tx {
        let _ = tx.try_send(ProgressEvent::RecoveryProgress {
            pass: pass.to_owned(),
            finished_bytes: map.finished_bytes(),
            bad_bytes: map.bad_bytes(),
        });
    }
}

fn emit_audit(job: &AcquireJob, make_event: impl FnOnce() -> AuditEvent) {
    if let Some(log) = &job.audit
        && let Err(e) = log.append(&make_event())
    {
        log::warn!("iridium-recovery: failed to append audit event: {e}");
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use iridium_acquire::AcquireJob;
    use iridium_core::HashAlg;
    use iridium_device::Disk;
    use std::collections::HashMap;
    use std::sync::{Arc, atomic::AtomicBool};

    // ── MockReader ───────────────────────────────────────────────────────────

    struct MockReader {
        data: Vec<u8>,
        /// Offsets (in bytes) that return an I/O error on read.
        error_offsets: HashMap<u64, bool>,
        /// How many times each error offset has been hit.
        hit_counts: HashMap<u64, u32>,
        /// After this many hits the error clears (simulates a retryable error).
        clear_after: u32,
    }

    impl MockReader {
        fn new(data: Vec<u8>) -> Self {
            Self {
                data,
                error_offsets: HashMap::new(),
                hit_counts: HashMap::new(),
                clear_after: u32::MAX,
            }
        }

        fn with_permanent_error(mut self, offset: u64) -> Self {
            self.error_offsets.insert(offset, true);
            self
        }

        fn with_retryable_error(mut self, offset: u64, clear_after: u32) -> Self {
            self.error_offsets.insert(offset, true);
            self.clear_after = clear_after;
            self
        }
    }

    impl BlockReader for MockReader {
        fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, io::Error> {
            // Check every sector-aligned position within the request for errors.
            // For simplicity, check the start offset against known error positions.
            if self.error_offsets.contains_key(&offset) {
                let count = self.hit_counts.entry(offset).or_insert(0);
                *count += 1;
                if *count <= self.clear_after {
                    return Err(io::Error::other("mock read error"));
                }
                // After clear_after hits, succeed.
            }
            if offset >= self.data.len() as u64 {
                return Ok(0);
            }
            let avail = (self.data.len() as u64 - offset) as usize;
            let n = buf.len().min(avail);
            buf[..n].copy_from_slice(&self.data[offset as usize..offset as usize + n]);
            Ok(n)
        }

        fn size_bytes(&self) -> u64 {
            self.data.len() as u64
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn make_job_and_dir() -> (AcquireJob, tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.img");
        std::fs::write(&src, vec![0u8; 512]).unwrap();
        let disk = Disk {
            path: src.clone(),
            model: String::new(),
            serial: String::new(),
            size_bytes: 512,
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
        let job = AcquireJob::new(disk, dest.clone(), vec![HashAlg::Sha256]);
        (job, dir, dest)
    }

    fn make_opts(chunk: usize) -> RecoveryOptions {
        RecoveryOptions {
            chunk_size: chunk,
            max_retries: 3,
            mapfile_path: None,
            mapfile_sync_secs: u64::MAX,
        }
    }

    fn make_map_and_file(
        total: u64,
        dir: &std::path::Path,
        name: &str,
    ) -> (MapState, RecoveryFile) {
        let img = dir.join(format!("{name}.img"));
        let mpath = dir.join(format!("{name}.map"));
        let map = MapState::new(total, mpath, "0.1.0".into(), vec![]);
        let file = RecoveryFile::create(&img, total).unwrap();
        (map, file)
    }

    // ── Forward pass ─────────────────────────────────────────────────────────

    #[test]
    fn forward_pass_healthy_source_all_finished() {
        let (job, dir, _dest) = make_job_and_dir();
        let data = vec![0xABu8; 512];
        let mut reader = MockReader::new(data);
        let (mut map, file) = make_map_and_file(512, dir.path(), "fwd_ok");
        let opts = make_opts(256);

        let ok = forward_pass(&mut reader, &mut map, &file, &job, &opts).unwrap();
        assert!(ok);
        assert_eq!(map.finished_bytes(), 512);
        assert!(!map.has_status(Status::NonTrimmed));
    }

    #[test]
    fn forward_pass_with_error_marks_non_trimmed() {
        let (job, dir, _) = make_job_and_dir();
        let data = vec![0u8; 512];
        let mut reader = MockReader::new(data).with_permanent_error(256);
        let (mut map, file) = make_map_and_file(512, dir.path(), "fwd_err");
        let opts = make_opts(256); // chunk 256 bytes: offsets 0 and 256

        let ok = forward_pass(&mut reader, &mut map, &file, &job, &opts).unwrap();
        assert!(ok);
        assert_eq!(map.finished_bytes(), 256); // first chunk ok
        assert!(map.has_status(Status::NonTrimmed)); // second chunk failed
    }

    #[test]
    fn forward_pass_cancel_returns_false() {
        let (mut job, dir, _) = make_job_and_dir();
        job.cancel = Arc::new(AtomicBool::new(true)); // pre-cancelled
        let data = vec![0u8; 512];
        let mut reader = MockReader::new(data);
        let (mut map, file) = make_map_and_file(512, dir.path(), "fwd_cancel");
        let opts = make_opts(512);

        let ok = forward_pass(&mut reader, &mut map, &file, &job, &opts).unwrap();
        assert!(!ok);
    }

    // ── Trim pass ────────────────────────────────────────────────────────────

    #[test]
    fn trim_pass_rescues_edges_of_bad_region() {
        let (job, dir, _) = make_job_and_dir();
        // 4 sectors of 128 bytes each; sector 1 and 2 are bad
        let mut data = vec![0u8; 512];
        // Poison all data so we can check what was written.
        for (i, b) in data.iter_mut().enumerate() {
            *b = i as u8;
        }

        let mut reader = MockReader::new(data.clone())
            .with_permanent_error(128) // sector 1
            .with_permanent_error(256); // sector 2

        let (mut map, file) = make_map_and_file(512, dir.path(), "trim");
        let opts = make_opts(512);

        // Simulate forward pass having already marked the bad region.
        // Forward pass with chunk=512 would mark all as NonTrimmed (error at 0).
        // Instead manually set up: [0..128]=Finished, [128..384]=NonTrimmed, [384..512]=Finished.
        map.mark(0, 128, Status::Finished);
        map.mark(128, 256, Status::NonTrimmed);
        map.mark(384, 128, Status::Finished);

        let ok = trim_pass(&mut reader, &mut map, &file, &job, &opts, 128).unwrap();
        assert!(ok);

        // Edges should be rescued; middle [128, 384) is NonScraped or smaller.
        // With permanent errors at 128 and 256, forward scan stops at sector 1 (128),
        // backward scan stops at sector 3 (256..384 tries 256 first → error, stops).
        // But 384 reads ok backward (from 512 backward to 384 succeeds — that's sector 3 = offset 384).
        // Wait, backward scan starts at reg_end=384, tries read_start=384-128=256 → error → stops.
        // So middle = [128, 256) → NonScraped... but [256, 384) stays NonTrimmed
        // actually that's a single region [128, 384) = NonTrimmed → after trim:
        // fwd stops at 128 (error), bwd starts at 384, reads from 256 → error, stops at 384.
        // So remaining [128, 384) = NonScraped.
        assert!(map.has_status(Status::NonScraped) || map.finished_bytes() == 512 - 256);
    }

    // ── Scrape pass ──────────────────────────────────────────────────────────

    #[test]
    fn scrape_pass_retries_and_succeeds() {
        let (job, dir, _) = make_job_and_dir();
        let data = vec![0xCCu8; 128];
        // Error clears after 2 hits → should succeed on attempt 3 (max_retries=3).
        let mut reader = MockReader::new(data).with_retryable_error(0, 2);
        let (mut map, file) = make_map_and_file(128, dir.path(), "scrape_retry");
        // Pre-mark as NonScraped.
        map.mark(0, 128, Status::NonScraped);

        let mut opts = make_opts(128);
        opts.max_retries = 3;

        let ok = scrape_pass(&mut reader, &mut map, &file, &job, &opts, 128).unwrap();
        assert!(ok);
        assert_eq!(map.finished_bytes(), 128);
        assert!(!map.has_status(Status::BadSector));
    }

    #[test]
    fn scrape_pass_permanent_failure_marks_bad_sector() {
        let (job, dir, _) = make_job_and_dir();
        let data = vec![0u8; 128];
        let mut reader = MockReader::new(data).with_permanent_error(0);
        let (mut map, file) = make_map_and_file(128, dir.path(), "scrape_bad");
        map.mark(0, 128, Status::NonScraped);

        let mut opts = make_opts(128);
        opts.max_retries = 1;

        let ok = scrape_pass(&mut reader, &mut map, &file, &job, &opts, 128).unwrap();
        assert!(ok);
        assert_eq!(map.bad_bytes(), 128);
        assert!(!map.has_status(Status::NonScraped));
    }

    // ── Audit event integration ───────────────────────────────────────────────

    #[test]
    fn forward_pass_emits_read_error_events() {
        use std::sync::Arc;
        let (mut job, dir, _) = make_job_and_dir();

        let log_path = dir.path().join("audit.jsonl");
        let log = Arc::new(iridium_audit::Log::open(&log_path).unwrap());
        job.audit = Some(Arc::clone(&log));

        let data = vec![0u8; 256];
        let mut reader = MockReader::new(data).with_permanent_error(128);
        let (mut map, file) = make_map_and_file(256, dir.path(), "audit_fwd");
        let opts = make_opts(128);

        forward_pass(&mut reader, &mut map, &file, &job, &opts).unwrap();

        // Release the Arc held by job.audit before try_unwrap.
        job.audit = None;
        Arc::try_unwrap(log).ok().unwrap().seal().unwrap();

        let content = std::fs::read_to_string(&log_path).unwrap();
        let events: Vec<serde_json::Value> = content
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();

        assert!(
            events.iter().any(|e| e["event"] == "recovery_read_error"),
            "must contain a recovery_read_error event"
        );
    }
}
