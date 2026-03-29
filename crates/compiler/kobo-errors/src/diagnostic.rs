use std::fmt;

use kobo_ir::KoboSpan;

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
pub struct CliSuggestion(pub String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KDiagnostic {
    pub code: KErrorCode,
    pub severity: Severity,
    pub primary: DiagLabel,
    pub secondary: Vec<DiagLabel>,
    pub explanation: DiagExplanation,
    pub decision: DiagDecision,
    pub help: Option<DiagHelp>,
    pub run: Option<CliSuggestion>,
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
            explanation: explanation.into(),
            decision: decision.into(),
            help: None,
            run: None,
        }
    }

    pub fn with_secondary_label(mut self, label: DiagLabel) -> Self {
        self.secondary.push(label);
        self
    }

    pub fn with_help(mut self, help: impl Into<DiagHelp>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn with_run(mut self, run: impl Into<CliSuggestion>) -> Self {
        self.run = Some(run.into());
        self
    }
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
