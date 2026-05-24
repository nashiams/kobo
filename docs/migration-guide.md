# Migration Guide

Kobo migration is scoped guarantee work. Users can start with the `dev` profile,
add `checked` evidence for selected stateful code, and use `release` where the
team wants stronger liveness, replay, and boundary gates.

`dev` keeps friction low while surfacing warnings and debt. `checked` adds more
structured evidence, witness generation, and boundary policy. `release` turns
unresolved lifecycle obligations and unsupported proof surfaces into blocking
diagnostics when the active scope requires it.

The guarantee policy applies to diagnostics, evidence, harnesses, proof
requirements, and rejection policy. The source language remains one Kobo
language, and accepted programs keep the same ordinary Rust-compatible runtime
meaning across profiles.

Example project policy:

```toml
[guarantees]
ownership = "record"
liveness = "checked"
replay = "checked"
boundaries = "record"
errors = "typed"

[ci.release]
deny_new_debt = true
strict_paths = ["src/payment/**", "src/auth/**"]
deny_downgrade_without_reason = true
```

Simulation controls follow the same ladder. Start with stable Kobo profiles and
keep backend-native knobs explicit. `default_profile` is used when `kobo test`
is run without `--sim`.

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

Pin backend-native controls only when the adapter can represent the requested
knob:

```text
kobo test --sim quick
kobo test --sim deep --profile async
kobo test --sim exhaustive --profile sync --backend loom --scheduler exhaustive
kobo inspect --sim --harness
```

Unsupported backend-native knobs such as Shuttle PCT scheduling remain visible
as scenario debt or can be run outside Kobo with imported witness metadata later;
Kobo should not silently pretend unsupported controls were applied.

Recommended migration path:

1. run `kobo check` and fix high-signal lifecycle diagnostics first;
2. declare or infer lifecycle obligations around one ward-sized risk area;
3. add ports and recordings for external boundaries;
4. run `kobo test --sim quick --witness-dir .kobo/witnesses`;
5. replay any `.kwit` witness before relying on it;
6. inspect migration debt with `kobo debt --summary` or `kobo debt --json`;
7. use `kobo debt --watch` while tightening a scoped area;
8. export clean Rust with `kobo inspect --clean --cargo <dir>` when the Kobo
   evidence surface is no longer needed in that area.

Debt is part of the product surface. Unknown helpers, external crates, FFI,
global stores, unsupported containers, and unsupported recursion should remain
visible as transfer, return, opaque, outside, debt, conservative escape, or
proof failure facts until the user supplies stronger evidence.
