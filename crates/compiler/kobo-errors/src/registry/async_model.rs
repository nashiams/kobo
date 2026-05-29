use crate::{KErrorCode, Severity};

use super::{
    entry, DiagnosticCategory, DiagnosticRegistryEntry, MachineEditPolicy, ModeBehavior,
    SeverityPolicy, SuggestionPolicy,
};
pub(super) fn entries() -> Vec<DiagnosticRegistryEntry> {
    use DiagnosticCategory::Async;
    use MachineEditPolicy::RefuseByDefault;
    use ModeBehavior::AsyncGuaranteeProfile;
    use Severity::Warning;
    use SeverityPolicy::AsyncModeDependent;
    use SuggestionPolicy::ReviewOnly;

    vec![
        entry(
            KErrorCode::K0060,
            "refcell-borrow-live-at-await",
            "borrow remains live across await",
            "A borrow would remain live while async code pauses.",
            "Kobo reports async ownership hazards before lowering code that would fail executor requirements.",
            Async,
            Warning,
            AsyncModeDependent,
            AsyncGuaranteeProfile,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0061,
            "future-send-boundary",
            "this async task cannot safely carry one captured value",
            "An async task may move to another worker, but one captured value must stay on the current task.",
            "The task may run on another worker thread, but one captured value is only safe on the current thread.\n\
Kobo does not hide that by inserting shared mutation for you.\n\
Use LocalSet when the task is intentionally single-thread local.\n\
Use #[kobo::async_shared] when shared async ownership is intentional.\n\
Capture only thread-safe data when the task really should move between worker threads.\n\
If mutation crosses many tasks, prefer message passing or actor ownership.\n\
If this crosses an external runtime boundary, record the boundary tradeoff explicitly.",
            Async,
            Warning,
            AsyncModeDependent,
            AsyncGuaranteeProfile,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0062,
            "mutex-guard-across-await",
            "async executor dependency is missing",
            "Async code was found, but Kobo could not find an executor dependency.",
            "Kobo needs an executor dependency such as tokio or async-std before it can lower async entry points with a concrete runtime.",
            Async,
            Warning,
            AsyncModeDependent,
            AsyncGuaranteeProfile,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0063,
            "strict-block-inside-async",
            "this strict borrow is inside code that can pause",
            "This strict block runs inside an async function.",
            "Kobo needs strict borrows to end before the function can pause or be cancelled.",
            Async,
            Warning,
            AsyncModeDependent,
            AsyncGuaranteeProfile,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0064,
            "strict-guard-live-across-yield",
            "strict guard remains live across await",
            "A guard-like value is live across an await point.",
            "Kobo reports guard liveness because another task can wait on the same guard and never make progress.",
            Async,
            Warning,
            AsyncModeDependent,
            AsyncGuaranteeProfile,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0065,
            "select-branch-cancel-safety",
            "select branch may not be cancel-safe",
            "A select branch contains an operation that may lose progress if cancelled.",
            "Kobo flags non-cancel-safe operations so async control flow stays reviewable before stronger simulation checks arrive.",
            Async,
            Warning,
            AsyncModeDependent,
            AsyncGuaranteeProfile,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0067,
            "handler-state-leaks-across-task",
            "handler request-state leaks across async boundary",
            "A handler-local request value is captured by a longer-lived async task.",
            "Request state should not outlive its request unless it is cloned, modeled, or moved into an explicit actor.",
            Async,
            Warning,
            AsyncModeDependent,
            AsyncGuaranteeProfile,
            ReviewOnly,
            RefuseByDefault,
        ),
    ]
}
