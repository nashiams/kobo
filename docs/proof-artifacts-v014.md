# Proof Artifacts And Admission Gates

v0.14 emits independently checked proof artifacts for modeled obligation flow. The proof surface is Kobo Core evidence: source hash, Core hash, CFG nodes and edges, obligation events, environments, template versions, boundary assumptions, async cancellation evidence, adapter confidence, replay grade, and candidate-track admission metadata.

The verifier in `kobo-proof` checks the artifact from certificate data. It rejects stale hashes, unknown unversioned fields, unknown event or boundary values, missing async cancel evidence, unresolved obligations, and replay claims that cross disallowed boundary or adapter evidence.

## Core Claim

The proof claim ends at Kobo Core. v0.14 does not prove generated Rust binary behavior, translation validation, arbitrary Tokio behavior, or correctness of arbitrary external crate effects.

## Explicit Non-Goals

These are the explicit non-goals for this proof surface.

- No arbitrary external ecosystem proof.
- No broad adapter maintenance promise.
- No whole-app Strict migration promise.
- No proof of hidden production runtime behavior.
- No graduation of research tracks without admission evidence.

## Adapter Honesty

Every adapter evidence entry records adapter confidence:

- `exact`: modeled and witnessed inside the declared scope.
- `modeled`: deterministic model evidence with assumptions in the ledger.
- `sampled`: probing evidence only.
- `metadata-only`: declarations, diagnostics, or summaries without replayable behavior.

The replay grade records whether the artifact is `exact`, `partial`, `not_replayable`, or `debt`. Unsupported, stale, sampled, metadata-only, opaque, outside, and unresolved-debt evidence cannot inflate the replay claim.

## Candidate-Track Admission

candidate-track admission is required before research work graduates. Each candidate must record inspect visibility, a manual Rust equivalent, Strict compatibility, diagnostic snapshots, and whether it needs whole-ecosystem modeling. Replay-related candidates also record replay grade and adapter confidence.

research tracks remain spikes until their admission evidence passes. This includes MSRV-aware emission, no_std and allocation-budget work, numeric cast refinement, ergonomic sugar, SMT and temporal research, broad adapter research, witness minimization, and optional model-checking backends.
