# Round 10 partial production-depth review

Reviewer: Cicero

Verdict: `VERDICT: PARTIAL`

Fresh review source: `.claude/prompt/agent/lang_improvements/added_2/v0.12.md`

Blocking findings to address:

1. Service generation does not support normal mutable service state. `crates/compiler/kobo-codegen/src/lower/service.rs:140-163` accepts every async service method, but the worker is generated as `async fn serve(service: #service_ident, ...)` at `service.rs:464-468` and dispatch calls `service.#method_ident(...).await` at `service.rs:543-548`. Any realistic `async fn foo(&mut self, ...)` service method will generate Rust that cannot borrow `service` mutably. The tests only cover `&self` service methods.

2. Graceful shutdown and cancellation are ordered incorrectly in generated service Rust. `shutdown()` cancels first, then sends `Shutdown` at `service.rs:488-493`; the worker checks `cancellation.is_cancelled()` before matching the message at `service.rs:469-478`. That can skip the generated `Shutdown` arm and its shutdown hook, so the source contract's graceful shutdown message and cancellation integration are only partially honored.

3. Service scenario hooks are side-channel stdout/global state, not deterministic scheduler hooks. `KoboServiceScenarioHook` stores events in a global `OnceLock<Mutex<Vec<_>>>` and prints `KOBO_SERVICE_HOOK` lines at `service.rs:255-317`; the harness parses stdout at `crates/compiler/kobo-sim-core/src/harness.rs:1963-1971`. That is useful evidence plumbing, but it is not the deterministic scheduling hook promised by source lines 47-55.

4. Service inspect/source-map support is mostly doc comments, not full generated-source map coverage. Service items get `#[doc = "kobo: service ... source_line=..."]` at `service.rs:343-389`, while `crates/compiler/kobo-codegen/src/lower/mod.rs:67-72` appends service/handler support after the lowering anchor map is built from plan support counts. Current tests check visible strings, not real `.kobo.map` coverage for generated service support.

5. Generated service API is private and same-module shaped. The message enum, handle struct, and error enum are generated without `pub` at `service.rs:350-357`, `service.rs:386-392`, and `service.rs:517-523`. That can work for single-file fixtures, but it is not a production service API shape for backend modules.

6. Handler leak detection is text-scanned and too narrow for the K0067 contract. `crates/compiler/kobo-analysis/src/handler_leak.rs:8-47` searches raw source for `#[kobo::handler]`, `async fn`, parameter names, and direct spawn captures; `handler_leak.rs:69-83` only checks `spawn {`, `tokio::spawn`, and `spawn_local`. It misses many longer-lived sinks, aliases, helper calls, struct storage, and nontrivial request-state leaks.

7. Handler cancel evidence is synthesized from support availability, not actual handler behavior. `crates/compiler/kobo-codegen/src/lower/handler.rs:88-123` adds `"cancel"` to every lowered handler if the support guard exists. The guard runs cancel cleanup on `Drop` at `handler.rs:598-618`, but tests can pass because metadata says cancel exists, not because disconnect/cancel behavior is robustly exercised through production handler execution.

8. Handler cleanup runtime is not production-grade lifecycle infrastructure. `handler.rs:409-445` spawns cleanup detached if a Tokio handle exists, otherwise uses a raw no-op waker loop with `thread::yield_now()`. There is no join, error propagation, timeout, or real logging boundary, while the source contract calls for cleanup hooks, error logging, metrics/tracing boundaries, and cancel support.

9. `spawn local {}` is implemented as source preprocessing plus simple scans, not a robust typed task-local feature. The parser rewrites raw text in `crates/compiler/kobo-parser/src/preprocess/spawn.rs:232-304` and recognizes `spawn local` with byte scanning at `spawn.rs:388-421`. Codegen lowers it to `tokio::task::spawn_local` at `crates/compiler/kobo-codegen/src/lower/rewrite/spawn.rs:49-64` and wraps functions in `LocalSet` at `rewrite.rs:1173-1190`. Send checks depend on simple `Rc`/`RefCell` text patterns in `crates/compiler/kobo-analysis/src/task_local.rs:26-67` and `task_local.rs:107-136`.

10. `#[kobo::parallel]` Send/Sync enforcement is shallow. `crates/compiler/kobo-analysis/src/parallel.rs:35-69` uses a local AST scanner, but the actual safety facts are limited to obvious `Rc` tokens and simple mutation patterns at `parallel.rs:160-189` and `parallel.rs:225-248`. This does not satisfy the source contract's "Rayon only when Send/Sync facts are satisfied" bar for generic/user-defined types and real ownership tiers.

11. Standalone `kobo debt --cargo` exists, but it is explicitly advisory and pattern-based. The report marks `precision: "advisory"` and `blocking: false` at `bin/kobo-cli/src/commands/debt.rs:108-116`. Ownership, liveness, and nondeterminism checks are mostly line substring scans at `debt.rs:462-576`, with handle observation based on string containment at `debt.rs:603-607`. This meets the minimum CLI presence, not production-depth Rust project analysis.

12. Runtime profile settings are visible but not consistently behavioral. `RuntimeProfileOptions` carries scheduler/record/activity/cancellation fields at `crates/compiler/kobo-codegen/src/lib.rs:101-137`, and witnesses serialize them at `bin/kobo-cli/src/commands/test_cmd.rs:732-760`, but scheduler behavior still maps from sim profile at `test_cmd.rs:1052-1058` and backend selection ignores `runtime_profile.scheduler` at `test_cmd.rs:2067-2075`. This is config visibility, not full runtime integration.

13. `record` and `activity` are boundary-policy plumbing rather than first-class constructs. The source asks for first-class `record`/`activity` replay boundary constructs; the implementation exposes `ScenarioBoundaryPolicy::Record` and `Activity` at `crates/compiler/kobo-ir/src/scenario.rs:91-130` and emits policy events in `bin/kobo-cli/src/commands/sim_model.rs:1458-1465`. The replay behavior has good tests, but the language surface is still annotation/declaration based.

14. The v0.12 tests are broad but often fixture-shaped. The focused suite checks many strings and metadata paths; only the parallel fixture clearly cargo-checks generated Rust. Service tests assert inspect strings and witness evidence, but they do not compile realistic mutable service output or prove actual shutdown hook execution through generated Rust.

Cicero verification:

- Read the full source contract with line numbers.
- Searched implementation and tests for `service`, `handler`, `parallel`, `record`, `activity`, `spawn local`, `LocalSet`, `debt`, `runtime profile`, `CancellationToken`, `scheduler`, and `scenario`.
- Read key files under `crates/compiler/kobo-codegen`, `kobo-analysis`, `kobo-parser`, `kobo-driver`, `kobo-ir`, `kobo-sim-core`, and `bin/kobo-cli/src/commands`.
- `cargo test -q -p kobo-cli --target-dir C:\tmp\kobo-review-a-target --test v12_service_runtime_contract --test v12_handler_lifecycle_contract --test v12_spawn_local_contract --test v12_standalone_debt_contract --test v12_parallel_contract --test v12_replay_boundary_contract --test v12_runtime_profile_contract --test v12_integration_service_sim_contract` passed after escalation; 55 passed, 0 failed.
- `git status --short` was empty.

Reviewer: Schrodinger

Verdict: `VERDICT: FULL PROD DEPTH`

Fresh review source: `.claude/prompt/agent/lang_improvements/added_2/v0.12.md`

Blocking findings to address:

1. None. Schrodinger did not find a partial or test-shaped v0.12 implementation against the source contract.

Schrodinger noted these areas looked materially covered: service, handler, parallel, record/activity, spawn-local, debt, runtime profile behavior, integrated v0.12 service fixture coverage, activity retry/idempotency mutation evidence, activity replay downgrade, and compile-proven generated Rayon output.

Schrodinger verification:

- `cargo test -q -p kobo-cli --test v12_service_runtime_contract --test v12_handler_lifecycle_contract --test v12_parallel_contract --test v12_replay_boundary_contract --test v12_spawn_local_contract --test v12_standalone_debt_contract --test v12_runtime_profile_contract --test v12_watch_groundwork_contract --test v12_integration_service_sim_contract -j 1` passed after escalation; 58 v0.12 tests.
- `cargo test -q -p kobo-cli --test v10_ecosystem_replay_contract --test v11_ecosystem_replay_honesty_contract -j 1` passed; 30 tests.
- `cargo check --workspace -j 1 --target-dir target-review-v012` passed.
- Ran targeted `rg` scans for record/activity/witness/replay, Rayon/Send/Sync, service/handler/spawn-local/debt, and anti-stub/placeholder terms.
- `git status --short` remained clean.
