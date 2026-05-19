# Implementation Notes: Phase 08 Watch Groundwork And Release Gates

v0.12 productizes service runtime evidence rather than claiming a new general replay engine.

## Watch Groundwork

`kobo watch --plan FILE` is bounded. It prints:

- the scoped input file;
- concrete `.kobo` files in the scope;
- rerun targets such as `kobo check FILE` and `kobo inspect FILE`;
- optional invalidation when `--changed FILE` is provided.

Unscoped workspace watch is disabled by default. The implementation must not start broad background compilation or a long-lived runtime contract without an explicit target.

## Product Loop

The integrated release fixture covers:

- service lowering and service runtime witness evidence;
- handler lifecycle obligations;
- record boundaries for replayed values;
- activity boundaries for external side effects;
- explicit `spawn local` scheduling evidence;
- safe parallel loop lowering;
- runtime profile material in generated output and witness digest evidence.

## Honesty Boundary

Allowed v0.12 claims:

- service and handler code lower to production-shaped Rust;
- runtime profile values are inspectable before execution and serialized after execution;
- record/activity evidence keeps external effects visible;
- activity and opaque boundaries keep witnesses partial when exact replay is not justified.

Forbidden v0.12 claims:

- exact replay of arbitrary external side effects;
- full ecosystem exploration for ordinary third-party crates;
- Failure Lab style fault-injection product claims;
- watch mode as an unbounded background runtime.
