# ADR 0002 — libewf Integration: Static FFI Bindings

**Status:** Accepted

## Context

iridium must produce forensically correct EWF images. libewf is the authoritative
implementation of the Expert Witness Format used by EnCase, X-Ways, and other tools.
Options considered: subprocess (ewfacquire), dynamic linking, static linking.

## Decision

Vendor libewf as a git submodule and compile it statically via `build.rs` in the
`iridium-ewf-sys` crate using the `cc` crate. Bindgen generates bindings at build time
with a committed fallback for offline builds.

A `system-libewf` feature flag allows developers to skip the vendored build.

## Consequences

- Single binary guarantee is maintained.
- Reproducible builds: the exact libewf revision is pinned in the submodule.
- Phase 1 of the roadmap is dedicated to implementing `build.rs`.
