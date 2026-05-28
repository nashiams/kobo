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
            "RefCell accessed >10,000 times in hot path",
            "A diagnostic owner recorded many dynamic borrow checks in a hot path.",
            "Kobo records runtime borrow counters so migration work can prioritize expensive shared-mutable paths.",
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
            "DiagOwner borrow counter saturated - count understated",
            "A diagnostic owner counter reached its maximum value.",
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
            "engine-owned binding capped to PlainOwned",
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
            "live borrow at move forces shared ownership",
            "A borrow remains live when a value moves.",
            "Kobo uses KIR liveness to keep this move from invalidating a later borrow use.",
            Performance,
            Warning,
            Always(Warning),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
    ]
}
