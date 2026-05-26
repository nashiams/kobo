import Lake
open Lake DSL

package koboProof where

lean_lib KoboProof where
  srcDir := "proof/lean"
  roots := #[
    `KoboCore,
    `ObligationRules,
    `Preservation,
    `NoSilentLoss,
    `SampleTrace,
    `CertificateSoundness
  ]
