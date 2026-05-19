# v0.12 Review Round 6: Partial Production Depth

Date: 2026-05-19

One final review agent returned `partial implemented or not full prod depth yet for v0.12` after commit `fd6badf`; the other returned `full prod depth for v0.12`. Because any partial verdict continues the loop, this round tracks the remaining evidence-provenance blockers.

## Blockers

- Handler lifecycle evidence is collected before lowering and labels terminal actions as `lowered-guard-calls`, even though reply/reject/cancel evidence is not actually scanned from the lowered guard calls.
- Parallel lowering records structured evidence after codegen lowering, but does not explicitly state that lowering was gated by the analysis pass and accepted only after no K0061 blocker diagnostics were present.
- Task-local strict async suppression still uses broad `source.contains("spawn local") && source.contains(binding_name)` matching instead of the dedicated task-local scanner facts.

## Required Next Pass

- Derive handler lifecycle evidence from the lowered Rust function body after `KoboHandlerLifecycleGuard` calls have been inserted.
- Make parallel runtime evidence state its accepted-lowering gate and preserve diagnostics as the rejection path.
- Replace broad task-local suppression with dedicated scanner facts for normal-spawn and local-zone behavior.
