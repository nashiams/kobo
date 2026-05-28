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
            "syntax-error-recovered",
            "syntax error recovered",
            "Kobo recovered from invalid syntax and continued compiling the remaining trustworthy source regions.",
            "Kobo keeps parsing after localized syntax errors so it can report other independent diagnostics. The broken source range is skipped by later compiler phases to avoid cascades.",
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
            "A delimiter was opened but not closed before the parser reached a synchronization boundary.",
            "Kobo resumes at the next safe item or statement boundary so one missing delimiter does not hide unrelated diagnostics.",
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
            "Kobo skipped an invalid item while preserving later parseable items.",
            "The skipped item is not analyzed further. Later phases operate only on parseable source regions so follow-on diagnostics stay trustworthy.",
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
            "Kobo stopped recovery after too many syntax errors to avoid misleading cascades.",
            "Fix the first reported syntax errors and rerun Kobo. The parser intentionally stops after the recovery budget is exhausted.",
            Parser,
            Error,
            Always(Error),
            ParserRecovery,
            ReviewOnly,
            RefuseByDefault,
        ),
    ]
}
