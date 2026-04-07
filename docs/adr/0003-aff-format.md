# ADR 0003 — AFF Format Support

**Status:** Proposed — decision deferred to Phase 4

## Context

The spec requires AFF (Advanced Forensic Format) output. Two candidates exist:

- **libaff / libaff4**: C libraries, require FFI, similar integration path to libewf.
- **aff4-rs**: Pure-Rust AFF4 implementation; avoids C FFI but maturity is uncertain.

## Decision

Evaluation deferred to Phase 4. A prototype will test `aff4-rs` against reference AFF4
tooling (pyaff4, libaff4). If `aff4-rs` passes interop tests it is preferred (no C FFI).
Otherwise vendor libaff4 via a `iridium-aff-sys` crate following the same pattern as
`iridium-ewf-sys`.

## Consequences

- Phase 4 must include an interop milestone before merging AFF writer code.
- This ADR will be updated to "Accepted" once the decision is made.
