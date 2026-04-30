// pipeline.rs — core read → hash → write loop.

use std::sync::atomic::Ordering;

use iridium_audit::{AuditEvent, DigestRecord, JobMetadata};
use iridium_hash::new_hasher;
use time::OffsetDateTime;

use crate::{AcquireError, AcquireJob, AcquireResult, ProgressEvent, writer::ImageWriter};

/// Run the acquisition pipeline to completion (or cancellation).
///
/// The caller constructs a concrete [`ImageWriter`] and passes it in so that
/// Phase 4 can substitute an `EwfWriter` without touching this function.
pub(crate) fn run(
    job: &AcquireJob,
    mut writer: Box<dyn ImageWriter>,
) -> Result<AcquireResult, AcquireError> {
    // Validate unconditionally in the pipeline.
    // Some public entry points also validate earlier so invalid jobs can be
    // rejected before creating the output, but callers that reach this
    // function are still checked here.
    crate::validate_job(job)?;

    let mut reader = job
        .source
        .open_read_only()
        .map_err(|e| AcquireError::DeviceOpen {
            path: job.source.path.clone(),
            source: e,
        })?;

    emit_audit(job, || AuditEvent::Start {
        ts: OffsetDateTime::now_utc(),
        iridium_version: env!("CARGO_PKG_VERSION").to_owned(),
        libewf_version: iridium_ewf::libewf_version().to_owned(),
        argv: std::env::args_os()
            .map(|a| a.to_string_lossy().into_owned())
            .collect(),
        job: job_metadata(job),
    });

    let total_bytes = job.source.size_bytes;
    send(job, ProgressEvent::Started { total_bytes });

    let mut hashers: Vec<Box<dyn iridium_hash::StreamHasher>> =
        job.algorithms.iter().copied().map(new_hasher).collect();

    let chunk_size = job.chunk_size;
    let mut buf = vec![0u8; chunk_size];
    let mut offset: u64 = 0;
    let mut bad_chunks: u64 = 0;

    loop {
        // Check for cancellation between chunks.
        if job.cancel.load(Ordering::Relaxed) {
            writer.discard()?;
            let result = AcquireResult {
                digests: vec![],
                bytes_processed: offset,
                bad_chunks,
                complete: false,
            };
            send(job, ProgressEvent::Cancelled { bytes_done: offset });
            emit_audit(job, || AuditEvent::Cancelled {
                ts: OffsetDateTime::now_utc(),
                bytes_processed: offset,
                bad_chunks,
            });
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
                let error_str = e.to_string();
                log::warn!(
                    "iridium-acquire: read error at offset {offset}: {error_str}; \
                     zero-filling chunk and continuing"
                );
                bad_chunks += 1;
                let fill_len = chunk_size.min((total_bytes - offset) as usize);
                // Zero-fill the existing buffer slice to avoid a fresh allocation.
                buf[..fill_len].fill(0);
                for h in &mut hashers {
                    h.update(&buf[..fill_len]);
                }
                writer.write_chunk(&buf[..fill_len])?;
                emit_audit(job, || AuditEvent::ReadError {
                    ts: OffsetDateTime::now_utc(),
                    offset,
                    length: fill_len as u64,
                    error: error_str,
                    bad_chunks_total: bad_chunks,
                });
                fill_len
            }
        };

        offset += n as u64;

        send(
            job,
            ProgressEvent::Chunk {
                bytes_done: offset,
                bad_chunks,
            },
        );
    }

    let digests: Vec<_> = hashers.into_iter().map(|h| h.finish()).collect();

    // Allow the writer to embed hash metadata before the file is sealed.
    // RawWriter ignores this; EwfWriter uses it to store digest strings.
    // Always finalize so the container is structurally valid; if both fail,
    // the structural error takes priority.
    let embed_result = writer.embed_digests(&digests);
    writer.finalize()?;
    embed_result?;

    let result = AcquireResult {
        digests,
        bytes_processed: offset,
        bad_chunks,
        complete: true,
    };

    send(
        job,
        ProgressEvent::Completed {
            result: result.clone(),
        },
    );

    emit_audit(job, || AuditEvent::Completed {
        ts: OffsetDateTime::now_utc(),
        bytes_processed: result.bytes_processed,
        bad_chunks: result.bad_chunks,
        digests: result
            .digests
            .iter()
            .map(|d| DigestRecord {
                algorithm: d.algorithm,
                hex: d.hex.clone(),
            })
            .collect(),
    });

    Ok(result)
}

fn send(job: &AcquireJob, event: ProgressEvent) {
    if let Some(tx) = &job.progress_tx {
        // Best-effort: never let a slow or full channel stall acquisition.
        // Callers that need guaranteed delivery of every event should supply
        // an unbounded channel (crossbeam_channel::unbounded).
        let _ = tx.try_send(event);
    }
}

/// Best-effort audit event emission.  The closure is only called when an
/// audit log is present, avoiding any allocation for event construction when
/// auditing is disabled.  Append failures are reported via `log::warn!` and
/// never abort the acquisition.
fn emit_audit(job: &AcquireJob, make_event: impl FnOnce() -> AuditEvent) {
    if let Some(audit_log) = &job.audit
        && let Err(e) = audit_log.append(&make_event())
    {
        log::warn!("iridium-audit: failed to append audit event: {e}");
    }
}

/// Build a [`JobMetadata`] snapshot from the current job state.
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
