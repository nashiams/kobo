use crate::{diagnostic_registry, DiagnosticRegistryEntry};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ExplainDetail {
    Human,
    Verbose,
}

pub fn explain_code(code_text: &str) -> Option<String> {
    explain_code_with_detail(code_text, ExplainDetail::Human)
}

pub fn explain_code_with_detail(code_text: &str, detail: ExplainDetail) -> Option<String> {
    let registry = diagnostic_registry();
    registry
        .find_by_code_text(code_text)
        .map(|entry| match detail {
            ExplainDetail::Human => render_human_explain_entry(entry),
            ExplainDetail::Verbose => render_verbose_explain_entry(entry),
        })
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

fn render_human_explain_entry(entry: &DiagnosticRegistryEntry) -> String {
    let mut rendered = format!("{}: {}\n", entry.code_text, entry.title);

    push_section(&mut rendered, "What happened", entry.summary);
    push_section(&mut rendered, "Why this matters", entry.explain);
    push_section(&mut rendered, "How to fix", fix_guidance(entry));
    push_section(
        &mut rendered,
        "More detail",
        &format!(
            "Run `kobo explain {} --verbose` for policy and machine-edit metadata.",
            entry.code_text
        ),
    );

    rendered
}

fn render_verbose_explain_entry(entry: &DiagnosticRegistryEntry) -> String {
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

fn push_section(rendered: &mut String, heading: &str, body: &str) {
    let body = body.trim();
    if body.is_empty() {
        return;
    }

    rendered.push('\n');
    rendered.push_str(heading);
    rendered.push('\n');
    rendered.push_str(body);
    rendered.push('\n');
}

fn fix_guidance(entry: &DiagnosticRegistryEntry) -> &'static str {
    match entry.code {
        crate::KErrorCode::K0001 => {
            "Use the value before it moves, clone it deliberately, or pass a borrow when ownership should stay with the caller."
        }
        crate::KErrorCode::K0002 => {
            "Shorten the earlier borrow, split the mutation into a later scope, or choose an ownership shape that makes the sharing explicit."
        }
        crate::KErrorCode::K0025 => {
            "Change the hint to match the actual use, or change the later code so the requested ownership shape is valid."
        }
        crate::KErrorCode::K0041 => {
            "End or drop the aliases before entering the @strict block. If the alias must stay live, keep that code outside the strict boundary."
        }
        crate::KErrorCode::K0042 => {
            "Move the closure outside the @strict block, pass only plain data into it, or rewrite the closure so it does not capture the guarded value."
        }
        crate::KErrorCode::K0062 => {
            "Add tokio or async-std to [dependencies], or keep this code synchronous until an executor is configured."
        }
        crate::KErrorCode::K0063 => {
            "Use @strict async fn when the function follows the async strict protocol, or move the strict block into a synchronous helper."
        }
        crate::KErrorCode::K0080 => {
            "Review the ownership design and choose an explicit structure before asking Kobo to migrate this path automatically."
        }
        crate::KErrorCode::K0100 => {
            "Call one of the required actions, or record explicit debt when the obligation is resolved outside the modeled scenario."
        }
        crate::KErrorCode::K0102 => {
            "Route the effect through a deterministic facade, record the effect stream, or mark the replay debt explicitly."
        }
        crate::KErrorCode::K0107 => {
            "Choose a boundary policy such as model, record, outside, opaque, or debt before claiming exact replay for this path."
        }
        crate::KErrorCode::K0108 => {
            "Keep the suppression reason specific and reviewable, then revisit it when the boundary or obligation becomes modeled."
        }
        _ => match entry.suggestion_policy {
            crate::SuggestionPolicy::MachineApplicableAllowed => {
                "Apply a machine-applicable suggestion only after checking that it preserves the source intent."
            }
            crate::SuggestionPolicy::BoundaryPolicy => {
                "Choose an explicit boundary policy so the guarantee remains reviewable."
            }
            crate::SuggestionPolicy::HelpOnly => {
                "Follow the help text from the diagnostic card and rerun Kobo."
            }
            crate::SuggestionPolicy::ReviewOnly
            | crate::SuggestionPolicy::None
            | crate::SuggestionPolicy::Unspecified => {
                "Review the highlighted source and make the ownership or boundary choice explicit."
            }
        },
    }
}
