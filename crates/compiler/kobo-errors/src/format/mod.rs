mod hints;
mod labels;
mod owner;
mod sections;
mod source;
mod span;
mod strict;
mod wrapping;

#[cfg(test)]
mod strict_tests;
#[cfg(test)]
mod tests;

use kobo_ir::FileSet;

use crate::diagnostic::{DiagnosticRelatedInfo, KDiagnostic};

use hints::push_hint_section;
use sections::{push_inline_section, push_section};
use source::render_label_group;

pub use owner::{render_k0020, render_k0021, DiagOwnerStats};
pub use span::render_span_compact;
pub use strict::{
    render_k0041, render_k0042, render_k0043, render_k0063, render_labeled_cross_boundary,
};

pub fn format_diagnostic(file_set: &FileSet, diagnostic: &KDiagnostic) -> String {
    let mut rendered = String::new();
    rendered.push_str(&format!(
        "{}[{}]: {}",
        diagnostic.severity,
        diagnostic.code,
        diagnostic.code.metadata().short_description
    ));
    rendered.push('\n');
    rendered.push_str(&render_label_group(file_set, diagnostic));
    push_inline_section(&mut rendered, "I found:", card_finding(diagnostic));
    push_inline_section(&mut rendered, "Why I care:", &diagnostic.explanation.0);
    push_section(&mut rendered, "Try this:", &diagnostic.decision.0);
    push_hint_section(&mut rendered, diagnostic);

    rendered
}

fn card_finding(diagnostic: &KDiagnostic) -> &str {
    diagnostic
        .finding
        .as_deref()
        .unwrap_or(&diagnostic.primary.text)
}

pub fn render_related_info(file_set: &FileSet, related: &DiagnosticRelatedInfo) -> String {
    let mut rendered = String::new();
    rendered.push_str("related: ");
    rendered.push_str(&render_span_compact(file_set, related.span));
    if !related.message.is_empty() {
        rendered.push_str(": ");
        rendered.push_str(&related.message);
    }
    rendered
}
