# v0.12 Review Round 8: Partial Production Depth

Date: 2026-05-19

Both review agents returned `partial implemented or not full prod depth yet for v0.12` after commit `543c7e0`. Round 7 blockers are fixed, but the reviewers identified two remaining production-depth gaps.

## Blockers

- Safe `#[kobo::parallel]` lowering still emits a plain Rust `for` loop over `par_iter()`. The generated shape is inspectable but is not compile-proven production Rayon Rust. The lowering must use Rayon adapter calls such as `for_each` and the test must compile the generated fixture.
- The integrated service/sim release test checks witness presence and partial replay honesty, but it does not itself prove recorded-value digest changes, deterministic replay reuse, and activity side effects staying outside deterministic replay. Those assertions currently live only in the isolated replay contract.

## Required Next Pass

- Lower safe parallel loops to compiling Rayon adapter code and add a generated Cargo compile regression.
- Infer the Rayon dependency for generated Cargo projects that use Rayon lowering.
- Extend the integrated service/sim contract so the combined fixture structurally proves record digest mutation, replay reuse, and activity non-rerun/external-internals exclusion.
