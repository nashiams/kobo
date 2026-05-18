use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Context;
use kobo_ir::{GuaranteePolicy, GuaranteeProfile};
use serde_json::Value;

use crate::ErrorFormat;

use super::declarations;
use super::sim_model;
use super::witness_evidence;

pub(super) fn cmd_replay(
    file: &Path,
    error_format: ErrorFormat,
    roundtrip_metadata: bool,
) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;
    let witness: Value = serde_json::from_str(&source)
        .with_context(|| format!("failed to parse witness {}", file.display()))?;
    validate_witness(&witness)?;

    if roundtrip_metadata {
        println!(
            "{}",
            serde_json::to_string_pretty(&witness)
                .context("failed to serialize witness metadata")?
        );
        return Ok(());
    }

    if witness["schema_version"].as_u64() == Some(1) {
        return replay_v1(&witness, file, error_format);
    }

    let source_path = witness["source"]["path"].as_str().unwrap_or("<unknown>");
    let span_start = witness["source"]["span"]["start"].as_u64().unwrap_or(0);
    let span_end = witness["source"]["span"]["end"].as_u64().unwrap_or(0);
    let scenario = witness["scenario"].as_str().unwrap_or("<unknown>");
    let event_count = witness["events"].as_array().map(Vec::len).unwrap_or(0);

    println!("Replay witness metadata-only");
    println!("schema_version: 0");
    println!("source: {source_path} span {span_start}..{span_end}");
    println!("scenario: {scenario}");
    println!("events: {event_count}");
    println!(
        "metadata-only: legacy schema_version 0 witnesses do not carry v0.9 exact replay data"
    );
    Ok(())
}

struct VerifiedSource {
    path: PathBuf,
    hash: String,
    source: String,
}

fn replay_v1(
    witness: &Value,
    witness_path: &Path,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    let guarantee = witness["replay_guarantee"]
        .as_str()
        .unwrap_or("not_replayable");
    if guarantee != "exact" {
        let opaque = witness["opaque_boundaries"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let assumptions = witness["boundary_assumptions"].clone();
        replay_blocked(guarantee, &opaque, assumptions, error_format)?;
        anyhow::bail!("{guarantee} replay cannot claim exact replay");
    }

    validate_shrink_metadata(witness, error_format)?;
    validate_exact_witness_contract(witness, error_format)?;
    let verified_source = verify_source_identity(witness, witness_path, error_format)?;
    let target = witness_target_scenario(witness)?;
    let seed = witness["seed"].as_u64().unwrap_or(0);
    let profile = witness["backend_profile"]
        .as_str()
        .or_else(|| witness["guarantee_profile"].as_str())
        .unwrap_or("checked");
    let sim_profile = witness["sim_profile"].as_str().unwrap_or("quick");
    let inject = witness_injections(witness);
    let mut session = super::session::build_session(
        &verified_source.path,
        Some(GuaranteePolicy::for_profile(GuaranteeProfile::Checked)),
    )?;
    let artifacts = kobo_driver::run_codegen_pipeline(&mut session, &verified_source.path)
        .map_err(|()| anyhow::anyhow!("failed to rebuild compiler scenario artifacts"))?;
    let scenario_program = kobo_driver::build_scenario_program(
        &artifacts,
        &target,
        verified_source.hash.clone(),
        profile,
    )?;
    let options = kobo_sim_core::ScenarioOptions {
        sim_profile: sim_profile.to_owned(),
        profile: profile.to_owned(),
        seed,
        inject,
        event_budget: None,
    };
    let run = kobo_sim_core::run_full_depth_from_program(
        &scenario_program,
        &artifacts.rs_source,
        &options,
        kobo_sim_core::EngineMode::Both,
    )?;
    validate_ecosystem_boundary_evidence(
        witness,
        &verified_source.path,
        &session.config,
        &run,
        error_format,
    )?;
    validate_declaration_evidence(witness, &verified_source.path, &run, error_format)?;
    validate_removed_events_replay_safe(witness, &run.events, error_format)?;

    let source_display = witness["source"]["path"].as_str().unwrap_or("<unknown>");
    let inferred_obligations =
        witness_evidence::inferred_obligations_json(source_display, &verified_source.source, &run);
    let fuzz_enabled = witness["fuzz"]["enabled"].as_bool().unwrap_or(false);
    let expected = serde_json::json!({
        "backend": backend_for_profile(&run.profile),
        "backend_replay": replay_token(&verified_source.hash, seed, &run),
        "ecosystem_scope": ecosystem_scope(&run),
        "full_ecosystem_exploration": full_ecosystem_exploration(&run),
        "replay_contract": replay_contract_json(&run),
        "execution_digest": execution_digest_json(&run),
        "harness_manifest": run.harness_manifest.clone(),
        "operation_coverage": witness_evidence::operation_coverage_json(&scenario_program, &run),
        "function_summaries": witness_evidence::function_summaries_json(&scenario_program, &run),
        "call_graph_obligation_summaries": witness_evidence::call_graph_obligation_summaries_json(&scenario_program, &run),
        "replay_grade": witness_evidence::replay_grade_json(&run, fuzz_enabled),
        "boundary_ledger": witness_evidence::boundary_ledger_json(&scenario_program, &run),
        "ecosystem_boundaries": ecosystem_boundaries_json(&verified_source.path, &session.config, &run),
        "declarations": declarations_json(&verified_source.path, &run),
        "inferred_obligations": inferred_obligations.clone(),
        "lifecycle_inference": {
            "mode": "observe",
            "source": "scenario_program",
            "template_version": "v0.10.1",
            "obligations": inferred_obligations,
        },
        "failure": failure_json(source_display, &verified_source.source, &run),
        "events": replay_events_json(witness, &run.events)?,
    });
    let observed = serde_json::json!({
        "backend": witness["backend"].clone(),
        "backend_replay": witness["backend_replay"].clone(),
        "ecosystem_scope": witness["ecosystem_scope"].clone(),
        "full_ecosystem_exploration": witness["full_ecosystem_exploration"].clone(),
        "replay_contract": witness["replay_contract"].clone(),
        "execution_digest": witness["execution_digest"].clone(),
        "harness_manifest": witness["harness_manifest"].clone(),
        "operation_coverage": witness["operation_coverage"].clone(),
        "function_summaries": witness["function_summaries"].clone(),
        "call_graph_obligation_summaries": witness["call_graph_obligation_summaries"].clone(),
        "replay_grade": witness["replay_grade"].clone(),
        "boundary_ledger": witness["boundary_ledger"].clone(),
        "ecosystem_boundaries": witness["ecosystem_boundaries"].clone(),
        "declarations": witness["declarations"].clone(),
        "inferred_obligations": witness["inferred_obligations"].clone(),
        "lifecycle_inference": witness["lifecycle_inference"].clone(),
        "failure": witness_failure_json(witness),
        "events": witness["events"].clone(),
    });
    if expected != observed {
        return replay_divergence(expected, observed, error_format);
    }

    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "replay": "exact",
            "source": verified_source.path.display().to_string(),
            "backend": backend_for_profile(&run.profile),
            "failure": witness["failure"],
            "events": run.events.len(),
        }))?
    );
    Ok(())
}

fn validate_shrink_metadata(witness: &Value, error_format: ErrorFormat) -> anyhow::Result<()> {
    if witness["exactness"].as_str() != Some("exact") {
        return Ok(());
    }
    if witness["shrink"].is_null() {
        return Ok(());
    }
    if witness["shrink"]["replay_checked"].as_bool() == Some(false) {
        let payload = serde_json::json!({
            "code": "K0106",
            "message": "witness shrink is unsafe: replay_checked=false cannot replace an exact witness",
            "shrink": witness["shrink"].clone(),
        });
        emit_replay_issue(&payload, error_format)?;
        anyhow::bail!("K0106 witness shrink is unsafe");
    }
    let Some(removed_ids) = witness["shrink"]["removed_event_ids"].as_array() else {
        return Ok(());
    };
    let original_event_count = witness["shrink"]["original_event_count"].as_u64();
    let shrunk_event_count = witness["shrink"]["shrunk_event_count"].as_u64();
    if original_event_count
        .zip(shrunk_event_count)
        .is_some_and(|(original, shrunk)| {
            original < shrunk || original - shrunk != removed_ids.len() as u64
        })
    {
        let payload = serde_json::json!({
            "code": "K0106",
            "message": "witness shrink metadata does not match event counts",
            "shrink": witness["shrink"].clone(),
        });
        emit_replay_issue(&payload, error_format)?;
        anyhow::bail!("K0106 witness shrink metadata is inconsistent");
    }
    if removed_ids.iter().any(|id| id.as_u64().is_none()) {
        let payload = serde_json::json!({
            "code": "K0106",
            "message": "witness shrink removed_event_ids must be numeric",
            "shrink": witness["shrink"].clone(),
        });
        emit_replay_issue(&payload, error_format)?;
        anyhow::bail!("K0106 witness shrink metadata is invalid");
    }
    Ok(())
}

fn verify_source_identity(
    witness: &Value,
    witness_path: &Path,
    error_format: ErrorFormat,
) -> anyhow::Result<VerifiedSource> {
    let Some(source_path) = witness["source"]["path"].as_str() else {
        return source_mismatch("missing source identity", error_format);
    };
    let Some(expected_hash) = witness["source"]["hash"].as_str() else {
        return source_mismatch("missing source hash", error_format);
    };
    let witness_dir = witness_path.parent().unwrap_or_else(|| Path::new("."));
    let source_path = sim_model::resolve_witness_source(source_path, witness_dir);
    let source = match std::fs::read_to_string(&source_path) {
        Ok(source) => source,
        Err(_) => {
            return source_mismatch(
                &format!("source mismatch: missing {}", source_path.display()),
                error_format,
            )
        }
    };
    let observed_hash = sim_model::source_hash(&source);
    if observed_hash != expected_hash {
        return source_mismatch("source mismatch: witness source hash changed", error_format);
    }
    Ok(VerifiedSource {
        path: source_path,
        hash: observed_hash,
        source,
    })
}

fn witness_target_scenario(witness: &Value) -> anyhow::Result<String> {
    let Some(target) = witness["target"].as_str() else {
        return witness_error("missing target");
    };
    let Some((_, scenario)) = target.rsplit_once(':') else {
        return witness_error("target must include source:path:scenario");
    };
    if scenario.trim().is_empty() {
        return witness_error("target scenario must not be empty");
    }
    Ok(scenario.to_owned())
}

fn failure_json(source_path: &str, source: &str, run: &kobo_sim_core::FullDepthRun) -> Value {
    let Some(failure) = run.failure.as_ref() else {
        return Value::Null;
    };
    serde_json::json!({
        "code": failure.code.as_str(),
        "primary_span": format!(
            "{}:{}:1",
            source_path,
            one_based_line_for_offset(source, failure.primary_start),
        ),
        "related_spans": related_spans_json(source_path, source, run),
    })
}

fn witness_failure_json(witness: &Value) -> Value {
    if witness["failure"].is_null() {
        return Value::Null;
    }
    serde_json::json!({
        "code": witness["failure"]["code"].clone(),
        "primary_span": witness["failure"]["primary_span"].clone(),
        "related_spans": witness["failure"]["related_spans"].clone(),
    })
}

fn execution_digest_json(run: &kobo_sim_core::FullDepthRun) -> Value {
    serde_json::json!({
        "engine": "semantic-sim",
        "semantic_engine": run.digest.semantic_engine.as_str(),
        "harness_engine": run.digest.harness_engine.as_str(),
        "model_version": run.digest.model_version.as_str(),
        "scenario_ir_hash": run.digest.scenario_ir_hash.as_str(),
        "operation_count": run.digest.operation_count,
        "event_hash": run.digest.semantic_trace_hash.as_str(),
        "semantic_trace_hash": run.digest.semantic_trace_hash.as_str(),
        "harness_trace_hash": run.digest.harness_trace_hash.as_str(),
        "fuzz_driver_trace_hash": run.digest.fuzz_driver_trace_hash.as_deref(),
        "fuzz_driver_event_count": run.digest.fuzz_driver_event_count,
        "agreement": run.digest.agreement.as_str(),
        "generated_rust_hash": run.digest.generated_rust_hash.as_deref(),
        "harness_manifest_hash": run.digest.harness_manifest_hash.as_deref(),
        "harness_exit_code": run.digest.harness_exit_code,
        "harness_event_count": run.digest.harness_event_count,
    })
}

fn replay_contract_json(run: &kobo_sim_core::FullDepthRun) -> Value {
    serde_json::json!({
        "scope": ecosystem_scope(run),
        "full_ecosystem_exploration": full_ecosystem_exploration(run),
        "facades": run
            .harness_manifest
            .as_ref()
            .map(|manifest| manifest.facades.clone())
            .unwrap_or_default(),
        "semantic_engine": run.digest.semantic_engine.as_str(),
        "harness_engine": run.digest.harness_engine.as_str(),
        "agreement": run.digest.agreement.as_str(),
    })
}

fn ecosystem_scope(run: &kobo_sim_core::FullDepthRun) -> String {
    run.harness_manifest
        .as_ref()
        .map(|manifest| manifest.execution_scope.clone())
        .unwrap_or_else(|| "semantic-only".to_owned())
}

fn validate_ecosystem_boundary_evidence(
    witness: &Value,
    file: &Path,
    config: &kobo_driver::KoboConfig,
    run: &kobo_sim_core::FullDepthRun,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    let expected = ecosystem_boundaries_json(file, config, run);
    if let Some(boundaries) = witness["ecosystem_boundaries"].as_array() {
        if boundaries.iter().any(|boundary| {
            boundary["policy"].as_str() == Some("record")
                && boundary["evidence"].as_str() != Some("recorded-event")
        }) {
            let payload = serde_json::json!({
                "code": "K0124",
                "message": "record boundary is missing recorded event evidence",
                "expected": expected,
                "observed": witness["ecosystem_boundaries"].clone(),
            });
            emit_replay_issue(&payload, error_format)?;
            anyhow::bail!("K0124 record boundary missing recorded evidence");
        }
    }
    if witness["ecosystem_boundaries"] == expected {
        return Ok(());
    }
    let payload = serde_json::json!({
        "code": "K0129",
        "message": "ecosystem boundary evidence changed; exact replay would overclaim external coverage",
        "expected": expected,
        "observed": witness["ecosystem_boundaries"].clone(),
    });
    emit_replay_issue(&payload, error_format)?;
    anyhow::bail!("K0129 ecosystem boundary evidence changed")
}

fn validate_declaration_evidence(
    witness: &Value,
    file: &Path,
    run: &kobo_sim_core::FullDepthRun,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    let expected = declarations_json(file, run);
    if witness["declarations"] == expected {
        return Ok(());
    }
    let payload = serde_json::json!({
        "code": "K0129",
        "message": "declaration evidence changed; exact replay would overclaim typed boundary coverage",
        "expected": expected,
        "observed": witness["declarations"].clone(),
    });
    emit_replay_issue(&payload, error_format)?;
    anyhow::bail!("K0129 declaration evidence changed")
}

fn ecosystem_boundaries_json(
    file: &Path,
    config: &kobo_driver::KoboConfig,
    run: &kobo_sim_core::FullDepthRun,
) -> Value {
    Value::Array(
        run.boundary_decisions
            .iter()
            .map(|decision| {
                let evidence = boundary_evidence_for_policy(
                    decision.policy.as_str(),
                    &decision.crate_name,
                    run,
                );
                serde_json::json!({
                    "crate": decision.crate_name,
                    "call_path": decision.call_path,
                    "policy": decision.policy.as_str(),
                    "reason": decision.reason,
                    "evidence": evidence,
                    "source_span": {
                        "start": decision.span_start,
                        "end": decision.span_end,
                    },
                    "declaration": declaration_metadata_for_boundary(file, run, &decision.crate_name),
                    "adapter": config.ecosystem_policy.adapter_for(&decision.crate_name).map(|adapter| serde_json::json!({
                        "package": adapter.package,
                        "reason": adapter.reason,
                    })),
                    "full_ecosystem_exploration": false,
                })
            })
            .collect(),
    )
}

fn boundary_evidence_for_policy(
    policy: &str,
    crate_name: &str,
    run: &kobo_sim_core::FullDepthRun,
) -> &'static str {
    match policy {
        "record"
            if run.events.iter().any(|event| {
                event.kind == "boundary-record" && event.label.as_deref() == Some(crate_name)
            }) =>
        {
            "recorded-event"
        }
        "activity"
            if run.events.iter().any(|event| {
                event.kind == "boundary-activity" && event.label.as_deref() == Some(crate_name)
            }) =>
        {
            "activity-result"
        }
        "model" => "modeled-facade",
        "typed" => "declaration",
        "stub" => "scenario-stub",
        "outside" => "outside-assumption",
        "opaque" | "debt" => "assumption",
        _ => "unverified",
    }
}

fn declarations_json(file: &Path, run: &kobo_sim_core::FullDepthRun) -> Value {
    Value::Array(
        run.boundary_decisions
            .iter()
            .filter_map(|decision| {
                if decision.policy.as_str() != "typed" {
                    return None;
                }
                let path = declarations::declaration_path_for(file, &decision.crate_name)?;
                let source = std::fs::read_to_string(&path).ok()?;
                let parsed = source.parse::<toml::Value>().ok()?;
                let schema_version = parsed
                    .get("schema_version")
                    .and_then(toml::Value::as_integer)
                    .unwrap_or(0);
                let version = parsed
                    .get("crate")
                    .and_then(toml::Value::as_table)
                    .and_then(|table| table.get("version"))
                    .and_then(toml::Value::as_str)
                    .unwrap_or("unknown");
                let hash = declarations::stable_hash(&source);
                Some(serde_json::json!({
                    "crate": decision.crate_name,
                    "path": path.display().to_string(),
                    "version": version,
                    "schema_version": schema_version,
                    "hash": hash,
                    "declaration_version": version,
                    "declaration_hash": hash,
                }))
            })
            .collect(),
    )
}

fn declaration_metadata_for_boundary(
    file: &Path,
    run: &kobo_sim_core::FullDepthRun,
    crate_name: &str,
) -> Option<Value> {
    run.boundary_decisions
        .iter()
        .find(|decision| decision.crate_name == crate_name && decision.policy.as_str() == "typed")
        .and_then(|_| {
            let path = declarations::declaration_path_for(file, crate_name)?;
            let source = std::fs::read_to_string(&path).ok()?;
            let parsed = source.parse::<toml::Value>().ok()?;
            let schema_version = parsed
                .get("schema_version")
                .and_then(toml::Value::as_integer)
                .unwrap_or(0);
            let version = parsed
                .get("crate")
                .and_then(toml::Value::as_table)
                .and_then(|table| table.get("version"))
                .and_then(toml::Value::as_str)
                .unwrap_or("unknown");
            Some(serde_json::json!({
                "path": path.display().to_string(),
                "version": version,
                "schema_version": schema_version,
                "hash": declarations::stable_hash(&source),
            }))
        })
}

fn full_ecosystem_exploration(run: &kobo_sim_core::FullDepthRun) -> bool {
    run.harness_manifest
        .as_ref()
        .is_some_and(|manifest| manifest.full_ecosystem_exploration)
}

fn related_spans_json(
    source_path: &str,
    source: &str,
    run: &kobo_sim_core::FullDepthRun,
) -> Vec<Value> {
    run.obligations
        .iter()
        .filter(|obligation| !obligation.is_discharged)
        .map(|obligation| {
            serde_json::json!({
                "label": format!("obligation `{}` declared here", obligation.binding),
                "span": {
                    "path": source_path,
                    "line": one_based_line_for_offset(source, obligation.declaration_span.0),
                    "start": obligation.declaration_span.0,
                    "end": obligation.declaration_span.1.max(obligation.declaration_span.0 + 1),
                },
            })
        })
        .collect()
}

fn replay_events_json(
    witness: &Value,
    events: &[kobo_sim_core::ScenarioEvent],
) -> anyhow::Result<Vec<Value>> {
    let removed = removed_event_ids(witness)?;
    let filtered = events
        .iter()
        .enumerate()
        .filter(|(index, _)| !removed.contains(index))
        .map(|(_, event)| event)
        .collect::<Vec<_>>();
    Ok(filtered
        .iter()
        .enumerate()
        .map(|(id, event)| {
            serde_json::json!({
                "id": id,
                "kind": event.kind,
                "label": event.label,
                "value": event.value,
            })
        })
        .collect())
}

fn removed_event_ids(witness: &Value) -> anyhow::Result<BTreeSet<usize>> {
    let Some(values) = witness["shrink"]["removed_event_ids"].as_array() else {
        return Ok(BTreeSet::new());
    };
    values
        .iter()
        .map(|value| {
            value
                .as_u64()
                .map(|id| id as usize)
                .ok_or_else(|| anyhow::anyhow!("removed_event_ids must be numeric"))
        })
        .collect()
}

fn validate_removed_events_replay_safe(
    witness: &Value,
    events: &[kobo_sim_core::ScenarioEvent],
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    for removed_id in removed_event_ids(witness)? {
        let Some(event) = events.get(removed_id) else {
            let payload = serde_json::json!({
                "code": "K0106",
                "message": "witness shrink removed_event_ids references a missing event",
                "removed_event_id": removed_id,
            });
            emit_replay_issue(&payload, error_format)?;
            anyhow::bail!("K0106 witness shrink removed a missing event");
        };
        if !is_replay_irrelevant_event(&event.kind) {
            let payload = serde_json::json!({
                "code": "K0106",
                "message": "witness shrink removed replay-critical semantic evidence",
                "removed_event_id": removed_id,
                "event": {
                    "kind": event.kind,
                    "label": event.label,
                    "value": event.value,
                },
            });
            emit_replay_issue(&payload, error_format)?;
            anyhow::bail!("K0106 witness shrink removed semantic evidence");
        }
    }
    Ok(())
}

fn is_replay_irrelevant_event(kind: &str) -> bool {
    matches!(
        kind,
        "scheduler-pct-seed" | "network-delayed" | "network-reordered" | "fuzz-shrink-candidate"
    )
}

fn witness_injections(witness: &Value) -> Option<String> {
    witness["injections"]["hooks"]
        .as_array()
        .map(|hooks| {
            hooks
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(",")
        })
        .filter(|hooks| !hooks.is_empty())
}

fn replay_token(source_identity: &str, seed: u64, run: &kobo_sim_core::FullDepthRun) -> String {
    let mut material = String::new();
    material.push_str(source_identity);
    material.push(':');
    material.push_str(&seed.to_string());
    material.push(':');
    material.push_str(backend_for_profile(&run.profile));
    material.push(':');
    material.push_str(&run.digest.semantic_trace_hash);
    material.push(':');
    material.push_str(&run.digest.harness_trace_hash);
    if let Some(fuzz_driver_trace_hash) = run.digest.fuzz_driver_trace_hash.as_deref() {
        material.push(':');
        material.push_str(fuzz_driver_trace_hash);
    }
    kobo_sim_core::digest::stable_hash(&material)
}

fn backend_for_profile(profile: &str) -> &'static str {
    match profile {
        "sync" => "loom",
        "stateful-input" => "proptest",
        "failpoint" => "failpoints",
        "network" | "network-design" => "turmoil",
        "distributed" | "madsim" => "madsim",
        _ => "shuttle",
    }
}

fn validate_exact_witness_contract(
    witness: &Value,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    let unsupported = witness["coverage"]["unsupported_constructs"]
        .as_array()
        .map(Vec::is_empty)
        .unwrap_or(true);
    if !unsupported {
        let payload = serde_json::json!({
            "code": "K0116",
            "message": "scenario coverage incomplete; exact replay is not allowed",
            "coverage": witness["coverage"].clone(),
        });
        emit_replay_issue(&payload, error_format)?;
        anyhow::bail!("K0116 scenario coverage incomplete");
    }
    let digest = &witness["execution_digest"];
    let semantic_engine = digest["semantic_engine"].as_str();
    let harness_engine = digest["harness_engine"].as_str();
    let agreement = digest["agreement"].as_str();
    let has_generated_harness =
        harness_engine.is_some_and(|engine| engine.starts_with("generated-rust"));
    if semantic_engine != Some("driver-kir-scenario")
        || !has_generated_harness
        || agreement != Some("matched")
    {
        let payload = serde_json::json!({
            "code": "K0117",
            "message": "semantic trace and harness trace diverged or are missing",
            "execution_digest": digest.clone(),
        });
        emit_replay_issue(&payload, error_format)?;
        anyhow::bail!("K0117 exact witness lacks semantic/harness agreement");
    }
    validate_exact_scope_contract(witness, error_format)?;
    Ok(())
}

fn validate_exact_scope_contract(witness: &Value, error_format: ErrorFormat) -> anyhow::Result<()> {
    let scope = witness["replay_contract"]["scope"].as_str();
    let top_level_scope = witness["ecosystem_scope"].as_str();
    let full_ecosystem = witness["replay_contract"]["full_ecosystem_exploration"].as_bool();
    let top_level_full_ecosystem = witness["full_ecosystem_exploration"].as_bool();
    let manifest_scope = witness["harness_manifest"]["execution_scope"].as_str();
    let manifest_full_ecosystem =
        witness["harness_manifest"]["full_ecosystem_exploration"].as_bool();
    let supported_scope = scope.is_some_and(|scope| {
        matches!(
            scope,
            "generated-user-rust"
                | "generated-user-rust-loom"
                | "generated-user-rust-adapter"
                | "generated-user-rust-os-facade"
        )
    });
    if !supported_scope
        || scope != top_level_scope
        || scope != manifest_scope
        || full_ecosystem != Some(false)
        || top_level_full_ecosystem != Some(false)
        || manifest_full_ecosystem != Some(false)
    {
        let payload = serde_json::json!({
            "code": "K0117",
            "message": "exact witness must disclose compiler-owned replay scope and must not claim full ecosystem exploration",
            "replay_contract": witness["replay_contract"].clone(),
            "ecosystem_scope": witness["ecosystem_scope"].clone(),
            "full_ecosystem_exploration": witness["full_ecosystem_exploration"].clone(),
            "harness_manifest": witness["harness_manifest"].clone(),
        });
        emit_replay_issue(&payload, error_format)?;
        anyhow::bail!("K0117 exact witness lacks replay scope contract");
    }
    Ok(())
}

fn one_based_line_for_offset(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn replay_divergence(
    expected: Value,
    observed: Value,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    let payload = serde_json::json!({
        "code": "K0104",
        "message": "kwit replay diverged",
        "expected": expected,
        "observed": observed,
    });
    emit_replay_issue(&payload, error_format)?;
    anyhow::bail!("K0104 replay divergence")
}

fn source_mismatch<T>(message: &str, error_format: ErrorFormat) -> anyhow::Result<T> {
    let payload = serde_json::json!({
        "code": "K0104",
        "message": message,
    });
    emit_replay_issue(&payload, error_format)?;
    anyhow::bail!("{message}")
}

fn replay_blocked(
    guarantee: &str,
    opaque_boundaries: &str,
    assumptions: Value,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    let payload = serde_json::json!({
        "code": "K0104",
        "message": format!("{guarantee} replay witness cannot claim exact replay"),
        "replay_guarantee": guarantee,
        "opaque_boundaries": opaque_boundaries,
        "boundary_assumptions": assumptions,
    });
    emit_replay_issue(&payload, error_format)
}

fn emit_replay_issue(payload: &Value, error_format: ErrorFormat) -> anyhow::Result<()> {
    match error_format {
        ErrorFormat::Json => {
            println!("{}", serde_json::to_string(payload)?);
        }
        ErrorFormat::Human => {
            eprintln!(
                "error[{}]: {}",
                payload["code"].as_str().unwrap_or("K0104"),
                payload["message"].as_str().unwrap_or("replay failed"),
            );
            if let Some(guarantee) = payload["replay_guarantee"].as_str() {
                eprintln!("replay guarantee: {guarantee}");
            }
            if let Some(boundaries) = payload["opaque_boundaries"].as_str() {
                if !boundaries.is_empty() {
                    eprintln!("opaque boundaries: {boundaries}");
                }
            }
            if !payload["boundary_assumptions"].is_null() {
                eprintln!("boundary assumptions: {}", payload["boundary_assumptions"]);
            }
            if !payload["expected"].is_null() {
                eprintln!("expected: {}", payload["expected"]);
            }
            if !payload["observed"].is_null() {
                eprintln!("observed: {}", payload["observed"]);
            }
        }
    }
    Ok(())
}

fn validate_witness(witness: &Value) -> anyhow::Result<()> {
    let Some(version) = witness["schema_version"].as_u64() else {
        return witness_error("missing schema_version");
    };
    if version == 1 {
        let mut required = vec![
            &["kobo_version"][..],
            &["target"][..],
            &["guarantee_profile"][..],
            &["seed"][..],
            &["backend_profile"][..],
            &["backend"][..],
            &["backend_replay"][..],
            &["replay_guarantee"][..],
            &["expanded_policy", "ownership"][..],
            &["expanded_policy", "liveness"][..],
            &["expanded_policy", "replay"][..],
            &["expanded_policy", "boundaries"][..],
            &["expanded_policy", "errors"][..],
            &["modeled_boundaries"][..],
            &["opaque_boundaries"][..],
            &["boundary_assumptions"][..],
            &["obligations"][..],
            &["boundary_decisions"][..],
            &["failure"][..],
            &["events"][..],
        ];
        if !witness["failure"].is_null() {
            required.push(&["failure", "code"][..]);
            required.push(&["failure", "primary_span"][..]);
            required.push(&["failure", "related_spans"][..]);
        }
        if witness["replay_guarantee"].as_str() == Some("exact") {
            required.push(&["source", "path"][..]);
            required.push(&["source", "hash"][..]);
            required.push(&["ecosystem_scope"][..]);
            required.push(&["full_ecosystem_exploration"][..]);
            required.push(&["replay_contract", "scope"][..]);
            required.push(&["replay_contract", "full_ecosystem_exploration"][..]);
            required.push(&["replay_contract", "facades"][..]);
            required.push(&["execution_digest", "engine"][..]);
            required.push(&["execution_digest", "model_version"][..]);
            required.push(&["execution_digest", "scenario_ir_hash"][..]);
            required.push(&["execution_digest", "operation_count"][..]);
            required.push(&["execution_digest", "event_hash"][..]);
            required.push(&["execution_digest", "generated_rust_hash"][..]);
            required.push(&["execution_digest", "harness_manifest_hash"][..]);
            required.push(&["execution_digest", "harness_exit_code"][..]);
            required.push(&["harness_manifest", "execution_scope"][..]);
            required.push(&["harness_manifest", "full_ecosystem_exploration"][..]);
            required.push(&["harness_manifest", "facades"][..]);
            required.push(&["harness_manifest", "harness_rs_path"][..]);
            required.push(&["harness_manifest", "stdout_hash"][..]);
            required.push(&["operation_coverage", "modeled"][..]);
            required.push(&["function_summaries"][..]);
            required.push(&["call_graph_obligation_summaries"][..]);
            required.push(&["replay_grade"][..]);
            required.push(&["boundary_ledger"][..]);
            required.push(&["ecosystem_boundaries"][..]);
            required.push(&["inferred_obligations"][..]);
            required.push(&["lifecycle_inference", "mode"][..]);
        }
        for path in required {
            if value_at(witness, path).is_none() {
                return witness_error(&format!("missing {}", path.join(".")));
            }
        }
        return Ok(());
    }
    if version != 0 {
        return witness_error("unsupported schema_version");
    }

    if witness["source"]["path"].as_str().is_none() {
        return witness_error("missing source.path");
    }
    if witness["source"]["span"]["start"].as_u64().is_none()
        || witness["source"]["span"]["end"].as_u64().is_none()
    {
        return witness_error("missing source.span");
    }

    Ok(())
}

fn value_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

fn witness_error<T>(message: &str) -> anyhow::Result<T> {
    eprintln!("error[K0115]: {message}");
    eprintln!("help: .kwit schema_version 0 requires source.path and source.span");
    anyhow::bail!("invalid witness")
}
