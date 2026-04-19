# ADR 0006 — EWF Writer: design and integration (Phase 4)

**Status:** Accepted  
**Date:** 2026-04-19

## Context

Phase 3 established the `ImageWriter` trait in `iridium-acquire` with a single concrete
implementation (`RawWriter`, flat `.img` output). Phase 4 adds `EwfWriter`, which produces
EnCase 6 EWF segment files (`.E01`) via `iridium-ewf` / libewf. Two non-trivial design
constraints had to be resolved before the trait extension was possible.

## Decisions

### 1. `EwfWriter` lives in `iridium-acquire`, not `iridium-ewf`

`ImageWriter` is defined in `iridium-acquire`. If `EwfWriter` were defined in `iridium-ewf`,
that crate would need to depend on `iridium-acquire` for the trait — creating a circular
dependency. Keeping `EwfWriter` in `iridium-acquire` (which depends on `iridium-ewf`) avoids
the cycle and keeps the writer abstraction in one place.

### 2. `embed_digests` default method for pre-finalize hash embedding

libewf requires hash values to be set on the handle **before** `write_finalize()` is called.
The pipeline previously finalized the writer before computing digests. To preserve the
`ImageWriter` contract without breaking `RawWriter` or requiring callers to orchestrate
two-phase finalization manually, a default `embed_digests(&mut self, digests: &[Digest])`
method was added to the trait. The pipeline calls it after hashing but before `finalize`:

```
hashers.finish() → writer.embed_digests(digests) → writer.finalize()
```

`RawWriter` keeps the inherited no-op default. Only EWF-style writers need to override it.

### 3. `unsafe impl Send for EwfHandle`

`EwfHandle` was `!Send` (via `PhantomData<*mut ()>`). `ImageWriter: Send` is required so the
pipeline can be moved to a background thread (Phase 7 GUI). `Send` permits transferring
ownership of a handle to another thread; it does **not** by itself justify concurrent use of
multiple `EwfHandle`s on different threads simultaneously.

At the Rust level, all `EwfHandle` methods take `&mut self`, so one handle cannot be used
concurrently through aliased references. Soundness additionally requires that libewf calls are
serialized process-wide — independent handles must not be driven concurrently from different
threads. Because this is a safety requirement for `unsafe impl Send`, it is **enforced inside
`iridium-ewf`** rather than assumed from caller behaviour: a `static LIBEWF_LOCK: Mutex<()>`
is acquired at the start of every FFI-calling method, including `Drop`. Safe Rust code cannot
bypass this lock, so the `Send` impl is sound regardless of how many handles or threads exist.

### 4. Short-write loop in `write_chunk`

`libewf_handle_write_buffer` can return fewer bytes than requested. `EwfWriter::write_chunk`
loops until all bytes in the chunk are consumed, matching the `write_all` semantics expected
by the pipeline. A return of `0` bytes without an error is treated as a write stall and
surfaced as `AcquireError::EwfWrite`.

### 5. Hashes stored via `set_hash_value` (hex strings)

libewf provides both `set_hash_value` (hex string, any algorithm) and `set_md5_hash` /
`set_sha1_hash` (raw bytes, fixed algorithms). Using the uniform `set_hash_value` path for
all three algorithms (MD5, SHA-1, SHA-256) simplifies the embedding loop — no per-algorithm
branching for the raw-byte variants, and SHA-256 is only supported by the hex-string API.

### 6. libewf constants re-exported from `iridium-ewf`

The format and media constants (`LIBEWF_FORMAT_*`, `LIBEWF_MEDIA_TYPE_*`, `LIBEWF_MEDIA_FLAG_*`)
are re-exported from `iridium-ewf` so callers do not need a direct dependency on
`iridium-ewf-sys`. This keeps `iridium-ewf-sys` as an implementation detail.

## Consequences

- `run_ewf(job)` is the one-line entry point for EWF output; `run_with_writer` remains the
  extension point for custom writers.
- Cancelled acquisitions produce an EWF without embedded hashes (write_finalize still runs;
  the file is valid but forensically incomplete by design).
- Phase 5 (audit log) will record digest values from `AcquireResult::digests`, which are
  computed in the pipeline regardless of output format.
- Phase 8 will vendor libewf via autotools; `EwfWriter` is unaffected by that build change.
