use kobo_errors::{DiagnosticLspPayload, KDiagnostic, KErrorCode};
use kobo_ir::FileSet;
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LspCodeAction {
    pub title: String,
    pub group: String,
    pub command: Option<String>,
}

pub fn diagnostic_payload(file_set: &FileSet, diagnostic: &KDiagnostic) -> DiagnosticLspPayload {
    DiagnosticLspPayload::from_diagnostic(file_set, diagnostic)
}

pub fn diagnostic_value(
    file_set: &FileSet,
    diagnostic: &KDiagnostic,
    include_actions: bool,
) -> serde_json::Result<Value> {
    let mut value = serde_json::to_value(diagnostic_payload(file_set, diagnostic))?;
    if include_actions {
        value["codeAction"] = json!(code_action_commands(diagnostic.code));
        value["codeActions"] = serde_json::to_value(code_actions_for(diagnostic.code))?;
    }
    Ok(value)
}

pub fn code_action_commands(code: KErrorCode) -> Vec<String> {
    code_actions_for(code)
        .into_iter()
        .filter_map(|action| action.command)
        .collect()
}

pub fn code_actions_for(code: KErrorCode) -> Vec<LspCodeAction> {
    let code_text = code.as_str();
    let mut actions = vec![LspCodeAction {
        title: format!("Explain {code_text}"),
        group: "explain".to_owned(),
        command: Some(format!("kobo explain {code_text}")),
    }];

    match code {
        KErrorCode::K0100 => {
            actions.push(LspCodeAction {
                title: "Run quick simulation".to_owned(),
                group: "simulation".to_owned(),
                command: Some("kobo test --sim quick --witness-dir .kobo/witnesses".to_owned()),
            });
            actions.push(LspCodeAction {
                title: "Replay witness".to_owned(),
                group: "replay".to_owned(),
                command: Some("kobo replay .kobo/witnesses/<witness>.kwit".to_owned()),
            });
        }
        KErrorCode::K0107 => {
            actions.push(LspCodeAction {
                title: "Run quick simulation".to_owned(),
                group: "simulation".to_owned(),
                command: Some("kobo test --sim quick".to_owned()),
            });
            actions.push(LspCodeAction {
                title: "Replay witness".to_owned(),
                group: "replay".to_owned(),
                command: Some("kobo replay .kobo/witnesses/<witness>.kwit".to_owned()),
            });
            for choice in ["model", "record", "stub", "outside", "opaque", "debt"] {
                actions.push(LspCodeAction {
                    title: format!("Boundary policy: {choice}"),
                    group: "boundary-policy".to_owned(),
                    command: None,
                });
            }
        }
        _ => {}
    }

    actions
}
