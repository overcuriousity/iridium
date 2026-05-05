# ADR 0009 — GUI architecture (Phase 7)

**Status:** Accepted
**Date:** 2026-04-30

## Context

Phase 7 adds the egui GUI shell to `iridium-app`. The acquisition and recovery
pipelines (Phases 3 and 6) are synchronous, single-threaded, and run on
whichever thread calls `run()` / `run_ewf()` / `run_recovery()` (see ADR 0005).
The GUI must show live progress without stalling the UI thread.

## Decision

**One worker thread per active job; unbounded progress channel; all UI state
owned by `eframe::App`.**

- `AppState` (the `eframe::App` impl) owns a job queue: `pending: VecDeque`,
  `active: Option<ActiveJob>`, `completed: Vec<CompletedJob>`.
- When a job is dequeued, `std::thread::spawn` runs the pipeline.  No thread
  pool — imaging is I/O-bound; concurrency between jobs would thrash the disk
  and complicate audit ordering.
- Progress is delivered over a `crossbeam_channel::unbounded()` channel.  The
  pipeline's `try_send` is lossless on an unbounded channel.
- Cancellation: `Arc<AtomicBool>` stored in both `ActiveJob` (for the Cancel
  button) and `AcquireJob` (consumed by the pipeline).
- On each `eframe::App::update()`: drain `progress_rx` with `try_iter()`, then
  call `ctx.request_repaint_after(100 ms)` while a job is active.  The worker
  thread also calls `egui::Context::request_repaint()` on terminal events to
  ensure the UI wakes immediately.
- Post-acquire verify pass (when opted in): runs in the same worker thread
  immediately after the pipeline returns, reusing the progress channel.

## Rationale

- Matches ADR 0005's intent: "The GUI (Phase 7) will call it on a dedicated
  background thread and poll the progress channel on the UI thread."
- Unbounded channel avoids back-pressure stalling acquisition (ADR 0005 uses
  `try_send` throughout).
- Immediate-mode egui requires that all UI state live on the App struct — no
  `Rc`/`RefCell` tricks needed since `eframe::App::update` has `&mut self`.
- Sequential job queue preserves audit log ordering and avoids concurrent
  O_DIRECT reads on the same physical device.

## Consequences

- Multiple pending jobs are processed one at a time in submission order.
  Users wanting parallel imaging on different physical devices must launch
  separate iridium instances (acceptable for the current airgap use case).
- If the verify pass needs more progress granularity in future it can be
  promoted to its own `ProgressEvent` variants without changing this
  architecture.
