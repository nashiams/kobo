# Proof Assistant Mechanization

Kobo uses Lean to mechanize the narrow obligation calculus that the independent proof verifier checks. The mechanized model covers Kobo's obligation core and proof verifier.

The rule synchronization source is `crates/compiler/kobo-proof/rules/obligation_rules.toml`. It lists the stable create, transfer, split, discharge, return, cancel, panic, and opaque rule IDs, their Rust verifier owners, and their Lean rule and theorem names.

The sample accepted .kproof trace has a matching mechanized trace model. This bridge is sample-scoped and checks that the selected trace shape has matching rule IDs, a template ID/version assumption, and an opaque ledger assumption in Lean.

Lean is required for release verification. Normal Kobo checking, running, inspecting, and proof artifact emission do not require users to install Lean.

Kobo does not prove generated Rust binary behavior, Tokio internals, third-party crate internals, or arbitrary business logic. Generated Rust-facing claims remain limited to translation validation of proof-relevant lowering metadata.
