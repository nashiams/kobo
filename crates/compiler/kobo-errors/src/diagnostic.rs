use std::fmt;

use kobo_ir::{KoboSpan, OwnershipDebtRecord};

use crate::codes::{KErrorCode, Severity};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DiagLabelKind {
    Primary,
    Secondary,
    Ownership,
    Addition,
    Removal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagLabel {
    pub span: KoboSpan,
    pub kind: DiagLabelKind,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagExplanation(pub String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagDecision(pub String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagHelp(pub String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagHint(pub String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliSuggestion(pub String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticNote {
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticRelatedInfo {
    pub span: KoboSpan,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextEdit {
    pub span: KoboSpan,
    pub replacement: String,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SuggestionApplicability {
    MachineApplicable,
    MaybeIncorrect,
    HasPlaceholders,
    Unspecified,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticSuggestion {
    pub message: String,
    pub applicability: SuggestionApplicability,
    pub edits: Vec<TextEdit>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticUpstream {
    pub tool: String,
    pub code: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticSuppression {
    pub span: KoboSpan,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KDiagnostic {
    pub code: KErrorCode,
    pub severity: Severity,
    pub primary: DiagLabel,
    pub secondary: Vec<DiagLabel>,
    pub finding: Option<String>,
    pub explanation: DiagExplanation,
    pub decision: DiagDecision,
    pub help: Option<DiagHelp>,
    pub hint: Option<DiagHint>,
    pub run: Option<CliSuggestion>,
    pub notes: Vec<DiagnosticNote>,
    pub related: Vec<DiagnosticRelatedInfo>,
    pub suggestions: Vec<DiagnosticSuggestion>,
    pub suppressed_by: Option<KoboSpan>,
    pub upstream: Option<DiagnosticUpstream>,
    pub suppression: Option<DiagnosticSuppression>,
    pub ownership_debt: Option<OwnershipDebtRecord>,
}

impl DiagLabel {
    pub fn new(span: KoboSpan, kind: DiagLabelKind, text: impl Into<String>) -> Self {
        Self {
            span,
            kind,
            text: text.into(),
        }
    }

    pub fn primary(span: KoboSpan, text: impl Into<String>) -> Self {
        Self::new(span, DiagLabelKind::Primary, text)
    }

    pub fn secondary(span: KoboSpan, text: impl Into<String>) -> Self {
        Self::new(span, DiagLabelKind::Secondary, text)
    }
}

impl KDiagnostic {
    pub fn new(
        code: KErrorCode,
        severity: Severity,
        primary: DiagLabel,
        explanation: impl Into<DiagExplanation>,
        decision: impl Into<DiagDecision>,
    ) -> Self {
        debug_assert_eq!(primary.kind, DiagLabelKind::Primary);

        Self {
            code,
            severity,
            primary,
            secondary: Vec::new(),
            finding: None,
            explanation: explanation.into(),
            decision: decision.into(),
            help: None,
            hint: None,
            run: None,
            notes: Vec::new(),
            related: Vec::new(),
            suggestions: Vec::new(),
            suppressed_by: None,
            upstream: None,
            suppression: None,
            ownership_debt: None,
        }
    }

    pub fn with_secondary_label(mut self, label: DiagLabel) -> Self {
        self.secondary.push(label);
        self
    }

    pub fn with_finding(mut self, finding: impl Into<String>) -> Self {
        self.finding = Some(finding.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<DiagHelp>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn with_hint(mut self, hint: impl Into<DiagHint>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_run(mut self, run: impl Into<CliSuggestion>) -> Self {
        self.run = Some(run.into());
        self
    }

    pub fn with_note(mut self, note: DiagnosticNote) -> Self {
        self.notes.push(note);
        self
    }

    pub fn with_related_info(mut self, related: DiagnosticRelatedInfo) -> Self {
        self.related.push(related);
        self
    }

    pub fn with_suggestion(mut self, suggestion: DiagnosticSuggestion) -> Self {
        self.suggestions.push(suggestion);
        self
    }

    pub fn with_suppressed_by(mut self, span: KoboSpan) -> Self {
        self.suppressed_by = Some(span);
        self
    }

    pub fn with_upstream(mut self, upstream: DiagnosticUpstream) -> Self {
        self.upstream = Some(upstream);
        self
    }

    pub fn with_suppression(mut self, suppression: DiagnosticSuppression) -> Self {
        self.suppressed_by = Some(suppression.span);
        self.suppression = Some(suppression);
        self
    }

    pub fn with_ownership_debt(mut self, record: OwnershipDebtRecord) -> Self {
        self.ownership_debt = Some(record);
        self
    }

    pub fn with_poisoned_related_info_trimmed(mut self, poisoned_spans: &[KoboSpan]) -> Self {
        let is_poisoned = |span: KoboSpan| {
            poisoned_spans
                .iter()
                .any(|poison| spans_overlap(*poison, span))
        };
        self.secondary.retain(|label| !is_poisoned(label.span));
        self.related.retain(|related| !is_poisoned(related.span));
        self.suggestions.retain_mut(|suggestion| {
            suggestion.edits.retain(|edit| !is_poisoned(edit.span));
            !suggestion.edits.is_empty()
        });
        self
    }
}

impl DiagnosticNote {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

impl DiagnosticRelatedInfo {
    pub fn new(span: KoboSpan, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }
}

impl TextEdit {
    pub fn replace(span: KoboSpan, replacement: impl Into<String>) -> Self {
        Self {
            span,
            replacement: replacement.into(),
        }
    }
}

impl DiagnosticSuggestion {
    pub fn new(
        message: impl Into<String>,
        applicability: SuggestionApplicability,
        edits: Vec<TextEdit>,
    ) -> Self {
        Self {
            message: message.into(),
            applicability,
            edits,
        }
    }
}

impl DiagnosticUpstream {
    pub fn new(tool: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            code: code.into(),
        }
    }
}

impl DiagnosticSuppression {
    pub fn new(span: KoboSpan, reason: impl Into<String>) -> Self {
        Self {
            span,
            reason: reason.into(),
        }
    }
}

impl SuggestionApplicability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MachineApplicable => "machine-applicable",
            Self::MaybeIncorrect => "maybe-incorrect",
            Self::HasPlaceholders => "has-placeholders",
            Self::Unspecified => "unspecified",
        }
    }
}

fn spans_overlap(left: KoboSpan, right: KoboSpan) -> bool {
    left.file_id == right.file_id && left.start < right.end && right.start < left.end
}

impl DiagExplanation {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl DiagDecision {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl DiagHelp {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl CliSuggestion {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<&str> for DiagExplanation {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for DiagExplanation {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for DiagDecision {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for DiagDecision {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for DiagHelp {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for DiagHelp {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for DiagHint {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for DiagHint {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for CliSuggestion {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for CliSuggestion {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Display for DiagExplanation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for DiagDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for DiagHelp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for DiagHint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for CliSuggestion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use kobo_ir::{FileId, KoboSpan};

    use crate::codes::{KErrorCode, Severity};

    use super::{DiagLabel, DiagLabelKind, KDiagnostic};

    #[test]
    fn diagnostic_builder_uses_structured_primary_label() {
        let diagnostic = KDiagnostic::new(
            KErrorCode::K0001,
            Severity::Error,
            DiagLabel::primary(KoboSpan::new(3, 8, FileId(0)), "value used here"),
            "binding was moved earlier",
            "no automatic rewrite applied",
        );

        assert_eq!(diagnostic.code, KErrorCode::K0001);
        assert_eq!(diagnostic.primary.kind, DiagLabelKind::Primary);
    }
}
