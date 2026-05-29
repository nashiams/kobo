use crate::{KErrorCode, Severity};

use super::{
    entry, DiagnosticCategory, DiagnosticRegistryEntry, MachineEditPolicy, ModeBehavior,
    SeverityPolicy, SuggestionPolicy,
};
pub(super) fn entries() -> Vec<DiagnosticRegistryEntry> {
    use DiagnosticCategory::Ownership;
    use MachineEditPolicy::AllowedWhenSuggestionMachineApplicable;
    use ModeBehavior::OwnershipGuaranteeProfile;
    use Severity::Warning;
    use SeverityPolicy::OwnershipModeDependent;
    use SuggestionPolicy::MachineApplicableAllowed;

    vec![
        entry(
            KErrorCode::K0001,
            "value-used-after-move",
            "value used after move",
            "A value is used after ownership has moved away from it.",
            "The later use would need the original value, but an earlier operation already took ownership of it. Kobo reports this before lowering so the source-level move is visible.",
            Ownership,
            Warning,
            OwnershipModeDependent,
            OwnershipGuaranteeProfile,
            MachineApplicableAllowed,
            AllowedWhenSuggestionMachineApplicable,
        ),
        entry(
            KErrorCode::K0002,
            "mutable-borrow-conflict",
            "mutable borrow overlaps another borrow",
            "A mutable borrow conflicts with an existing borrow.",
            "The mutable access overlaps another live borrow. Kobo reports the overlap so the source can shorten one borrow or make shared mutation explicit.",
            Ownership,
            Warning,
            OwnershipModeDependent,
            OwnershipGuaranteeProfile,
            MachineApplicableAllowed,
            AllowedWhenSuggestionMachineApplicable,
        ),
        entry(
            KErrorCode::K0019,
            "ownership-diagnostic",
            "ownership diagnostic",
            "An ownership condition needs a stronger mode-dependent guarantee.",
            "K0019 is the compatibility bucket for ownership diagnostics that have not yet been assigned a narrower code.",
            Ownership,
            Warning,
            OwnershipModeDependent,
            OwnershipGuaranteeProfile,
            SuggestionPolicy::ReviewOnly,
            MachineEditPolicy::RefuseByDefault,
        ),
    ]
}
