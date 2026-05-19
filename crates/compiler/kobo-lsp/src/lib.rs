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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ProtocolSourceFacts {
    has_liveness_obligation: bool,
    has_terminal_action: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProtocolObligation {
    type_name: String,
    actions: Vec<String>,
    line: usize,
    character: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProtocolDiagnosticFact {
    line: usize,
    character_start: usize,
    character_end: usize,
    code: &'static str,
    message: String,
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
        let actions = code_actions_for_diagnostic(diagnostic);
        value["codeAction"] = json!(action_commands(&actions));
        value["codeActions"] = serde_json::to_value(actions)?;
    }
    Ok(value)
}

pub fn code_action_commands(code: KErrorCode) -> Vec<String> {
    action_commands(&code_actions_for(code))
}

pub fn editor_capabilities() -> Value {
    json!({
        "diagnostics": {
            "provider": "kobo-lsp",
            "source": "compiler-diagnostics",
        },
        "hoverProvider": true,
        "codeActionProvider": true,
        "documentLinkProvider": true,
        "definitionProvider": {
            "delegate": "rust-analyzer",
            "source_map": "generated-rust-to-kobo",
        },
        "runnables": [
            {
                "command": "kobo.testScenario",
                "title": "Run Kobo scenario",
                "cli": "kobo test --sim quick --witness-dir .kobo/witnesses",
            },
            {
                "command": "kobo.replayWitness",
                "title": "Replay Kobo witness",
                "cli": "kobo replay <witness>",
            },
            {
                "command": "kobo.explainDiagnostic",
                "title": "Explain diagnostic",
                "cli": "kobo explain <code>",
            }
        ],
        "witnessLinks": {
            "pattern": ".kobo/witnesses/*.kwit",
            "command": "kobo.replayWitness",
        },
    })
}

pub fn initialize_response(id: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "serverInfo": {
                "name": "kobo-lsp",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "capabilities": editor_capabilities(),
        },
    })
}

pub fn protocol_document_snapshot(uri: &str, source: &str, witness_path: Option<&str>) -> Value {
    json!({
        "uri": uri,
        "diagnostics": protocol_diagnostics(source),
        "hover": protocol_hover(source),
        "codeActions": protocol_code_actions(witness_path),
        "documentLinks": protocol_document_links(uri, source, witness_path),
        "runnables": editor_capabilities()["runnables"].clone(),
        "definitionProvider": editor_capabilities()["definitionProvider"].clone(),
    })
}

fn protocol_diagnostics(source: &str) -> Vec<Value> {
    if let Some(fact) = unresolved_liveness_diagnostic(source) {
        return vec![json!({
            "range": {
                "start": {"line": fact.line, "character": fact.character_start},
                "end": {"line": fact.line, "character": fact.character_end},
            },
            "severity": 1,
            "code": fact.code,
            "source": "kobo",
            "message": fact.message,
        })];
    }
    Vec::new()
}

fn unresolved_liveness_diagnostic(source: &str) -> Option<ProtocolDiagnosticFact> {
    let semantic_source = source_without_text(source);
    let obligations = ward_obligations(&semantic_source);
    for obligation in obligations {
        if let Some(fact) = unresolved_binding_for_obligation(source, &semantic_source, &obligation)
        {
            return Some(fact);
        }
    }
    let facts = protocol_source_facts(source);
    (facts.has_liveness_obligation && !facts.has_terminal_action).then(|| ProtocolDiagnosticFact {
        line: 0,
        character_start: 0,
        character_end: 1,
        code: "K0100",
        message: "unresolved liveness obligation".to_owned(),
    })
}

fn ward_obligations(source: &str) -> Vec<ProtocolObligation> {
    source
        .lines()
        .enumerate()
        .filter_map(|(line, text)| ward_obligation_from_line(line, text))
        .collect()
}

fn ward_obligation_from_line(line: usize, text: &str) -> Option<ProtocolObligation> {
    let character = text.find("obligation ")?;
    let rest = text[character + "obligation ".len()..].trim_start();
    let (type_name, after_type) = rest.split_once(char::is_whitespace)?;
    let actions_text = after_type.trim_start().strip_prefix("must")?.trim();
    let actions = actions_text
        .split('|')
        .flat_map(str::split_whitespace)
        .filter(|action| !action.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    (!type_name.is_empty() && !actions.is_empty()).then(|| ProtocolObligation {
        type_name: type_name.to_owned(),
        actions,
        line,
        character,
    })
}

fn unresolved_binding_for_obligation(
    original_source: &str,
    semantic_source: &str,
    obligation: &ProtocolObligation,
) -> Option<ProtocolDiagnosticFact> {
    for (line, text) in semantic_source.lines().enumerate() {
        let Some(binding) = binding_constructed_on_line(text, &obligation.type_name) else {
            continue;
        };
        if binding_has_terminal_action(semantic_source, &binding, &obligation.actions) {
            continue;
        }
        let original_line = original_source.lines().nth(line).unwrap_or(text);
        let character_start = original_line.find(&binding).unwrap_or(0);
        return Some(ProtocolDiagnosticFact {
            line,
            character_start,
            character_end: character_start + binding.len(),
            code: "K0100",
            message: format!(
                "unresolved liveness obligation `{binding}` requires {}",
                obligation.actions.join(" | ")
            ),
        });
    }
    Some(ProtocolDiagnosticFact {
        line: obligation.line,
        character_start: obligation.character,
        character_end: obligation.character + obligation.type_name.len(),
        code: "K0100",
        message: format!(
            "unresolved liveness obligation for `{}`",
            obligation.type_name
        ),
    })
}

fn binding_constructed_on_line(line: &str, type_name: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("let ")?;
    let rest = rest.strip_prefix("mut ").unwrap_or(rest);
    if !rest.contains(&format!("{type_name} {{")) {
        return None;
    }
    let binding = rest
        .split(|ch: char| ch == ':' || ch == '=' || ch.is_whitespace())
        .next()?
        .trim();
    (!binding.is_empty()).then(|| binding.to_owned())
}

fn binding_has_terminal_action(source: &str, binding: &str, actions: &[String]) -> bool {
    actions.iter().any(|action| {
        let call = format!("{binding}.{action}(");
        let spaced_call = format!("{binding}.{action} (");
        source.contains(&call) || source.contains(&spaced_call)
    })
}

fn protocol_source_facts(source: &str) -> ProtocolSourceFacts {
    let semantic_source = source_without_text(source);
    ProtocolSourceFacts {
        has_liveness_obligation: semantic_source.contains("must_call")
            || semantic_source.contains("obligation "),
        has_terminal_action: [
            ".ack()",
            ".nack()",
            ".requeue()",
            ".reply()",
            ".reject()",
            ".cancel()",
            ".close()",
            ".commit()",
            ".rollback()",
        ]
        .iter()
        .any(|action| semantic_source.contains(action)),
    }
}

fn source_without_text(source: &str) -> String {
    let mut scrubbed = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut in_string = false;
    let mut in_line_comment = false;
    while let Some(ch) = chars.next() {
        if in_line_comment {
            if ch == '\n' {
                in_line_comment = false;
                scrubbed.push('\n');
            } else {
                scrubbed.push(' ');
            }
            continue;
        }
        if in_string {
            if ch == '\\' {
                scrubbed.push(' ');
                if let Some(next) = chars.next() {
                    scrubbed.push(if next == '\n' { '\n' } else { ' ' });
                }
            } else if ch == '"' {
                in_string = false;
                scrubbed.push(' ');
            } else {
                scrubbed.push(if ch == '\n' { '\n' } else { ' ' });
            }
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'/') {
            scrubbed.push(' ');
            if let Some(next) = chars.next() {
                scrubbed.push(if next == '\n' { '\n' } else { ' ' });
            }
            in_line_comment = true;
            continue;
        }
        if ch == '"' {
            in_string = true;
            scrubbed.push(' ');
            continue;
        }
        scrubbed.push(ch);
    }
    scrubbed
}

fn protocol_hover(source: &str) -> Value {
    let contents = if source.contains("ward ") {
        "Kobo ward model: states, obligations, scenarios, ports, recordings, and debt."
    } else if source.contains("must_call") {
        "Kobo must_call obligation: every path must use one terminal action."
    } else {
        "Kobo source: Rust-shaped code with gradual runtime guarantees."
    };
    json!({
        "contents": {
            "kind": "markdown",
            "value": contents,
        },
    })
}

fn protocol_code_actions(witness_path: Option<&str>) -> Vec<LspCodeAction> {
    let replay_command = witness_path.map(|path| format!("kobo replay {path}"));
    code_actions_for_code_and_replay(KErrorCode::K0100, replay_command.as_deref())
}

fn protocol_document_links(uri: &str, source: &str, witness_path: Option<&str>) -> Vec<Value> {
    let mut links = Vec::new();
    if let Some(path) = witness_path {
        links.push(json!({
            "range": {
                "start": {"line": 0, "character": 0},
                "end": {"line": 0, "character": 1},
            },
            "target": path,
            "tooltip": "Replay Kobo witness",
        }));
    }
    if source.contains("fn ") {
        links.push(json!({
            "range": {
                "start": {"line": 0, "character": 0},
                "end": {"line": 0, "character": 1},
            },
            "target": format!("{uri}#generated-rust"),
            "tooltip": "Open generated Rust through source map",
        }));
    }
    links
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

fn code_actions_for_diagnostic(diagnostic: &KDiagnostic) -> Vec<LspCodeAction> {
    let replay_command = diagnostic
        .run
        .as_ref()
        .map(|run| run.0.as_str())
        .filter(|command| command.starts_with("kobo replay "))
        .filter(|command| !command.contains("<witness>"));
    code_actions_for_code_and_replay(diagnostic.code, replay_command)
}

fn code_actions_for_code_and_replay(
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
                    command: None,
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

fn action_commands(actions: &[LspCodeAction]) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use super::code_actions_for;
    use kobo_errors::KErrorCode;

    #[test]
    fn active_k012x_codes_have_specific_lsp_actions() {
        for code in [
            KErrorCode::K0120,
            KErrorCode::K0121,
            KErrorCode::K0122,
            KErrorCode::K0123,
            KErrorCode::K0124,
            KErrorCode::K0125,
            KErrorCode::K0126,
            KErrorCode::K0127,
            KErrorCode::K0128,
            KErrorCode::K0129,
        ] {
            let actions = code_actions_for(code);
            assert!(
                actions.len() > 1,
                "{} should expose a code-specific action beyond Explain",
                code.as_str()
            );
            assert!(
                actions.iter().any(|action| action.group != "explain"),
                "{} should not fall back to explain-only LSP coverage",
                code.as_str()
            );
        }
    }
}
