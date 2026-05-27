# Proof Assistant Mechanization

Kobo uses Lean as the single proof assistant target for the mechanized obligation core. Rocq/Coq and Isabelle remain documented fallbacks, but this repository does not scaffold them for this slice.

The allowed product claim is narrow: the mechanized model covers Kobo's obligation core and proof verifier. The model includes obligation states, the eight rule IDs in `crates/compiler/kobo-proof/rules/obligation_rules.toml`, modeled exits, explicit template assumptions, and a sample certificate trace bridge.

The sample accepted .kproof trace has a matching mechanized trace model. That bridge is sample-scoped; it does not prove every future certificate, generated Rust binary behavior, Tokio internals, or third-party crate behavior.

Local release commands:

```powershell
cd proof/lean
lake build KoboProof
lake env lean KoboCore.lean
lake env lean ObligationRules.lean
lake env lean Preservation.lean
lake env lean NoSilentLoss.lean
lake env lean SampleTrace.lean
lake env lean CertificateSoundness.lean
```

The Lean toolchain is pinned in `lean-toolchain` and mirrored in `proof/lean/lean-toolchain` for tools opened from the proof directory.
