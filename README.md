# kobo

Kobo makes unreproducible stateful bugs reproducible. It is a Rust-shaped
language and toolchain for moving risky stateful code into stronger evidence
without changing the ordinary runtime meaning of accepted programs.

## Gradual Guarantees

Kobo uses gradual guarantees. It is a Rust-shaped static language with scoped
evidence levels, not a dynamic type migration system. Script, Checked, and
Strict are scoped modes for diagnostics, evidence, harnesses, proof
requirements, and rejection policy. The mode invariant is: Script, Checked, and
Strict preserve the same ordinary runtime behavior for accepted code.
Script, Checked, and Strict preserve the same ordinary runtime behavior for accepted code.

## Modeled Wards

Modeled wards describe stateful risk zones such as durable queues and async
gateways. A ward can declare state, lifecycle obligations, invariants, temporal
checks, scenarios, ports, recordings, and debt. Ports identify external
surfaces, recordings capture replayable boundary evidence, and opaque boundaries
keep unknown external internals visible instead of pretending they are
deterministic.

Kobo can replay modeled wards and recorded boundaries. Unknown crate internals
stay outside the replay claim unless they are exposed through recorded boundary
evidence.

## CLI Workflow

The primary workflow is:

- `kobo check` for diagnostics and early warning precursors.
- `kobo test --sim quick --witness-dir .kobo/witnesses` for scenario evidence.
- `kobo replay <witness.kwit>` for exact witness validation when evidence allows.
- `kobo inspect --clean --cargo <dir>` for the clean Rust exit ramp.
- `kobo debt --summary`, `kobo debt --json`, and `kobo debt --watch` for scoped
  migration and precursor tracking.
- `kobo watch --plan <file>` for restartable scoped modes that persist and reload
  a bounded watch plan.

## Editor Workflow

`kobo-lsp` reports diagnostics, hover and explain actions, scenario runnables,
witness links, and Rust navigation through rust-analyzer/source-map delegation.
The VS Code contribution under `editors/vscode` registers `.kobo` syntax and the
run, replay, and explain actions.

## Guides

- [Modeled Ward Tutorial](docs/modeled-wards.md)
- [Strict Liveness Reference](docs/replay-and-strict-liveness.md)
- [Failure Lab](docs/failure-lab.md)
- [Editor And Clean Rust Workflow](docs/editor-clean-rust-workflow.md)
- [Migration Guide](docs/migration-guide.md)
