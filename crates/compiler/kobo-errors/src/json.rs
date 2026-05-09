use serde::Serialize;
use serde_json::Value;

use kobo_ir::{FileSet, KoboSpan};

use crate::{DiagLabel, DiagnosticRelatedInfo, KDiagnostic, TextEdit};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiagnosticJson {
    pub schema_version: u32,
    pub code: String,
    pub slug: String,
    pub title: String,
    pub category: String,
    pub severity: String,
    pub message: String,
    pub primary: DiagnosticJsonSpan,
    pub secondary: Vec<DiagnosticJsonSpan>,
    pub related: Vec<DiagnosticJsonRelated>,
    pub explanation: String,
    pub decision: String,
    pub notes: Vec<String>,
    pub suggestions: Vec<DiagnosticJsonSuggestion>,
    pub suppressed_by: Option<DiagnosticJsonSpan>,
    pub suppression: Option<DiagnosticJsonSuppression>,
    pub upstream: Option<DiagnosticJsonUpstream>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiagnosticJsonSpan {
    pub file_id: u32,
    pub file_path: Option<String>,
    pub byte_start: u32,
    pub byte_end: u32,
    pub line_start: u32,
    pub column_start: u32,
    pub line_end: u32,
    pub column_end: u32,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiagnosticJsonRelated {
    pub span: DiagnosticJsonSpan,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiagnosticJsonSuggestion {
    pub message: String,
    pub applicability: String,
    pub edits: Vec<DiagnosticJsonTextEdit>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiagnosticJsonTextEdit {
    pub file_id: u32,
    pub file_path: Option<String>,
    pub byte_start: u32,
    pub byte_end: u32,
    pub line_start: u32,
    pub column_start: u32,
    pub line_end: u32,
    pub column_end: u32,
    pub replacement: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiagnosticJsonSuppression {
    pub span: DiagnosticJsonSpan,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiagnosticJsonUpstream {
    pub tool: String,
    pub code: String,
}

impl DiagnosticJson {
    pub fn from_diagnostic(file_set: &FileSet, diagnostic: &KDiagnostic) -> Self {
        let metadata = registry_metadata(diagnostic);
        Self {
            schema_version: 1,
            code: diagnostic.code.as_str().to_owned(),
            slug: metadata.slug,
            title: metadata.title,
            category: metadata.category,
            severity: diagnostic.severity.as_str().to_owned(),
            message: diagnostic.primary.text.clone(),
            primary: label_to_json_span(file_set, &diagnostic.primary),
            secondary: diagnostic
                .secondary
                .iter()
                .map(|label| label_to_json_span(file_set, label))
                .collect(),
            related: diagnostic
                .related
                .iter()
                .map(|related| related_to_json(file_set, related))
                .collect(),
            explanation: diagnostic.explanation.0.clone(),
            decision: diagnostic.decision.0.clone(),
            notes: diagnostic
                .notes
                .iter()
                .map(|note| note.text.clone())
                .collect(),
            suggestions: diagnostic
                .suggestions
                .iter()
                .map(|suggestion| DiagnosticJsonSuggestion {
                    message: suggestion.message.clone(),
                    applicability: suggestion.applicability.as_str().to_owned(),
                    edits: suggestion
                        .edits
                        .iter()
                        .map(|edit| edit_to_json(file_set, edit))
                        .collect(),
                })
                .collect(),
            suppressed_by: diagnostic
                .suppressed_by
                .map(|span| span_to_json_span(file_set, span, None)),
            suppression: diagnostic.suppression.as_ref().map(|suppression| {
                DiagnosticJsonSuppression {
                    span: span_to_json_span(file_set, suppression.span, None),
                    reason: suppression.reason.clone(),
                }
            }),
            upstream: diagnostic
                .upstream
                .as_ref()
                .map(|upstream| DiagnosticJsonUpstream {
                    tool: upstream.tool.clone(),
                    code: upstream.code.clone(),
                }),
        }
    }
}

pub fn diagnostic_to_json_value(file_set: &FileSet, diagnostic: &KDiagnostic) -> Value {
    serde_json::to_value(DiagnosticJson::from_diagnostic(file_set, diagnostic))
        .expect("diagnostic JSON DTO should serialize")
}

pub(crate) fn span_to_json_span(
    file_set: &FileSet,
    span: KoboSpan,
    label: Option<String>,
) -> DiagnosticJsonSpan {
    let file = file_set.get(span.file_id);
    let (line_start, column_start) = zero_based_line_col(file_set, span.file_id, span.start);
    let (line_end, column_end) = zero_based_line_col(file_set, span.file_id, span.end);

    DiagnosticJsonSpan {
        file_id: span.file_id.0,
        file_path: file.map(|entry| entry.path().display().to_string()),
        byte_start: span.start,
        byte_end: span.end,
        line_start,
        column_start,
        line_end,
        column_end,
        label,
    }
}

pub(crate) fn edit_to_json(file_set: &FileSet, edit: &TextEdit) -> DiagnosticJsonTextEdit {
    let span = span_to_json_span(file_set, edit.span, None);
    DiagnosticJsonTextEdit {
        file_id: span.file_id,
        file_path: span.file_path,
        byte_start: span.byte_start,
        byte_end: span.byte_end,
        line_start: span.line_start,
        column_start: span.column_start,
        line_end: span.line_end,
        column_end: span.column_end,
        replacement: edit.replacement.clone(),
    }
}

fn label_to_json_span(file_set: &FileSet, label: &DiagLabel) -> DiagnosticJsonSpan {
    span_to_json_span(file_set, label.span, Some(label.text.clone()))
}

fn related_to_json(file_set: &FileSet, related: &DiagnosticRelatedInfo) -> DiagnosticJsonRelated {
    DiagnosticJsonRelated {
        span: span_to_json_span(file_set, related.span, Some(related.message.clone())),
        message: related.message.clone(),
    }
}

struct DiagnosticJsonMetadata {
    slug: String,
    title: String,
    category: String,
}

fn registry_metadata(diagnostic: &KDiagnostic) -> DiagnosticJsonMetadata {
    let registry = crate::diagnostic_registry();
    if let Some(entry) = registry.get(diagnostic.code) {
        return DiagnosticJsonMetadata {
            slug: entry.slug.to_owned(),
            title: entry.title.to_owned(),
            category: entry.category.as_str().to_owned(),
        };
    }

    DiagnosticJsonMetadata {
        slug: diagnostic.code.as_str().to_ascii_lowercase(),
        title: diagnostic.code.metadata().short_description.to_owned(),
        category: "unknown".to_owned(),
    }
}

fn zero_based_line_col(file_set: &FileSet, file_id: kobo_ir::FileId, offset: u32) -> (u32, u32) {
    let Some(file) = file_set.get(file_id) else {
        return (0, offset);
    };
    let (line, column) = file.line_col(offset);
    (
        line.saturating_sub(1) as u32,
        column.saturating_sub(1) as u32,
    )
}
