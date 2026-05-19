# v0.12 Review Round 9: Partial Production Depth

Date: 2026-05-19

One review agent returned `partial implemented or not full prod depth yet for v0.12` after commit `1eb0b25`. The round 8 blockers are fixed, but the production-depth anti-gaming gate still needs an activity policy mutation.

## Blocker

- The activity fixtures prove retry/idempotency metadata exists and that activity internals are not replayed, but they do not mutate the activity retry/idempotency policy and compare the resulting witness evidence.

## Required Next Pass

- Add a mutated activity declaration variant.
- Assert integrated activity witness metadata changes for retry and idempotency while preserving the activity boundary replay downgrade.
