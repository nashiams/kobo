use kobo_errors::KErrorCode;
use kobo_ir::KoboSpan;
use serde_json::{json, Value};

use crate::actions::{code_actions_for_code_and_replay, LspCodeAction};
use crate::analysis::protocol_document_analysis;
use crate::document_links::protocol_document_links;
use crate::navigation::source_map_metadata;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProtocolDiagnosticFact {
    pub(crate) line: usize,
    pub(crate) character_start: usize,
    pub(crate) character_end: usize,
    pub(crate) code: &'static str,
    pub(crate) source: &'static str,
    pub(crate) fact_source: &'static str,
    pub(crate) message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProtocolRange {
    pub(crate) line: usize,
    pub(crate) character_start: usize,
    pub(crate) character_end: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProtocolNavigationSite {
    pub(crate) binding: String,
    pub(crate) span: KoboSpan,
    pub(crate) source_range: ProtocolRange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProtocolRustNavigation {
    pub(crate) generated_uri: String,
    pub(crate) source_map_uri: String,
    pub(crate) source_range: ProtocolRange,
    pub(crate) generated_range: ProtocolRange,
    pub(crate) mapping_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProtocolDocumentAnalysis {
    pub(crate) diagnostics: Vec<ProtocolDiagnosticFact>,
    pub(crate) hover: ProtocolHoverKind,
    pub(crate) rust_navigation: Option<ProtocolRustNavigation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProtocolHoverKind {
    Ward,
    MustCall,
    RustShape,
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

pub(crate) fn protocol_hover(
    kind: ProtocolHoverKind,
    navigation: Option<&ProtocolRustNavigation>,
) -> Value {
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

pub(crate) fn protocol_code_actions(
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

pub(crate) fn protocol_runnables(navigation: Option<&ProtocolRustNavigation>) -> Vec<Value> {
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

pub(crate) fn protocol_definitions(navigation: Option<&ProtocolRustNavigation>) -> Vec<Value> {
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

pub(crate) fn protocol_range_json(range: &ProtocolRange) -> Value {
    json!({
        "start": {"line": range.line, "character": range.character_start},
        "end": {"line": range.line, "character": range.character_end},
    })
}

pub(crate) fn default_document_range() -> Value {
    json!({
        "start": {"line": 0, "character": 0},
        "end": {"line": 0, "character": 1},
    })
}
