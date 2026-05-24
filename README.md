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

## Proof Boundaries

Kobo proof artifacts use deliberately narrow wording. `proof: modeled Core obligation flow`
means the verifier checked the modeled Kobo Core obligation facts under recorded
assumptions, loop invariants, and declared bounds. It does not prove generated Rust binary behavior
or arbitrary library internals.

Loop proof is accepted through inferred protocol invariants, checked user
invariants, or complete bounded enumeration. Complete finite exploration may
say `bounded proof: all <N> histories explored under declared bounds`; sampled,
timed-out, duplicate-inflated, or dimension-incomplete exploration must say
`evidence only`.

Generated Rust-facing claims require translation validation. `translation validated`
means codegen-emitted lowering metadata preserves the modeled Core obligation
trace and source-map anchors for proof-relevant events. It is not a whole-program
Rust correctness claim.

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
engines when needed: `kobo sim scout --why`, `kobo inspect --sim --harness`,
`kobo test --sim exhaustive --profile sync --backend loom --scheduler exhaustive`,
and backend-native controls when the adapter can represent them without changing
normal source imports. Unsupported native adapter knobs are rejected with
scenario-debt guidance rather than accepted as metadata.

Stable simulation defaults live in `Kobo.toml`:

```toml
[sim]
default_profile = "quick"
show_backend_choices = false

[sim.profile.quick]
schedule_budget = 10000
seed_count = 16
shrink = "off"

[sim.profile.deep]
schedule_budget = 1000000
seed_count = 1024
shrink = "best-effort"

[sim.backend.loom]
enabled = true
scheduler = "exhaustive"
replay_token = "record"
max_branches = 100000
checkpoint_replay = true
```

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
