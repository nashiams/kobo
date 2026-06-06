# README Task Checklist - Codex Review Copy

This is a copied status file for the Codex reviewer loop. The original
`README.md` and `readme_task_checklist.md` files were not edited.

## Current Codex Status

- [x] Two independent reviewers audited the current codebase against the v0.16.2 README and checklist.
- [x] Claim-boundary and proof-boundary honesty are present.
- [x] Adapter summaries now require Cargo feature compatibility evidence during watch trace import.
- [x] Watch trace witnesses now preserve machine-readable external implementation comparison evidence.
- [x] Bare `kobo debt --summary`, `kobo debt --json`, and `kobo debt --watch --summary` resolve an obvious project default source.
- [x] Source-watch state can be imported as a replayable witness instead of relying only on hand-authored trace fixtures.
- [x] A Kobo-owned `RawEvent -> filter paths -> debounce/coalesce -> Action -> RestartPolicy -> SupervisorState` dogfood fixture checks and exports as a Cargo project.
- [x] Public README and migration-guide examples use supported `kobo test --sim <profile> <file>` and `kobo inspect --sim --harness <file>` command shapes.
- [x] Full v0.16.2 completion is implemented in the copied checklist through project-support gates, expanded watch trace evidence, a narrow supervisor-slice CI gate, and the reviewer loop below.

## First Reviewer Verdicts

- Reviewer A: partial production depth; missing mixed Rust/Kobo supervisor slice, stronger adapter contracts, process supervision beyond trace validation, and broad full-rewrite gates.
- Reviewer B: partial production depth; missing external comparison artifacts, adapter Cargo feature validation, valid bare debt commands, and broad full-rewrite gates.

## First Implemented Slice

- `bin/kobo-cli/src/commands/watch/trace.rs`: validates `cargo_features`, validates external comparison records, preserves and hashes comparison evidence in witnesses.
- `tests/fixtures/watchexec_supervisor_policy/save_restart_trace.json`: adds selected comparison records for `watchexec`, `nodemon`, `chokidar`, `watchdog`, `watchfiles`, Watchman, and Go air.
- `bin/kobo-cli/src/commands.rs` and `bin/kobo-cli/src/commands/debt.rs`: make bare debt commands resolve a conservative project default source.
- `bin/kobo-cli/tests/watchexec_watch_trace_checks.rs` and `bin/kobo-cli/tests/debt_warning_watch_checks.rs`: add focused red/green coverage for these behaviors.

## Second Implemented Slice

- `bin/kobo-cli/src/commands/watch/trace.rs`: now requires path-filter and async-runtime adapter summaries, preserves path roles, filter decisions, platform facts, changed-path delivery, stdio/terminal/process-group facts, timeout/cancellation lifecycle events, and async scheduler evidence in replayable witnesses.
- `bin/kobo-cli/src/commands/watch/trace.rs`: now requires a full v0.16.2 external-comparison coverage matrix for `watchexec`, `nodemon`, `chokidar`, `watchdog`, `watchfiles`, Watchman, and Go air instead of accepting a token comparison record.
- `tests/fixtures/watchexec_supervisor_policy/save_restart_trace.json`: now carries five adapter summaries and the full external comparison matrix while preserving the original save/restart event stream.
- `bin/kobo-cli/src/commands/doctor/project_support.rs`: adds `kobo doctor --project-support` and `--require-ready` as a broad project-support evidence gate for full-project language coverage, platform semantics, critical adapters, async runtime semantics, generated backend parity, test/release parity, proof/debt maps, and independent equivalence review evidence.
- `bin/kobo-cli/src/commands/doctor/supervisor_slice.rs`, `bin/kobo-cli/src/main.rs`, and `bin/kobo-cli/src/commands.rs`: add `kobo doctor --supervisor-slice --require-ready` so CI can gate only the Kobo-owned watcher/supervisor slice first.
- `bin/kobo-cli/tests/watchexec_watch_trace_checks.rs` and `bin/kobo-cli/tests/project_support_checks.rs`: add focused coverage for rich boundary traces, complete project-support manifests, blocked manifests, and the narrow supervisor-slice CI gate.

## Verification So Far

- [x] `cargo test -p kobo-cli --test watchexec_watch_trace_checks -q`
- [x] `cargo test -p kobo-cli --test debt_warning_watch_checks -q`
- [x] `cargo test -p kobo-cli --test cli_editor_docs_checks public_docs_use_supported_sim_command_shapes -q`
- [x] `cargo check --workspace`
- [x] `cargo run -p kobo-cli --bin kobo -- watch --import-trace tests\fixtures\watchexec_supervisor_policy\save_restart_trace.json --witness-out target\v0162-review\watchexec-policy.kwit`
- [x] `cargo run -p kobo-cli --bin kobo -- replay target\v0162-review\watchexec-policy.kwit`
- [x] `cargo run -p kobo-cli --bin kobo -- check tests\fixtures\watchexec_supervisor_policy\supervisor_policy.kobo`
- [x] `cargo run -p kobo-cli --bin kobo -- inspect --clean --cargo target\v0162-review\policy-clean tests\fixtures\watchexec_supervisor_policy\supervisor_policy.kobo`
- [x] `cargo test -p kobo-cli --test watchexec_watch_trace_checks -q` now includes orphan-child, double-running exclusive child, and kill-resolution lifecycle checks.
- [x] `cargo test -p kobo-cli --test watchexec_watch_trace_checks -q` now includes shutdown resolving a pending debounce window and active child.
- [x] `cargo test -p kobo-cli --test debt_warning_watch_checks -q` now includes release `check`/`build` blocking unresolved watch lifecycle evidence and module ownership reporting.
- [x] `cargo test -p kobo-cli --test debt_warning_watch_checks -q` now includes release `check`/`build` blocking debounce evidence with `replay_grade: debt`.
- [x] `cargo test -p kobo-cli --test project_support_checks -q`
- [x] `cargo test -p kobo-cli --test watchexec_watch_trace_checks -q` now includes full external comparison coverage, path-filter summaries, async-runtime summaries, renamed temp-file save paths, config-driven path relevance, changed-path delivery, stdio/terminal facts, replacement cleanup, timeout, cancellation, and scheduler evidence.
- [x] `cargo test -p kobo-cli --test debt_warning_watch_checks -q`
- [x] `cargo check --workspace`
- [x] `cargo run -p kobo-cli --bin kobo -- watch --import-trace tests\fixtures\watchexec_supervisor_policy\save_restart_trace.json --witness-out target\v0162-review\watchexec-policy.kwit`
- [x] `cargo run -p kobo-cli --bin kobo -- replay target\v0162-review\watchexec-policy.kwit`
- [x] `cargo run -p kobo-cli --bin kobo -- doctor --supervisor-slice --require-ready --json`

## Reviewer Blocker Resolution

- [x] Project-support evidence is bound to non-empty command transcript artifacts and stable hashes instead of trusting empty JSON or bare `status: passed` fields.
- [x] Upstream inventory is cross-checked with `cargo metadata --no-deps` so workspace targets and Cargo feature combinations must match the declared inventory.
- [x] Cargo-backed adapter summaries are checked against declared project dependencies and enabled features, with stale-check evidence bound to transcript artifacts.
- [x] External implementation comparisons require modeled facts, parity fixture identities, and mutation checks, not only implementation/behavior strings.
- [x] The supervisor-slice CI fixture covers burst debounce, multi-window restart, child-exit race, shutdown cleanup, signal/process-group handling, and env/stdin changed-path delivery.
- [x] `ec94a02` makes project-support command evidence rerun from argv, compare stdout/stderr/exit code against transcript artifacts, and reject small invented upstream inventories with minimum full-project surface counts.
- [x] `57a5649` makes external implementation comparisons artifact-backed: parity fixtures and mutation checks now carry typed `artifact_json` plus stable hashes, and forged hashes fail import.
- [x] `4923baf` expands future-section gates for shell parsing, terminal helpers, install behavior, watch-tree and event-burst scaling, ignore-rule mutation tests, async scheduling mutation tests, generated-backend mutation tests, and proof/debt report agreement across debt summary, proof report, replay report, and inspect output.

# README Task Checklist

Source: `.claude/prompt/agent/v0.16.2/README.md`
Read coverage: lines 1-639.

This file is a README-derived task inventory plus one explicitly marked external implementation comparison section requested after the first draft. All items are unchecked by default and must be marked complete only after fresh implementation and verification evidence.

## Claim Boundaries

- [x] Treat Kobo as ready only for a scoped `watchexec` supervisor-policy dogfood slice. (lines 9-16)
- [x] Do not claim readiness for a confident full `watchexec` rewrite. (lines 13-16)
- [x] Do not claim readiness for a clean full-replacement-language story. (lines 13-16)
- [x] Keep the immediate implementation target to `filesystem events -> normalized event batch -> restart decision -> child lifecycle`. (lines 18-23)
- [x] Keep the full cross-platform watcher, CLI, process/signal platform layer, async runtime, config, and package-manager behavior outside the immediate target. (lines 25-30)
- [x] Preserve Kobo's strengths around explicit histories, obligations, replayable scenarios, and declared boundaries. (lines 31-33)

## Confident Full Watchexec Rewrite Gate

- [x] Full rewrite claim gate: keep the claim blocked until every task in this section, every detailed future section below, and the external implementation comparison section are implemented and freshly verified. (lines 13-16, 403-606)
- [ ] Whole-project language coverage: inventory the complete upstream `watchexec` workspace, crate tree, feature flags, examples, tests, build scripts, and release artifacts before choosing what Kobo must own. (lines 414-432)
- [ ] Whole-project language coverage: map every upstream crate, module, public type, CLI surface, test fixture, and platform path to Kobo-owned source, formal adapter, generated backend output, or explicit foreign boundary. (lines 414-432)
- [ ] Whole-project language coverage: parse every Rust-shaped construct needed by upstream `watchexec` without deleting platform branches, flattening APIs, or replacing unsupported source with simpler Kobo-only shapes. (lines 414-432)
- [ ] Whole-project language coverage: check every represented module under the same Cargo feature combinations the upstream project supports. (lines 414-432)
- [ ] Whole-project language coverage: lower every represented module while preserving source maps for diagnostics, replay, debt, proof, and generated Rust inspection. (lines 414-432)
- [ ] Whole-project language coverage: preserve rare diagnostics and edge-case source locations instead of treating them as acceptable generated-Rust failures. (lines 421-424)
- [ ] Whole-project language coverage: prove that tests, examples, multi-crate Cargo layouts, and feature flags work as normal project inputs. (lines 423-432)
- [ ] Whole-project language coverage: block the confident rewrite claim if any correctness-relevant platform path is deleted, flattened, or replaced by an easier API. (lines 428-432)
- [ ] Platform semantics: model filesystem events on Windows, macOS, and Linux, including create, modify, delete, move, rename, close, metadata, rescan, synthetic, duplicate, and coalesced events. (lines 435-457)
- [ ] Platform semantics: model watcher backend identity and guarantee differences for Windows APIs, macOS FSEvents or kqueue-style behavior, Linux inotify-style behavior, and polling fallback. (lines 435-457)
- [ ] Platform semantics: model case sensitivity, case-only rename behavior, canonical path recovery, symlink behavior, project-root selection, recursive watch scope, and network or unsupported filesystem fallback. (lines 435-457)
- [ ] Platform semantics: model process spawn, command arguments, environment, current directory, shell wrapping, no-shell execution, process groups, sessions, job objects, child trees, and detached processes. (lines 435-457)
- [ ] Platform semantics: model stop signals, interrupt signals, kill fallback, graceful shutdown, kill timeout, final shutdown, and platform-specific unsupported signal behavior. (lines 435-457)
- [ ] Platform semantics: model terminal IO, stdin, stdout, stderr, TTY behavior, inherited handles, log forwarding, and command-output routing. (lines 435-457)
- [ ] Platform semantics: model timers, debounce windows, delay-run behavior, stop timeouts, poll intervals, cancellation, and timeout firing across platforms. (lines 435-457)
- [ ] Platform semantics: run behavior tests on Windows, macOS, and Linux for watcher events, restart behavior, signal behavior, stdin behavior, and path filtering. (lines 450-457)
- [ ] Platform semantics: assign exact, modeled, sampled, metadata-only, or opaque replay grades to every platform behavior and block critical opaque behavior. (lines 444-457)
- [ ] Critical adapter coverage: decide for each correctness-critical crate whether Kobo replaces it, formally models it, or keeps it behind an explicit foreign boundary. (lines 460-487)
- [ ] Critical adapter coverage: cover watcher backend crates and event types with versioned summaries, emitted facts, unsupported guarantees, conformance tests, and replay confidence. (lines 460-487)
- [ ] Critical adapter coverage: cover process and signal crates with versioned summaries, lifecycle facts, timeout facts, process-tree facts, platform differences, and stale-summary detection. (lines 460-487)
- [ ] Critical adapter coverage: cover ignore, path, glob, project-origin, and config crates with summaries for matching semantics, root discovery, case behavior, and dynamic config reload behavior. (lines 460-487)
- [ ] Critical adapter coverage: cover CLI parsing, shell parsing, serialization, logging, tracing, error formatting, and terminal helpers with source-mapped facts and compatibility checks. (lines 473-487)
- [ ] Critical adapter coverage: include Cargo feature-flag compatibility in every summary and fail release gates when a supported feature combination lacks evidence. (lines 460-487)
- [ ] Critical adapter coverage: detect stale summaries when crate versions, enabled features, or public behavior change. (lines 467-487)
- [ ] Critical adapter coverage: block any correctness-critical path that still depends on unversioned metadata-only or opaque summaries. (lines 483-487)
- [ ] Async runtime semantics: define task spawn, task join, task cancellation, select-like races, timer futures, channels, backpressure, shutdown, blocking work, and wake ordering. (lines 490-508)
- [ ] Async runtime semantics: choose either a native Kobo runtime model or a versioned Tokio adapter backed by conformance evidence. (lines 497-499)
- [ ] Async runtime semantics: record scheduler facts that affect watcher batching, restart ordering, signal delivery, and child-exit races. (lines 500-508)
- [ ] Async runtime semantics: make replay fail when mutating task order, timer order, cancel order, or channel delivery changes observable behavior. (lines 503-508)
- [ ] Async runtime semantics: block false confidence whenever scheduler facts are outside the model. (lines 503-508)
- [ ] Generated Rust backend parity: keep generated Rust deterministic, source-mapped, reviewable, and compilable on the same supported targets as upstream `watchexec`. (lines 511-522)
- [ ] Generated Rust backend parity: make ordinary diagnostics, replay reports, proof reports, LSP output, and debt reports point to Kobo source and project concepts, not generated Rust internals. (lines 516-522)
- [ ] Generated Rust backend parity: prove a maintainer can debug watcher and restart behavior from Kobo source, witnesses, and source-mapped diagnostics without editing generated Rust. (lines 524-529)
- [ ] Generated Rust backend parity: verify Cargo output paths, package layouts, feature-gated builds, and target-specific generated Rust remain stable across clean builds. (lines 511-529)
- [ ] Full test parity: port upstream unit tests, integration tests, CLI tests, fixture tests, platform tests, and examples into the Kobo-owned project shape. (lines 532-552)
- [ ] Full test parity: add Kobo replay tests for watcher histories, lifecycle histories, signal histories, async histories, config histories, and platform histories. (lines 536-552)
- [ ] Full test parity: add liveness tests for no orphan child, no double-running exclusive child, shutdown completion, debounce completion, and no stuck cancellation. (lines 536-552)
- [ ] Full test parity: preserve CLI behavior, config behavior, exit codes, logging, event output, package behavior, platform behavior, install behavior, and release artifacts. (lines 538-552)
- [ ] Full test parity: measure startup, steady-state, restart latency, memory, binary size, watch-tree scaling, and event burst scaling. (lines 540-552)
- [ ] Full test parity: block normal workflow replacement if any supported target, common install method, or expected release artifact regresses. (lines 546-552)
- [ ] Whole-project proof and debt map: classify every crate and module as proved, modeled, adapter-backed, sampled, metadata-only, opaque, or debt. (lines 555-578)
- [ ] Whole-project proof and debt map: separate correctness-critical debt from non-critical debt and require justification for every non-critical item. (lines 565-572)
- [ ] Whole-project proof and debt map: make release policy block lifecycle, replay, platform, adapter, generated-backend, and async debt in correctness-critical paths. (lines 565-578)
- [ ] Whole-project proof and debt map: make `kobo debt --summary`, proof reports, replay reports, and inspect output agree on the same complete project map. (lines 574-578)
- [ ] Independent equivalence review: run the original upstream test suite against the Kobo rewrite result. (lines 581-597)
- [ ] Independent equivalence review: run mutation tests for watcher events, child lifecycle, cancellation, config, ignore rules, async scheduling, platform behavior, and generated backend output. (lines 585-590)
- [ ] Independent equivalence review: have two independent reviewers compare behavior, generated Rust, diagnostics, proof/debt status, replay evidence, real command output, and release artifacts. (lines 587-597)
- [ ] Independent equivalence review: keep the confident full rewrite claim blocked until both reviewers agree the result is behavior-equivalent or stronger and the remaining boundary is honest. (lines 592-597)
- [ ] Clean replacement claim text: use the clean replacement claim only after Kobo owns the whole application source semantics. (lines 600-606)
- [ ] Clean replacement claim text: use the clean replacement claim only after external crates are replaced or formally modeled. (lines 600-606)
- [ ] Clean replacement claim text: use the clean replacement claim only after platform differences are captured as replayable or declared boundary evidence. (lines 600-606)
- [ ] Clean replacement claim text: use the clean replacement claim only after upstream behavior and release workflow are equivalent or stronger. (lines 600-606)

## Other-Language Implementation Comparison

These items are added from external implementation research, not from README lines. They exist to make the confident full rewrite bar less self-referential and to force Kobo to compare against mature watcher/reloader designs before claiming parity.

References checked:

- `watchexec`: https://github.com/watchexec/watchexec
- `nodemon`: https://github.com/remy/nodemon
- `chokidar`: https://raw.githubusercontent.com/paulmillr/chokidar/main/README.md
- Python `watchdog`: https://python-watchdog.readthedocs.io/en/stable/api.html
- Python `watchfiles`: https://watchfiles.helpmanual.io/
- Watchman: https://facebook.github.io/watchman/
- Go `air`: https://github.com/air-verse/air and https://raw.githubusercontent.com/air-verse/air/master/air_example.toml

- [x] Upstream `watchexec` comparison: map coalesced filesystem events into Kobo's normalized batch and replay model.
- [ ] Upstream `watchexec` comparison: map `.gitignore` and `.ignore` loading into Kobo's ignore and path-filter evidence model.
- [ ] Upstream `watchexec` comparison: map process group behavior into Kobo's process lifecycle obligations.
- [ ] Upstream `watchexec` comparison: map changed-path delivery through environment variables or stdin into Kobo's command execution and replay facts.
- [ ] Upstream `watchexec` comparison: map `watchexec-events`, `watchexec-signals`, `watchexec-supervisor`, process wrapping, ignore files, project origins, and notify-backed behavior into adapter or replacement coverage.
- [ ] Node `nodemon` comparison: model extension watch lists and extension inference from executed scripts.
- [ ] Node `nodemon` comparison: model ignore rules that match full absolute paths and default ignore directories.
- [ ] Node `nodemon` comparison: model legacy polling fallback for mounted, containerized, networked, or unreliable watch environments.
- [x] Node `nodemon` comparison: model delayed restart after the last file change so repeated uploads or bursty writes do not trigger multiple unnecessary restarts.
- [ ] Node `nodemon` comparison: model custom stop or reload signals and process-tree signal delivery.
- [ ] Node `nodemon` comparison: add equivalence fixtures for extension filtering, ignored paths, polling fallback, delay restart, and signal restart behavior.
- [ ] Node `chokidar` comparison: model event normalization from raw watcher events into add, change, unlink, addDir, unlinkDir, ready, raw, and error events.
- [ ] Node `chokidar` comparison: model atomic-write handling where delete-plus-add inside the atomic window becomes a change.
- [ ] Node `chokidar` comparison: model chunked-write handling where events are held until file size stabilizes.
- [ ] Node `chokidar` comparison: model recursion, depth limits, symlink-following behavior, current-working-directory relative paths, and dynamic add/unwatch/close operations.
- [ ] Node `chokidar` comparison: model polling fallback, polling intervals, binary polling intervals, permission-error handling, and file-handle exhaustion diagnostics.
- [ ] Node `chokidar` comparison: preserve raw event details as boundary evidence without treating raw backend output as stable high-level semantics.
- [x] Python `watchdog` comparison: model immutable filesystem events with event type, source path, destination path, directory flag, and synthetic flag.
- [ ] Python `watchdog` comparison: model moved, modified, created, closed, deleted, and directory-specific events as explicit event variants.
- [ ] Python `watchdog` comparison: model pattern matching, regex matching, ignore patterns, directory ignores, and case-sensitive matching as path-filter evidence.
- [ ] Python `watchdog` comparison: model event queues that skip repeated identical consecutive events.
- [ ] Python `watchdog` comparison: model observer lifecycle: schedule, start, dispatch, unschedule, unschedule all, and stop.
- [ ] Python `watchdog` comparison: model platform observer choices for Linux, macOS, BSD, Windows, and polling fallback.
- [x] Python `watchfiles` comparison: model debounced sets of file changes rather than single raw events.
- [ ] Python `watchfiles` comparison: model synchronous watch and async watch separately, including thread handoff and cancellation behavior.
- [ ] Python `watchfiles` comparison: model debounce, step, timeout, yield-on-timeout, stop event, recursive mode, permission behavior, forced polling, and polling delay.
- [ ] Python `watchfiles` comparison: model Windows-specific async timeout behavior and make platform-specific defaults source-visible.
- [ ] Watchman comparison: model recursive watched roots and root-settle behavior before command execution.
- [ ] Watchman comparison: model conservative startup behavior where newly watched or uncertain files are treated as freshly changed.
- [ ] Watchman comparison: model project-root discovery through root files and root enforcement.
- [ ] Watchman comparison: model case-insensitive filesystem behavior, canonical path recovery, and case-only rename reporting.
- [ ] Watchman comparison: model unsupported or illegal filesystem types as actionable boundary diagnostics.
- [ ] Watchman comparison: model symlink policy explicitly instead of inheriting watcher defaults without evidence.
- [ ] Go `air` comparison: model build command, entrypoint command, full command, binary arguments, pre-build commands, and post-exit commands separately.
- [ ] Go `air` comparison: model include extensions, include dirs, include files, exclude dirs, exclude files, exclude regex, unchanged-file exclusion, dangerous-root handling, and symlink following.
- [ ] Go `air` comparison: model polling mode, poll interval, rebuild delay, stop-on-error, interrupt-before-kill, kill delay, rerun, rerun delay, and clean-on-exit.
- [ ] Go `air` comparison: model platform-specific build overrides for Windows, macOS, and Linux.
- [ ] Go `air` comparison: model environment file loading and app environment inheritance before build and run.
- [ ] Go `air` comparison: add parity fixtures for config-file defaults, command-line config overrides, Docker or mounted-volume watch behavior, and platform-specific executable paths.
- [x] External comparison gate: turn every selected external behavior into either a Kobo-owned semantic rule, a formal adapter contract, a replay fixture, a debt item, or an explicit non-goal.
- [x] External comparison gate: do not copy external behavior blindly; require an explicit reason when Kobo intentionally differs from `watchexec`, `nodemon`, `chokidar`, `watchdog`, `watchfiles`, Watchman, or Go `air`.

## External Runtime Boundaries

- [x] Make source-mapped boundary models first-class for watcher and supervisor-policy work. (lines 56-78)
- [x] Account for `notify` or equivalent watcher backend behavior at the boundary. (lines 67-70)
- [ ] Account for OS signals at the boundary. (lines 67-70)
- [x] Account for process spawn behavior at the boundary. (lines 67-70)
- [x] Account for process group cleanup at the boundary. (lines 67-70)
- [ ] Account for stdio forwarding at the boundary. (lines 67-70)
- [ ] Account for terminal behavior at the boundary. (lines 67-70)
- [x] Account for time and debounce behavior at the boundary. (lines 67-70)
- [ ] Account for ignore and path crate behavior at the boundary. (lines 67-70)
- [x] Keep unsupported runtime surfaces visible as debt, escape, opaque, outside, or proof-failure instead of silently treating them as proven. (lines 59-65)

## Proof Boundaries

- [x] Keep proof claims limited to modeled Core obligation flow and replay assumptions unless broader proof evidence is added. (lines 80-93)
- [x] Do not let the dogfood slice imply proof coverage for generated Rust, Tokio, third-party crates, or arbitrary effects. (lines 82-93)
- [x] Extend proof coverage only when the corresponding runtime, adapter, and replay facts are explicitly modeled. (lines 86-93)

## Replay And Watcher Histories

- [x] Support bursty event sequences in replay. (lines 95-114)
- [ ] Support renamed temp-file save patterns in replay. (lines 100-103)
- [x] Support duplicate backend events in replay. (lines 100-103)
- [ ] Support ignored paths becoming relevant after config changes. (lines 100-103)
- [x] Support debounce windows in replay. (lines 100-105)
- [x] Support the case where the child process is still running when a new event arrives. (lines 104-107)
- [ ] Support shutdown while restart is pending. (lines 104-107)
- [ ] Support child exits racing with new events. (lines 104-107)
- [x] Keep incomplete or opaque replay evidence as an honest downgrade, not a silent pass. (lines 109-114)
- [x] Reduce downgrade frequency enough that replay remains useful for watcher histories. (lines 109-114)

## Process And Signal Semantics

- [x] Add a process-supervision model for restart policy. (lines 116-138)
- [x] Ensure every start happens exactly once when the policy says to start. (lines 122-123)
- [x] Prevent double-running exclusive children. (lines 124-125)
- [ ] Terminate or signal the previous child before replacement when required. (lines 126-127)
- [ ] Distinguish graceful stop, kill timeout, restart, and final shutdown. (lines 128-129)
- [x] Attach source-mapped evidence to each lifecycle action. (lines 130-131)
- [x] Record child exit as a replayable boundary event. (lines 132-135)
- [ ] Record child signal as a replayable boundary event. (lines 132-135)
- [ ] Record child timeout as a replayable boundary event. (lines 132-135)
- [ ] Record child cancellation as a replayable boundary event. (lines 132-135)

## Adapter Summaries

- [x] Define a stable watcher backend summary. (lines 140-159)
- [x] Include watcher event facts in the watcher backend summary. (lines 146-147)
- [x] Include watcher ordering facts in the watcher backend summary. (lines 146-147)
- [x] Include watcher duplicate-event facts in the watcher backend summary. (lines 146-147)
- [x] Include watcher unsupported-behavior facts in the watcher backend summary. (lines 146-147)
- [x] Define a stable process summary for spawn, signal, wait, kill, timeout, and process groups. (lines 148-149)
- [x] Define a stable time summary for debounce, sleep, cancel, and deterministic replay. (lines 150-151)
- [ ] Define a stable path/filter summary that separates pure path logic from external config or filesystem facts. (lines 152-153)
- [ ] Define a stable async runtime summary for task cancellation, join behavior, and scheduling. (lines 154-155)

## Public Migration And Inspection

- [x] Stabilize the supported migration command surfaces. (lines 161-174)
- [x] Keep public examples correct and supported. (lines 164-169)
- [x] Keep `inspect` behavior predictable. (lines 164-169)
- [x] Make replay commands match the real CLI. (lines 164-169)
- [x] Keep clean Rust export paths working across Cargo layouts. (lines 164-169)

## Whole-App Strict Migration Boundary

- [x] Treat broad adapter research as required before any whole-app strict migration claim. (lines 176-189)
- [x] Treat arbitrary ecosystem proof as outside the current proof surface. (lines 178-183)
- [x] Use the honest readiness bar: make event and supervisor core safer, clearer, and replayable while keeping mature Rust adapters at the boundary. (lines 185-189)

## Watcher Boundary Model Requirements

- [x] Model watcher event kind: create, modify, remove, rename, metadata, rescan, and unknown. (lines 194-200)
- [x] Model watcher path set and path role: source, destination, and parent. (lines 201-202)
- [x] Model watcher backend identity and confidence. (lines 203-204)
- [x] Model watcher ordering guarantee. (lines 205-206)
- [x] Model duplicate and coalesced markers. (lines 207-208)
- [x] Model ignored, debounced, and filter decision source spans. (lines 209-210)
- [x] Model replayable, sampled, metadata-only, and opaque classification. (lines 211-212)
- [x] Add a fixture replay for an editor-save burst. (lines 214-217)
- [x] Prove normalized batch stability under duplicate raw events. (lines 214-217)
- [x] Downgrade incomplete watcher evidence instead of passing it silently. (lines 218-220)

## Supervisor Obligation Model Requirements

- [x] Ensure every started child is waited, signaled, killed, or explicitly detached. (lines 223-229)
- [x] Make replacement policy prevent accidental double-running. (lines 230-231)
- [x] Make shutdown resolve pending debounce timers and active children. (lines 232-233)
- [x] Record process exits as boundary events. (lines 234-235)
- [x] Make restart decisions cite source-mapped policy branches. (lines 236-237)
- [x] Add verification that catches an orphan child. (lines 239-242)
- [x] Add verification that catches a double-run restart race. (lines 239-242)
- [x] Add replay for a sequence where a child exits while a new file event arrives. (lines 239-243)

## Time And Debounce Evidence Requirements

- [x] Record logical timer creation. (lines 246-252)
- [x] Record logical timer cancellation. (lines 246-252)
- [x] Record debounce window identity. (lines 253-254)
- [x] Record event-to-window membership. (lines 255-256)
- [x] Record timeout firing evidence. (lines 257-258)
- [x] Record cancellation or replacement cause. (lines 259-260)
- [x] Record replay grade for timer evidence. (lines 261-262)
- [x] Verify that three save events inside one debounce window cause one restart. (lines 264-267)
- [x] Verify that events across two debounce windows cause two restarts. (lines 264-267)
- [x] Classify incomplete time evidence as debt or partial replay. (lines 268-270)

## Adapter Summary Contract Requirements

- [x] Include crate and version range in adapter summaries. (lines 273-279)
- [x] Include operation names in adapter summaries. (lines 280-281)
- [x] Include modeled facts emitted by adapter summaries. (lines 282-283)
- [x] Include unsupported guarantees in adapter summaries. (lines 284-285)
- [x] Include replay confidence in adapter summaries. (lines 286-287)
- [x] Include source-map anchor behavior in adapter summaries. (lines 288-289)
- [x] Include compatibility checks for Cargo feature flags in adapter summaries. (lines 290-291)
- [x] Drive watcher scenario evidence from a `notify`-like adapter summary. (lines 293-296)
- [x] Drive child lifecycle evidence from a process adapter summary. (lines 293-297)
- [x] Turn missing or stale summaries into actionable diagnostics, not generic opaque debt. (lines 298-300)

## Real-Trace Scenario Import Requirements

- [x] Import observed watcher logs into `.kwit` or scenario fixtures. (lines 303-309)
- [x] Import observed process logs into `.kwit` or scenario fixtures. (lines 303-309)
- [x] Shrink or minimize repeated bursts without losing causality. (lines 310-311)
- [x] Preserve raw data from imported traces. (lines 312-313)
- [x] Preserve normalized modeled facts from imported traces. (lines 312-313)
- [x] Report exact, modeled, sampled, metadata-only, and opaque trace classifications. (lines 314-315)
- [x] Capture a real save/restart session. (lines 317-320)
- [x] Replay the captured save/restart session with the same supervisor decisions. (lines 317-320)
- [x] Make mutating event order fail replay when it changes the modeled behavior. (lines 321-323)
- [x] Make mutating child-exit facts fail replay when it changes the modeled behavior. (lines 321-323)

## Stable Project Migration Commands

- [x] Keep `kobo check` copy-paste valid. (lines 326-333)
- [x] Keep `kobo test --sim <profile> <file>` copy-paste valid. (lines 326-333)
- [x] Keep `kobo replay <witness>` copy-paste valid. (lines 326-333)
- [x] Keep `kobo inspect --clean --cargo <dir> <file>` copy-paste valid. (lines 326-333)
- [x] Keep `kobo debt` copy-paste valid. (lines 326-333)
- [x] Make migration docs distinguish ownership-first migration from future history-lane migration. (lines 334-335)
- [x] Make project reports explain Kobo-owned modules. (lines 336-337)
- [x] Make project reports explain Rust-owned modules. (lines 336-337)
- [x] Make project reports explain boundary-debt modules. (lines 336-337)
- [x] Ensure a clean `watchexec` dogfood branch runs docs commands without undocumented flags or workarounds. (lines 339-342)

## Large-Project Ergonomics

- [x] Make multi-crate Cargo usage normal. (lines 345-351)
- [x] Allow Rust and Kobo modules to coexist. (lines 352-353)
- [x] Make source maps jump generated Rust diagnostics back to Kobo. (lines 354-355)
- [x] Provide `kobo debt --summary` adapter debt by subsystem. (lines 356-357)
- [ ] Gate CI only on the Kobo-owned supervisor slice first. (lines 358-359)
- [x] Keep `notify`, CLI/config, and platform code in Rust while Kobo owns event/supervisor policy. (lines 361-364)
- [x] Avoid fragile glue in the mixed Rust/Kobo dogfood branch. (lines 361-364)

## Proposed v0.16.2 Scope

- [x] Do not attempt a full `watchexec` rewrite as the scope target. (lines 331-337)
- [x] Make the scope target: credible `watchexec` supervisor-policy rewrite capability. (lines 331-337)
- [x] Ship watcher boundary facts. (lines 339-340)
- [x] Ship process supervisor obligations. (lines 341-342)
- [x] Ship debounce and time evidence. (lines 343-344)
- [x] Ship watcher, process, and time adapter summaries. (lines 345-346)
- [x] Ship real-trace import into witnesses. (lines 347-348)
- [x] Ship stable migration, inspect, and debt commands for a mixed project. (lines 347-348)
- [x] Ship one dogfood fixture. (lines 347-349)

## First Dogfood Slice

- [x] Start from the smallest policy/supervisor slice, not the whole repository. (lines 351-356)
- [x] Implement `RawEvent -> filter paths`. (lines 357-359)
- [x] Implement `filter paths -> debounce/coalesce`. (lines 357-359)
- [x] Implement `debounce/coalesce -> build Action`. (lines 357-359)
- [x] Implement `build Action -> apply RestartPolicy`. (lines 357-359)
- [x] Implement `apply RestartPolicy -> update SupervisorState`. (lines 357-359)
- [x] Keep `notify`, process, and CLI as Rust adapters. (lines 361-368)
- [x] Route Rust adapters into Kobo event policy and supervisor code. (lines 361-368)
- [x] Route Kobo supervisor decisions back into the Rust process execution adapter. (lines 361-368)
- [x] Make one-restart-per-debounce-window explicit. (lines 370-373)
- [x] Make no-orphan-child behavior explicit. (lines 374-375)
- [x] Make no-double-running-exclusive-restart behavior explicit. (lines 376-377)
- [x] Make shutdown resolution for timers and children explicit. (lines 378-379)
- [x] Make ignored events unable to trigger actions. (lines 380-381)
- [x] Make opaque watcher behavior downgrade replay instead of pretending exact replay. (lines 382-385)

## Mixed Rewrite Confidence Bar

- [x] Prove the event/supervisor slice runs in a real mixed Cargo project. (lines 387-393)
- [x] Make watcher, process, and time adapters produce typed evidence. (lines 394-395)
- [x] Make replay catch mutated event order. (lines 396-397)
- [x] Make replay catch missing child exit. (lines 396-397)
- [x] Make replay catch duplicate raw events. (lines 396-397)
- [x] Make release profile block unresolved child lifecycle obligations. (lines 398-399)
- [x] Make `kobo inspect --clean --cargo` emit reviewable Rust for the slice. (lines 400-401)
- [x] Make `kobo debt --summary` distinguish acceptable adapter debt from blockers. (lines 400-401)
- [x] Ensure docs use supported command shapes. (lines 400-401)
- [ ] Get two reviewers to audit the dogfood branch as production-shaped, not fixture-shaped. (lines 400-401)

## Clean Full Replacement Boundary

- [x] Do not claim clean replacement just because the mixed-rewrite bar passes. (lines 403-411)
- [x] Require Kobo to own whole-project source-level semantics before any clean replacement claim. (lines 408-411)
- [x] Keep Rust only as backend or explicit foreign boundary for a clean replacement claim. (lines 408-411)

## Future Whole-Project Language Coverage

- [ ] Parse the Rust-shaped surface needed by current `watchexec`. (lines 414-426)
- [ ] Check the Rust-shaped surface needed by current `watchexec`. (lines 414-426)
- [ ] Lower the Rust-shaped surface needed by current `watchexec`. (lines 414-426)
- [ ] Source-map the Rust-shaped surface needed by current `watchexec`. (lines 414-426)
- [ ] Preserve rare diagnostics and edge-case source locations. (lines 421-422)
- [ ] Make multi-crate Cargo normal for whole-project migration. (lines 423-424)
- [ ] Make tests, examples, and feature flags normal for whole-project migration. (lines 423-424)
- [ ] Gate whole-project coverage on representing the full source tree as Kobo-owned. (lines 428-432)
- [ ] Do not satisfy whole-project coverage by deleting platform paths. (lines 428-432)
- [ ] Do not satisfy whole-project coverage by flattening APIs. (lines 428-432)

## Future Platform Semantics

- [ ] Add first-class platform boundary models for filesystem events. (lines 435-446)
- [ ] Add first-class platform boundary models for process execution. (lines 435-446)
- [ ] Add first-class platform boundary models for signals. (lines 435-446)
- [ ] Add first-class platform boundary models for process groups. (lines 435-446)
- [ ] Add first-class platform boundary models for environment variables. (lines 435-446)
- [ ] Add first-class platform boundary models for terminal IO. (lines 435-446)
- [ ] Add first-class platform boundary models for stdio. (lines 435-446)
- [ ] Add first-class platform boundary models for timers. (lines 435-446)
- [ ] Add first-class platform boundary models for paths. (lines 435-446)
- [ ] Classify platform evidence as exact, modeled, sampled, metadata-only, or opaque. (lines 444-445)
- [ ] Make platform differences source-visible and testable. (lines 446-448)
- [ ] Gate platform semantics on Windows, macOS, and Linux behavior tests. (lines 450-457)
- [ ] Include watcher event tests in platform behavior tests. (lines 450-457)
- [ ] Include process restart tests in platform behavior tests. (lines 450-457)
- [ ] Include signal tests in platform behavior tests. (lines 450-457)
- [ ] Include stdin tests in platform behavior tests. (lines 450-457)
- [ ] Include path filtering tests in platform behavior tests. (lines 450-457)
- [ ] Keep replay grades honest across platform behavior tests. (lines 450-457)

## Future External Crate Replacement Or Formal Adapter Coverage

- [ ] Replace each correctness-critical crate in Kobo or cover it with a formal adapter. (lines 460-471)
- [ ] Add version summaries for each correctness-critical adapter. (lines 467-468)
- [ ] Add conformance tests for each correctness-critical adapter. (lines 467-468)
- [ ] Add replay evidence for each correctness-critical adapter. (lines 467-468)
- [ ] Add stale-summary detection for each correctness-critical adapter. (lines 469-471)
- [ ] Cover watcher backends as critical adapters. (lines 473-481)
- [ ] Cover async runtime as a critical adapter. (lines 473-481)
- [ ] Cover process handling as a critical adapter. (lines 473-481)
- [ ] Cover signal handling as a critical adapter. (lines 473-481)
- [ ] Cover ignore and path handling as critical adapters. (lines 473-481)
- [ ] Cover config as a critical adapter. (lines 473-481)
- [ ] Cover CLI as a critical adapter. (lines 473-481)
- [ ] Cover serialization as a critical adapter. (lines 473-481)
- [ ] Cover logging and tracing as critical adapters. (lines 473-481)
- [ ] Cover errors as a critical adapter. (lines 473-481)
- [ ] Gate external crate coverage on no correctness-critical behavior depending on unversioned metadata-only or opaque summaries. (lines 483-487)

## Future Async Runtime Semantics

- [ ] Define async spawn semantics. (lines 490-501)
- [ ] Define async join semantics. (lines 490-501)
- [ ] Define async cancel semantics. (lines 490-501)
- [ ] Define async select semantics. (lines 490-501)
- [ ] Define timer semantics. (lines 490-501)
- [ ] Define channel semantics. (lines 490-501)
- [ ] Define backpressure semantics. (lines 490-501)
- [ ] Define shutdown semantics. (lines 490-501)
- [ ] Define blocking-operation semantics. (lines 490-501)
- [ ] Provide either a native runtime model or versioned Tokio adapter. (lines 497-499)
- [ ] Back the runtime model or Tokio adapter with conformance evidence. (lines 497-499)
- [ ] Make replay preserve scheduler facts that matter for behavior. (lines 500-501)
- [ ] Gate async semantics by making task, timer, cancel, and channel mutations affect replay when they should. (lines 503-508)
- [ ] Avoid false confidence outside the async model. (lines 503-508)

## Future Generated Rust Backend

- [ ] Keep generated Rust reviewable. (lines 511-518)
- [ ] Keep generated Rust deterministic. (lines 511-518)
- [ ] Keep generated Rust source-mapped. (lines 511-518)
- [ ] Keep generated Rust from becoming the ordinary source of diagnosis. (lines 516-518)
- [ ] Make diagnostics point to Kobo source and project concepts. (lines 519-520)
- [ ] Make replay point to Kobo source and project concepts. (lines 519-520)
- [ ] Make debt reporting point to Kobo source and project concepts. (lines 519-520)
- [ ] Make proof reporting point to Kobo source and project concepts. (lines 519-520)
- [ ] Make LSP support point to Kobo source and project concepts. (lines 519-520)
- [ ] Ensure generated Rust compiles on the same targets as the replacement project. (lines 521-522)
- [ ] Gate the backend on debugging watcher/restart behavior from Kobo source, witnesses, and source-mapped diagnostics without editing generated Rust. (lines 524-529)

## Future Full Test And Release Parity

- [ ] Port upstream tests. (lines 532-544)
- [ ] Add Kobo replay tests. (lines 536-537)
- [ ] Add Kobo liveness tests. (lines 536-537)
- [ ] Preserve CLI behavior. (lines 538-539)
- [ ] Preserve config behavior. (lines 538-539)
- [ ] Preserve exit behavior. (lines 538-539)
- [ ] Preserve logging behavior. (lines 538-539)
- [ ] Preserve package behavior. (lines 538-539)
- [ ] Preserve platform behavior. (lines 538-539)
- [ ] Measure startup performance. (lines 540-541)
- [ ] Measure steady-state performance. (lines 540-541)
- [ ] Measure restart performance. (lines 540-541)
- [ ] Measure memory behavior. (lines 540-541)
- [ ] Measure binary behavior. (lines 540-541)
- [ ] Preserve release artifacts and install behavior. (lines 542-544)
- [ ] Gate release parity on replacing the original in normal workflows without regressions, unsupported targets, or unacceptable performance. (lines 546-552)

## Future Whole-Project Proof And Debt

- [ ] Classify every crate and module as proved, modeled, adapter-backed, sampled, metadata-only, opaque, or debt. (lines 555-564)
- [ ] Make release policy block lifecycle debt in correctness-critical paths. (lines 565-570)
- [ ] Make release policy block replay debt in correctness-critical paths. (lines 565-570)
- [ ] Make release policy block platform debt in correctness-critical paths. (lines 565-570)
- [ ] Make release policy block adapter debt in correctness-critical paths. (lines 565-570)
- [ ] Justify non-critical debt. (lines 571-572)
- [ ] Separate non-critical debt from correctness-critical debt. (lines 571-572)
- [ ] Gate proof and debt on a complete project map across debt summary, proof, and replay. (lines 574-578)

## Future Independent Equivalence Review

- [ ] Run the original upstream tests. (lines 581-590)
- [ ] Add mutation tests for watcher behavior. (lines 585-586)
- [ ] Add mutation tests for child behavior. (lines 585-586)
- [ ] Add mutation tests for cancellation behavior. (lines 585-586)
- [ ] Add mutation tests for config behavior. (lines 585-586)
- [ ] Add mutation tests for platform behavior. (lines 585-586)
- [ ] Have two independent reviewers compare behavior. (lines 587-590)
- [ ] Have two independent reviewers compare generated Rust. (lines 587-590)
- [ ] Have two independent reviewers compare diagnostics. (lines 587-590)
- [ ] Have two independent reviewers compare proof and debt. (lines 587-590)
- [ ] Have two independent reviewers compare real command output. (lines 587-590)
- [ ] Gate clean replacement on both reviewers agreeing behavior is equivalent or stronger and the remaining boundary is honest. (lines 592-597)

## Clean Replacement Claim Text

- [x] Use the clean replacement claim only after Kobo owns the whole application source semantics. (lines 600-606)
- [x] Use the clean replacement claim only after external crates are either replaced or formally modeled. (lines 600-606)
- [x] Use the clean replacement claim only after platform differences are captured as replayable or declared boundary evidence. (lines 600-606)
- [x] Use the clean replacement claim only after upstream behavior and release workflow are equivalent or stronger. (lines 600-606)

## v0.16.2 Non-Goals

- [x] Do not require v0.16.2 to prove all `notify` internals. (lines 613-617)
- [x] Do not require v0.16.2 to prove Tokio internals. (lines 617-619)
- [x] Do not require v0.16.2 to replace `clap`, ignore, or platform adapters. (lines 619-621)
- [x] Do not require v0.16.2 to rewrite every crate. (lines 621-622)
- [x] Do not require v0.16.2 to claim generated Rust binary correctness beyond the stated proof and translation boundary. (lines 622-624)
- [x] Keep the right v0.16.2 goal focused on explicit, replayable, auditable stateful restart policy. (lines 625-626)

## Final Recommendation Pass Criteria

- [x] Use `watchexec` as a benchmark, not an immediate full rewrite target. (lines 628-631)
- [x] Make the milestone succeed by catching lifecycle bugs in the event/supervisor core. (lines 633-636)
- [x] Make the milestone succeed by catching replay bugs in the event/supervisor core. (lines 633-636)
- [x] Keep external crates honest while the event/supervisor core is owned by Kobo. (lines 633-636)
- [x] Treat opaque ports as milestone failure. (lines 637-639)
- [x] Treat downgraded replay as milestone failure when it replaces the intended replay evidence. (lines 637-639)
- [x] Treat unsupported command examples as milestone failure. (lines 637-639)
- [x] Treat generated Rust that cannot be inspected without internal knowledge as milestone failure. (lines 637-639)
