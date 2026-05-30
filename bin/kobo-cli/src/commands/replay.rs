use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Context;
use kobo_ir::{GuaranteePolicy, GuaranteeProfile, ScenarioProgram};
use serde_json::Value;

use crate::ErrorFormat;

use super::formal_core;
use super::sim_model;
use super::test_cmd;
use super::witness_evidence;
use super::{declarations, summary_validation};

pub(super) fn cmd_replay(
    file: &Path,
    error_format: ErrorFormat,
    roundtrip_metadata: bool,
    backend_native: bool,
) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;
    let witness: Value = serde_json::from_str(&source)
        .with_context(|| format!("failed to parse witness {}", file.display()))?;
    validate_witness(&witness)?;
    if backend_native {
        validate_backend_native_replay(&witness)?;
        eprintln!("Loom backend replay evidence accepted; running Kobo replay validation");
    }

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
        "metadata-only: legacy schema_version 0 witnesses do not carry exact replay event history"
    );
    Ok(())
}

fn validate_backend_native_replay(witness: &Value) -> anyhow::Result<()> {
    let backend = witness["backend"].as_str();
    let profile = witness["backend_profile"].as_str();
    let guarantee = witness["replay_guarantee"].as_str();
    let harness_engine = witness["execution_digest"]["harness_engine"].as_str();
    if backend == Some("loom")
        && profile == Some("sync")
        && guarantee == Some("exact")
        && harness_engine == Some("generated-rust-loom-process")
    {
        validate_backend_native_controls(witness)?;
        return Ok(());
    }
    anyhow::bail!(
        "unsupported backend option: --backend-native replay is only supported for exact Loom witnesses; use normal `kobo replay`, keep the inspected backend-native harness, or mark unsupported knobs as scenario debt"
    )
}

fn validate_backend_native_controls(witness: &Value) -> anyhow::Result<()> {
    let controls = &witness["backend_controls"];
    if controls["backend"].as_str() != Some("loom") {
        anyhow::bail!(
            "unsupported backend option: backend_controls.backend must be `loom` for --backend-native replay; replay without backend-native controls or mark unsupported knobs as scenario debt"
        );
    }
    if controls["backend_native"].as_bool() != Some(true) {
        anyhow::bail!(
            "unsupported backend option: backend_controls.backend_native must be true for --backend-native replay; mark unsupported knobs as scenario debt or replay without backend-native controls"
        );
    }
    if let Some(scheduler) = controls["scheduler"].as_str() {
        if scheduler != "exhaustive" {
            anyhow::bail!(
                "unsupported backend option: backend_controls.scheduler `{scheduler}` is not linked for Loom replay; replay without backend-native controls or mark unsupported knobs as scenario debt"
            );
        }
    }
    if !(controls["max_branches"].is_null() || controls["max_branches"].as_u64().is_some()) {
        anyhow::bail!(
            "unsupported backend option: backend_controls.max_branches must be a positive integer or null for --backend-native replay"
        );
    }
    if controls["replay_token"].as_str() != Some("record") {
        anyhow::bail!(
            "unsupported backend option: backend_controls.replay_token must be `record` for --backend-native replay; mark unsupported knobs as scenario debt or replay without backend-native controls"
        );
    }
    let Some(token) = witness["backend_replay_token"].as_str() else {
        anyhow::bail!(
            "unsupported backend option: backend_replay_token is required for --backend-native replay"
        );
    };
    if token.trim().is_empty() || witness["backend_replay"].as_str() != Some(token) {
        anyhow::bail!(
            "unsupported backend option: backend_replay_token must match backend_replay for --backend-native replay"
        );
    }
    let evidence = &witness["backend_replay_evidence"];
    if evidence["backend"].as_str() != Some("loom") {
        anyhow::bail!(
            "unsupported backend option: backend_replay_evidence.backend must be `loom` for --backend-native replay"
        );
    }
    if evidence["source"].as_str() != Some("generated-loom-harness") {
        anyhow::bail!(
            "unsupported backend option: backend_replay_evidence.source must be `generated-loom-harness` for --backend-native replay"
        );
    }
    let Some(harness_replay_id) = evidence["harness_replay_id"].as_str() else {
        anyhow::bail!(
            "unsupported backend option: backend_replay_evidence.harness_replay_id is required for --backend-native replay"
        );
    };
    if !harness_replay_id.starts_with("loom-harness:") || harness_replay_id != token {
        anyhow::bail!(
            "unsupported backend option: backend_replay_evidence.harness_replay_id must match backend_replay for --backend-native replay"
        );
    }
    let Some(kobo_hash) = evidence["kobo_verification_hash"].as_str() else {
        anyhow::bail!(
            "unsupported backend option: backend_replay_evidence.kobo_verification_hash is required for --backend-native replay"
        );
    };
    if kobo_hash == harness_replay_id {
        anyhow::bail!(
            "unsupported backend option: backend_replay_evidence must keep generated harness replay IDs separate from Kobo verification hashes"
        );
    }
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
    validate_exact_witness_scope(witness, error_format)?;
    validate_checkpoint_artifact_available(witness, error_format)?;
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
        event_budget: replay_event_budget(witness),
        scheduler: kobo_sim_core::SchedulerPolicy::from_name(
            witness["backend_controls"]["scheduler"].as_str(),
        ),
        loom_max_branches: witness["backend_controls"]["max_branches"].as_u64(),
        loom_checkpoint_replay: witness["backend_controls"]["checkpoint_replay"]
            .as_bool()
            .unwrap_or(false),
    };
    let run = run_replay_seed_portfolio(
        &scenario_program,
        &artifacts.rs_source,
        &options,
        replay_seed_count(witness),
    )?;
    validate_ecosystem_boundary_evidence(
        witness,
        &verified_source.path,
        &session.config,
        &run,
        error_format,
    )?;
    validate_declaration_evidence(
        witness,
        &verified_source.path,
        &session.config,
        &run,
        error_format,
    )?;
    validate_removed_events_replay_safe(witness, &run.events, error_format)?;
    validate_checkpoint_replay_metadata(witness, &run, error_format)?;

    let source_display = witness["source"]["path"].as_str().unwrap_or("<unknown>");
    let inferred_obligations = witness_evidence::inferred_obligations_json(
        source_display,
        &verified_source.source,
        &scenario_program,
        &run,
    );
    let fuzz_enabled = witness["fuzz"]["enabled"].as_bool().unwrap_or(false);
    let summaries = summary_usage_json(&session.config, &scenario_program)?;
    let formal_core =
        formal_core::formal_core_json(source_display, &verified_source.source, &scenario_program);
    let proof_seed = formal_core::proof_seed_json(
        source_display,
        &verified_source.source,
        &scenario_program,
        &run,
    );
    let strict_liveness = formal_core::strict_liveness_json(
        source_display,
        &verified_source.source,
        &scenario_program,
        &run,
    );
    let trace_checks = test_cmd::trace_checks_json(source_display, &verified_source.source, &run);
    let model_vs_implementation =
        test_cmd::model_vs_implementation_json(source_display, &verified_source.source, seed, &run);
    let flagship_demo = test_cmd::flagship_demo_json(&scenario_program, &run);
    let runtime_profile = runtime_profile_json(&session.config, sim_profile, seed, &run);
    let runtime_profile_hash =
        kobo_sim_core::digest::stable_hash(&serde_json::to_string(&runtime_profile)?);
    let replay_token_mode = witness["backend_controls"]["replay_token"]
        .as_str()
        .unwrap_or("record");
    let expected = serde_json::json!({
        "backend": backend_for_profile(&run.profile),
        "backend_version": backend_version_json(backend_for_profile(&run.profile)),
        "backend_replay": test_cmd::backend_replay_id(
            replay_token_mode,
            &verified_source.hash,
            seed,
            &run,
        ),
        "backend_replay_evidence": test_cmd::backend_replay_evidence_json(
            replay_token_mode,
            &verified_source.hash,
            seed,
            &run,
        ),
        "checkpoint_replay": checkpoint_replay_json(
            witness["backend_controls"]["checkpoint_replay"]
                .as_bool()
                .unwrap_or(false),
            &run,
        ),
        "ecosystem_scope": ecosystem_scope(&run),
        "full_ecosystem_exploration": full_ecosystem_exploration(&run),
        "replay_contract": replay_scope_json(&run),
        "runtime_profile": runtime_profile,
        "execution_digest": execution_digest_json(&run, &runtime_profile_hash),
        "harness_manifest": run.harness_manifest.clone(),
        "operation_coverage": witness_evidence::operation_coverage_json(&scenario_program, &run),
        "function_summaries": witness_evidence::function_summaries_json(&scenario_program, &run),
        "call_graph_obligation_summaries": witness_evidence::call_graph_obligation_summaries_json(&scenario_program, &run),
        "formal_core": formal_core,
        "proof_seed": proof_seed,
        "strict_liveness": strict_liveness,
        "invariant_checks": trace_checks["invariant_checks"].clone(),
        "temporal_checks": trace_checks["temporal_checks"].clone(),
        "model_vs_implementation": model_vs_implementation,
        "flagship_demo": flagship_demo,
        "replay_grade": witness_evidence::replay_grade_json(&run, fuzz_enabled),
        "boundary_ledger": witness_evidence::boundary_ledger_json(&scenario_program, &run),
        "ecosystem_boundaries": ecosystem_boundaries_json(&verified_source.path, &session.config, &run),
        "declarations": declarations_json(&verified_source.path, &session.config, &run),
        "summaries": summaries,
        "inferred_obligations": inferred_obligations.clone(),
        "lifecycle_inference": {
            "mode": "observe",
            "source": "scenario_program",
            "template_schema": "lifecycle-template",
            "schema_version": 1,
            "obligations": inferred_obligations,
        },
        "failure": failure_json(source_display, &verified_source.source, &run),
        "events": replay_events_json(witness, &run.events)?,
    });
    let observed = serde_json::json!({
        "backend": witness["backend"].clone(),
        "backend_version": witness["backend_version"].clone(),
        "backend_replay": witness["backend_replay"].clone(),
        "backend_replay_evidence": witness["backend_replay_evidence"].clone(),
        "checkpoint_replay": witness["checkpoint_replay"].clone(),
        "ecosystem_scope": witness["ecosystem_scope"].clone(),
        "full_ecosystem_exploration": witness["full_ecosystem_exploration"].clone(),
        "replay_contract": witness["replay_contract"].clone(),
        "runtime_profile": witness["runtime_profile"].clone(),
        "execution_digest": witness["execution_digest"].clone(),
        "harness_manifest": witness["harness_manifest"].clone(),
        "operation_coverage": witness["operation_coverage"].clone(),
        "function_summaries": witness["function_summaries"].clone(),
        "call_graph_obligation_summaries": witness["call_graph_obligation_summaries"].clone(),
        "formal_core": witness["formal_core"].clone(),
        "proof_seed": witness["proof_seed"].clone(),
        "strict_liveness": witness["strict_liveness"].clone(),
        "invariant_checks": witness["invariant_checks"].clone(),
        "temporal_checks": witness["temporal_checks"].clone(),
        "model_vs_implementation": witness["model_vs_implementation"].clone(),
        "flagship_demo": witness["flagship_demo"].clone(),
        "replay_grade": witness["replay_grade"].clone(),
        "boundary_ledger": witness["boundary_ledger"].clone(),
        "ecosystem_boundaries": witness["ecosystem_boundaries"].clone(),
        "declarations": witness["declarations"].clone(),
        "summaries": witness["summaries"].clone(),
        "inferred_obligations": witness["inferred_obligations"].clone(),
        "lifecycle_inference": witness["lifecycle_inference"].clone(),
        "failure": witness_failure_json(witness),
        "events": normalized_witness_events_json(witness),
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

fn replay_event_budget(witness: &Value) -> Option<u64> {
    witness["scheduler"]["event_budget"]
        .as_u64()
        .or_else(|| witness["backend_controls"]["max_branches"].as_u64())
}

fn replay_seed_count(witness: &Value) -> u64 {
    configured_seed_count_from_events(witness)
        .or_else(|| witness["scheduler"]["configured_seed_count"].as_u64())
        .or_else(|| witness["scheduler"]["seed_count"].as_u64())
        .unwrap_or(1)
        .max(1)
}

fn configured_seed_count_from_events(witness: &Value) -> Option<u64> {
    witness["events"]
        .as_array()?
        .iter()
        .filter(|event| event["kind"].as_str() == Some("scheduler-seed-case"))
        .filter_map(|event| event["label"].as_str())
        .find_map(seed_count_from_label)
}

fn seed_count_from_label(label: &str) -> Option<u64> {
    label.split(';').find_map(|part| {
        part.strip_prefix("count=")
            .and_then(|count| count.parse::<u64>().ok())
    })
}

fn run_replay_seed_portfolio(
    scenario_program: &ScenarioProgram,
    generated_rust: &str,
    options: &kobo_sim_core::ScenarioOptions,
    seed_count: u64,
) -> anyhow::Result<kobo_sim_core::FullDepthRun> {
    if seed_count <= 1 {
        return kobo_sim_core::run_full_depth_from_program(
            scenario_program,
            generated_rust,
            options,
            kobo_sim_core::EngineMode::Both,
        )
        .map_err(Into::into);
    }

    let mut combined_events = Vec::new();
    let mut last_run = None;
    for index in 0..seed_count {
        let seed = options.seed.wrapping_add(index);
        combined_events.push(seed_case_event(index, seed, seed_count));
        let exploration = run_replay_seed_case(
            scenario_program,
            generated_rust,
            options,
            seed,
            kobo_sim_core::EngineMode::SemanticOnly,
        )?;
        if exploration.failure.is_some() || index + 1 == seed_count {
            let mut selected_run = run_replay_seed_case(
                scenario_program,
                generated_rust,
                options,
                seed,
                kobo_sim_core::EngineMode::Both,
            )?;
            combined_events.extend(selected_run.events.iter().cloned());
            selected_run.events = combined_events;
            refresh_seed_portfolio_digest(&mut selected_run);
            return Ok(selected_run);
        }
        combined_events.extend(exploration.events.iter().cloned());
        last_run = Some(exploration);
    }

    let mut run =
        last_run.ok_or_else(|| anyhow::anyhow!("seed portfolio had no replay cases to execute"))?;
    run.events = combined_events;
    refresh_seed_portfolio_digest(&mut run);
    Ok(run)
}

fn run_replay_seed_case(
    scenario_program: &ScenarioProgram,
    generated_rust: &str,
    options: &kobo_sim_core::ScenarioOptions,
    seed: u64,
    engine: kobo_sim_core::EngineMode,
) -> anyhow::Result<kobo_sim_core::FullDepthRun> {
    let mut case_options = options.clone();
    case_options.seed = seed;
    Ok(kobo_sim_core::run_full_depth_from_program(
        scenario_program,
        generated_rust,
        &case_options,
        engine,
    )?)
}

fn seed_case_event(index: u64, seed: u64, seed_count: u64) -> kobo_sim_core::ScenarioEvent {
    kobo_sim_core::ScenarioEvent {
        kind: "scheduler-seed-case".to_owned(),
        label: Some(format!("index={index};count={seed_count}")),
        value: Some(seed),
        io: None,
    }
}

fn refresh_seed_portfolio_digest(run: &mut kobo_sim_core::FullDepthRun) {
    run.digest.semantic_trace_hash = kobo_sim_core::digest::events_hash(&run.events);
    if !run.digest.harness_trace_hash.is_empty() {
        run.digest.harness_trace_hash = kobo_sim_core::digest::stable_hash(&format!(
            "seed-portfolio:{}:{}",
            run.digest.harness_trace_hash, run.digest.semantic_trace_hash
        ));
    }
    if run.digest.agreement == "matched" {
        run.digest.agreement = "matched+seed-portfolio".to_owned();
    }
}

fn validate_shrink_metadata(witness: &Value, error_format: ErrorFormat) -> anyhow::Result<()> {
    if witness["exactness"].as_str() != Some("exact") {
        formal_core::validate_formal_core_witness(witness)?;
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

fn validate_checkpoint_replay_metadata(
    witness: &Value,
    run: &kobo_sim_core::FullDepthRun,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    if witness["checkpoint_replay"]["enabled"].as_bool() != Some(true) {
        return Ok(());
    }
    let expected = checkpoint_replay_json(true, run);
    if witness["checkpoint_replay"] == expected {
        return Ok(());
    }
    let payload = serde_json::json!({
        "code": "K0106",
        "message": "checkpoint_replay metadata does not match replayed trace",
        "expected": expected,
        "observed": witness["checkpoint_replay"].clone(),
    });
    emit_replay_issue(&payload, error_format)?;
    anyhow::bail!("K0106 checkpoint_replay metadata is inconsistent");
}

fn validate_checkpoint_artifact_available(
    witness: &Value,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    if witness["checkpoint_replay"]["enabled"].as_bool() != Some(true) {
        return Ok(());
    }
    let Some(path) = witness["checkpoint_replay"]["checkpoint_path"].as_str() else {
        let payload = serde_json::json!({
            "code": "K0106",
            "message": "checkpoint replay is missing checkpoint artifact path",
            "checkpoint_replay": witness["checkpoint_replay"].clone(),
        });
        emit_replay_issue(&payload, error_format)?;
        anyhow::bail!("K0106 checkpoint artifact is missing");
    };
    let expected_hash = witness["checkpoint_replay"]["checkpoint_artifact_hash"].as_str();
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) => {
            let payload = serde_json::json!({
                "code": "K0106",
                "message": "checkpoint artifact is missing or unreadable",
                "checkpoint_path": path,
                "error": error.to_string(),
            });
            emit_replay_issue(&payload, error_format)?;
            anyhow::bail!("K0106 checkpoint artifact is missing");
        }
    };
    let observed_hash = kobo_sim_core::digest::stable_hash(&contents);
    if expected_hash == Some(observed_hash.as_str()) {
        return Ok(());
    }
    let payload = serde_json::json!({
        "code": "K0106",
        "message": "checkpoint artifact hash does not match witness",
        "checkpoint_path": path,
        "expected": expected_hash,
        "observed": observed_hash,
    });
    emit_replay_issue(&payload, error_format)?;
    anyhow::bail!("K0106 checkpoint artifact hash is inconsistent");
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

fn runtime_profile_json(
    config: &kobo_driver::KoboConfig,
    sim_profile: &str,
    seed: u64,
    run: &kobo_sim_core::FullDepthRun,
) -> Value {
    let profile = &config.runtime_profile;
    serde_json::json!({
        "service": {
            "buffer": profile.service_buffer,
            "backpressure": profile.service_backpressure,
        },
        "scenario": {
            "scheduler": profile.scheduler,
            "sim_profile": sim_profile,
            "backend_profile": run.profile,
            "seed": seed,
            "event_budget": profile.scenario_event_budget,
        },
        "record": {
            "default": profile.record,
        },
        "activity": {
            "default": profile.activity,
        },
        "runtime": {
            "cancellation": profile.cancellation,
        },
    })
}

fn execution_digest_json(run: &kobo_sim_core::FullDepthRun, runtime_profile_hash: &str) -> Value {
    serde_json::json!({
        "engine": "semantic-sim",
        "semantic_engine": run.digest.semantic_engine.as_str(),
        "harness_engine": run.digest.harness_engine.as_str(),
        "model_schema": run.digest.model_schema.as_str(),
        "schema_version": run.digest.schema_version,
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
        "runtime_profile_hash": runtime_profile_hash,
    })
}

fn backend_version_json(backend: &str) -> Value {
    let capability = kobo_sim_core::backend::capabilities()
        .iter()
        .find(|capability| capability.name == backend);
    serde_json::json!({
        "backend": backend,
        "adapter_source": "kobo-sim-core",
        "adapter_version": env!("CARGO_PKG_VERSION"),
        "integration_level": capability.map(|capability| capability.integration_level),
        "scenario_execution": capability.map(|capability| capability.scenario_execution),
    })
}

fn checkpoint_replay_json(enabled: bool, run: &kobo_sim_core::FullDepthRun) -> Value {
    let checkpoint_path = run
        .harness_manifest
        .as_ref()
        .and_then(|manifest| manifest.checkpoint_path.as_deref());
    let checkpoint_artifact_hash = run
        .harness_manifest
        .as_ref()
        .and_then(|manifest| manifest.checkpoint_artifact_hash.as_deref());
    let artifact_validated =
        enabled && checkpoint_path.is_some() && checkpoint_artifact_hash.is_some();
    serde_json::json!({
        "enabled": artifact_validated,
        "semantic_trace_hash": if artifact_validated {
            Some(run.digest.semantic_trace_hash.as_str())
        } else {
            None
        },
        "harness_trace_hash": if artifact_validated {
            Some(run.digest.harness_trace_hash.as_str())
        } else {
            None
        },
        "event_count": if artifact_validated {
            Some(run.events.len())
        } else {
            None
        },
        "checkpoint_path": if artifact_validated {
            checkpoint_path
        } else {
            None
        },
        "checkpoint_artifact_hash": if artifact_validated {
            checkpoint_artifact_hash
        } else {
            None
        },
        "artifact_validation": if artifact_validated {
            Some("loom-checkpoint-hash")
        } else {
            None
        },
        "source": if artifact_validated {
            Some("loom-builder-checkpoint")
        } else {
            None
        },
    })
}

fn replay_scope_json(run: &kobo_sim_core::FullDepthRun) -> Value {
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
    validate_run_boundary_declarations(file, config, run, error_format)?;
    let expected = ecosystem_boundaries_json(file, config, run);
    if let Some(boundaries) = witness["ecosystem_boundaries"].as_array() {
        if boundaries.iter().any(|boundary| {
            boundary["policy"].as_str() == Some("record")
                && boundary["evidence"].as_str() != Some("recorded-event")
        }) {
            let payload = serde_json::json!({
                "code": "K0124",
                "message": "record boundary is missing recorded event evidence",
                "expected": expected.clone(),
                "observed": witness["ecosystem_boundaries"].clone(),
            });
            emit_replay_issue(&payload, error_format)?;
            anyhow::bail!("K0124 record boundary missing recorded evidence");
        }
    }
    if let Some(expected_boundaries) = expected.as_array() {
        for expected_boundary in expected_boundaries {
            if expected_boundary["policy"].as_str() != Some("record") {
                continue;
            }
            if !observed_boundary_matches(witness, expected_boundary, "record") {
                continue;
            }
            let Some(expected_label) = boundary_event_label_from_json(expected_boundary) else {
                continue;
            };
            if witness_events_contain_boundary_label(witness, "boundary-record", &expected_label) {
                if record_boundary_has_io_capture(witness, expected_boundary) {
                    continue;
                }
                let payload = serde_json::json!({
                    "code": "K0124",
                    "message": "record boundary is missing replayable boundary I/O capture",
                    "expected": expected.clone(),
                    "observed": {
                        "ecosystem_boundaries": witness["ecosystem_boundaries"].clone(),
                    },
                });
                emit_replay_issue(&payload, error_format)?;
                anyhow::bail!("K0124 record boundary missing boundary I/O capture");
            }
            let payload = serde_json::json!({
                "code": "K0124",
                "message": "record boundary is missing call-specific recorded event evidence",
                "expected_event": {
                    "kind": "boundary-record",
                    "label": expected_label,
                },
                "expected": expected.clone(),
                "observed": {
                    "ecosystem_boundaries": witness["ecosystem_boundaries"].clone(),
                    "events": witness["events"].clone(),
                },
            });
            emit_replay_issue(&payload, error_format)?;
            anyhow::bail!("K0124 record boundary missing call-specific evidence");
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

fn record_boundary_has_io_capture(witness: &Value, expected_boundary: &Value) -> bool {
    let Some(observed_boundaries) = witness["ecosystem_boundaries"].as_array() else {
        return false;
    };
    let expected_call_path = expected_boundary["call_path"].as_str();
    let expected_span = &expected_boundary["source_span"];
    observed_boundaries.iter().any(|observed| {
        let replay_key = observed["capture"]["io_capture"]["replay_key"].as_str();
        observed["policy"].as_str() == Some("record")
            && observed["call_path"].as_str() == expected_call_path
            && observed["call_arguments"] == expected_boundary["call_arguments"]
            && observed["return_type"] == expected_boundary["return_type"]
            && observed["source_span"] == *expected_span
            && observed["capture"]["io_capture"]["mode"].as_str() == Some("recorded-boundary-io")
            && observed["capture"]["io_capture"]["request_hash"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
            && observed["capture"]["io_capture"]["response_hash"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
            && observed["capture"]["io_capture"]["replay_key"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
            && observed["capture"]["io_capture"] == expected_boundary["capture"]["io_capture"]
            && witness["boundary_ledger"]
                .as_array()
                .is_some_and(|entries| {
                    entries.iter().any(|entry| {
                        entry["policy"].as_str() == Some("record")
                            && entry["boundary"] == observed["crate"]
                            && entry["call_path"] == observed["call_path"]
                            && entry["call_arguments"] == observed["call_arguments"]
                            && entry["return_type"] == observed["return_type"]
                            && entry["source_span"] == observed["source_span"]
                            && entry["io_capture"] == observed["capture"]["io_capture"]
                    })
                })
            && replay_key.is_some_and(|key| {
                witness["events"].as_array().is_some_and(|events| {
                    events.iter().any(|event| {
                        event["kind"].as_str() == Some("boundary-record")
                            && event["label"].as_str() == Some(key)
                            && event["io_capture"] == observed["capture"]["io_capture"]
                    })
                })
            })
    })
}

fn observed_boundary_matches(witness: &Value, expected_boundary: &Value, policy: &str) -> bool {
    let Some(observed_boundaries) = witness["ecosystem_boundaries"].as_array() else {
        return false;
    };
    observed_boundaries.iter().any(|observed| {
        observed["policy"].as_str() == Some(policy)
            && observed["crate"] == expected_boundary["crate"]
            && observed["call_path"] == expected_boundary["call_path"]
            && observed["call_arguments"] == expected_boundary["call_arguments"]
            && observed["return_type"] == expected_boundary["return_type"]
            && observed["call_shape"] == expected_boundary["call_shape"]
            && observed["source_span"] == expected_boundary["source_span"]
    })
}

fn boundary_event_label_from_json(boundary: &Value) -> Option<String> {
    let identity = boundary["call_path"]
        .as_str()
        .or_else(|| boundary["crate"].as_str())?;
    let span = &boundary["source_span"];
    let start = span["start"].as_u64()?;
    let end = span["end"].as_u64()?;
    Some(format!("{identity}@{start}..{end}"))
}

fn witness_events_contain_boundary_label(witness: &Value, kind: &str, label: &str) -> bool {
    witness["events"].as_array().is_some_and(|events| {
        events.iter().any(|event| {
            event["kind"].as_str() == Some(kind) && event["label"].as_str() == Some(label)
        })
    })
}

fn validate_declaration_evidence(
    witness: &Value,
    file: &Path,
    config: &kobo_driver::KoboConfig,
    run: &kobo_sim_core::FullDepthRun,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    let expected = declarations_json(file, config, run);
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

fn validate_run_boundary_declarations(
    file: &Path,
    config: &kobo_driver::KoboConfig,
    run: &kobo_sim_core::FullDepthRun,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    for decision in &run.boundary_decisions {
        let policy = decision.policy.as_str();
        let adapter = config.ecosystem_policy.adapter_for(&decision.crate_name);
        if policy == "model" && adapter.is_none() {
            let payload = serde_json::json!({
                "code": "K0123",
                "message": "model boundary has no adapter package",
                "crate": decision.crate_name,
                "policy": policy,
                "call_path": decision.call_path,
            });
            emit_replay_issue(&payload, error_format)?;
            anyhow::bail!(
                "K0123: model boundary for `{}` has no adapter package",
                decision.crate_name
            );
        }
        if let Some(adapter) = adapter {
            if let Err(message) = super::ecosystem::validate_model_adapter_package(adapter) {
                let payload = serde_json::json!({
                    "code": "K0123",
                    "message": "adapter package failed validation",
                    "crate": decision.crate_name,
                    "policy": policy,
                    "call_path": decision.call_path,
                    "detail": message,
                });
                emit_replay_issue(&payload, error_format)?;
                anyhow::bail!(
                    "K0123: adapter package for `{}` failed validation: {}",
                    decision.crate_name,
                    payload["detail"]
                        .as_str()
                        .unwrap_or("invalid adapter metadata")
                );
            }
        }
        if !matches!(policy, "typed" | "activity") {
            continue;
        }
        if let Err(error) = declarations::declaration_facts_for_boundary(
            file,
            config,
            &decision.crate_name,
            policy,
            decision.call_path.as_deref(),
        ) {
            let code = declaration_error_code(policy, error.key);
            let payload = serde_json::json!({
                "code": code,
                "message": "configured ecosystem boundary metadata failed validation",
                "crate": decision.crate_name,
                "policy": policy,
                "call_path": decision.call_path,
                "key": error.key,
                "path": error.path.display().to_string(),
                "detail": error.message,
            });
            emit_replay_issue(&payload, error_format)?;
            anyhow::bail!(
                "{code}: configured {policy} metadata for `{}` failed validation at {} key `{}`: {}",
                decision.crate_name,
                error.path.display(),
                error.key,
                error.message
            );
        }
    }
    Ok(())
}

fn declaration_error_code(policy: &str, key: &str) -> &'static str {
    match (policy, key) {
        ("activity", "activity") => "K0125",
        ("typed", "declaration") => "K0122",
        _ => "K0121",
    }
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
                let evidence = boundary_evidence_for_policy(decision, run);
                serde_json::json!({
                    "crate": decision.crate_name,
                    "call_path": decision.call_path,
                    "call_arguments": decision.call_arguments,
                    "return_type": decision.return_type,
                    "call_shape": decision.call_shape.as_str(),
                    "policy": decision.policy.as_str(),
                    "reason": decision.reason,
                    "evidence": evidence,
                    "capture": boundary_capture_json(decision, run),
                    "activity_metadata": activity_metadata_for_boundary(file, config, decision),
                    "source_span": {
                        "start": decision.span_start,
                        "end": decision.span_end,
                    },
                    "declaration": declaration_metadata_for_boundary(file, config, run, &decision.crate_name, decision.policy.as_str()),
                    "adapter": config.ecosystem_policy.adapter_for(&decision.crate_name).map(|adapter| serde_json::json!({
                        "package": adapter.package,
                        "version": adapter.version,
                        "confidence": adapter.confidence,
                        "source": adapter.source,
                        "registry": adapter.registry,
                        "checksum": adapter.checksum,
                        "compatible_crate": adapter.compatible_crate,
                        "metadata_path": adapter.metadata_path.as_ref().map(|path| path.display().to_string()),
                        "trust_policy": adapter.trust_policy,
                        "signed_by": adapter.signed_by,
                        "validated": adapter.validated,
                        "adapter_runtime": adapter.adapter_runtime,
                        "capture": adapter.capture,
                        "reason": adapter.reason,
                    })),
                    "full_ecosystem_exploration": false,
                })
            })
            .collect(),
    )
}

fn boundary_evidence_for_policy(
    decision: &kobo_sim_core::BoundaryDecision,
    run: &kobo_sim_core::FullDepthRun,
) -> &'static str {
    let expected_label = boundary_event_label(decision);
    match decision.policy.as_str() {
        "record"
            if run.events.iter().any(|event| {
                event.kind == "boundary-record"
                    && event.label.as_deref() == Some(expected_label.as_str())
            }) =>
        {
            "recorded-event"
        }
        "activity"
            if run.events.iter().any(|event| {
                event.kind == "boundary-activity"
                    && event.label.as_deref() == Some(expected_label.as_str())
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

fn boundary_capture_json(
    decision: &kobo_sim_core::BoundaryDecision,
    run: &kobo_sim_core::FullDepthRun,
) -> Option<Value> {
    let expected_label = boundary_event_label(decision);
    let event = run.events.iter().find(|event| {
        matches!(
            event.kind.as_str(),
            "boundary-record" | "boundary-activity" | "boundary-model" | "boundary-stub"
        ) && event.label.as_deref() == Some(expected_label.as_str())
    })?;
    let capture_source = if run.harness_manifest.is_some() {
        "facade-call-capture"
    } else {
        "semantic-boundary-capture"
    };
    Some(serde_json::json!({
        "mode": "boundary-call-capture",
        "capture_source": capture_source,
        "event_kind": event.kind.clone(),
        "event_label": event.label.clone(),
        "event_value": event.value,
        "io_capture": boundary_decision_io_capture_json(decision),
        "call_path": decision.call_path.clone(),
        "call_arguments": decision.call_arguments.clone(),
        "return_type": decision.return_type.clone(),
        "call_shape": decision.call_shape.as_str(),
        "source_span": {
            "start": decision.span_start,
            "end": decision.span_end,
        },
        "external_internals_replayed": false,
    }))
}

fn boundary_decision_io_capture_json(decision: &kobo_sim_core::BoundaryDecision) -> Option<Value> {
    if !matches!(decision.policy.as_str(), "record" | "activity") {
        return None;
    }
    let capture = decision.recorded_io.as_ref()?;
    Some(boundary_io_capture_json(capture))
}

fn boundary_io_capture_json(capture: &kobo_sim_core::BoundaryIoCapture) -> Value {
    serde_json::json!({
        "mode": capture.mode.clone(),
        "replay_key": capture.replay_key.clone(),
        "request": boundary_io_payload_json(&capture.request),
        "response": boundary_io_payload_json(&capture.response),
        "request_hash": capture.request_hash.clone(),
        "response_hash": capture.response_hash.clone(),
    })
}

fn boundary_io_payload_json(payload: &kobo_sim_core::BoundaryIoPayload) -> Value {
    let mut fields = serde_json::Map::new();
    for field in &payload.fields {
        fields.insert(field.key.clone(), Value::String(field.value.clone()));
    }
    serde_json::json!({
        "kind": payload.kind.clone(),
        "payload": fields,
    })
}

fn boundary_event_label(decision: &kobo_sim_core::BoundaryDecision) -> String {
    format!(
        "{}@{}..{}",
        decision
            .call_path
            .as_deref()
            .unwrap_or(decision.crate_name.as_str()),
        decision.span_start,
        decision.span_end
    )
}

fn declarations_json(
    file: &Path,
    config: &kobo_driver::KoboConfig,
    run: &kobo_sim_core::FullDepthRun,
) -> Value {
    Value::Array(
        run.boundary_decisions
            .iter()
            .filter_map(|decision| {
                if decision.policy.as_str() != "typed" {
                    return None;
                }
                let facts = match declarations::declaration_facts_for_config(
                    file,
                    &decision.crate_name,
                    config,
                ) {
                    declarations::DeclarationLookup::Valid(facts) => facts,
                    declarations::DeclarationLookup::Missing
                    | declarations::DeclarationLookup::Invalid(_) => return None,
                };
                Some(serde_json::json!({
                    "crate": decision.crate_name,
                    "path": facts.path.display().to_string(),
                    "version": facts.version.clone(),
                    "schema_version": facts.schema_version,
                    "hash": facts.hash.clone(),
                    "declaration_version": facts.version.clone(),
                    "declaration_hash": facts.hash.clone(),
                }))
            })
            .collect(),
    )
}

fn summary_usage_json(
    config: &kobo_driver::KoboConfig,
    program: &ScenarioProgram,
) -> anyhow::Result<Value> {
    let mut summaries = Vec::new();
    summaries.push(formal_core::summary_json(program));
    for summary in &config.ecosystem_policy.summaries {
        let valid = summary_validation::load_valid_summary(summary)?;
        let parsed = valid.value;
        let obligations = parsed
            .get("obligations")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let functions = parsed
            .get("functions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let solver_metadata = parsed
            .get("solver_metadata")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({"engine": "unknown", "outcome": "missing"}));
        summaries.push(serde_json::json!({
            "crate": summary.crate_name,
            "path": summary.path.display().to_string(),
            "summary_hash": valid.hash,
            "schema_version": valid.schema_version,
            "solver_metadata": solver_metadata,
            "obligation_count": obligations.len(),
            "function_count": functions.len(),
            "obligations": obligations,
            "functions": functions,
        }));
    }
    Ok(Value::Array(summaries))
}

fn declaration_metadata_for_boundary(
    file: &Path,
    config: &kobo_driver::KoboConfig,
    run: &kobo_sim_core::FullDepthRun,
    crate_name: &str,
    policy: &str,
) -> Option<Value> {
    run.boundary_decisions
        .iter()
        .find(|decision| {
            decision.crate_name == crate_name
                && decision.policy.as_str() == policy
                && matches!(policy, "typed" | "activity")
        })
        .and_then(|_| {
            let facts = match declarations::declaration_facts_for_config(file, crate_name, config) {
                declarations::DeclarationLookup::Valid(facts) => facts,
                declarations::DeclarationLookup::Missing
                | declarations::DeclarationLookup::Invalid(_) => return None,
            };
            Some(declaration_metadata_json(&facts))
        })
}

fn activity_metadata_for_boundary(
    file: &Path,
    config: &kobo_driver::KoboConfig,
    decision: &kobo_sim_core::BoundaryDecision,
) -> Option<Value> {
    if decision.policy.as_str() != "activity" {
        return None;
    }
    let facts = match declarations::declaration_facts_for_config(file, &decision.crate_name, config)
    {
        declarations::DeclarationLookup::Valid(facts) => facts,
        declarations::DeclarationLookup::Missing | declarations::DeclarationLookup::Invalid(_) => {
            return None;
        }
    };
    let activity = declarations::activity_fact_for_call(&facts, decision.call_path.as_deref())?;
    Some(serde_json::json!({
        "path": activity.path.clone(),
        "retry": activity.retry.clone(),
        "idempotency": activity.idempotency.clone(),
        "result": activity.result.clone(),
        "compensation": activity.compensation.clone(),
        "declaration_hash": facts.hash.clone(),
        "declaration_version": facts.version.clone(),
    }))
}

fn declaration_metadata_json(facts: &declarations::DeclarationFacts) -> Value {
    let mut value = serde_json::json!({
        "path": facts.path.display().to_string(),
        "version": facts.version.clone(),
        "schema_version": facts.schema_version,
        "hash": facts.hash.clone(),
    });
    if let Some(package) = facts.metadata_package.as_ref() {
        value
            .as_object_mut()
            .expect("declaration metadata json should be an object")
            .insert(
                "metadata_package".to_owned(),
                serde_json::json!({
                    "package": package.package.clone(),
                    "version": package.version.clone(),
                    "path": package.path.display().to_string(),
                    "source": package.source.clone(),
                    "registry": package.registry.clone(),
                    "checksum": package.checksum.clone(),
                    "signed_by": package.signed_by.clone(),
                    "validated": package.validated,
                }),
            );
    }
    value
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
                    "mapped": obligation.declaration_span.1 > obligation.declaration_span.0,
                    "snippet": line_snippet(source, obligation.declaration_span.0),
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
        .filter(|(_, event)| !is_replay_irrelevant_event(&event.kind))
        .map(|(_, event)| event)
        .collect::<Vec<_>>();
    Ok(filtered
        .iter()
        .enumerate()
        .map(|(id, event)| {
            let mut value = serde_json::json!({
                "id": id,
                "kind": event.kind.clone(),
                "label": event.label.clone(),
                "value": event.value,
            });
            if let Some(io) = event.io.as_ref() {
                value
                    .as_object_mut()
                    .expect("event json should be an object")
                    .insert("io_capture".to_owned(), boundary_io_capture_json(io));
            }
            value
        })
        .collect())
}

fn normalized_witness_events_json(witness: &Value) -> Vec<Value> {
    witness["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|event| {
            !event["kind"]
                .as_str()
                .is_some_and(is_replay_irrelevant_event)
        })
        .enumerate()
        .map(|(id, event)| {
            let mut event = event.clone();
            if let Some(object) = event.as_object_mut() {
                object.insert("id".to_owned(), serde_json::json!(id));
            }
            event
        })
        .collect()
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
    let original_event_count = witness["shrink"]["original_event_count"]
        .as_u64()
        .unwrap_or(events.len() as u64) as usize;
    for removed_id in removed_event_ids(witness)? {
        if removed_id >= original_event_count {
            let payload = serde_json::json!({
                "code": "K0106",
                "message": "witness shrink removed_event_ids references a missing event",
                "removed_event_id": removed_id,
            });
            emit_replay_issue(&payload, error_format)?;
            anyhow::bail!("K0106 witness shrink removed a missing event");
        };
        let Some(event) = events.get(removed_id) else {
            continue;
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
        "scheduler-seed-case"
            | "scheduler-portfolio"
            | "scheduler-pct-seed"
            | "network-delayed"
            | "network-reordered"
            | "fuzz-shrink-candidate"
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

fn backend_for_profile(profile: &str) -> &'static str {
    match profile {
        "sync" => "loom",
        "async" => "generated-rust-process",
        "stateful-input" => "proptest",
        "failpoint" => "failpoints",
        "network" | "network-design" => "network-loopback",
        "distributed" | "madsim" => "generated-rust-process",
        _ => "unknown",
    }
}

fn validate_exact_witness_scope(witness: &Value, error_format: ErrorFormat) -> anyhow::Result<()> {
    let replay_blocking_unsupported = witness["coverage"]["unsupported_constructs"]
        .as_array()
        .map(|constructs| {
            constructs
                .iter()
                .any(|construct| construct.as_str() != Some("lifecycle_method_await_initializer"))
        })
        .unwrap_or(false);
    if replay_blocking_unsupported {
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
    let has_replayable_agreement = agreement
        .is_some_and(|agreement| agreement == "semantic-only" || agreement.starts_with("matched"));
    if semantic_engine != Some("driver-kir-scenario")
        || (!has_generated_harness && agreement != Some("semantic-only"))
        || !has_replayable_agreement
    {
        let payload = serde_json::json!({
            "code": "K0117",
            "message": "semantic trace and harness trace diverged or are missing",
            "execution_digest": digest.clone(),
        });
        emit_replay_issue(&payload, error_format)?;
        anyhow::bail!("K0117 exact witness lacks semantic/harness agreement");
    }
    validate_exact_scope_metadata(witness, error_format)?;
    Ok(())
}

fn validate_exact_scope_metadata(witness: &Value, error_format: ErrorFormat) -> anyhow::Result<()> {
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
            "message": "exact witness must disclose the Kobo-managed replay scope and must not claim full ecosystem exploration",
            "replay_contract": witness["replay_contract"].clone(),
            "ecosystem_scope": witness["ecosystem_scope"].clone(),
            "full_ecosystem_exploration": witness["full_ecosystem_exploration"].clone(),
            "harness_manifest": witness["harness_manifest"].clone(),
        });
        emit_replay_issue(&payload, error_format)?;
        anyhow::bail!("K0117 exact witness lacks replay scope evidence");
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

fn line_snippet(source: &str, offset: usize) -> String {
    let bounded = offset.min(source.len());
    let line_start = source[..bounded]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let line_end = source[bounded..]
        .find('\n')
        .map(|index| bounded + index)
        .unwrap_or(source.len());
    source[line_start..line_end].trim().to_owned()
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
            &["backend_version"][..],
            &["backend_version", "backend"][..],
            &["backend_version", "adapter_source"][..],
            &["backend_version", "adapter_version"][..],
            &["backend_replay"][..],
            &["backend_replay_token"][..],
            &["backend_controls"][..],
            &["backend_controls", "backend"][..],
            &["backend_controls", "scheduler"][..],
            &["backend_controls", "max_branches"][..],
            &["backend_controls", "backend_native"][..],
            &["backend_controls", "replay_token"][..],
            &["backend_controls", "checkpoint_replay"][..],
            &["checkpoint_replay"][..],
            &["checkpoint_replay", "enabled"][..],
            &["sim_profile"][..],
            &["sim_config"][..],
            &["sim_config", "default_profile"][..],
            &["sim_config", "show_backend_choices"][..],
            &["sim_config", "profiles"][..],
            &["sim_config", "backends"][..],
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
            required.push(&["execution_digest", "model_schema"][..]);
            required.push(&["execution_digest", "schema_version"][..]);
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
            required.push(&["formal_core"][..]);
            required.push(&["proof_seed"][..]);
            required.push(&["strict_liveness"][..]);
            required.push(&["invariant_checks"][..]);
            required.push(&["temporal_checks"][..]);
            required.push(&["model_vs_implementation"][..]);
            required.push(&["flagship_demo"][..]);
            required.push(&["replay_grade"][..]);
            required.push(&["boundary_ledger"][..]);
            required.push(&["ecosystem_boundaries"][..]);
            required.push(&["summaries"][..]);
            required.push(&["inferred_obligations"][..]);
            required.push(&["lifecycle_inference", "mode"][..]);
        }
        for path in required {
            if value_at(witness, path).is_none() {
                return witness_error(&format!("missing {}", path.join(".")));
            }
        }
        formal_core::validate_formal_core_witness(witness)?;
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
