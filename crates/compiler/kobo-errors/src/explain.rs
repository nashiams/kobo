use crate::{diagnostic_registry, DiagnosticRegistryEntry};

pub fn explain_code(code_text: &str) -> Option<String> {
    let registry = diagnostic_registry();
    registry
        .find_by_code_text(code_text)
        .map(render_explain_entry)
}

pub fn unknown_code_message(code_text: &str) -> String {
    let registry = diagnostic_registry();
    match registry.nearest_code_text(code_text) {
        Some(nearest) => {
            format!("unknown diagnostic code `{code_text}`\nnearest registered code: {nearest}")
        }
        None => format!("unknown diagnostic code `{code_text}`"),
    }
}

fn render_explain_entry(entry: &DiagnosticRegistryEntry) -> String {
    format!(
        "{} - {}\nslug: {}\ncategory: {}\nstatus: {}\nseverity: {}\nmode policy: {}\nsuggestion policy: {}\nmachine edits: {}\n\n{}\n",
        entry.code_text,
        entry.title,
        entry.slug,
        entry.category.as_str(),
        entry.status.as_str(),
        entry.default_severity.as_str(),
        entry.mode_behavior.as_str(),
        entry.suggestion_policy.as_str(),
        entry.machine_edit_policy.as_str(),
        entry.explain,
    )
}
