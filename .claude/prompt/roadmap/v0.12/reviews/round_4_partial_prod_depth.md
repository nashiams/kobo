# v0.12 Review Round 4: Partial Production Depth

Date: 2026-05-19

At least one final review agent returned `partial implemented or not full prod depth yet for v0.12` after commit `2481334`.

## Blockers

- Service runtime codegen now emits dispatch and client helpers, but the service scenario hooks are marker-only and current service/integration gates define service methods without driving them through the generated client and receiver path.
- Parallel accepted-loop evidence still carries a constant proof string instead of compiler-owned proof facts that name the iterator, captured bindings, and rejection checks used to justify Rayon lowering.
- The integrated service/sim fixture has `spawn local { ward.task(); }` but does not cover a concrete non-Send local value inside the integrated product path.
- Handler lifecycle generation records cleanup, metrics, and tracing, but terminal action evidence is still token-search based and cancel cleanup has a best-effort no-runtime fallback. The next pass must make the generated lifecycle trace explicit enough for evidence to cite lowered guard calls rather than raw source shape.

## Required Next Pass

- Extend service runtime support with recorded scenario hook events and exercise the generated client API from service and integration scenarios.
- Replace constant parallel/task-local proof strings with structured lowering evidence from AST/codegen facts.
- Add non-Send `Rc` state to the integrated spawn-local path and assert the resulting LocalSet/spawn-local evidence.
- Derive handler terminal evidence from lowered lifecycle guard calls and make cancel cleanup fallback visible as explicit evidence.
