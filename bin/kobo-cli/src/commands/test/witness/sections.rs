use super::super::backend::reserved_backend_fit_json;
use super::super::fuzz::{fuzz_plan_json, FuzzPlan};
use super::super::json_schema::{
    backend_version_json, boundary_assumptions_json, boundary_decisions_json,
    boundary_policies_json, checkpoint_replay_json, coverage_json, declarations_json,
    ecosystem_boundaries_json, execution_digest_json, expanded_policy_json, failure_json,
    flagship_demo_json, handler_lifecycle_json, modeled_boundaries_json, obligation_events_json,
    obligations_json, parallel_lowering_json, runtime_profile_json, scenario_coverage_json,
    scheduler_json, service_runtime_json, sim_config_json, source_spans_json, summary_usage_json,
    task_local_zones_json,
};
use super::super::model_compare::model_vs_implementation_json;
use super::super::proof::{proof_replay_grade, witness_replay_grade};
use super::super::replay_scope::{
    ecosystem_scope, exactness_json, full_ecosystem_exploration, injections_json,
    replay_scope_json, shrink_event_stream, shrink_json,
};
use super::super::trace_checks::trace_checks_json;
use super::super::{
    formal_core, serde_json, witness_evidence, BackendExpertOptions, FullDepthRun, Path,
    ScenarioDocument, ScenarioProgram,
};

pub(super) type WitnessObject = serde_json::Map<String, serde_json::Value>;

pub(super) fn base_witness_object(
    source_path: &str,
    guarantee_profile: &str,
    sim_profile: &str,
    seed: u64,
    backend_name: &str,
    backend_replay: String,
    backend_replay_evidence: serde_json::Value,
    run: &FullDepthRun,
) -> WitnessObject {
    let mut object = serde_json::Map::new();
    object.insert("schema_version".to_owned(), serde_json::json!(1));
    object.insert("kobo_version".to_owned(), env!("CARGO_PKG_VERSION").into());
    object.insert(
        "target".to_owned(),
        format!("{}:{}", source_path, run.target).into(),
    );
    object.insert("sim_profile".to_owned(), sim_profile.into());
    object.insert("guarantee_profile".to_owned(), guarantee_profile.into());
    object.insert("seed".to_owned(), serde_json::json!(seed));
    object.insert("backend_profile".to_owned(), run.profile.clone().into());
    object.insert("backend".to_owned(), backend_name.into());
    object.insert(
        "reserved_backend_fit".to_owned(),
        reserved_backend_fit_json(&run.profile),
    );
    object.insert("backend_replay".to_owned(), backend_replay.clone().into());
    object.insert("backend_replay_token".to_owned(), backend_replay.into());
    object.insert(
        "backend_replay_evidence".to_owned(),
        backend_replay_evidence,
    );
    object.insert(
        "replay_guarantee".to_owned(),
        run.replay_guarantee.as_str().into(),
    );
    object.insert("exactness".to_owned(), exactness_json(run).into());
    object.insert(
        "scenario".to_owned(),
        serde_json::json!({
            "name": run.target,
            "profile": run.profile,
        }),
    );
    object
}

pub(super) fn insert_policy_and_scheduler_sections(
    object: &mut WitnessObject,
    source_path: &str,
    document: &ScenarioDocument,
    guarantee_profile: &str,
    sim_profile: &str,
    seed: u64,
    inject: Option<&str>,
    fuzz_plan: Option<&FuzzPlan>,
    config: &kobo_driver::KoboConfig,
    expert_options: &BackendExpertOptions,
    effective_scheduler: Option<&str>,
    effective_max_branches: Option<u64>,
    effective_replay_token: &str,
    effective_checkpoint_replay: bool,
    run: &FullDepthRun,
    backend_name: &str,
) -> anyhow::Result<()> {
    object.insert(
        "backend_controls".to_owned(),
        serde_json::json!({
            "backend": backend_name,
            "reserved_backend_fit": reserved_backend_fit_json(&run.profile),
            "scheduler": effective_scheduler,
            "max_branches": effective_max_branches,
            "backend_native": expert_options.backend_native,
            "replay_token": effective_replay_token,
            "checkpoint_replay": effective_checkpoint_replay,
        }),
    );
    object.insert(
        "backend_version".to_owned(),
        backend_version_json(backend_name),
    );
    object.insert(
        "source".to_owned(),
        serde_json::json!({
            "path": source_path,
            "hash": document.source_hash,
        }),
    );
    object.insert(
        "expanded_policy".to_owned(),
        expanded_policy_json(guarantee_profile),
    );
    object.insert("fuzz".to_owned(), fuzz_plan_json(fuzz_plan));
    object.insert("injections".to_owned(), injections_json(inject));
    object.insert(
        "checkpoint_replay".to_owned(),
        checkpoint_replay_json(effective_checkpoint_replay, run),
    );
    object.insert("ecosystem_scope".to_owned(), ecosystem_scope(run).into());
    object.insert(
        "full_ecosystem_exploration".to_owned(),
        full_ecosystem_exploration(run).into(),
    );
    object.insert("replay_contract".to_owned(), replay_scope_json(run));
    object.insert(
        "scheduler".to_owned(),
        scheduler_json(
            sim_profile,
            seed,
            run,
            effective_scheduler.or(Some(config.runtime_profile.scheduler.as_str())),
        ),
    );
    object.insert(
        "harness_manifest".to_owned(),
        serde_json::to_value(run.harness_manifest.clone())?,
    );
    Ok(())
}

pub(super) fn insert_coverage_and_boundary_sections(
    object: &mut WitnessObject,
    file: &Path,
    document: &ScenarioDocument,
    scenario_program: &ScenarioProgram,
    config: &kobo_driver::KoboConfig,
    run: &FullDepthRun,
    source_path: &str,
    primary_span: Option<String>,
    sim_profile: &str,
    effective_shrink: &str,
) -> anyhow::Result<()> {
    let event_stream = shrink_event_stream(run, sim_profile, effective_shrink);
    let witness_events = super::super::json_schema::events_json(&event_stream.events);

    object.insert("coverage".to_owned(), coverage_json(run));
    object.insert(
        "operation_coverage".to_owned(),
        witness_evidence::operation_coverage_json(scenario_program, run),
    );
    object.insert("scenario_coverage".to_owned(), scenario_coverage_json(run));
    object.insert(
        "function_summaries".to_owned(),
        witness_evidence::function_summaries_json(scenario_program, run),
    );
    object.insert(
        "shrink".to_owned(),
        shrink_json(run, effective_shrink, &event_stream),
    );
    object.insert(
        "modeled_boundaries".to_owned(),
        serde_json::to_value(modeled_boundaries_json(run))?,
    );
    object.insert(
        "opaque_boundaries".to_owned(),
        serde_json::to_value(run.opaque_boundaries.clone())?,
    );
    object.insert(
        "boundary_policies".to_owned(),
        serde_json::Value::Array(boundary_policies_json(run)),
    );
    object.insert(
        "ecosystem_boundaries".to_owned(),
        serde_json::Value::Array(ecosystem_boundaries_json(file, config, run)),
    );
    object.insert(
        "boundary_assumptions".to_owned(),
        serde_json::Value::Array(boundary_assumptions_json(run)),
    );
    object.insert(
        "obligations".to_owned(),
        serde_json::Value::Array(obligations_json(source_path, &document.source, run)),
    );
    object.insert(
        "obligation_events".to_owned(),
        serde_json::Value::Array(obligation_events_json(run)),
    );
    object.insert(
        "boundary_decisions".to_owned(),
        serde_json::Value::Array(boundary_decisions_json(run)),
    );
    object.insert(
        "available_boundary_policies".to_owned(),
        serde_json::json!([
            "typed", "model", "record", "activity", "stub", "outside", "opaque", "debt"
        ]),
    );
    object.insert(
        "failure".to_owned(),
        failure_json(source_path, &document.source, run, primary_span),
    );
    object.insert(
        "source_spans".to_owned(),
        serde_json::Value::Array(source_spans_json(source_path, &document.source, run)),
    );
    object.insert(
        "events".to_owned(),
        serde_json::Value::Array(witness_events.clone()),
    );
    object.insert(
        "event_stream".to_owned(),
        serde_json::Value::Array(witness_events),
    );
    Ok(())
}

pub(super) fn insert_proof_and_model_sections(
    object: &mut WitnessObject,
    file: &Path,
    document: &ScenarioDocument,
    scenario_program: &ScenarioProgram,
    config: &kobo_driver::KoboConfig,
    run: &FullDepthRun,
    seed: u64,
    fuzz_plan: Option<&FuzzPlan>,
    source_path: &str,
) -> anyhow::Result<()> {
    let inferred_obligations = witness_evidence::inferred_obligations_json(
        source_path,
        &document.source,
        scenario_program,
        run,
    );
    let formal_core_json =
        formal_core::formal_core_json(source_path, &document.source, scenario_program);
    let proof_seed =
        formal_core::proof_seed_json(source_path, &document.source, scenario_program, run);
    let trace_checks = trace_checks_json(source_path, &document.source, run);
    let requested_proof_grade = proof_replay_grade(&run.replay_guarantee);
    let mut adapter_confidence = kobo_driver::proof::adapter_evidence(
        scenario_program,
        &config.ecosystem_policy.adapters,
        requested_proof_grade.clone(),
    );
    let adapter_adjusted_proof_grade = kobo_driver::proof::adapter_adjusted_replay_grade(
        requested_proof_grade,
        &adapter_confidence,
    );
    for adapter in &mut adapter_confidence {
        adapter.replay_grade = adapter_adjusted_proof_grade.clone();
    }
    let candidate_admission = kobo_driver::proof::candidate_admission_evidence(
        file,
        &document.source,
        adapter_adjusted_proof_grade.clone(),
        &adapter_confidence,
    );
    object.insert(
        "call_graph_obligation_summaries".to_owned(),
        witness_evidence::call_graph_obligation_summaries_json(scenario_program, run),
    );
    object.insert("formal_core".to_owned(), formal_core_json);
    object.insert("proof_seed".to_owned(), proof_seed);
    object.insert(
        "strict_liveness".to_owned(),
        formal_core::strict_liveness_json(source_path, &document.source, scenario_program, run),
    );
    object.insert(
        "invariant_checks".to_owned(),
        trace_checks["invariant_checks"].clone(),
    );
    object.insert(
        "temporal_checks".to_owned(),
        trace_checks["temporal_checks"].clone(),
    );
    object.insert(
        "model_vs_implementation".to_owned(),
        model_vs_implementation_json(source_path, &document.source, seed, run),
    );
    object.insert(
        "flagship_demo".to_owned(),
        flagship_demo_json(scenario_program, run),
    );
    object.insert(
        "replay_grade".to_owned(),
        serde_json::json!(witness_replay_grade(
            run,
            fuzz_plan.is_some(),
            &adapter_adjusted_proof_grade,
        )),
    );
    object.insert(
        "adapter_confidence".to_owned(),
        serde_json::to_value(&adapter_confidence)?,
    );
    object.insert(
        "candidate_admission".to_owned(),
        serde_json::to_value(&candidate_admission)?,
    );
    object.insert(
        "boundary_ledger".to_owned(),
        witness_evidence::boundary_ledger_json(scenario_program, run),
    );
    object.insert(
        "inferred_obligations".to_owned(),
        inferred_obligations.clone(),
    );
    object.insert(
        "declarations".to_owned(),
        serde_json::Value::Array(declarations_json(file, config, run)),
    );
    object.insert(
        "summaries".to_owned(),
        serde_json::Value::Array(summary_usage_json(config, scenario_program)?),
    );
    object.insert(
        "lifecycle_inference".to_owned(),
        serde_json::json!({
            "mode": "observe",
            "source": "scenario_program",
            "template_schema": "lifecycle-template",
            "schema_version": 1,
            "obligations": inferred_obligations,
        }),
    );
    Ok(())
}

pub(super) fn insert_runtime_sections(
    object: &mut WitnessObject,
    config: &kobo_driver::KoboConfig,
    runtime_evidence: &kobo_codegen::RuntimeEvidence,
    run: &FullDepthRun,
    sim_profile: &str,
    seed: u64,
) -> anyhow::Result<()> {
    let runtime_profile = runtime_profile_json(config, sim_profile, seed, run);
    let runtime_profile_hash =
        kobo_sim_core::digest::stable_hash(&serde_json::to_string(&runtime_profile)?);
    object.insert("runtime_profile".to_owned(), runtime_profile);
    object.insert("sim_config".to_owned(), sim_config_json(config));
    object.insert(
        "execution_digest".to_owned(),
        execution_digest_json(run, &runtime_profile_hash),
    );
    object.insert(
        "service_runtime".to_owned(),
        service_runtime_json(runtime_evidence, run),
    );
    object.insert(
        "parallel_lowering".to_owned(),
        parallel_lowering_json(runtime_evidence),
    );
    object.insert(
        "task_local_zones".to_owned(),
        task_local_zones_json(runtime_evidence),
    );
    object.insert(
        "handler_lifecycle".to_owned(),
        handler_lifecycle_json(runtime_evidence, run),
    );
    object.insert(
        "available_boundary_ledger_statuses".to_owned(),
        serde_json::json!([
            "modeled",
            "recordable",
            "activity",
            "opaque",
            "outside",
            "debt"
        ]),
    );
    Ok(())
}
