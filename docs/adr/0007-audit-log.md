# ADR 0007 — Append-only JSONL audit log (Phase 5)

**Status:** Accepted  
**Date:** 2026-04-30

## Context

Phases 1–4 delivered device enumeration, the acquisition pipeline, the hash layer, and the EWF
writer.  Read errors were logged only via `log::warn!` (a developer-facing text sink), and no
structured record of what was acquired — when, by whom, with what parameters, and with what
outcome — was ever persisted to disk.

Spec §4.3 requires: append-only, exportable log; precise timestamps on every event; exact
command and parameters recorded; library/program versions captured; optional PGP operator
signature.  Spec §2.3 requires read errors to be logged in a separate log file per occurrence.

## Decisions

### 1. JSONL (newline-delimited JSON) as the wire format

Each audit event is one JSON object on its own line, terminated by `\n`.  The file is opened
in append mode so multiple acquisitions can share a single log file and a crash mid-acquisition
leaves previously written events intact.  JSONL is human-readable with standard tools (`cat`,
`jq`), trivially parseable, and self-describing — no schema migration is needed when new event
types are added.

### 2. One `sync_data` per append

Each `Log::append` call calls `File::sync_data()` before returning.  This ensures every event
survives an OS or power failure without buffering events in memory.  The cost is acceptable
because audit events are infrequent compared to the millions of chunk writes in a typical
acquisition — a sync per event adds negligible I/O load.

### 3. `AuditEvent` discriminant via `#[serde(tag = "event")]`

The enum serialises as `{"event":"<variant_name>", ...fields}`.  The `rename_all = "snake_case"`
attribute produces lowercase, underscore-separated event names (`"start"`, `"read_error"`, …),
consistent with common log conventions.  Timestamps use RFC 3339 (ISO 8601 with mandatory
timezone offset) via `time::serde::rfc3339`.

### 4. `time` crate for timestamps

`time 0.3` was added as a workspace dependency (`features = ["serde", "formatting"]`).  It
produces compact, lossless RFC 3339 strings, has no C dependencies (clean static-musl build),
and its licence (MIT/Apache-2.0) is already covered by the `deny.toml` allow-list.  `chrono`
was considered but rejected due to a historically larger MSRV surface and the
`RUSTSEC-2020-0159` time-zone-related soundness issue.

### 5. Best-effort emission; audit errors never abort acquisition

If `Log::append` returns an error (e.g. the log volume is full), the pipeline logs a
`log::warn!` message and continues.  Aborting a forensic acquisition because the audit log
is unavailable would be worse than producing an image without a complete log.  The caller
is responsible for monitoring `AuditError`s if stricter behaviour is required.

### 6. `Log::seal` consumes the handle

`seal(self)` appends a final `{"event":"sealed",...}` record and the `Log` is then dropped.
Consuming `self` makes it a compile-time error to append further events after sealing, while
the absence of an explicit `close` method means accidentally dropping `Log` without sealing
is possible — callers must call `seal` to close a session cleanly.

### 7. `AcquireJob.audit: Option<Arc<Log>>`

The pipeline receives the audit log through the existing `AcquireJob` parameter rather than
through a side-channel or global.  `Option<Arc<Log>>` allows jobs with no audit log (the
default, backward-compatible) and enables sharing a single log across multiple jobs on
different threads.

### 8. `AcquireJob.format: Option<ImageFormat>` for metadata capture

The pipeline does not know which concrete `ImageWriter` was supplied via `run_with_writer`.
A `format: Option<ImageFormat>` field on `AcquireJob` lets callers annotate the output
format for inclusion in the `Start` event.  The `run()` and `run_ewf()` convenience entry
points set this automatically.

### 9. `argv` populated from `std::env::args()` for now

Spec §4.3 requires "every exact command with exact parameters" to be logged.  In Phase 5,
`iridium-app` is still a stub with no CLI.  The `Start` event captures raw process argv via
`std::env::args()`, which is correct for a CLI invocation and passable for the test harness.
Phase 7 will replace this with the resolved, typed clap configuration once the CLI shell
exists.

### 10. `libewf_version()` safe wrapper in `iridium-ewf-sys`

`iridium_ewf_sys::libewf_version()` wraps `libewf_get_version()` into a `&'static str` and
is re-exported from `iridium-ewf`.  The pipeline calls `iridium_ewf::libewf_version()` to
capture the library version in the `Start` event without introducing a direct dependency on
the sys crate.

## Consequences

- Every acquisition that supplies `AcquireJob::audit` produces a structured, durable event
  trail satisfying spec §2.3 and §4.3 (except PGP signing and resolved argv, deferred below).
- Existing callers that do not set `audit` are unaffected; the field defaults to `None`.
- Phase 7 (CLI/GUI shell) should populate `argv` from the resolved clap struct and provide a
  user-facing flag for specifying the log path.
- Phase 8 (or later) will add PGP signing of sealed logs via the `pgp` (rpgp) crate
  (MIT/Apache-2.0, no C deps, licence-clean against `deny.toml`).  The previously-stubbed
  `pgp-signing` cargo feature was removed in this phase — it will be re-added when the
  implementation lands.
- Segment-cleanup failures in `EwfWriter::discard` are still reported only via `log::warn!`.
  A future phase can surface them as `SegmentCleanupFailed` audit events once a suitable
  plumbing strategy is agreed.

## Alternatives considered

| Alternative | Reason rejected |
|---|---|
| `tracing` crate instead of `log` for pipeline | Would add a large dep for what is purely consumer-facing structured storage, not observability telemetry |
| Binary / CBOR format | Loses human-readability; JSONL is universally supported by forensic tooling |
| `chrono` for timestamps | `time` is lighter, licence-equivalent, and avoids the RUSTSEC-2020-0159 soundness concern |
| `sequoia-openpgp` for signing | LGPL-2.0+ is not in `deny.toml`'s allow-list; rpgp is equivalent and licence-clean |
| Fatal audit errors | Aborts the forensic acquisition on log failure — producing no image is worse than producing one with an incomplete log |
