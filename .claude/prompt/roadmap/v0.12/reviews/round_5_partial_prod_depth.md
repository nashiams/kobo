# v0.12 Review Round 5: Partial Production Depth

Date: 2026-05-19

One final review agent returned `partial implemented or not full prod depth yet for v0.12` after commit `b36325c`; the other returned `full prod depth for v0.12`. Because any partial verdict continues the loop, this round tracks the remaining blockers.

## Blockers

- Phase 00 foundation regressed: `v10_ecosystem_replay_contract::tokio_spawn_body_is_replayed_by_semantic_and_harness_paths` fails with K0117 semantic/harness divergence after the harness Tokio facade change.
- Service scenario hook state is generated and service client/dispatch now execute, but witness evidence still serializes static codegen hook metadata instead of runtime hook events.
- Parallel accepted-loop proof is structured, but codegen still lowers after a separate analysis diagnostic pass rather than carrying the precondition as explicit lowering metadata.
- Normal `spawn` with non-Send captures can still be silently lowered to `spawn_local` when the diagnostic scan misses the case.
- Handler lifecycle evidence is codegen-owned, but cancel detection still includes a token-shaped fallback.

## Required Next Pass

- Restore v10/v11 Phase 00 gates while preserving service worker execution.
- Separate user-modeled Tokio spawn events from generated service worker spawning.
- Surface service hook runtime events in witness evidence where the harness executes them.
- Tighten normal spawn lowering so only explicit `spawn local` may choose task-local execution for non-Send captures.
