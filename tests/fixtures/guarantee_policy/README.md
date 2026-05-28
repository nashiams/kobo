# Guarantee Policy Oracle Fixtures

These fixtures are the source corpus for guarantee-policy, replay, simulation,
boundary, and LSP integration tests. The Rust harnesses copy these files into
isolated temporary projects before running the public `kobo` CLI.

The fixtures intentionally live outside the harness crate so a zero-context
implementor can inspect the expected behavior without reading Rust test code
first.

## Anti-Gaming Rules

- Do not replace fixture-backed assertions with canned stdout.
- Diagnostics must use the actual source file, symbol, line, and policy path.
- Template files contain uppercase placeholders such as `__SCENARIO__`; tests
  replace them with runtime-generated names to reject hardcoded symbols.
- Shifted fixtures must continue to report shifted source lines. A fixed line
  number is a failing implementation.
- `.kwit` replay fixtures are structured schemas, not arbitrary blobs.

## Groups

- `policy`: guarantee profile, downgrade, and path-glob policy behavior.
- `strict`: release/strict ownership debt, audit tiers, and field granularity.
- `sim`: sim init, deterministic event streams, liveness, nondeterminism, and traps.
- `replay`: witness schema and replay exactness/divergence.
- `errors`: profile-specific error strategy and replay boundary policy.
- `lsp`: registry-backed diagnostic payloads and source ranges.
- `traps`: fixtures whose only purpose is to catch stubs and string-only output.
