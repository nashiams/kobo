# Proof Assistant Mechanization

Kobo uses Lean as the single proof assistant target for the mechanized obligation core. Rocq/Coq and Isabelle remain documented fallbacks, but this repository does not scaffold them for this slice.

The allowed product claim is narrow: v0.16 begins mechanized soundness for Kobo's obligation core and proof verifier. The mechanized model covers obligation states, the eight rule IDs in `crates/compiler/kobo-proof/rules/obligation_rules.toml`, modeled exits, explicit template assumptions, and a sample certificate trace bridge.

The sample accepted .kproof trace has a matching mechanized trace model. That bridge is sample-scoped; it does not prove every future certificate, generated Rust binary behavior, Tokio internals, or third-party crate behavior.

Local release commands:

```powershell
lake build
lake env lean proof/lean/KoboCore.lean
lake env lean proof/lean/ObligationRules.lean
lake env lean proof/lean/Preservation.lean
lake env lean proof/lean/NoSilentLoss.lean
lake env lean proof/lean/CertificateSoundness.lean
```

The Lean toolchain is pinned in `lean-toolchain` and mirrored in `proof/lean/lean-toolchain` for tools opened from the proof directory.
