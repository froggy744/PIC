# Progressive Network Import Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Show network photos during discovery and continue unfinished, explicitly authorized imports after reopening PIC.

**Architecture:** A bounded discovery worker streams directory entries to one catalog writer. A single bulk enrichment worker consumes durable pending rows and returns results to the writer; existing foreground thumbnail requests keep priority. SQLite stores job state, directory checkpoints, seen paths and metadata readiness. Local and SD imports retain their current path.

**Tech Stack:** Rust, rusqlite/SQLite WAL, std threads/channels, GTK/GIO, libnfs C FFI.

**Spec:** `docs/development/2026-10-06-progressive-network-import-checklist.md` (approved for implementation in the conversation).

## Global Constraints

- Originals stay on network shares; no privileged helper or CPU affinity.
- One import writer; no source I/O in write transactions.
- Discovery queue holds at most 1,024 records; commit at 128 records or 200 ms.
- Existing photo IDs, edits, albums and metadata survive catalog discovery.
- Only unfinished explicitly authorized jobs continue at startup; explicit Stop persists.
- Recovery and partial discovery must not reconcile deletions.

## Review Focus

- NAS failure during listing preserves all existing catalog records.
- An enrichment result for an old fingerprint cannot overwrite a newer source.
- A full queue and cancellation must not deadlock worker shutdown.
- Restart checkpoints cannot skip records not committed to SQLite.
- UI/onboarding count additions once while metadata and thumbnails advance independently.

### Task 1: Durable catalog and recovery state

**Files:** `src/db.rs`, `src/db/network_import.rs` (new).
**Interfaces:** Job registration/resumable-root queries; lightweight catalog insert/update; version-checked enrichment; directory/seen SQL tables.

- [x] Write failing DB tests for placeholder preservation, fingerprint invalidation, stale results, job recovery/Stop and schema migration.
- [x] Run tests, implement schema and APIs, then rerun DB tests.

### Task 2: NFS streaming discovery

**Files:** `native/private_nfs.c`, `src/private_nfs.rs`, `src/network_shares.rs`.
**Interfaces:** `visit_scan(uri, visitor)` emits entries with optional metadata and a cancellation-aware callback; worker-owned reusable NFS listing session is closed on worker exit.

- [x] Test callback URI/attribute conversion and interruption.
- [x] Add streaming FFI carrying size/mtime, session reuse and finite NFS timeouts.
- [x] Keep existing picker listing behavior and SMB fallback compatible; run network URI tests and build checks.

### Task 3: Progressive pipeline

**Files:** `src/scanner.rs`, `src/scanner/progressive.rs`, `src/scanner/progressive/tests.rs` (new).
**Interfaces:** `progressive::scan(root, database, events, control, resume)`; injectable source boundary for deterministic integration tests; independent `NetworkProgress` event.

- [x] Write failing pipeline tests with a blocked later directory and real SQLite: first photo visible before traversal completion, timed partial flush, recovery, disconnection, cancellation and stale enrichment.
- [x] Implement streaming discovery, bounded queues, durable directory acknowledgements and catalog writes.
- [x] Add bounded background enrichment and safe complete-scan reconciliation.
- [x] Verify tests and existing scanner regressions.

### Task 4: UI, queued imports and restart

**Files:** `src/window.rs`, `src/window/build.rs`, `src/window/onboarding.rs`, `src/main.rs`.
**Interfaces:** Resume roots carried through the existing scan job; combined progress counters; metadata updates reuse existing photo update handling without counting additions twice.

- [x] Test resumed roots are authorized only by stored job state; network imports queue instead of superseding the active import; Stop pauses queued jobs.
- [x] Wire startup continuation after the window is ready, independent counters, serialized root scheduling and explicit resume through folder import.
- [x] Run onboarding and scan-authorization tests; inspect gallery update and selection behavior.

### Task 5: Verification and review

- [x] Run the full test suite and build/check commands, then inspect the complete diff.
- [x] Request a whole-change review; fix material findings and rerun affected tests.
- [x] Update the checklist with actual results and remaining live-NAS validation limits.
- [x] Leave the completed implementation on the feature branch for user review.


Validation: full suite 562 passed, 0 failed, 106 ignored; see the linked
checklist for coverage and live NAS/display validation still outstanding.
The implementation remains on the feature branch, without merging or pushing.
