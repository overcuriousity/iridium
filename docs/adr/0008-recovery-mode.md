# ADR 0008 — Recovery Mode Pipeline

**Status:** Accepted  
**Phase:** 6

## Context

The Phase 3 acquisition pipeline (`iridium-acquire`) handles read errors by
zero-filling and continuing — correct for healthy disks, inadequate for
failing media.  On a dying disk the priority is to rescue as much data as
possible before the disk degrades further.  The forensic standard
(`dd_rescue`, GNU `ddrescue`, spec §5.1) is to:

1. Skip bad regions on a fast forward pass.
2. Re-attack bad regions at sector granularity from both ends.
3. Retry individual failed sectors before declaring them unrecoverable.
4. Record sector-level status in a mapfile for post-mortem analysis and
   potential resume.

## Decision

Implement recovery as a separate `iridium-recovery` crate with its own
`run_recovery(job, opts)` entry point rather than as a flag on `AcquireJob`.
The existing pipeline is forward-sequential by design; recovery requires
out-of-order writes that are incompatible with `ImageWriter`'s sequential
contract.

### Four-pass algorithm

| Pass | Reads at | On success | On error |
|------|----------|------------|----------|
| 1 — forward | chunk_size (1 MiB default) | mark `+`, write | mark `*`, zero-fill, continue |
| 2 — trim | sector_size from both ends | mark `+`, write | stop scan from that side |
| 3 — scrape | sector_size, up to N retries | mark `+`, write | mark `-`, zero-fill |
| 4 — hash | 1 MiB sequential re-read of output image | compute digests | — |

Pass 4 hashes the completed output image (not the source device) because
streaming hashes computed during out-of-order writes are meaningless.

### Output format: raw only

EWF (`libewf`) requires strictly sequential writes; recovery writes at
arbitrary offsets.  Recovery therefore produces a raw `.img` file via
`pwrite`.  A future transcode helper can convert raw → EWF.

### Mapfile format: GNU ddrescue-compatible

The mapfile uses the [GNU ddrescue v1.27 mapfile
format](https://www.gnu.org/software/ddrescue/manual/ddrescue_manual.html#Mapfile-structure):

```
# Mapfile. Created by iridium-recovery v<ver>
# Command line: <argv>
# Start time:   <RFC3339>
# Current time: <RFC3339>
# current_pos  current_status  current_pass
0x000000000000  ?  1
#      pos        size  status
0x000000000000  0x000010000000  +
0x000010000000  0x000000001000  -
```

Status characters follow the ddrescue convention (`?` non-tried, `*`
non-trimmed, `/` non-scraped, `-` bad-sector, `+` finished).

This format was chosen over custom JSON because:

- Interoperable with `ddrescuelog`, `ddrescue --domain-mapfile`, and common
  forensic tooling.
- Accepted as evidence documentation in chain-of-custody workflows.
- A mapfile is a natural forensic artefact alongside the image.

The mapfile is rewritten atomically (write `.map.tmp` → rename) after each
pass and periodically within passes (default every 30 s).

### Resume: deferred

Phase 6 writes but does not consume the mapfile on startup.  Resume from a
partial run (reading the existing mapfile and skipping finished regions) is
deferred to a future phase.

### BlockReader abstraction

A local `BlockReader` trait (defined in `iridium-recovery::passes`) adapts
`iridium_device::DeviceReader` and allows a `MockReader` in unit tests
without requiring a real device, sysfs, or `O_DIRECT` support.

### Audit events

Five new `AuditEvent` variants are added to `iridium-audit` (non-breaking
because the enum uses `#[serde(tag)]`):

- `RecoveryStarted` — analogous to `Start`; captures job metadata and mapfile path.
- `RecoveryPassStarted` — emitted at the beginning of each pass.
- `RecoveryReadError` — emitted per bad sector (carries ddrescue status char).
- `MapfileFlushed` — emitted after each atomic mapfile rewrite.
- `RecoveryCompleted` — carries final digests, finished and bad byte counts.

## Consequences

- Recovery mode produces a raw `.img` + a `.map` alongside the existing
  normal-acquisition `.img` / `.E01` artefacts.
- Post-acquisition hashing adds one full sequential re-read of the output
  image; this is unavoidable and forensically correct.
- EWF output from recovery requires a separate transcode step (not yet
  implemented).
- Resume support requires reading the mapfile at startup and initialising
  `MapState` from its contents instead of from scratch.
