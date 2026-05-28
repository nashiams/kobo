use crate::diagnostic::KDiagnostic;

use super::wrapping::{push_wrapped_multiline_text, push_wrapped_text};
pub(crate) fn push_section(rendered: &mut String, heading: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }

    rendered.push('\n');
    rendered.push_str(heading);
    rendered.push('\n');
    push_wrapped_multiline_text(rendered, value);
}

pub(crate) fn push_more_section(rendered: &mut String, diagnostic: &KDiagnostic) {
    rendered.push('\n');
    rendered.push_str("More:");
    rendered.push('\n');
    push_wrapped_text(
        rendered,
        "  ",
        "  ",
        &format!(
            "Run `kobo explain {}` for examples and deeper context.",
            diagnostic.code
        ),
    );

    if let Some(help) = diagnostic.help.as_ref().filter(|help| !help.is_empty()) {
        rendered.push('\n');
        push_wrapped_text(rendered, "  Also: ", "  ", &help.0);
    }

    if let Some(run) = diagnostic.run.as_ref().filter(|run| !run.is_empty()) {
        rendered.push('\n');
        push_wrapped_text(rendered, "  Rerun: ", "  ", &run.0);
    }
}
