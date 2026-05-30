use crate::{KErrorCode, Severity};

use super::{
    entry, DiagnosticCategory, DiagnosticRegistryEntry, MachineEditPolicy, ModeBehavior,
    SeverityPolicy, SuggestionPolicy,
};
pub(super) fn entries() -> Vec<DiagnosticRegistryEntry> {
    use DiagnosticCategory::Parser;
    use MachineEditPolicy::RefuseByDefault;
    use ModeBehavior::ParserRecovery;
    use Severity::Error;
    use SeverityPolicy::Always;
    use SuggestionPolicy::ReviewOnly;

    vec![
        entry(
            KErrorCode::K0110,
            "syntax-not-readable-yet",
            "I could not read this syntax yet",
            "The source text does not form a complete Kobo item or expression.",
            "I keep reading later source only after skipping the broken range, so one local mistake does not hide unrelated messages.",
            Parser,
            Error,
            Always(Error),
            ParserRecovery,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0111,
            "unclosed-delimiter",
            "unclosed delimiter",
            "A delimiter was opened but not closed before the next safe stopping point.",
            "I resume at the next safe item or statement so one missing delimiter does not hide unrelated messages.",
            Parser,
            Error,
            Always(Error),
            ParserRecovery,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0112,
            "invalid-item-skipped",
            "invalid item skipped",
            "I skipped one invalid item while keeping later readable items.",
            "I do not analyze the skipped item further, so later messages stay tied to source I could read.",
            Parser,
            Error,
            Always(Error),
            ParserRecovery,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0113,
            "parser-recovery-limit-reached",
            "parser recovery limit reached",
            "I stopped after too many syntax errors to avoid misleading follow-up messages.",
            "Fix the first reported syntax mistakes and rerun Kobo once the file is readable again.",
            Parser,
            Error,
            Always(Error),
            ParserRecovery,
            ReviewOnly,
            RefuseByDefault,
        ),
    ]
}
