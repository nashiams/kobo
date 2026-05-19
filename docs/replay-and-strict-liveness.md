# Strict Liveness Reference

Strict liveness checks whether lifecycle obligations can reach an exit without a
valid terminal action. It is built around the Core CFG, where each block carries
an obligation environment through statements, terminators, and joins.

The checker tracks these obligation outcomes:

- active: the obligation is still live.
- discharged: a terminal action such as `ack`, `close`, `reply`, or `cancel`
  consumed the obligation.
- transfer: ownership moved to a helper and needs a trusted summary.
- returned: ownership came back from a helper.
- escaped: ownership left the local proof surface.
- proof failure: Kobo cannot justify a safe outcome from the available facts.

A join combines incoming path states. If one path still has an active obligation
and another path discharged it, Kobo reports a path-sensitive liveness problem
unless a source-mapped merge, summary, or explicit transfer explains the
difference. Unknown recursion, unsupported containers, FFI, global stores, and
opaque exits become conservative escape, debt, or proof failure facts.

Replay validation is separate from ordinary execution. The replay validation
step reads a `.kwit` witness that records
the scenario, scheduler seed, boundary evidence, trace events, source spans, and
obligation states that were observed. `kobo replay <witness.kwit>` validates
that evidence. When a recording or boundary is incomplete, Kobo downgrades the
replay status instead of claiming more than the witness can support.

The mode invariant is unchanged: Script, Checked, and Strict preserve the same
ordinary runtime behavior for accepted code. Scoped modes change diagnostics,
evidence requirements, harness generation, proof obligations, and rejection
policy. They do not rewrite the accepted program's ordinary runtime meaning.
