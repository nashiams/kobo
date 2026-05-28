use kobo_errors::{KDiagnostic, KErrorCode};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LspCodeAction {
    pub title: String,
    pub group: String,
    pub command: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticArtifact {
    pub code: String,
    pub witness_path: String,
    pub source_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WitnessArtifact {
    pub path: String,
    pub source_hash: String,
}

pub fn code_action_commands(code: KErrorCode) -> Vec<String> {
    action_commands(&code_actions_for(code))
}

pub fn code_actions_for(code: KErrorCode) -> Vec<LspCodeAction> {
    code_actions_for_code_and_replay(code, None)
}

pub fn actions_for_diagnostic_with_artifacts(
    diagnostic: &DiagnosticArtifact,
    artifacts: &[WitnessArtifact],
) -> Vec<LspCodeAction> {
    let code = parse_code(&diagnostic.code).unwrap_or(KErrorCode::K0100);
    let replay_command = artifacts
        .iter()
        .find(|artifact| {
            artifact.path == diagnostic.witness_path
                && artifact.source_hash == diagnostic.source_hash
        })
        .map(|artifact| format!("kobo replay {}", artifact.path));
    code_actions_for_code_and_replay(code, replay_command.as_deref())
}

pub(crate) fn code_actions_for_diagnostic(diagnostic: &KDiagnostic) -> Vec<LspCodeAction> {
    let replay_command = diagnostic
        .run
        .as_ref()
        .map(|run| run.0.as_str())
        .filter(|command| command.starts_with("kobo replay "))
        .filter(|command| !command.contains("<witness>"));
    code_actions_for_code_and_replay(diagnostic.code, replay_command)
}

pub(crate) fn code_actions_for_code_and_replay(
    code: KErrorCode,
    replay_command: Option<&str>,
) -> Vec<LspCodeAction> {
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
            if let Some(command) = replay_command {
                actions.push(LspCodeAction {
                    title: "Replay witness".to_owned(),
                    group: "replay".to_owned(),
                    command: Some(command.to_owned()),
                });
            }
        }
        KErrorCode::K0107 => {
            actions.push(LspCodeAction {
                title: "Run quick simulation".to_owned(),
                group: "simulation".to_owned(),
                command: Some("kobo test --sim quick".to_owned()),
            });
            if let Some(command) = replay_command {
                actions.push(LspCodeAction {
                    title: "Replay witness".to_owned(),
                    group: "replay".to_owned(),
                    command: Some(command.to_owned()),
                });
            }
            for choice in [
                "typed", "model", "record", "activity", "stub", "outside", "opaque", "debt",
            ] {
                actions.push(LspCodeAction {
                    title: format!("Boundary policy: {choice}"),
                    group: "boundary-policy".to_owned(),
                    command: Some("kobo explain K0107 --verbose".to_owned()),
                });
            }
        }
        KErrorCode::K0120 => {
            actions.push(LspCodeAction {
                title: "Validate ecosystem policy".to_owned(),
                group: "ecosystem-policy".to_owned(),
                command: Some("kobo doctor --deps --json".to_owned()),
            });
        }
        KErrorCode::K0121 => {
            actions.push(LspCodeAction {
                title: "Validate declaration file".to_owned(),
                group: "declaration".to_owned(),
                command: Some("kobo check --replay-critical --error-format=json".to_owned()),
            });
            actions.push(LspCodeAction {
                title: "Regenerate declaration".to_owned(),
                group: "declaration".to_owned(),
                command: None,
            });
        }
        KErrorCode::K0122 => {
            actions.push(LspCodeAction {
                title: "Create declaration file".to_owned(),
                group: "declaration".to_owned(),
                command: None,
            });
            actions.push(LspCodeAction {
                title: "Use record boundary".to_owned(),
                group: "boundary-policy".to_owned(),
                command: None,
            });
            actions.push(LspCodeAction {
                title: "Use opaque boundary".to_owned(),
                group: "boundary-policy".to_owned(),
                command: None,
            });
        }
        KErrorCode::K0123 => {
            actions.push(LspCodeAction {
                title: "Install or update adapter".to_owned(),
                group: "adapter".to_owned(),
                command: None,
            });
            actions.push(LspCodeAction {
                title: "Use record boundary".to_owned(),
                group: "boundary-policy".to_owned(),
                command: None,
            });
        }
        KErrorCode::K0124 => {
            actions.push(LspCodeAction {
                title: "Regenerate recorded witness".to_owned(),
                group: "replay".to_owned(),
                command: Some("kobo test --sim quick --witness-dir .kobo/witnesses".to_owned()),
            });
            actions.push(LspCodeAction {
                title: "Use activity boundary".to_owned(),
                group: "boundary-policy".to_owned(),
                command: None,
            });
        }
        KErrorCode::K0125 => {
            actions.push(LspCodeAction {
                title: "Add activity retry metadata".to_owned(),
                group: "declaration".to_owned(),
                command: None,
            });
            actions.push(LspCodeAction {
                title: "Run quick simulation".to_owned(),
                group: "simulation".to_owned(),
                command: Some("kobo test --sim quick".to_owned()),
            });
        }
        KErrorCode::K0126 => {
            actions.push(LspCodeAction {
                title: "Rebuild upstream summary".to_owned(),
                group: "summary".to_owned(),
                command: Some("kobo build".to_owned()),
            });
        }
        KErrorCode::K0127 => {
            actions.push(LspCodeAction {
                title: "Review generated declaration".to_owned(),
                group: "bindgen".to_owned(),
                command: None,
            });
            actions.push(LspCodeAction {
                title: "Regenerate bindgen draft".to_owned(),
                group: "bindgen".to_owned(),
                command: Some("kobo bindgen --path <crate>".to_owned()),
            });
        }
        KErrorCode::K0128 => {
            actions.push(LspCodeAction {
                title: "Run Cargo build".to_owned(),
                group: "cargo".to_owned(),
                command: Some("cargo build".to_owned()),
            });
            actions.push(LspCodeAction {
                title: "Inspect dependencies".to_owned(),
                group: "cargo".to_owned(),
                command: Some("kobo doctor --deps --json".to_owned()),
            });
        }
        KErrorCode::K0129 => {
            actions.push(LspCodeAction {
                title: "Replay witness".to_owned(),
                group: "replay".to_owned(),
                command: replay_command
                    .map(str::to_owned)
                    .or_else(|| Some("kobo replay <witness>".to_owned())),
            });
            actions.push(LspCodeAction {
                title: "Keep replay partial".to_owned(),
                group: "boundary-policy".to_owned(),
                command: None,
            });
        }
        _ => {}
    }

    actions
}

pub(crate) fn action_commands(actions: &[LspCodeAction]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|action| action.command.clone())
        .collect()
}

fn parse_code(code: &str) -> Option<KErrorCode> {
    KErrorCode::ALL
        .iter()
        .copied()
        .find(|candidate| candidate.as_str() == code)
}

pub mod test_support {
    use super::{DiagnosticArtifact, WitnessArtifact};

    pub fn diagnostic_with_artifact(
        code: &str,
        witness_path: &str,
        source_hash: &str,
    ) -> DiagnosticArtifact {
        DiagnosticArtifact {
            code: code.to_owned(),
            witness_path: witness_path.to_owned(),
            source_hash: source_hash.to_owned(),
        }
    }

    pub fn witness_artifact(path: &str, source_hash: &str) -> WitnessArtifact {
        WitnessArtifact {
            path: path.to_owned(),
            source_hash: source_hash.to_owned(),
        }
    }
}
