# v0.12 Review Round 1: Partial Production Depth

Reviewer verdict: `VERDICT: PARTIAL`

This file records the first reviewer gate failure from the required two-agent loop.

## Blockers To Address

1. Finish `#[kobo::service(buffer=N)]` production lowering: typed message enum, bounded channel, block-on-full send path, shutdown, cancellation token evidence, source-mapped inspect output, scenario hooks, and no backend scheduler imports in user output.
2. Complete the service contract target with every test required by `contracts/service_runtime.md`, not only typed messages and channel capacity.
3. Add the remaining release gate test targets required by the roadmap: handler lifecycle, parallel lowering, replay boundaries, spawn-local zones, standalone Rust debt, runtime profile, watch groundwork, and integrated service simulation.
4. Implement full handler lifecycle behavior: reply/reject/cancel obligations, cleanup, tracing/metrics boundaries, request-state leak diagnostics, disconnect scenarios, and must-call handler token metadata.
5. Implement safe Rayon generation for `#[kobo::parallel]` only when Send/Sync and mutation facts prove the loop is safe.
6. Implement first-class `record` and `activity` constructs with witness metadata, digest changes, replay reuse, retry/idempotency metadata, and exact-replay downgrades for external internals.
7. Finish explicit `spawn local {}` task-local zones through `LocalSet` / `spawn_local` without weakening Send diagnostics.
8. Add standalone Rust-only `kobo debt --cargo .` scanning with liveness and nondeterminism boundary candidates in human and JSON output.
9. Replace older watch-loop behavior with bounded watch planning that names changed files and rerun targets and rejects unscoped workspace watch by default.

## Required Loop Action

Implement these blockers, rerun verification, then summon two reviewers again. Do not claim v0.12 full production depth until both reviewers return a full-depth verdict.
