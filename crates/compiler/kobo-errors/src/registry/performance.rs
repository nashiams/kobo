use crate::{KErrorCode, Severity};

use super::{
    entry, DiagnosticCategory, DiagnosticRegistryEntry, MachineEditPolicy, ModeBehavior,
    SeverityPolicy, SuggestionPolicy,
};
pub(super) fn entries() -> Vec<DiagnosticRegistryEntry> {
    use DiagnosticCategory::Performance;
    use MachineEditPolicy::NotApplicable;
    use ModeBehavior::NoModeDependency;
    use Severity::Warning;
    use SeverityPolicy::Always;
    use SuggestionPolicy::HelpOnly;

    vec![
        entry(
            KErrorCode::K0020,
            "hot-refcell-borrow-counter",
            "shared borrow checked many times in a hot path",
            "Runtime evidence recorded many shared-borrow checks in a hot path.",
            "Kobo records borrow counters so migration work can prioritize expensive shared-mutable paths.",
            Performance,
            Warning,
            Always(Warning),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0021,
            "borrow-counter-saturated",
            "borrow counter reached its limit",
            "A runtime borrow counter reached its maximum value.",
            "The program continued, but the reported count is a lower bound because the counter saturated.",
            Performance,
            Warning,
            Always(Warning),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0026,
            "relax-attribute-ineffective",
            "relax attribute has no effect or is malformed",
            "A relax attribute is ineffective or invalid for the current context.",
            "Kobo reports relax misuse so mode boundaries stay explicit and reviewable.",
            Performance,
            Warning,
            Always(Warning),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0031,
            "engine-owned-capped",
            "engine-owned value kept directly owned",
            "A framework-managed engine binding was prevented from escalating to shared ownership.",
            "Engine-managed state must stay owned by the framework unless the source explicitly opts into a supported escape path.",
            Performance,
            Warning,
            Always(Warning),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0032,
            "live-borrow-at-move",
            "move while a borrow is live needs shared ownership",
            "A borrow remains live when a value moves.",
            "Kobo keeps this move from invalidating a later borrow use by reporting the required ownership rewrite.",
            Performance,
            Warning,
            Always(Warning),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
    ]
}
