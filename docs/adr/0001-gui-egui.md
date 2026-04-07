# ADR 0001 — GUI Framework: egui

**Status:** Accepted

## Context

iridium needs a GUI framework that can be statically linked into a single musl binary
for airgap deployment. GTK4 and Qt require system libraries and break the static-binary
constraint. The framework must run on Linux and be maintained in pure Rust.

## Decision

Use **egui** (+ eframe) for the GUI.

## Consequences

- No system GUI library dependencies; the binary is fully self-contained.
- Immediate-mode design requires explicit state management in `iridium-app`.
- egui crates are added to `[workspace.dependencies]` but only pulled into
  `iridium-app` at Phase 7 to avoid breaking earlier phase builds.
