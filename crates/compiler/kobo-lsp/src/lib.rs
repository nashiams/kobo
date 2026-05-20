use std::path::Path;

use kobo_codegen::{CodegenOptions, KoboSourceMap, RsSpan};
use kobo_errors::{DiagnosticLspPayload, KDiagnostic, KErrorCode};
use kobo_ir::{
    FileId, FileSet, Kir, KoboSpan, NodeIdGen, ScenarioOpKind, ScenarioProgram, SolutionMap,
};
use kobo_parser::PreprocessSourceMap;
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProtocolDiagnosticFact {
    line: usize,
    character_start: usize,
    character_end: usize,
    code: &'static str,
    source: &'static str,
    fact_source: &'static str,
    message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProtocolRange {
    line: usize,
    character_start: usize,
    character_end: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProtocolNavigationSite {
    binding: String,
    span: KoboSpan,
    source_range: ProtocolRange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProtocolRustNavigation {
    generated_uri: String,
    source_map_uri: String,
    source_range: ProtocolRange,
    generated_range: ProtocolRange,
    mapping_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProtocolDocumentAnalysis {
    diagnostics: Vec<ProtocolDiagnosticFact>,
    hover: ProtocolHoverKind,
    rust_navigation: Option<ProtocolRustNavigation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProtocolHoverKind {
    Ward,
    MustCall,
    RustShape,
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
    let analysis = protocol_document_analysis(uri, source);
    json!({
        "uri": uri,
        "diagnostics": protocol_diagnostics(&analysis),
        "hover": protocol_hover(analysis.hover, analysis.rust_navigation.as_ref()),
        "codeActions": protocol_code_actions(witness_path, analysis.rust_navigation.as_ref()),
        "documentLinks": protocol_document_links(witness_path, analysis.rust_navigation.as_ref()),
        "runnables": protocol_runnables(analysis.rust_navigation.as_ref()),
        "definitions": protocol_definitions(analysis.rust_navigation.as_ref()),
        "definitionProvider": editor_capabilities()["definitionProvider"].clone(),
    })
}

fn protocol_diagnostics(analysis: &ProtocolDocumentAnalysis) -> Vec<Value> {
    analysis
        .diagnostics
        .iter()
        .map(|fact| {
            json!({
            "range": {
                "start": {"line": fact.line, "character": fact.character_start},
                "end": {"line": fact.line, "character": fact.character_end},
            },
            "severity": 1,
            "code": fact.code,
            "source": fact.source,
            "data": {
                "fact_source": fact.fact_source,
            },
            "message": fact.message,
            })
        })
        .collect()
}

fn protocol_document_analysis(uri: &str, source: &str) -> ProtocolDocumentAnalysis {
    compiler_document_analysis(uri, source)
}

fn compiler_document_analysis(uri: &str, source: &str) -> ProtocolDocumentAnalysis {
    let mut id_gen = NodeIdGen::new();
    let file_id = FileId(0);
    let ward_mapped = kobo_parser::preprocess_ward_syntax_mapped(source, file_id);
    let has_ward_syntax = !ward_mapped.metadata.wards.is_empty();
    let preprocess_source_map = ward_mapped.source_map;
    let rewritten = ward_mapped.rewritten;
    let outcome = kobo_parser::parse_file_recovering(
        &rewritten,
        file_id,
        &mut id_gen,
        kobo_parser::RecoveryMode::Recover,
    );
    let mut diagnostics =
        parse_recovery_diagnostics(source, &outcome.diagnostics, &preprocess_source_map);
    let Some(ast) = outcome.file else {
        return ProtocolDocumentAnalysis {
            diagnostics,
            hover: if has_ward_syntax {
                ProtocolHoverKind::Ward
            } else {
                ProtocolHoverKind::RustShape
            },
            rust_navigation: None,
        };
    };
    let kir = kobo_transform::build_kir(
        &ast,
        &mut id_gen,
        kobo_transform::TransformOptions::default(),
    );
    let programs = kir.scenario_programs();
    diagnostics.extend(programs.iter().flat_map(|program| {
        compiler_liveness_diagnostics(source, program, &preprocess_source_map)
    }));
    let hover = if has_ward_syntax {
        ProtocolHoverKind::Ward
    } else if programs.iter().any(program_has_obligation) || !diagnostics.is_empty() {
        ProtocolHoverKind::MustCall
    } else {
        ProtocolHoverKind::RustShape
    };
    let rust_navigation =
        compiler_rust_navigation(uri, source, &preprocess_source_map, &ast, &kir, &programs);
    ProtocolDocumentAnalysis {
        diagnostics,
        hover,
        rust_navigation,
    }
}

fn compiler_rust_navigation(
    uri: &str,
    source: &str,
    preprocess_source_map: &PreprocessSourceMap,
    ast: &kobo_parser::KoboFile,
    kir: &Kir,
    programs: &[ScenarioProgram],
) -> Option<ProtocolRustNavigation> {
    let navigation_site = first_navigation_site(source, programs, preprocess_source_map)?;
    let codegen_output = kobo_codegen::codegen_file(
        kir,
        ast,
        &SolutionMap::new(),
        Path::new("main.kobo"),
        Path::new("main.rs"),
        &CodegenOptions::default(),
    );
    let generated_range =
        generated_range_from_source_map(&codegen_output.source_map, navigation_site.span)
            .or_else(|| {
                generated_binding_range(&codegen_output.rs_source, &navigation_site.binding)
            })
            .unwrap_or_else(|| navigation_site.source_range.clone());

    Some(ProtocolRustNavigation {
        generated_uri: generated_uri_for(uri),
        source_map_uri: source_map_uri_for(uri),
        source_range: navigation_site.source_range,
        generated_range,
        mapping_count: codegen_output.source_map.x_kobo_mappings.len(),
    })
}

fn first_navigation_site(
    source: &str,
    programs: &[ScenarioProgram],
    preprocess_source_map: &PreprocessSourceMap,
) -> Option<ProtocolNavigationSite> {
    programs.iter().find_map(|program| {
        program
            .operations
            .iter()
            .find_map(|operation| match &operation.kind {
                ScenarioOpKind::CreateObligation { binding, .. } => {
                    let original_span = original_span_for(preprocess_source_map, operation.span);
                    Some(ProtocolNavigationSite {
                        binding: binding.clone(),
                        span: operation.span,
                        source_range: protocol_range_from_span(source, original_span),
                    })
                }
                _ => None,
            })
    })
}

fn generated_range_from_source_map(
    source_map: &KoboSourceMap,
    source_span: KoboSpan,
) -> Option<ProtocolRange> {
    source_map
        .lookup_rs_spans(source_span)
        .into_iter()
        .next()
        .map(protocol_range_from_rs_span)
}

fn generated_binding_range(generated_source: &str, binding: &str) -> Option<ProtocolRange> {
    generated_source
        .lines()
        .enumerate()
        .find_map(|(line, text)| {
            let character_start = text.find(binding)?;
            Some(ProtocolRange {
                line,
                character_start,
                character_end: character_start + binding.len(),
            })
        })
}

fn compiler_liveness_diagnostics(
    source: &str,
    program: &ScenarioProgram,
    preprocess_source_map: &PreprocessSourceMap,
) -> Vec<ProtocolDiagnosticFact> {
    program
        .operations
        .iter()
        .filter_map(|operation| match &operation.kind {
            ScenarioOpKind::CreateObligation {
                binding, actions, ..
            } if !compiler_binding_is_resolved(program, binding, actions) => {
                let original_span = original_span_for(preprocess_source_map, operation.span);
                let (line, character_start, character_end) =
                    line_range_from_span(source, original_span);
                Some(ProtocolDiagnosticFact {
                    line,
                    character_start,
                    character_end,
                    code: "K0100",
                    source: "kobo-compiler",
                    fact_source: "compiler-scenario-program",
                    message: format!(
                        "unresolved liveness obligation `{binding}` requires {}",
                        actions.join(" | ")
                    ),
                })
            }
            _ => None,
        })
        .collect()
}

fn compiler_binding_is_resolved(
    program: &ScenarioProgram,
    binding: &str,
    actions: &[String],
) -> bool {
    program
        .operations
        .iter()
        .any(|operation| match &operation.kind {
            ScenarioOpKind::Discharge {
                binding: candidate,
                action,
            } => candidate == binding && actions.iter().any(|expected| expected == action),
            ScenarioOpKind::Transfer {
                binding: candidate,
                proven,
                ..
            } => candidate == binding && *proven,
            ScenarioOpKind::ExternalBoundary { .. } => true,
            _ => false,
        })
}

fn original_span_for(source_map: &PreprocessSourceMap, span: KoboSpan) -> KoboSpan {
    source_map.rewritten_span_to_original(span).unwrap_or(span)
}

fn parse_recovery_diagnostics(
    source: &str,
    diagnostics: &[KDiagnostic],
    preprocess_source_map: &PreprocessSourceMap,
) -> Vec<ProtocolDiagnosticFact> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            let original_span = original_span_for(preprocess_source_map, diagnostic.primary.span);
            let (line, character_start, character_end) =
                line_range_from_span(source, original_span);
            ProtocolDiagnosticFact {
                line,
                character_start,
                character_end,
                code: diagnostic.code.as_str(),
                source: "kobo-compiler",
                fact_source: "compiler-parse-recovery",
                message: diagnostic
                    .finding
                    .clone()
                    .unwrap_or_else(|| diagnostic.primary.text.clone()),
            }
        })
        .collect()
}

fn program_has_obligation(program: &ScenarioProgram) -> bool {
    program
        .operations
        .iter()
        .any(|operation| matches!(operation.kind, ScenarioOpKind::CreateObligation { .. }))
}

fn line_range_from_span(source: &str, span: KoboSpan) -> (usize, usize, usize) {
    let range = protocol_range_from_span(source, span);
    (range.line, range.character_start, range.character_end)
}

fn protocol_range_from_span(source: &str, span: KoboSpan) -> ProtocolRange {
    let start = span.start as usize;
    let end = span.end.max(span.start + 1) as usize;
    let mut line = 0usize;
    let mut line_start = 0usize;
    for (index, byte) in source.as_bytes().iter().enumerate() {
        if index >= start {
            break;
        }
        if *byte == b'\n' {
            line += 1;
            line_start = index + 1;
        }
    }
    let character_start = start.saturating_sub(line_start);
    let character_end = end.saturating_sub(line_start).max(character_start + 1);
    ProtocolRange {
        line,
        character_start,
        character_end,
    }
}

fn protocol_range_from_rs_span(rs_span: RsSpan) -> ProtocolRange {
    ProtocolRange {
        line: rs_span.line,
        character_start: rs_span.column_start,
        character_end: rs_span.column_end.max(rs_span.column_start + 1),
    }
}

fn protocol_hover(kind: ProtocolHoverKind, navigation: Option<&ProtocolRustNavigation>) -> Value {
    let contents = match kind {
        ProtocolHoverKind::Ward => {
            "Kobo ward model: states, obligations, scenarios, ports, recordings, and debt."
        }
        ProtocolHoverKind::MustCall => {
            "Kobo must_call obligation: every path must use one terminal action."
        }
        ProtocolHoverKind::RustShape => {
            "Kobo source: Rust-shaped code with gradual runtime guarantees."
        }
    };
    let mut hover = json!({
        "contents": {
            "kind": "markdown",
            "value": contents,
        },
    });
    if let Some(navigation) = navigation {
        hover["range"] = protocol_range_json(&navigation.source_range);
        hover["data"] = source_map_metadata(navigation);
    }
    hover
}

fn protocol_code_actions(
    witness_path: Option<&str>,
    navigation: Option<&ProtocolRustNavigation>,
) -> Vec<Value> {
    let replay_command = witness_path.map(|path| format!("kobo replay {path}"));
    code_actions_for_code_and_replay(KErrorCode::K0100, replay_command.as_deref())
        .into_iter()
        .map(|action| protocol_code_action_value(action, navigation))
        .collect()
}

fn protocol_code_action_value(
    action: LspCodeAction,
    navigation: Option<&ProtocolRustNavigation>,
) -> Value {
    let mut value = serde_json::to_value(action).unwrap_or_else(|_| json!({}));
    if let (Some(object), Some(navigation)) = (value.as_object_mut(), navigation) {
        object.insert(
            "range".to_owned(),
            protocol_range_json(&navigation.source_range),
        );
        object.insert("data".to_owned(), source_map_metadata(navigation));
    }
    value
}

fn protocol_document_links(
    witness_path: Option<&str>,
    navigation: Option<&ProtocolRustNavigation>,
) -> Vec<Value> {
    let mut links = Vec::new();
    let link_range = navigation
        .map(|navigation| protocol_range_json(&navigation.source_range))
        .unwrap_or_else(default_document_range);
    if let Some(path) = witness_path {
        let mut link = json!({
            "range": link_range.clone(),
            "target": path,
            "tooltip": "Replay Kobo witness",
        });
        if let Some(navigation) = navigation {
            link["data"] = source_map_metadata(navigation);
        }
        links.push(link);
    }
    if let Some(navigation) = navigation {
        links.push(json!({
            "range": protocol_range_json(&navigation.source_range),
            "target": navigation.generated_uri,
            "tooltip": "Open generated Rust through source map",
            "data": source_map_metadata(navigation),
        }));
    }
    links
}

fn protocol_runnables(navigation: Option<&ProtocolRustNavigation>) -> Vec<Value> {
    let mut runnables = editor_capabilities()["runnables"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if let Some(navigation) = navigation {
        for runnable in &mut runnables {
            runnable["range"] = protocol_range_json(&navigation.source_range);
            runnable["data"] = source_map_metadata(navigation);
        }
    }
    runnables
}

fn protocol_definitions(navigation: Option<&ProtocolRustNavigation>) -> Vec<Value> {
    navigation
        .map(|navigation| {
            vec![json!({
                "uri": navigation.generated_uri,
                "range": protocol_range_json(&navigation.generated_range),
                "data": {
                    "delegate": "rust-analyzer",
                    "source_map": navigation.source_map_uri,
                    "source_range": protocol_range_json(&navigation.source_range),
                    "generated_range": protocol_range_json(&navigation.generated_range),
                    "mapping_count": navigation.mapping_count,
                },
            })]
        })
        .unwrap_or_default()
}

fn protocol_range_json(range: &ProtocolRange) -> Value {
    json!({
        "start": {"line": range.line, "character": range.character_start},
        "end": {"line": range.line, "character": range.character_end},
    })
}

fn default_document_range() -> Value {
    json!({
        "start": {"line": 0, "character": 0},
        "end": {"line": 0, "character": 1},
    })
}

fn source_map_metadata(navigation: &ProtocolRustNavigation) -> Value {
    json!({
        "delegate": "rust-analyzer",
        "source_map": navigation.source_map_uri,
        "generated_uri": navigation.generated_uri,
        "source_range": protocol_range_json(&navigation.source_range),
        "generated_range": protocol_range_json(&navigation.generated_range),
        "mapping_count": navigation.mapping_count,
    })
}

fn generated_uri_for(uri: &str) -> String {
    uri.strip_suffix(".kobo")
        .map(|stem| format!("{stem}.rs"))
        .unwrap_or_else(|| format!("{uri}.rs"))
}

fn source_map_uri_for(uri: &str) -> String {
    uri.strip_suffix(".kobo")
        .map(|stem| format!("{stem}.kobo.map"))
        .unwrap_or_else(|| format!("{uri}.kobo.map"))
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
