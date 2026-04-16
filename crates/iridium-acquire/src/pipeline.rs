// pipeline.rs — core read → hash → write loop.

use std::sync::atomic::Ordering;

use iridium_hash::new_hasher;

use crate::{AcquireError, AcquireJob, AcquireResult, ProgressEvent, writer::ImageWriter};

/// Run the acquisition pipeline to completion (or cancellation).
///
/// The caller constructs a concrete [`ImageWriter`] and passes it in so that
/// Phase 4 can substitute an `EwfWriter` without touching this function.
pub(crate) fn run(
    job: &AcquireJob,
    mut writer: Box<dyn ImageWriter>,
) -> Result<AcquireResult, AcquireError> {
    if job.algorithms.is_empty() {
        return Err(AcquireError::NoAlgorithms);
    }
    if job.chunk_size == 0 {
        return Err(AcquireError::InvalidChunkSize);
    }

    // TODO(phase-5): wire iridium-audit — log acquisition start with job metadata.
    audit_start(job);

    let total_bytes = job.source.size_bytes;
    send(&job, ProgressEvent::Started { total_bytes });

    let mut hashers: Vec<Box<dyn iridium_hash::StreamHasher>> =
        job.algorithms.iter().copied().map(new_hasher).collect();

    let chunk_size = job.chunk_size;
    let mut buf = vec![0u8; chunk_size];
    let mut offset: u64 = 0;
    let mut bad_sectors: u64 = 0;

    let mut reader = job
        .source
        .open_read_only()
        .map_err(|e| AcquireError::DeviceOpen {
            path: job.source.path.clone(),
            source: e,
        })?;

    loop {
        // Check for cancellation between chunks.
        if job.cancel.load(Ordering::Relaxed) {
            writer.finalize()?;
            let result = AcquireResult {
                digests: vec![],
                bytes_processed: offset,
                bad_sectors,
                complete: false,
            };
            send(&job, ProgressEvent::Cancelled { bytes_done: offset });
            // TODO(phase-5): wire iridium-audit — log cancellation.
            audit_end(&result);
            return Ok(result);
        }

        let n = match reader.read_at(offset, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let chunk = &buf[..n];
                for h in &mut hashers {
                    h.update(chunk);
                }
                writer.write_chunk(chunk)?;
                n
            }
            Err(e) => {
                // Zero-fill the sector(s) covered by this chunk and continue.
                log::warn!(
                    "iridium-acquire: read error at offset {offset}: {e}; \
                     zero-filling chunk and continuing"
                );
                bad_sectors += 1;
                let fill_len = chunk_size.min((total_bytes - offset) as usize);
                // Zero-fill the existing buffer slice to avoid a fresh allocation.
                buf[..fill_len].fill(0);
                for h in &mut hashers {
                    h.update(&buf[..fill_len]);
                }
                writer.write_chunk(&buf[..fill_len])?;
                fill_len
            }
        };

        offset += n as u64;

        send(
            &job,
            ProgressEvent::Chunk {
                bytes_done: offset,
                bad_sectors,
            },
        );
    }

    writer.finalize()?;

    let digests: Vec<_> = hashers.into_iter().map(|h| h.finish()).collect();

    let result = AcquireResult {
        digests,
        bytes_processed: offset,
        bad_sectors,
        complete: true,
    };

    send(
        &job,
        ProgressEvent::Completed {
            result: result.clone(),
        },
    );

    // TODO(phase-5): wire iridium-audit — log acquisition end with digests.
    audit_end(&result);

    Ok(result)
}

fn send(job: &AcquireJob, event: ProgressEvent) {
    if let Some(tx) = &job.progress_tx {
        match event {
            // Terminal and control events must be delivered if the receiver is alive.
            ProgressEvent::Started { .. }
            | ProgressEvent::Completed { .. }
            | ProgressEvent::Cancelled { .. } => {
                let _ = tx.send(event);
            }
            // High-frequency progress updates are best-effort under backpressure.
            _ => {
                let _ = tx.try_send(event);
            }
        }
    }
}

// ── Phase 5 stubs ─────────────────────────────────────────────────────────────

/// TODO(phase-5): replace with real `iridium_audit::Log::record_start()` call.
fn audit_start(_job: &AcquireJob) {}

/// TODO(phase-5): replace with real `iridium_audit::Log::record_end()` call.
fn audit_end(_result: &AcquireResult) {}
