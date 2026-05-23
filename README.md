# kobo

Kobo makes unreproducible stateful bugs reproducible. It is a Rust-shaped
language and toolchain for moving risky stateful code into stronger evidence
without changing the ordinary runtime meaning of accepted programs.

## Gradual Guarantees

Kobo is one language. Its gradual guarantees live in guarantee policy, CI
gates, and scoped enforcement, not in separate source identities. Projects can
use the `dev`, `checked`, and `release` profiles, or spell out `[guarantees]`
directly for ownership, liveness, replay, boundary, and error policy.

The ordinary runtime meaning of accepted programs stays Rust-compatible across
profiles. Tightening a path changes diagnostics, evidence, harnesses, proof
requirements, and rejection policy; it does not ask the user to rewrite normal
Kobo source as a backend harness.

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
- `kobo watch --plan <file>` for restartable scoped guarantee work that persists
  and reloads a bounded watch plan.

Experts can stay on the Kobo workflow while inspecting or pinning simulation
engines when needed: `kobo sim scout --why`, `kobo inspect --sim --harness
--backend shuttle`, `kobo test --sim deep --backend shuttle --scheduler pct`,
and backend-native controls when the adapter can represent them without
changing normal source imports.

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
