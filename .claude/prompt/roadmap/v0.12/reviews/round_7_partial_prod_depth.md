# v0.12 Review Round 7: Partial Production Depth

Date: 2026-05-19

Both review agents returned `partial implemented or not full prod depth yet for v0.12` after commit `d56384c`. This round tracks the remaining production-depth blockers before another full review.

## Blockers

- Parallel lowering still emits Rayon in default inspect when an unsafe `#[kobo::parallel]` loop only produces a K0061 warning. The witness says `accepted-lowering-gate:no-K0061-blockers`, but codegen still rewrites syntax-shaped loops to `par_iter()` without enforcing that safety gate in all profiles.
- Handler lifecycle evidence now scans lowered handler function bodies for reply/reject, but cancel is only present in lowered guard support/drop code. The evidence must include cancel through a lowered support scan rather than overclaiming it as a per-function guard call.

## Required Next Pass

- Block Rayon lowering whenever the parallel safety facts include non-Send captures, shared mutation, or missing ward-boundary policy, even if the active guarantee profile would report only a warning.
- Preserve strict diagnostics for unsafe parallel loops while making default inspect emit serial code with an explicit blocked-parallel marker.
- Derive handler cancel metadata from lowered guard/drop support code and expose an honest evidence source for the combined scan.
