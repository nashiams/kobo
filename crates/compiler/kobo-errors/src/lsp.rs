use serde::Serialize;
use serde_json::Value;

use kobo_ir::{FileSet, KoboSpan};

use crate::{json::DiagnosticJson, DiagnosticRelatedInfo, KDiagnostic, Severity};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DiagnosticLspPayload {
    pub range: LspRange,
    pub severity: u32,
    pub code: Option<String>,
    pub code_description: Option<String>,
    pub source: String,
    pub message: String,
    pub related_information: Vec<LspRelatedInformation>,
    pub data: Value,
    pub uri: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LspRelatedInformation {
    pub uri: Option<String>,
    pub range: LspRange,
    pub message: String,
}

impl DiagnosticLspPayload {
    pub fn from_diagnostic(file_set: &FileSet, diagnostic: &KDiagnostic) -> Self {
        let code = diagnostic.code.as_str().to_owned();
        let json = DiagnosticJson::from_diagnostic(file_set, diagnostic);
        Self {
            range: span_to_lsp_range(file_set, diagnostic.primary.span),
            severity: severity_to_lsp(diagnostic.severity),
            code: Some(code.clone()),
            code_description: Some(format!("kobo explain {code}")),
            source: "kobo".to_owned(),
            message: diagnostic.explanation.0.clone(),
            related_information: diagnostic
                .related
                .iter()
                .map(|related| related_to_lsp(file_set, related))
                .collect(),
            data: serde_json::to_value(json).expect("diagnostic JSON DTO should serialize"),
            uri: uri_for_span(file_set, diagnostic.primary.span),
        }
    }
}

fn related_to_lsp(file_set: &FileSet, related: &DiagnosticRelatedInfo) -> LspRelatedInformation {
    LspRelatedInformation {
        uri: uri_for_span(file_set, related.span),
        range: span_to_lsp_range(file_set, related.span),
        message: related.message.clone(),
    }
}

fn span_to_lsp_range(file_set: &FileSet, span: KoboSpan) -> LspRange {
    LspRange {
        start: position_for_offset(file_set, span.file_id, span.start),
        end: position_for_offset(file_set, span.file_id, span.end),
    }
}

fn position_for_offset(file_set: &FileSet, file_id: kobo_ir::FileId, offset: u32) -> LspPosition {
    let Some(file) = file_set.get(file_id) else {
        return LspPosition {
            line: 0,
            character: offset,
        };
    };
    let (line, column) = file.line_col(offset);
    LspPosition {
        line: line.saturating_sub(1) as u32,
        character: column.saturating_sub(1) as u32,
    }
}

fn uri_for_span(file_set: &FileSet, span: KoboSpan) -> Option<String> {
    file_set
        .get(span.file_id)
        .map(|entry| entry.path().display().to_string())
}

fn severity_to_lsp(severity: Severity) -> u32 {
    match severity {
        Severity::Error => 1,
        Severity::Warning => 2,
        Severity::Note => 3,
    }
}
