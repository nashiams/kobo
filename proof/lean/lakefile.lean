import Lake
open Lake DSL

package koboProof where

lean_lib KoboProof where
  srcDir := "."
  roots := #[
    `KoboCore,
    `ObligationRules,
    `Preservation,
    `NoSilentLoss,
    `SampleTrace,
    `CertificateSoundness
  ]
