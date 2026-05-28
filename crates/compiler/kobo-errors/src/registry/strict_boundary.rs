use crate::{KErrorCode, Severity};

use super::{
    entry, DiagnosticCategory, DiagnosticRegistryEntry, MachineEditPolicy, ModeBehavior,
    SeverityPolicy, SuggestionPolicy,
};
pub(super) fn entries() -> Vec<DiagnosticRegistryEntry> {
    use DiagnosticCategory::StrictBoundary;
    use MachineEditPolicy::RefuseByDefault;
    use ModeBehavior::NoModeDependency;
    use Severity::Error;
    use SeverityPolicy::Always;
    use SuggestionPolicy::ReviewOnly;

    vec![
        entry(
            KErrorCode::K0025,
            "ownership-hint-cannot-be-used",
            "Kobo cannot use this ownership hint",
            "A hint asks Kobo for an ownership shape this value's actual use cannot support.",
            "A moved value has only one owner. Code that needs repeated mutable use must use a shape that supports that sharing.",
            StrictBoundary,
            Error,
            Always(Error),
            NoModeDependency,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0030,
            "resource-handle-moved",
            "resource handle moved - cannot alias file handle",
            "A resource handle cannot be safely aliased through ownership wrapping.",
            "Kobo keeps resource ownership explicit because duplicating or aliasing OS-backed handles can change behavior.",
            StrictBoundary,
            Error,
            Always(Error),
            NoModeDependency,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0041,
            "aliased-strict-entry",
            "cannot enter @strict block - value has active aliases",
            "An @strict block would start while aliases are still active.",
            "Strict regions promise a simple ownership shape at the boundary. Any live alias must end before entry so the generated guards can be introduced and removed in one clear scope.",
            StrictBoundary,
            Error,
            Always(Error),
            NoModeDependency,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0042,
            "closure-crosses-strict-boundary",
            "closure captures value across @strict boundary",
            "A closure captures a value whose strict-boundary guard must stay local to the block.",
            "The closure could run after the strict boundary has finished. Kobo rejects that capture so guard lifetimes remain visible in source.",
            StrictBoundary,
            Error,
            Always(Error),
            NoModeDependency,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0043,
            "moved-inside-strict-block",
            "value moved inside @strict block - cannot restore on exit",
            "A value moved inside a strict block cannot be restored to its wrapper on exit.",
            "Kobo keeps strict boundary exit behavior explicit so generated ownership state remains coherent.",
            StrictBoundary,
            Error,
            Always(Error),
            NoModeDependency,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0044,
            "labeled-jump-crosses-strict-boundary",
            "labeled break or continue crosses @strict boundary",
            "A labeled control-flow jump would exit an @strict region unsafely.",
            "Kobo rejects labeled jumps that bypass strict-region guard teardown.",
            StrictBoundary,
            Error,
            Always(Error),
            NoModeDependency,
            ReviewOnly,
            RefuseByDefault,
        ),
    ]
}
