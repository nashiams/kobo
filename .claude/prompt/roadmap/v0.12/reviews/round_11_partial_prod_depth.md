# Round 11 partial production-depth review

Reviewer: Kierkegaard

Verdict: `VERDICT: PARTIAL`

Fresh review source: `.claude/prompt/agent/lang_improvements/added_2/v0.12.md`

Blocking findings to address:

1. `#[kobo::parallel]` can emit Rayon without proven `Send`/`Sync` facts. `crates/compiler/kobo-analysis/src/parallel.rs:232-307` only recognizes obvious `Rc`/`RefCell`/`Cell` patterns and a narrow user-struct case; `crates/compiler/kobo-codegen/src/lower/rewrite.rs:821-900` treats no blockers as `send-sync`; `crates/compiler/kobo-codegen/src/lower/parallel.rs:13-38` and `139-156` then lower to `.par_iter()`/`for_each` without adding or checking generic bounds. Missing behavior: conservative rejection or real type/trait fact validation for generic, external, and unknown element/capture types before Rayon is emitted.

2. `activity` does not record concrete per-run external result metadata. `crates/compiler/kobo-sim-core/src/core.rs:887-903` excludes `Activity` from replay-owned boundaries and marks it explicit partial; `crates/compiler/kobo-sim-core/src/harness.rs:51-55` returns before generated harness execution when opaque boundaries exist; `crates/compiler/kobo-sim-core/src/harness.rs:605-680` and `1931-1938` only generate facades for replay-owned policies; `bin/kobo-cli/src/commands/test_cmd.rs:1563-1588` emits declaration metadata only. Missing behavior: actual activity result/status capture tied to the side-effect execution, with retry/idempotency metadata, while keeping the side effect outside deterministic replay.

3. `spawn local {}` is not production-depth in deterministic simulation. The parser/codegen path rewrites textual local-spawn markers into `tokio::task::spawn_local` (`crates/compiler/kobo-parser/src/preprocess/spawn.rs:226-385`, `crates/compiler/kobo-codegen/src/lower/rewrite/spawn.rs:49-69`) and wraps functions in a `LocalSet` (`crates/compiler/kobo-codegen/src/lower/rewrite.rs:1173-1190`), but the harness implementation discards the future and returns a ready handle (`crates/compiler/kobo-sim-core/src/harness.rs:1515-1525`). Missing behavior: the local future body must be scheduled/polled in the simulation harness so task-local execution and cancellation evidence are real.

4. Service scenario hooks are side-channel evidence, not scheduler-owned deterministic hooks. Generated service support records hooks through a global `Mutex<Vec<_>>` and stdout lines (`crates/compiler/kobo-codegen/src/lower/service.rs:242-323`); the harness parses those lines (`crates/compiler/kobo-sim-core/src/harness.rs:1986-2005`), but filters `service-scheduler-hook` out of comparable trace agreement (`crates/compiler/kobo-sim-core/src/harness.rs:132-138`). Missing behavior: hooks that actually drive/participate in deterministic service interleavings, shutdown, and cancellation semantics.

5. `K0067` handler request-state leak detection is too shallow for production depth. `crates/compiler/kobo-analysis/src/handler_leak.rs:161-183` only checks spawn calls and narrow sink calls; sink detection is substring/name-list based (`crates/compiler/kobo-analysis/src/handler_leak.rs:248-258`); there is no assignment, return, field/global storage, or type/lifetime escape analysis, and warnings point at the first binding occurrence rather than the escape site (`crates/compiler/kobo-analysis/src/handler_leak.rs:117-125`). Missing behavior: real request-state escape analysis for longer-lived sinks with source-mapped escape spans.

6. Handler lifecycle evidence still overclaims terminal/cancel support from scaffolding. `crates/compiler/kobo-codegen/src/lower/handler.rs:88-123` adds `cancel` evidence to every handler if shared support contains cancel cleanup; `crates/compiler/kobo-codegen/src/lower/handler.rs:323-377` records reply/reject from return shape rather than proving each request lifecycle path; `bin/kobo-cli/src/commands/test_cmd.rs:819-852` serializes that evidence directly. Missing behavior: per-handler/per-scenario proof that reply, reject, cancel, cleanup, tracing, and metrics actually occur on the relevant lifecycle paths.

Kierkegaard noted these non-blocking areas looked materially covered: service runtime is beyond MVP with typed message enums, bounded Tokio channels, public handle/error types, mutable service dispatch, shutdown APIs, source-map runtime evidence, and service fixtures; `record` boundaries have first-class policy plumbing and replay-owned harness support; `kobo debt --cargo` exists as an advisory Rust-only scanner for liveness/nondeterminism candidates.

Kierkegaard verification:

- Read HEAD `92e6a4d3e0fc9a07bb309eb2f335571b8ba633ad` against `.claude/prompt/agent/lang_improvements/added_2/v0.12.md`.
- Inspected service, handler, parallel, activity, spawn-local, and debt implementation and tests.
- Did not run cargo tests in this review pass; verdict is from source and fixture inspection.

Reviewer: Singer

Verdict: `VERDICT: PARTIAL`

Fresh review source: `.claude/prompt/agent/lang_improvements/added_2/v0.12.md`

Blocking findings to address:

1. Service lowering is not production-complete for normal Rust signatures. `crates/compiler/kobo-codegen/src/lower/service.rs:79-103`, `129-163`, `349-361`, `403-506`, and `558-583` derive the service name and methods without preserving impl generics, method generics, lifetimes, where clauses, or validating receivers. A generic service or method can generate undeclared type params in the message enum; borrowed params can be moved through mpsc without a supported lifetime model; by-value receivers can move the service in the dispatch loop. Expected behavior: either generate correct typed service Rust or emit a source-mapped diagnostic for unsupported service signatures.

2. `try-send` backpressure reports full channels as closed channels. `service.rs:403-430` switches to `try_send`, but `service.rs:522` has only `SendClosed`, and `service.rs:558-583` maps every send error to `SendClosed`. `TrySendError::Full` is a distinct configured backpressure outcome, not service shutdown. This violates the proposal's bounded channel/backpressure contract.

3. Handler lifecycle evidence overclaims terminal actions. `crates/compiler/kobo-codegen/src/lower/handler.rs:103-120` builds evidence by scanning generated guard calls; `193-195` records terminal actions by guard method name; `323-357` maps `Ok` to reply and `Err` to reject regardless of actual request-token behavior; cancel is pushed when support exists. A handler can perform a reject/cancel-shaped action and return `Ok`, while lifecycle evidence still says reply. Expected behavior: terminal obligations must be tied to actual `reply | reject | cancel` paths.

4. Handler error logging is not implemented at the handler boundary. The Err branch in `handler.rs:323-357` records reject/metrics and returns the error, but there is no structured log/tracing of the handler error. The only `eprintln!` paths are cleanup-runtime failures in `handler.rs:409-434`. The proposal explicitly requires handler "error logging".

5. Parallel Send/Sync enforcement is too shallow for production Rayon generation. `crates/compiler/kobo-codegen/src/lower/rewrite.rs:820-845` only checks captured bindings from loop body tokens; `crates/compiler/kobo-analysis/src/parallel.rs:90-138` tracks obvious local bindings, not function parameters or iterator item types; `239-276` detects a small set of obvious `Rc` shapes. A loop over `values: Vec<Rc<_>>` can be lowered by `crates/compiler/kobo-codegen/src/lower/parallel.rs:139-154` to Rayon and then fail in Rust instead of producing a Kobo diagnostic. This misses "generate Rayon only when Send/Sync facts are satisfied".

6. Explicit parallel boundary policy disables Rayon lowering instead of proving controlled parallelism. `crates/compiler/kobo-codegen/src/lower/parallel.rs:23-28` returns `BoundaryPolicy` for any `policy` attr before the safety gate can emit Rayon; `rewrite.rs:584-593` only inserts a marker/evidence. A safe CPU-bound loop with explicit inside/outside policy remains serial, which does not satisfy the combined "safe loop emits Rayon" and "ward boundary requires explicit policy" requirements.

7. Parallel shared-mutation detection misses common mutation forms. `crates/compiler/kobo-analysis/src/parallel.rs:146-164` and `crates/compiler/kobo-codegen/src/lower/rewrite.rs:1133-1138` handle direct assignment, while mutation methods are limited at `parallel.rs:191-199` and `rewrite.rs:1157-1169`; the text fallback only checks a few methods at `parallel.rs:432-443`. Assignment operators such as `total += x` and many mutation APIs are not covered, so unsafe loops can reach Rayon or rustc errors instead of Kobo diagnostics.

8. `spawn local {}` safety is still source-text shaped, not semantic task-local ownership. `crates/compiler/kobo-analysis/src/task_local.rs:26-67`, `69-105`, `107-189`, and `191-270` scan raw source for `Rc`, `RefCell`, `spawn local`, and a few escape patterns. This misses aliases, generic non-Send types, function parameters, and handle escapes through structs/tuples/closures. The lowering does create `LocalSet`/`spawn_local` in `rewrite.rs:623-647` and `1176-1190`, but the safety gate is not production-depth.

9. Standalone `kobo debt --cargo` is still a narrow advisory text scan. `bin/kobo-cli/src/commands/debt.rs:487-539` detects only textual `std::thread::spawn` / `tokio::spawn`; `541-590` checks hard-coded nondeterminism strings; `618-621` treats `.join(`, `.await`, or `.abort(` as observed. It misses aliased imports, helper-joined handles, stored handles, and indirect dependency calls. This satisfies some fixture cases but is not a production-depth Rust-only debt scanner.

Singer verification:

- Read the v0.12 proposal, focused service/handler/parallel/replay/spawn/debt implementation paths, and the v0.12 tests.
- `git status --short` was clean.
- Ran the focused v0.12 cargo test command covering service runtime, handler lifecycle, parallel, replay boundaries, spawn local, standalone debt, runtime profile, watch groundwork, and integrated service simulation; it passed after network escalation for the Rayon dependency.
