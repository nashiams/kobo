# Migration Guide

Kobo migration is scoped. Users can start with Script, add Checked evidence for
selected stateful code, and use Strict where the team wants stronger liveness
and replay gates.

Script keeps friction low while surfacing warnings and debt. Checked adds more
structured evidence, witness generation, and boundary policy. Strict turns
unresolved lifecycle obligations and unsupported proof surfaces into blocking
diagnostics when the active scope requires it.

Scoped modes apply to diagnostics, evidence, harnesses, proof requirements, and
rejection policy. These scoped modes do not fork runtime behavior. The mode invariant
is: Script, Checked, and Strict preserve the same ordinary runtime behavior for
accepted code.

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
