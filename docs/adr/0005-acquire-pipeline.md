# ADR 0005 — Acquisition pipeline design (Phase 3)

**Status:** Accepted  
**Date:** 2026-04-09

## Context

Phase 3 implements the core imaging pipeline in `iridium-acquire`:
`DeviceReader → HashFanOut → ImageWriter`.

The pipeline must compute one or more streaming hashes (MD5, SHA-1, SHA-256)
simultaneously with the read and write, matching Guymager's forensic guarantees:
input hash = output hash, computed over the same byte stream, documented in the
acquisition report.

## Decision

**Single-threaded, sequential read → hash → write loop per chunk.**

Each iteration:
1. Read one chunk (default 1 MiB, configurable) from `DeviceReader`.
2. Feed the chunk to every active `StreamHasher` in sequence.
3. Write the chunk to the `ImageWriter`.
4. Emit a `ProgressEvent::Chunk` on the progress channel.
5. Check the cancel flag; abort cleanly if set.

On a read error, the chunk is zero-filled, `bad_sectors` is incremented, and
the loop continues (Phase 6 adds dd_rescue-style recovery).

## Rationale

- **I/O is the bottleneck.**  Even fast NVMe tops out at ~3–7 GB/s; hashing
  three algorithms over a 1 MiB chunk costs ~1 ms on modern hardware.
  Parallelising the hashing would add synchronisation overhead for no
  measurable gain.

- **Simplicity is a forensic virtue.**  A single-threaded loop has no shared
  state, no race conditions, and is trivially auditable.  Guymager's core
  pipeline is also sequential per chunk; its threading is used for the GUI
  progress update, not for the I/O path itself.

- **The `ImageWriter` trait decouples output format from the loop.**  Phase 4
  drops in `EwfWriter` without touching `pipeline.rs`.

- **`crossbeam-channel` handles progress delivery.**  The pipeline uses a
  non-blocking `try_send` so a slow UI thread cannot stall acquisition.

## Consequences

- The pipeline runs on whatever thread calls `run()` / `run_with_writer()`.
  The GUI (Phase 7) will call it on a dedicated background thread and poll the
  progress channel on the UI thread.

- If profiling on fast NVMe ever shows that hashing is a bottleneck (unlikely
  with ≤3 algorithms), a multi-buffer FIFO can be introduced in Phase 7 without
  changing the `ImageWriter` or `StreamHasher` APIs.

- `iridium-audit` integration is stubbed in Phase 3 (no-op `audit_start` /
  `audit_end` functions in `pipeline.rs`) and will be wired in Phase 5.

## Alternatives considered

| Option | Rejected because |
|--------|-----------------|
| Rayon parallel hashing | Overhead > benefit for ≤3 algorithms; adds complexity |
| Tokio async pipeline | O_DIRECT block device reads are synchronous; async adds no value |
| Crossbeam pipeline (dedicated reader/writer threads) | Premature complexity; revisit in Phase 7 if needed |
