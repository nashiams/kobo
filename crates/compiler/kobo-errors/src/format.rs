use crate::diagnostic::{CliSuggestion, DiagHelp, KDiagnostic};

/// Plain-text diagnostic formatter for the v0.1 pipeline.
pub fn format_diagnostic(diagnostic: &KDiagnostic) -> String {
    debug_assert_eq!(diagnostic.code, diagnostic.message.code());

    let mut rendered = format_header(diagnostic);
    rendered.push('\n');
    rendered.push_str(&format_span(diagnostic));
    rendered.push('\n');
    rendered.push_str("= kobo decision: ");
    rendered.push_str(&diagnostic.decision.0);

    if let Some(help) = diagnostic.help.as_ref() {
        push_help(&mut rendered, help);
    }

    if let Some(run) = diagnostic.run.as_ref() {
        push_run(&mut rendered, run);
    }

    rendered
}

fn format_header(diagnostic: &KDiagnostic) -> String {
    format!(
        "{}[{}]: {}",
        diagnostic.severity, diagnostic.code, diagnostic.message
    )
}

fn format_span(diagnostic: &KDiagnostic) -> String {
    format!(
        "  at file#{}:{}..{}",
        diagnostic.primary_span.file_id.0,
        diagnostic.primary_span.start,
        diagnostic.primary_span.end,
    )
}

fn push_help(rendered: &mut String, help: &DiagHelp) {
    if help.is_empty() {
        return;
    }

    rendered.push('\n');
    rendered.push_str("help: ");
    rendered.push_str(&help.0);
}

fn push_run(rendered: &mut String, run: &CliSuggestion) {
    if run.is_empty() {
        return;
    }

    rendered.push('\n');
    rendered.push_str("run: ");
    rendered.push_str(&run.0);
}

#[cfg(test)]
mod tests {
    use kobo_ir::{FileId, KoboSpan};

    use crate::codes::Severity;
    use crate::diagnostic::{DiagMessage, KDiagnostic};

    use super::format_diagnostic;

    #[test]
    fn formatter_renders_header_and_optional_lines() {
        let diagnostic = KDiagnostic::new(
            DiagMessage::K0001,
            Severity::Error,
            KoboSpan::new(4, 9, FileId(2)),
            "inserted clone at move site",
        )
        .with_help("clone explicitly at the call site")
        .with_run("kobo check src/main.kobo:12");

        let rendered = format_diagnostic(&diagnostic);

        assert!(rendered.contains("error[K0001]: value used after move"));
        assert!(rendered.contains("at file#2:4..9"));
        assert!(rendered.contains("help: clone explicitly at the call site"));
        assert!(rendered.contains("run: kobo check src/main.kobo:12"));
    }
}
