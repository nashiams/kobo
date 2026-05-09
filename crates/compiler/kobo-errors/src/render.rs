use crate::{format, KDiagnostic};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DiagnosticOutputFormat {
    HumanCard,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticRenderer {
    pub color_mode: ColorMode,
    pub output_format: DiagnosticOutputFormat,
}

impl DiagnosticRenderer {
    pub fn new(color_mode: ColorMode, output_format: DiagnosticOutputFormat) -> Self {
        Self {
            color_mode,
            output_format,
        }
    }

    pub fn render(&self, file_set: &kobo_ir::FileSet, diagnostic: &KDiagnostic) -> String {
        let rendered = match self.output_format {
            DiagnosticOutputFormat::HumanCard => render_human_card(file_set, diagnostic),
        };
        self.apply_color_mode(rendered)
    }

    fn apply_color_mode(&self, rendered: String) -> String {
        match self.color_mode {
            ColorMode::Never => strip_ansi(&rendered),
            ColorMode::Auto | ColorMode::Always => rendered,
        }
    }
}

pub fn render_diagnostic_card(
    file_set: &kobo_ir::FileSet,
    diagnostic: &KDiagnostic,
    color: ColorMode,
) -> String {
    DiagnosticRenderer::new(color, DiagnosticOutputFormat::HumanCard).render(file_set, diagnostic)
}

pub fn render_diagnostic_header_only(diagnostic: &KDiagnostic) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{}[{}]: {}",
        diagnostic.severity.as_str(),
        diagnostic.code.as_str(),
        diagnostic.code.metadata().short_description
    ));
    out.push('\n');
    out.push_str("  ");
    out.push_str(&diagnostic.primary.text);
    out.push('\n');
    if !diagnostic.explanation.is_empty() {
        out.push_str("  why: ");
        out.push_str(&diagnostic.explanation.0);
        out.push('\n');
    }
    if !diagnostic.decision.is_empty() {
        out.push_str("  fix: ");
        out.push_str(&diagnostic.decision.0);
        out.push('\n');
    }
    out
}

fn render_human_card(file_set: &kobo_ir::FileSet, diagnostic: &KDiagnostic) -> String {
    let mut out = format::format_diagnostic(file_set, diagnostic);

    for related in &diagnostic.related {
        out.push('\n');
        out.push_str(&format::render_related_info(file_set, related));
    }

    for note in &diagnostic.notes {
        out.push('\n');
        out.push_str("note: ");
        out.push_str(&note.text);
    }

    for suggestion in &diagnostic.suggestions {
        out.push('\n');
        out.push_str("help: ");
        out.push_str(&suggestion.message);
        out.push_str(" [");
        out.push_str(suggestion.applicability.as_str());
        out.push(']');
        for edit in &suggestion.edits {
            out.push('\n');
            out.push_str("  replace ");
            out.push_str(&format::render_span_compact(file_set, edit.span));
            out.push_str(" with `");
            out.push_str(&edit.replacement);
            out.push('`');
        }
    }

    out
}

fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}
