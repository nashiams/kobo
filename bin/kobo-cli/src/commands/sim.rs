use std::path::{Path, PathBuf};

use anyhow::Context;
use kobo_sim_core::backend;
use serde_json::{json, Value};

use super::sim_model;

pub(super) fn cmd_sim_init(
    target: &str,
    minimal: bool,
    profile: Option<&str>,
) -> anyhow::Result<()> {
    let (file, symbol) = target
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("--target must use FILE:SYMBOL"))?;
    let file = Path::new(file);
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read simulation target {}", file.display()))?;
    let Some(signature) = target_signature(&source, symbol) else {
        anyhow::bail!(
            "simulation target `{symbol}` was not found in {}",
            file.display()
        );
    };

    let document = sim_model::parse_document(source);
    let selected_profile = sim_model::target_profile(&document, symbol, profile);
    let backend = sim_model::backend_for_profile(&selected_profile);
    let artifacts = write_sim_scaffold(
        file,
        symbol,
        &selected_profile,
        backend.as_str(),
        minimal,
        &document,
        &signature,
    )?;
    let value = json!({
        "sim": {
            "target": symbol,
            "profile": selected_profile,
            "backend": backend.as_str(),
            "minimal": minimal,
            "scaffold": artifacts.scaffold_path.display().to_string(),
            "islands": [
                {
                    "name": artifacts.scenario_name,
                    "target": symbol,
                    "backend": backend.as_str(),
                    "path": artifacts.island_path.display().to_string(),
                    "source": file.display().to_string(),
                }
            ],
            "backend_imports_in_user_source": false,
        }
    });
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

struct SimArtifacts {
    scaffold_path: PathBuf,
    island_path: PathBuf,
    scenario_name: String,
}

#[derive(Clone, Debug)]
struct TargetSignature {
    is_async: bool,
    params: Vec<String>,
}

#[derive(Clone, Debug)]
struct InputFixture {
    name: String,
    type_name: String,
    expression: String,
}

fn write_sim_scaffold(
    file: &Path,
    symbol: &str,
    profile: &str,
    backend: &str,
    minimal: bool,
    document: &sim_model::ScenarioDocument,
    signature: &TargetSignature,
) -> anyhow::Result<SimArtifacts> {
    let scaffold_dir = std::env::current_dir()
        .context("failed to determine current directory")?
        .join(".kobo")
        .join("sim");
    std::fs::create_dir_all(&scaffold_dir)
        .with_context(|| format!("failed to create {}", scaffold_dir.display()))?;
    let source_path = sim_model::cli_relative_path(file)?;
    let scenario_name = format!("__kobo_sim_{}", safe_identifier(symbol));
    let island_path = scaffold_dir.join(format!("{symbol}.scenario.kobo"));
    let island_source = sim_island_source(
        &source_path,
        symbol,
        &scenario_name,
        profile,
        backend,
        signature,
    );
    std::fs::write(&island_path, island_source)
        .with_context(|| format!("failed to write {}", island_path.display()))?;

    let scaffold_path = scaffold_dir.join(format!("{symbol}.sim.json"));
    let input_fixtures = input_fixtures_for(signature);
    let scaffold = json!({
        "schema_version": 1,
        "target": {
            "source": source_path,
            "symbol": symbol,
            "async": signature.is_async,
            "params": signature.params,
        },
        "input_fixtures": input_fixtures
            .iter()
            .map(|fixture| {
                json!({
                    "name": fixture.name,
                    "type": fixture.type_name,
                    "expression": fixture.expression,
                })
            })
            .collect::<Vec<_>>(),
        "profile": profile,
        "backend": backend,
        "minimal": minimal,
        "scenario_metadata": {
            "name": scenario_name,
            "island": sim_model::cli_relative_path(&island_path)?,
            "profile": profile,
            "backend": backend,
        },
        "source_hash": document.source_hash,
        "backend_imports_in_user_source": false,
    });
    std::fs::write(&scaffold_path, serde_json::to_string_pretty(&scaffold)?)
        .with_context(|| format!("failed to write {}", scaffold_path.display()))?;
    Ok(SimArtifacts {
        scaffold_path,
        island_path,
        scenario_name,
    })
}

pub(super) fn cmd_sim_scout(
    file: &Path,
    json_output: bool,
    why: bool,
    backend_recommendations: bool,
) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;

    if backend_recommendations {
        let recommendations = backend_recommendations_for(&source);
        print_value(
            json!({
            "version": "v0.10",
            "backend_fit": recommendations,
            "executed": true,
            "execution_surface": "generated user Rust process adapters plus compiler-owned semantic agreement",
            "note": "v0.10 executes generated user Rust through Loom/scheduler/network/filesystem adapter surfaces while preserving semantic and harness trace agreement",
            }),
            json_output,
        )?;
        return Ok(());
    }

    let scout = if why {
        scout_why_source(&source, file)
    } else {
        scout_source(&source, file)
    };
    print_value(scout, json_output)
}

fn target_signature(source: &str, symbol: &str) -> Option<TargetSignature> {
    source
        .lines()
        .map(str::trim)
        .find_map(|line| parse_target_signature(line, symbol))
}

fn parse_target_signature(line: &str, symbol: &str) -> Option<TargetSignature> {
    let candidates = [
        ("pub async fn ", true),
        ("async fn ", true),
        ("pub fn ", false),
        ("fn ", false),
    ];
    for (prefix, is_async) in candidates {
        let Some(rest) = line.strip_prefix(prefix) else {
            continue;
        };
        let rest = rest.strip_prefix(symbol)?;
        let rest = rest.trim_start();
        let params = rest.strip_prefix('(')?;
        let end = params.find(')')?;
        let params = params[..end]
            .split(',')
            .map(str::trim)
            .filter(|param| !param.is_empty())
            .map(str::to_owned)
            .collect();
        return Some(TargetSignature { is_async, params });
    }
    None
}

fn sim_island_source(
    source_path: &str,
    symbol: &str,
    scenario_name: &str,
    profile: &str,
    backend: &str,
    signature: &TargetSignature,
) -> String {
    let async_prefix = if signature.is_async { "async " } else { "" };
    let await_suffix = if signature.is_async { ".await" } else { "" };
    let mut source = format!(
        r#"#[kobo::scenario(profile = "{profile}")]
{async_prefix}fn {scenario_name}() {{
    // kobo: target {source_path}:{symbol}
    // kobo: backend-profile {profile}
    // kobo: backend {backend}
"#
    );
    if signature.params.is_empty() {
        source.push_str(&format!("    {symbol}(){await_suffix};\n"));
    } else if let Some(arguments) = generated_arguments(signature) {
        source.push_str(&format!("    {symbol}({arguments}){await_suffix};\n"));
    } else {
        source.push_str(&format!(
            "    // kobo: target inputs required: {}\n",
            signature.params.join(", ")
        ));
    }
    source.push_str("}\n");
    source
}

fn generated_arguments(signature: &TargetSignature) -> Option<String> {
    let fixtures = input_fixtures_for(signature);
    (fixtures.len() == signature.params.len()).then(|| {
        fixtures
            .into_iter()
            .map(|fixture| fixture.expression)
            .collect::<Vec<_>>()
            .join(", ")
    })
}

fn input_fixtures_for(signature: &TargetSignature) -> Vec<InputFixture> {
    signature
        .params
        .iter()
        .filter_map(|param| input_fixture_for_param(param))
        .collect()
}

fn input_fixture_for_param(param: &str) -> Option<InputFixture> {
    let (name, type_name) = param.split_once(':')?;
    let name = name.trim().trim_start_matches("mut ").trim().to_owned();
    let type_name = type_name.trim().to_owned();
    let expression = fixture_expression_for_type(&type_name)?;
    Some(InputFixture {
        name,
        type_name,
        expression,
    })
}

fn fixture_expression_for_type(type_name: &str) -> Option<String> {
    let compact_type = type_name.replace(' ', "");
    match compact_type.as_str() {
        "bool" => Some("false".to_owned()),
        "&str" | "&'staticstr" => Some("\"kobo-sim\"".to_owned()),
        "String" | "std::string::String" => Some("String::from(\"kobo-sim\")".to_owned()),
        "()" => Some("()".to_owned()),
        "usize" | "u8" | "u16" | "u32" | "u64" | "u128" | "isize" | "i8" | "i16" | "i32"
        | "i64" | "i128" => Some(format!("0_{compact_type}")),
        "f32" => Some("0.0_f32".to_owned()),
        "f64" => Some("0.0_f64".to_owned()),
        _ if compact_type.starts_with("Option<") => Some("None".to_owned()),
        _ if compact_type.starts_with("Vec<") => Some("Vec::new()".to_owned()),
        _ => None,
    }
}

fn safe_identifier(value: &str) -> String {
    let ident = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if ident
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
    {
        ident
    } else {
        format!("_{ident}")
    }
}

pub(super) fn cmd_sim_backends(json_output: bool) -> anyhow::Result<()> {
    let backends = backend::capabilities()
        .iter()
        .map(backend_capability_json)
        .collect::<Vec<_>>();
    print_value(
        json!({
            "version": "v0.10",
            "executed": true,
            "execution_surface": "generated-rust-process plus compiler-owned modeled facades",
            "v085_metadata_registry": v085_metadata_registry_json(),
            "backends": backends,
        }),
        json_output,
    )
}

fn backend_capability_json(capability: &backend::BackendCapability) -> Value {
    json!({
        "name": capability.name,
        "display_name": capability.display_name,
        "role": capability.role,
        "executes_in_v10": capability.executes_in_v10,
        "integration_level": capability.integration_level,
        "scenario_execution": capability.scenario_execution,
        "ecosystem_scope": capability.ecosystem_scope,
        "full_ecosystem_exploration": capability.full_ecosystem_exploration,
    })
}

fn v085_metadata_registry_json() -> Value {
    let backends = backend::v085_metadata_capabilities()
        .iter()
        .map(|capability| {
            json!({
                "name": capability.name,
                "display_name": capability.display_name,
                "backend_fit": capability.backend_fit,
                "metadata_only": true,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "version": "v0.8.5",
        "executed": false,
        "metadata_only": true,
        "backends": backends,
    })
}

fn scout_source(source: &str, file: &Path) -> serde_json::Value {
    let target = scenario_or_function_name(source);
    let mut reasons = scout_reasons(source);

    if reasons.is_empty() {
        return json!({
            "top_target": null,
            "summary": "no high-value simulation target",
            "reasons": [],
            "next_command": null,
        });
    }

    reasons.sort();
    reasons.dedup();
    json!({
        "top_target": target.unwrap_or_else(|| "module".to_owned()),
        "score": reasons.len(),
        "reasons": reasons,
        "expected_first_value": "one liveness, nondeterminism, or boundary finding",
        "next_command": format!("kobo sim scout --json {}", file.display()),
    })
}

fn scout_why_source(source: &str, file: &Path) -> serde_json::Value {
    let recommendations = backend_recommendations_for(source);
    let scout = scout_source(source, file);

    json!({
        "kobo_contract": "Kobo is Rust-shaped and Cargo-native; backend choices are possible engines, not user source imports.",
        "source_import_policy": "normal Kobo source stays framework-shaped; backend replacement types are not default diagnostics.",
        "backend_choice": "v0.10 executes compiler-owned generated user Rust harnesses through scheduler, filesystem, network, and Loom adapter surfaces.",
        "backend_fit": recommendations,
        "inspect_transparency": "use kobo inspect --sim for v0.10 facade and generated-harness transparency.",
        "executed": true,
        "scout": scout,
    })
}

fn scout_reasons(source: &str) -> Vec<&'static str> {
    let mut reasons = Vec::new();
    if source.contains("kobo::scenario") {
        reasons.push("scenario metadata");
    }
    if sim_model::profile_shape_for_source(source) == sim_model::TargetProfileShape::Network {
        reasons.push("network boundary");
    }
    if source.contains("tokio::spawn") || source.contains("spawn(") {
        reasons.push("async spawn boundary");
    }
    if source.contains("select!") {
        reasons.push("select cancellation boundary");
    }
    if source.contains("must_call") || contains_obligation_word(source) {
        reasons.push("liveness obligation vocabulary");
    }
    if contains_raw_nondeterminism(source) {
        reasons.push("raw nondeterminism");
    }
    reasons
}

fn backend_recommendations_for(source: &str) -> serde_json::Value {
    match sim_model::profile_shape_for_source(source) {
        sim_model::TargetProfileShape::Network => json!([
            {
                "name": "network",
                "backend_fit": "generated Rust loopback network harness",
                "executes_in_v10": true
            },
            {
                "name": "Loom",
                "backend_fit": "sync concurrency interleavings around network-facing state",
                "executes_in_v10": true
            },
            {
                "name": "Shuttle",
                "backend_fit": "async spawn/select schedule exploration around network-facing tasks",
                "executes_in_v10": true
            },
            {
                "name": "Turmoil",
                "backend_fit": "network schedule exploration through generated Rust loopback adapter",
                "executes_in_v10": true
            },
            {
                "name": "Madsim",
                "backend_fit": "distributed schedule exploration through generated Rust adapter",
                "executes_in_v10": true
            }
        ]),
        sim_model::TargetProfileShape::Async => json!([
            {"name": "Loom", "backend_fit": "sync concurrency interleavings"},
            {"name": "Shuttle", "backend_fit": "async spawn/select schedule exploration"}
        ]),
        sim_model::TargetProfileShape::StatefulInput => json!([
            {"name": "proptest", "backend_fit": "input and property exploration"}
        ]),
        sim_model::TargetProfileShape::Failpoint => json!([
            {"name": "failpoints", "backend_fit": "manual failure injection points"}
        ]),
        sim_model::TargetProfileShape::Sync => json!([
            {"name": "Loom", "backend_fit": "sync concurrency interleavings"}
        ]),
    }
}

fn scenario_or_function_name(source: &str) -> Option<String> {
    if let Some(name) = extract_scenario_name(source) {
        return Some(name);
    }

    source
        .lines()
        .map(str::trim)
        .find(|line| {
            line.starts_with("fn ")
                || line.starts_with("async fn ")
                || line.starts_with("pub fn ")
                || line.starts_with("pub async fn ")
        })
        .map(super::debt::extract_fn_name_from_line)
}

fn extract_scenario_name(source: &str) -> Option<String> {
    let marker = "scenario";
    let name_marker = "name";
    for line in source.lines().map(str::trim) {
        if !line.contains(marker) || !line.contains(name_marker) {
            continue;
        }
        let name_start = line.find(name_marker)?;
        let after_name = &line[name_start + name_marker.len()..];
        let quote_start = after_name.find('"')?;
        let rest = &after_name[quote_start + 1..];
        let quote_end = rest.find('"')?;
        return Some(rest[..quote_end].to_owned());
    }
    None
}

fn contains_raw_nondeterminism(source: &str) -> bool {
    source.contains("SystemTime::now")
        || source.contains("Instant::now")
        || source.contains("rand::")
        || source.contains("thread_rng")
        || source.contains("std::fs::")
        || source.contains("std::process::")
}

fn contains_obligation_word(source: &str) -> bool {
    [
        "commit", "rollback", "ack", "nack", "reply", "cancel", "finish", "abort",
    ]
    .iter()
    .any(|word| source.contains(word))
}

fn print_value(value: serde_json::Value, json_output: bool) -> anyhow::Result<()> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&value).context("failed to serialize scout output")?
        );
    } else {
        println!("{value}");
    }
    Ok(())
}
