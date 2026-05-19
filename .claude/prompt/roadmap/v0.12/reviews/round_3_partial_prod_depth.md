# v0.12 Review Round 3: Partial Production Depth

Date: 2026-05-19

Both final review agents returned `partial implemented or not full prod depth yet for v0.12`.

## Shared Blockers

- Service runtime lowering emits typed support items, but does not yet generate a production-shaped receiver loop, dispatch match, method invocation path, per-method client API, owned shutdown future, and scenario hook plumbing.
- Handler lifecycle lowering still contains no-op production boundaries: metrics and span close paths do not record real lifecycle facts, and cancel/drop cleanup flips state without invoking registered cleanup.
- Handler witness evidence is reconstructed from source shape and marks terminal actions too broadly instead of reflecting actual lowered lifecycle obligations.
- Parallel lowering and `spawn local` safety gates still depend on narrow source-string scans rather than compiler-owned proof material that the lowering can cite.
- Witness evidence for service runtime and handler lifecycle is partly rebuilt by `kobo test` source scanning, so it can drift from generated Rust/runtime behavior.
- The integrated service/sim contract is still shallow: it needs two service methods, record/activity behavior, safe and rejected parallel paths, spawn-local non-Send coverage, cancellation or disconnect coverage, and a standalone `kobo debt --cargo .` assertion.

## Required Next Pass

- Move service and handler evidence to compiler-owned lowering metadata, then have test/replay consume that metadata instead of reparsing source text.
- Generate production-shaped service runtime Rust with actual dispatch and method client helpers.
- Make handler lifecycle cleanup/metrics/span obligations observable in generated code and witness evidence.
- Tighten parallel and task-local analysis evidence so accepted lowering cites proof material and rejected cases carry real diagnostics.
- Expand integrated gates to cover the full acceptance fixture rather than string-only visibility.
