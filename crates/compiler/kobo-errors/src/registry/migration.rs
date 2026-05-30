use crate::{KErrorCode, Severity};

use super::{
    entry, DiagnosticCategory, DiagnosticRegistryEntry, MachineEditPolicy, ModeBehavior,
    SeverityPolicy, SuggestionPolicy,
};
pub(super) fn entries() -> Vec<DiagnosticRegistryEntry> {
    use DiagnosticCategory::{MigrationBoundary, RustcRemap};
    use MachineEditPolicy::{NotApplicable, RefuseByDefault};
    use ModeBehavior::NoModeDependency;
    use Severity::{Error, Warning};
    use SeverityPolicy::Always;
    use SuggestionPolicy::{HelpOnly, ReviewOnly};

    vec![
        entry(
            KErrorCode::K0090,
            "external-crate-migration-boundary",
            "migration cannot continue - value crosses into external crate",
            "Ownership depends on a boundary Kobo cannot currently solve.",
            "External crate boundaries require an explicit ownership hint, summary, or deferral instead of silent inference.",
            MigrationBoundary,
            Error,
            Always(Error),
            NoModeDependency,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0095,
            "macro-generated-ownership-unknown",
            "ownership of macro-generated value cannot be inferred",
            "A macro-generated value lacks enough source structure for ownership inference.",
            "Kobo reports macro boundaries explicitly because generated ownership facts may not map cleanly back to source.",
            MigrationBoundary,
            Error,
            Always(Error),
            NoModeDependency,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0096,
            "legacy-mode-directive-profile-alias",
            "legacy mode directive is a guarantee profile alias",
            "A source file used `//! kobo:mode = ...`, which is retained only as a compatibility alias for the equivalent guarantee profile.",
            "Use `--profile dev|checked|release` or project guarantee policy instead. Kobo source remains one language; gradualness belongs to policy, CI gates, and scoped enforcement.",
            MigrationBoundary,
            Warning,
            Always(Warning),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0099,
            "rust-error-mapped-to-source",
            "Rust reported an error in generated code",
            "I mapped the Rust error back to the original Kobo source.",
            "You should not need to inspect generated Rust for routine errors. K0099 is the bridge diagnostic when Rust remains the final safety check.",
            RustcRemap,
            Error,
            Always(Error),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
    ]
}
