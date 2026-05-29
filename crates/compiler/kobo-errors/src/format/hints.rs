use crate::diagnostic::KDiagnostic;
use crate::DiagnosticCategory;
use kobo_ir::OwnershipDebtKind;

use super::sections::push_inline_section;

pub(crate) fn push_hint_section(rendered: &mut String, diagnostic: &KDiagnostic) {
    push_inline_section(rendered, "Hint:", &diagnostic_hint_line(diagnostic));
}

fn diagnostic_hint_line(diagnostic: &KDiagnostic) -> String {
    let mut hint = diagnostic
        .hint
        .as_ref()
        .map(|hint| hint.0.trim().to_owned())
        .filter(|hint| !hint.is_empty())
        .or_else(|| {
            diagnostic
                .help
                .as_ref()
                .map(|help| help.0.trim().to_owned())
                .filter(|help| !help.is_empty())
        })
        .unwrap_or_default();

    if !hint.is_empty() {
        if !hint.ends_with('.') {
            hint.push('.');
        }
        hint.push(' ');
    }

    hint.push_str(&format!("Run `kobo explain {}`.", diagnostic.code));

    if let Some(command) = doctor_command_for_diagnostic(diagnostic) {
        hint.push(' ');
        hint.push_str(&format!(
            "Run `{command}` to check project and dependency shape."
        ));
    }

    if let Some(run) = diagnostic.run.as_ref().filter(|run| !run.is_empty()) {
        hint.push(' ');
        hint.push_str("Rerun with `");
        hint.push_str(&run.0);
        hint.push_str("`.");
    }

    hint
}

fn doctor_command_for_diagnostic(diagnostic: &KDiagnostic) -> Option<&'static str> {
    if diagnostic
        .ownership_debt
        .as_ref()
        .is_some_and(|record| record.kind == OwnershipDebtKind::RustcEscape)
    {
        return None;
    }

    let registry = crate::diagnostic_registry();
    let entry = registry.get(diagnostic.code)?;
    match entry.category {
        DiagnosticCategory::BoundaryPolicy
        | DiagnosticCategory::MigrationBoundary
        | DiagnosticCategory::RustcRemap => Some("kobo doctor --deps"),
        _ => None,
    }
}
