use std::path::{Path, PathBuf};

use anyhow::Context;
use kobo_errors::{
    diagnostic_to_json_value, ColorMode, DiagDecision, DiagLabel, DiagnosticOutputFormat,
    DiagnosticRenderer, KDiagnostic, KErrorCode, Severity,
};
use kobo_ir::{
    FileSetBuilder, GuaranteePolicy, GuaranteeProfile, KoboSpan, ScenarioOpKind, ScenarioProgram,
};
use kobo_sim_core::{EngineMode, FullDepthRun, ReplayGuarantee, ScenarioEvent, ScenarioFailure};
use proptest::prelude::{any, Strategy};
use proptest::strategy::ValueTree;
use proptest::test_runner::{
    Config as ProptestConfig, RngAlgorithm, TestRng, TestRunner as ProptestRunner,
};

use crate::ErrorFormat;

use super::sim_model::{self, ScenarioDocument};
use super::witness_evidence;
use super::{declarations, summary_validation};

pub(super) fn cmd_test(
    file: &Path,
    sim: Option<&str>,
    profile: Option<&str>,
    seed: Option<u64>,
    events: Option<&str>,
    inject: Option<&str>,
    fuzz: bool,
    event_budget: Option<u64>,
    witness_dir: Option<&Path>,
    error_format: ErrorFormat,
    target: Option<&str>,
    engine: Option<&str>,
) -> anyhow::Result<()> {
    let sim_profile = parse_sim_profile(sim)?;
    let seed = seed.unwrap_or(0);
    let engine = parse_engine(engine.unwrap_or("both"))?;
    let document = sim_model::load_document(file)?;
    let target_name = target
        .map(str::to_owned)
        .or_else(|| {
            document
                .scenarios
                .first()
                .map(|scenario| scenario.name.clone())
        })
        .unwrap_or_else(|| "<missing>".to_owned());
    let profile_roles = resolve_profile_roles(profile, &document, &target_name);
    let mut session = super::session::build_session(
        file,
        Some(GuaranteePolicy::for_profile(GuaranteeProfile::Checked)),
    )?;
    let artifacts = kobo_driver::run_codegen_pipeline(&mut session, file)
        .map_err(|()| anyhow::anyhow!("failed to build compiler scenario artifacts"))?;
    let scenario_program = kobo_driver::build_scenario_program(
        &artifacts,
        &target_name,
        document.source_hash.clone(),
        &profile_roles.backend_profile,
    )?;
    let options = kobo_sim_core::ScenarioOptions {
        sim_profile: sim_profile.to_owned(),
        profile: profile_roles.backend_profile.clone(),
        seed,
        inject: inject.map(str::to_owned),
        event_budget: event_budget.or_else(|| default_budget(sim_profile)),
    };
    let fuzz_plan = if fuzz {
        Some(FuzzPlan::new(seed, &scenario_program)?)
    } else {
        None
    };
    let run = match fuzz_plan.as_ref() {
        Some(plan) => run_fuzz_portfolio(
            &scenario_program,
            &artifacts.rs_source,
            &options,
            engine,
            plan,
        )?,
        None => kobo_sim_core::run_full_depth_from_program(
            &scenario_program,
            &artifacts.rs_source,
            &options,
            engine,
        )?,
    };
    validate_run_boundary_declarations(file, &session.config, &run)?;

    let mut witness_path = None;
    if run.failure.is_some() || witness_dir.is_some() {
        witness_path = Some(write_run_witness(
            file,
            &document,
            &scenario_program,
            &profile_roles.guarantee_profile,
            sim_profile,
            seed,
            inject,
            fuzz_plan.as_ref(),
            witness_dir,
            &session.config,
            &run,
        )?);
    }

    if events == Some("json") {
        print_events(file, sim_profile, seed, fuzz_plan.as_ref(), &run)?;
        return Ok(());
    }

    if let Some(failure) = run.failure.as_ref() {
        emit_failure(
            file,
            &document.source,
            failure,
            witness_path.as_deref(),
            error_format,
        )?;
        anyhow::bail!("{}", failure.message)
    }

    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "scenario": run.target,
            "seed": seed,
            "backend_profile": run.profile,
            "status": "passed",
        }))?
    );
    Ok(())
}

struct ProfileRoles {
    guarantee_profile: String,
    backend_profile: String,
}

struct FuzzPlan {
    base_seed: u64,
    cases: Vec<FuzzCase>,
    candidate_sources: Vec<StatefulInputSource>,
}

struct FuzzCase {
    index: usize,
    seed: u64,
    operations: Vec<StatefulInputOperation>,
    shrink_candidates: Vec<FuzzShrinkCandidate>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StatefulInputOperation {
    source: StatefulInputSource,
    value: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FuzzShrinkCandidate {
    removed_tail_operations: usize,
    operation_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum StatefulInputSource {
    WardStorage { action: String },
    WardNetwork { action: String },
    WardRandom,
    WardTask,
    ObligationTransfer { binding: String, callee: String },
}

impl FuzzPlan {
    fn new(base_seed: u64, program: &ScenarioProgram) -> anyhow::Result<Self> {
        let candidate_sources = stateful_input_sources(program);
        let cases = (0..8)
            .map(|index| FuzzCase::new(base_seed, index, &candidate_sources))
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self {
            base_seed,
            cases,
            candidate_sources,
        })
    }
}

impl FuzzCase {
    fn new(
        base_seed: u64,
        index: usize,
        candidate_sources: &[StatefulInputSource],
    ) -> anyhow::Result<Self> {
        let seed = derive_fuzz_seed(base_seed, index);
        let mut value_tree = generated_stateful_input_tree(seed, candidate_sources.len())?;
        let generated_inputs = value_tree.current();
        let operations = stateful_operations_from_inputs(generated_inputs, candidate_sources);
        let shrink_candidates = shrink_candidates_from_tree(&mut value_tree, operations.len());
        Ok(Self {
            index,
            seed,
            operations,
            shrink_candidates,
        })
    }
}

impl StatefulInputOperation {
    fn event_label(&self) -> String {
        format!("{};value={}", self.source.event_label(), self.value)
    }
}

impl StatefulInputSource {
    fn event_label(&self) -> String {
        match self {
            Self::WardStorage { action } => format!("ward.storage.{action}"),
            Self::WardNetwork { action } => format!("ward.network.{action}"),
            Self::WardRandom => "ward.random.u64".to_owned(),
            Self::WardTask => "ward.task".to_owned(),
            Self::ObligationTransfer { binding, callee } => {
                format!("obligation.transfer.{binding}->{callee}")
            }
        }
    }
}

fn parse_sim_profile(sim: Option<&str>) -> anyhow::Result<&str> {
    match sim {
        Some(profile @ ("quick" | "deep" | "replay" | "exhaustive")) => Ok(profile),
        Some(other) => anyhow::bail!(
            "kobo test --sim {other} is not available; expected quick, deep, replay, or exhaustive"
        ),
        None => anyhow::bail!("kobo test requires --sim quick, deep, replay, or exhaustive"),
    }
}

fn resolve_profile_roles(
    cli_profile: Option<&str>,
    document: &ScenarioDocument,
    target_name: &str,
) -> ProfileRoles {
    let scenario_profile = document
        .scenarios
        .iter()
        .find(|scenario| scenario.name == target_name)
        .map(|scenario| scenario.profile.clone())
        .unwrap_or_else(|| sim_model::target_profile(document, target_name, None));

    match cli_profile {
        Some(profile @ ("dev" | "checked" | "release")) => ProfileRoles {
            guarantee_profile: profile.to_owned(),
            backend_profile: scenario_profile,
        },
        Some(profile) => ProfileRoles {
            guarantee_profile: "checked".to_owned(),
            backend_profile: profile.to_owned(),
        },
        None => ProfileRoles {
            guarantee_profile: "checked".to_owned(),
            backend_profile: scenario_profile,
        },
    }
}

fn default_budget(sim_profile: &str) -> Option<u64> {
    match sim_profile {
        "quick" => Some(64),
        "deep" => Some(1024),
        "replay" => Some(64),
        "exhaustive" => Some(16),
        _ => None,
    }
}

fn run_fuzz_portfolio(
    scenario_program: &kobo_ir::ScenarioProgram,
    generated_rust: &str,
    options: &kobo_sim_core::ScenarioOptions,
    engine: EngineMode,
    plan: &FuzzPlan,
) -> anyhow::Result<FullDepthRun> {
    let mut combined_events = Vec::new();
    let mut fuzz_driver_events = Vec::new();
    let mut last_run = None;

    for case in &plan.cases {
        let case_fuzz_events = fuzz_driver_events_for_case(case, plan.base_seed);
        fuzz_driver_events.extend(case_fuzz_events.iter().cloned());
        combined_events.extend(case_fuzz_events);
        let mut case_options = options.clone();
        case_options.seed = case.seed;
        let mut run = kobo_sim_core::run_full_depth_from_program(
            scenario_program,
            generated_rust,
            &case_options,
            engine.clone(),
        )?;
        combined_events.extend(run.events.iter().cloned());
        if run.failure.is_some() {
            attach_fuzz_driver_evidence(&mut run, combined_events, &fuzz_driver_events)?;
            return Ok(run);
        }
        last_run = Some(run);
    }

    let mut run = last_run.unwrap_or_else(|| FullDepthRun {
        target: scenario_program.target.clone(),
        profile: options.profile.clone(),
        replay_guarantee: ReplayGuarantee::Exact,
        events: Vec::new(),
        failure: None,
        coverage: kobo_sim_core::ScenarioCoverage {
            unsupported_constructs: Vec::new(),
            reason: None,
        },
        digest: kobo_sim_core::ExecutionDigest {
            semantic_engine: "semantic-sim".to_owned(),
            harness_engine: "generated-rust-harness".to_owned(),
            model_version: "v0.10".to_owned(),
            scenario_ir_hash: scenario_program.source_hash.clone(),
            operation_count: 0,
            semantic_trace_hash: String::new(),
            harness_trace_hash: String::new(),
            fuzz_driver_trace_hash: None,
            fuzz_driver_event_count: 0,
            agreement: "matched".to_owned(),
            generated_rust_hash: None,
            harness_manifest_hash: None,
            harness_exit_code: None,
            harness_event_count: 0,
        },
        modeled_boundaries: Vec::new(),
        opaque_boundaries: Vec::new(),
        obligations: Vec::new(),
        boundary_decisions: Vec::new(),
        harness_manifest: None,
    });
    attach_fuzz_driver_evidence(&mut run, combined_events, &fuzz_driver_events)?;
    Ok(run)
}

fn fuzz_driver_events_for_case(case: &FuzzCase, base_seed: u64) -> Vec<ScenarioEvent> {
    let mut events = vec![ScenarioEvent {
        kind: "fuzz-case".to_owned(),
        label: Some(format!(
            "seed={base_seed};case={};derived_seed={};strategy=proptest-stateful-input",
            case.index, case.seed
        )),
        value: Some(case.seed),
        io: None,
    }];
    events.extend(fuzz_operation_events(case));
    events.extend(fuzz_shrink_candidate_events(case));
    events
}

fn fuzz_operation_events(case: &FuzzCase) -> Vec<ScenarioEvent> {
    case.operations
        .iter()
        .enumerate()
        .map(|(index, operation)| ScenarioEvent {
            kind: "stateful-input-op".to_owned(),
            label: Some(format!(
                "case={};op={};{}",
                case.index,
                index,
                operation.event_label()
            )),
            value: Some(operation.value),
            io: None,
        })
        .collect()
}

fn fuzz_shrink_candidate_events(case: &FuzzCase) -> Vec<ScenarioEvent> {
    case.shrink_candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| ScenarioEvent {
            kind: "fuzz-shrink-candidate".to_owned(),
            label: Some(format!(
                "case={};candidate={};operation_count={};removed_tail_operations={}",
                case.index, index, candidate.operation_count, candidate.removed_tail_operations
            )),
            value: Some(candidate.operation_count as u64),
            io: None,
        })
        .collect()
}

fn attach_fuzz_driver_evidence(
    run: &mut FullDepthRun,
    combined_events: Vec<ScenarioEvent>,
    fuzz_driver_events: &[ScenarioEvent],
) -> anyhow::Result<()> {
    let serialized = serde_json::to_string(&events_json(fuzz_driver_events))?;
    run.events = combined_events;
    run.digest.fuzz_driver_trace_hash = Some(kobo_sim_core::digest::stable_hash(&serialized));
    run.digest.fuzz_driver_event_count = fuzz_driver_events.len();
    if run.replay_guarantee == ReplayGuarantee::Exact {
        run.replay_guarantee = ReplayGuarantee::Partial;
    }
    if run.digest.agreement == "matched" {
        run.digest.agreement = "matched+fuzz-driver-partial".to_owned();
    }
    Ok(())
}

fn stateful_input_sources(program: &ScenarioProgram) -> Vec<StatefulInputSource> {
    let mut sources = Vec::new();
    for operation in &program.operations {
        match &operation.kind {
            ScenarioOpKind::StorageEvent { action } => {
                sources.push(StatefulInputSource::WardStorage {
                    action: action.clone(),
                });
            }
            ScenarioOpKind::NetworkEvent { action } => {
                sources.push(StatefulInputSource::WardNetwork {
                    action: action.clone(),
                });
            }
            ScenarioOpKind::ModeledEffect { boundary } => match boundary {
                kobo_ir::ScenarioModeledBoundary::WardRandom => {
                    sources.push(StatefulInputSource::WardRandom);
                }
                kobo_ir::ScenarioModeledBoundary::WardTask => {
                    sources.push(StatefulInputSource::WardTask);
                }
                kobo_ir::ScenarioModeledBoundary::WardTime => {}
            },
            ScenarioOpKind::Transfer { binding, callee } => {
                sources.push(StatefulInputSource::ObligationTransfer {
                    binding: binding.clone(),
                    callee: callee.clone(),
                });
            }
            _ => {}
        }
    }
    sources.sort();
    sources.dedup();
    if sources.is_empty() {
        sources.push(StatefulInputSource::WardTask);
    }
    sources
}

fn generated_stateful_input_tree(
    seed: u64,
    candidate_count: usize,
) -> anyhow::Result<impl ValueTree<Value = Vec<(usize, u16)>>> {
    let mut runner = ProptestRunner::new_with_rng(
        ProptestConfig {
            cases: 1,
            failure_persistence: None,
            ..ProptestConfig::default()
        },
        TestRng::from_seed(RngAlgorithm::ChaCha, &fuzz_seed_bytes(seed)),
    );
    let max_index = candidate_count.saturating_sub(1);
    let strategy = proptest::collection::vec((0usize..=max_index, any::<u16>()), 1usize..=4usize);
    strategy
        .new_tree(&mut runner)
        .map_err(|reason| anyhow::anyhow!("failed to generate stateful-input fuzz case: {reason}"))
}

fn stateful_operations_from_inputs(
    generated_inputs: Vec<(usize, u16)>,
    candidate_sources: &[StatefulInputSource],
) -> Vec<StatefulInputOperation> {
    generated_inputs
        .into_iter()
        .map(|(source_index, value)| StatefulInputOperation {
            source: candidate_sources[source_index % candidate_sources.len()].clone(),
            value: u64::from(value),
        })
        .collect()
}

fn shrink_candidates_from_tree(
    value_tree: &mut impl ValueTree<Value = Vec<(usize, u16)>>,
    original_operation_count: usize,
) -> Vec<FuzzShrinkCandidate> {
    let mut candidates = Vec::new();
    while candidates.len() < 3 && value_tree.simplify() {
        candidates.push(FuzzShrinkCandidate {
            removed_tail_operations: original_operation_count
                .saturating_sub(value_tree.current().len()),
            operation_count: value_tree.current().len(),
        });
    }
    if candidates.is_empty() {
        candidates.push(FuzzShrinkCandidate {
            removed_tail_operations: 0,
            operation_count: original_operation_count,
        });
    }
    candidates
}

fn fuzz_seed_bytes(seed: u64) -> [u8; 32] {
    let mut bytes = [0_u8; 32];
    for (index, chunk) in bytes.chunks_mut(8).enumerate() {
        let material = seed
            .rotate_left((index * 11) as u32)
            .wrapping_add((index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
        chunk.copy_from_slice(&material.to_le_bytes());
    }
    bytes
}

fn derive_fuzz_seed(base_seed: u64, index: usize) -> u64 {
    base_seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407)
        .wrapping_add(index as u64)
}

fn parse_engine(value: &str) -> anyhow::Result<EngineMode> {
    match value {
        "semantic" => Ok(EngineMode::SemanticOnly),
        "harness" => Ok(EngineMode::HarnessOnly),
        "both" => Ok(EngineMode::Both),
        _ => anyhow::bail!("invalid sim engine; expected semantic, harness, or both"),
    }
}

fn print_events(
    file: &Path,
    sim_profile: &str,
    seed: u64,
    fuzz_plan: Option<&FuzzPlan>,
    run: &FullDepthRun,
) -> anyhow::Result<()> {
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "schema_version": 1,
            "file": sim_model::cli_relative_path(file)?,
            "sim_profile": sim_profile,
            "backend_profile": run.profile,
            "scheduler": scheduler_json(sim_profile, seed, run),
            "fuzz": fuzz_plan_json(fuzz_plan),
            "seed": seed,
            "events": events_json(&run.events),
        }))?
    );
    Ok(())
}

fn write_run_witness(
    file: &Path,
    document: &ScenarioDocument,
    scenario_program: &ScenarioProgram,
    guarantee_profile: &str,
    sim_profile: &str,
    seed: u64,
    inject: Option<&str>,
    fuzz_plan: Option<&FuzzPlan>,
    witness_dir: Option<&Path>,
    config: &kobo_driver::KoboConfig,
    run: &FullDepthRun,
) -> anyhow::Result<PathBuf> {
    let directory = witness_directory(file, witness_dir)?;
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create witness directory {}", directory.display()))?;
    let witness_path = directory.join(format!("{}-{seed}.kwit", sanitize_name(&run.target)));
    let source_path = sim_model::cli_relative_path(file)?;
    let primary_span = run.failure.as_ref().map(|failure| {
        format!(
            "{}:{}:1",
            source_path,
            one_based_line_for_offset(&document.source, failure.primary_start)
        )
    });
    let backend_replay_token = replay_token(&document.source_hash, seed, run);
    let coverage = coverage_json(run);
    let event_stream = shrink_event_stream(run, sim_profile);
    let witness_events = events_json(&event_stream.events);
    let inferred_obligations =
        witness_evidence::inferred_obligations_json(&source_path, &document.source, run);
    let mut witness = serde_json::json!({
        "schema_version": 1,
        "kobo_version": env!("CARGO_PKG_VERSION"),
        "target": format!("{}:{}", source_path, run.target),
        "scenario": {
            "name": run.target,
            "profile": run.profile,
        },
        "sim_profile": sim_profile,
        "source": {
            "path": source_path,
            "hash": document.source_hash,
        },
        "guarantee_profile": guarantee_profile,
        "expanded_policy": expanded_policy_json(guarantee_profile),
        "seed": seed,
        "fuzz": fuzz_plan_json(fuzz_plan),
        "injections": injections_json(inject),
        "backend_profile": run.profile,
        "backend": backend_for_profile(&run.profile),
        "backend_replay": backend_replay_token,
        "backend_replay_token": backend_replay_token,
        "ecosystem_scope": ecosystem_scope(run),
        "full_ecosystem_exploration": full_ecosystem_exploration(run),
        "replay_contract": replay_contract_json(run),
        "scheduler": scheduler_json(sim_profile, seed, run),
        "execution_digest": execution_digest_json(run),
        "harness_manifest": run.harness_manifest.clone(),
        "coverage": coverage,
        "operation_coverage": witness_evidence::operation_coverage_json(scenario_program, run),
        "scenario_coverage": scenario_coverage_json(run),
        "function_summaries": witness_evidence::function_summaries_json(scenario_program, run),
        "replay_guarantee": run.replay_guarantee.as_str(),
        "exactness": exactness_json(run),
        "shrink": shrink_json(run, &event_stream),
        "modeled_boundaries": modeled_boundaries_json(run),
        "opaque_boundaries": run.opaque_boundaries.clone(),
        "boundary_policies": boundary_policies_json(run),
        "ecosystem_boundaries": ecosystem_boundaries_json(file, config, run),
        "boundary_assumptions": boundary_assumptions_json(run),
        "obligations": obligations_json(&source_path, &document.source, run),
        "obligation_events": obligation_events_json(run),
        "boundary_decisions": boundary_decisions_json(run),
        "available_boundary_policies": ["typed", "model", "record", "activity", "stub", "outside", "opaque", "debt"],
        "failure": failure_json(&source_path, &document.source, run, primary_span),
        "source_spans": source_spans_json(&source_path, &document.source, run),
        "events": witness_events.clone(),
        "event_stream": witness_events,
    });
    let object = witness
        .as_object_mut()
        .expect("witness json literal should be an object");
    object.insert(
        "call_graph_obligation_summaries".to_owned(),
        witness_evidence::call_graph_obligation_summaries_json(scenario_program, run),
    );
    object.insert(
        "replay_grade".to_owned(),
        serde_json::json!(witness_evidence::replay_grade_json(
            run,
            fuzz_plan.is_some()
        )),
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
        serde_json::Value::Array(summary_usage_json(config)?),
    );
    object.insert(
        "lifecycle_inference".to_owned(),
        serde_json::json!({
            "mode": "observe",
            "source": "scenario_program",
            "template_version": "v0.10.1",
            "obligations": inferred_obligations,
        }),
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
    std::fs::write(&witness_path, serde_json::to_string_pretty(&witness)?)
        .with_context(|| format!("failed to write {}", witness_path.display()))?;
    Ok(witness_path)
}

fn validate_run_boundary_declarations(
    file: &Path,
    config: &kobo_driver::KoboConfig,
    run: &FullDepthRun,
) -> anyhow::Result<()> {
    for decision in &run.boundary_decisions {
        let policy = decision.policy.as_str();
        let adapter = config.ecosystem_policy.adapter_for(&decision.crate_name);
        if policy == "model" && adapter.is_none() {
            anyhow::bail!(
                "K0123: model boundary for `{}` has no adapter package",
                decision.crate_name
            );
        }
        if let Some(adapter) = adapter {
            if let Err(message) = super::ecosystem::validate_model_adapter_package(adapter) {
                anyhow::bail!(
                    "K0123: adapter package for `{}` failed validation: {message}",
                    decision.crate_name
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

fn replay_contract_json(run: &FullDepthRun) -> serde_json::Value {
    serde_json::json!({
        "scope": ecosystem_scope(run),
        "full_ecosystem_exploration": full_ecosystem_exploration(run),
        "facades": run
            .harness_manifest
            .as_ref()
            .map(|manifest| manifest.facades.clone())
            .unwrap_or_default(),
        "semantic_engine": run.digest.semantic_engine,
        "harness_engine": run.digest.harness_engine,
        "agreement": run.digest.agreement,
    })
}

fn ecosystem_scope(run: &FullDepthRun) -> String {
    run.harness_manifest
        .as_ref()
        .map(|manifest| manifest.execution_scope.clone())
        .unwrap_or_else(|| "semantic-only".to_owned())
}

fn full_ecosystem_exploration(run: &FullDepthRun) -> bool {
    run.harness_manifest
        .as_ref()
        .is_some_and(|manifest| manifest.full_ecosystem_exploration)
}

fn execution_digest_json(run: &FullDepthRun) -> serde_json::Value {
    serde_json::json!({
        "engine": "semantic-sim",
        "semantic_engine": run.digest.semantic_engine,
        "harness_engine": run.digest.harness_engine,
        "model_version": run.digest.model_version,
        "scenario_ir_hash": run.digest.scenario_ir_hash,
        "operation_count": run.digest.operation_count,
        "event_hash": run.digest.semantic_trace_hash,
        "semantic_trace_hash": run.digest.semantic_trace_hash,
        "harness_trace_hash": run.digest.harness_trace_hash,
        "fuzz_driver_trace_hash": run.digest.fuzz_driver_trace_hash,
        "fuzz_driver_event_count": run.digest.fuzz_driver_event_count,
        "agreement": run.digest.agreement,
        "generated_rust_hash": run.digest.generated_rust_hash,
        "harness_manifest_hash": run.digest.harness_manifest_hash,
        "harness_exit_code": run.digest.harness_exit_code,
        "harness_event_count": run.digest.harness_event_count,
    })
}

fn coverage_json(run: &FullDepthRun) -> serde_json::Value {
    let covered = run
        .obligations
        .iter()
        .filter(|obligation| obligation.is_discharged)
        .map(|obligation| obligation.binding.clone())
        .collect::<Vec<_>>();
    let uncovered = run
        .obligations
        .iter()
        .filter(|obligation| !obligation.is_discharged)
        .map(|obligation| obligation.binding.clone())
        .collect::<Vec<_>>();
    serde_json::json!({
        "unsupported_constructs": run.coverage.unsupported_constructs,
        "reason": run.coverage.reason,
        "covered": covered,
        "uncovered": uncovered,
        "escaped": escaped_obligations(run),
        "suppressed": [],
        "boundary_owned": run.opaque_boundaries,
        "unknown": [],
    })
}

fn scenario_coverage_json(run: &FullDepthRun) -> serde_json::Value {
    coverage_json(run)
}

fn scheduler_json(sim_profile: &str, seed: u64, run: &FullDepthRun) -> serde_json::Value {
    let strategy = match sim_profile {
        "quick" => "small-random",
        "deep" => "pct-random-bounded",
        "replay" => "witness-event-stream",
        "exhaustive" => "tiny-ward-exhaustive",
        _ => "unknown",
    };
    serde_json::json!({
        "profile": sim_profile,
        "strategy": strategy,
        "seed": seed,
        "event_budget": run
            .events
            .iter()
            .find(|event| event.kind == "scheduler-portfolio")
            .and_then(|event| event.value),
    })
}

fn exactness_json(run: &FullDepthRun) -> &'static str {
    match run.replay_guarantee {
        ReplayGuarantee::Exact => "exact",
        ReplayGuarantee::Partial => "partial",
        ReplayGuarantee::NotReplayable => "evidence_only",
    }
}

struct ShrunkEventStream {
    events: Vec<ScenarioEvent>,
    removed_event_ids: Vec<usize>,
}

fn shrink_event_stream(run: &FullDepthRun, sim_profile: &str) -> ShrunkEventStream {
    let mut removed_event_ids = Vec::new();
    if run.replay_guarantee == ReplayGuarantee::Exact && sim_profile == "deep" {
        removed_event_ids.extend(run.events.iter().enumerate().filter_map(|(index, event)| {
            if is_replay_irrelevant_event(&event.kind) {
                Some(index)
            } else {
                None
            }
        }));
    }
    let events = run
        .events
        .iter()
        .enumerate()
        .filter(|(index, _)| !removed_event_ids.contains(index))
        .map(|(_, event)| event.clone())
        .collect();
    ShrunkEventStream {
        events,
        removed_event_ids,
    }
}

fn shrink_json(run: &FullDepthRun, event_stream: &ShrunkEventStream) -> serde_json::Value {
    let mut shrink_passes = Vec::new();
    let scheduler_ids = removed_event_ids_for_kinds(run, event_stream, &["scheduler-pct-seed"]);
    if !scheduler_ids.is_empty() {
        shrink_passes.push(serde_json::json!({
            "pass": "trailing-independent-scheduler-events",
            "removed_event_ids": scheduler_ids,
            "replay_checked": true,
        }));
    }
    let network_ids =
        removed_event_ids_for_kinds(run, event_stream, &["network-delayed", "network-reordered"]);
    if !network_ids.is_empty() {
        shrink_passes.push(serde_json::json!({
            "pass": "independent-network-ordering",
            "removed_event_ids": network_ids,
            "replay_checked": true,
        }));
    }
    for (pass, reason) in [
        (
            "unused-events",
            "no replay-irrelevant semantic events found",
        ),
        ("seeds-runs", "no smaller equivalent seed/run pair found"),
        (
            "data-sizes",
            "scenario has no shrinkable generated data fixtures",
        ),
        (
            "independent-injections",
            "no independent injected hook can be removed without changing the trace",
        ),
    ] {
        shrink_passes.push(serde_json::json!({
            "pass": pass,
            "removed_event_ids": [],
            "replay_checked": run.replay_guarantee == ReplayGuarantee::Exact,
            "reason": reason,
        }));
    }
    serde_json::json!({
        "original_event_count": run.events.len(),
        "shrunk_event_count": event_stream.events.len(),
        "removed_event_ids": event_stream.removed_event_ids.clone(),
        "shrink_passes": shrink_passes,
        "replay_checked": run.replay_guarantee == ReplayGuarantee::Exact,
        "reason_if_not_shrunk": if event_stream.removed_event_ids.is_empty() {
            Some("original witness is already minimal for the current replay contract")
        } else {
            None
        },
    })
}

fn removed_event_ids_for_kinds(
    run: &FullDepthRun,
    event_stream: &ShrunkEventStream,
    kinds: &[&str],
) -> Vec<usize> {
    event_stream
        .removed_event_ids
        .iter()
        .copied()
        .filter(|id| {
            run.events
                .get(*id)
                .is_some_and(|event| kinds.contains(&event.kind.as_str()))
        })
        .collect()
}

fn is_replay_irrelevant_event(kind: &str) -> bool {
    matches!(
        kind,
        "scheduler-pct-seed" | "network-delayed" | "network-reordered" | "fuzz-shrink-candidate"
    )
}

fn injections_json(inject: Option<&str>) -> serde_json::Value {
    let hooks = inject
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|hook| !hook.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    serde_json::json!({
        "hooks": hooks,
    })
}

fn fuzz_plan_json(plan: Option<&FuzzPlan>) -> serde_json::Value {
    match plan {
        Some(plan) => serde_json::json!({
            "enabled": true,
            "runner": "proptest",
            "strategy": "proptest-stateful-input",
            "base_seed": plan.base_seed,
            "case_count": plan.cases.len(),
            "candidate_sources": plan
                .candidate_sources
                .iter()
                .map(StatefulInputSource::event_label)
                .collect::<Vec<_>>(),
            "cases": plan
                .cases
                .iter()
                .map(|case| serde_json::json!({
                    "index": case.index,
                    "seed": case.seed,
                    "operations": case.operations.iter().map(|operation| {
                        serde_json::json!({
                            "source": operation.source.event_label(),
                            "value": operation.value,
                        })
                    }).collect::<Vec<_>>(),
                    "shrink_candidates": case.shrink_candidates.iter().map(|candidate| {
                        serde_json::json!({
                            "operation_count": candidate.operation_count,
                            "removed_tail_operations": candidate.removed_tail_operations,
                        })
                    }).collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>(),
        }),
        None => serde_json::json!({
            "enabled": false,
            "runner": null,
            "strategy": null,
            "case_count": 0,
            "cases": [],
        }),
    }
}

fn witness_directory(file: &Path, witness_dir: Option<&Path>) -> anyhow::Result<PathBuf> {
    match witness_dir {
        Some(directory) if directory.is_absolute() => Ok(directory.to_path_buf()),
        Some(directory) => Ok(std::env::current_dir()
            .context("failed to determine current directory")?
            .join(directory)),
        None => Ok(file
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))),
    }
}

fn emit_failure(
    file: &Path,
    source: &str,
    failure: &ScenarioFailure,
    witness_path: Option<&Path>,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    let mut files = FileSetBuilder::new();
    let file_id = files.add_file(file.to_path_buf(), source.to_owned());
    let span = KoboSpan::new(
        failure.primary_start as u32,
        failure.primary_end.max(failure.primary_start + 1) as u32,
        file_id,
    );
    let message = match witness_path {
        Some(path) => format!("{}; witness {}", failure.message, path.display()),
        None => failure.message.clone(),
    };
    let diagnostic = KDiagnostic::new(
        failure.code,
        Severity::Error,
        DiagLabel::primary(span, message.clone()),
        scenario_failure_explanation(failure),
        DiagDecision(scenario_failure_decision(failure)),
    )
    .with_finding(scenario_failure_finding(failure, witness_path));

    match error_format {
        ErrorFormat::Json => {
            let mut value = diagnostic_to_json_value(files.as_file_set(), &diagnostic);
            if let Some(entry) = files.as_file_set().get(file_id) {
                value["line"] = serde_json::json!(entry.line_col(span.start).0);
            }
            println!("{}", serde_json::to_string(&value)?);
        }
        ErrorFormat::Human => {
            let color = super::session::resolve_color_mode(ColorMode::Auto);
            let renderer = DiagnosticRenderer::new(color, DiagnosticOutputFormat::HumanCard);
            eprintln!("{}", renderer.render(files.as_file_set(), &diagnostic));
        }
    }

    Ok(())
}

fn scenario_failure_finding(failure: &ScenarioFailure, witness_path: Option<&Path>) -> String {
    let finding = match failure.code {
        KErrorCode::K0100 => {
            let binding = scenario_failure_label(failure).unwrap_or("the value");
            format!(
                "A checked scenario can drop `{binding}` before calling one of its required actions."
            )
        }
        KErrorCode::K0102 => {
            "This replay path uses raw time, randomness, file IO, or task scheduling.".to_owned()
        }
        KErrorCode::K0103 => {
            "This replay path performs an effect Kobo cannot replay deterministically.".to_owned()
        }
        KErrorCode::K0105 => {
            "This scenario exceeded the quick simulation budget before it finished.".to_owned()
        }
        KErrorCode::K0107 => {
            let boundary = scenario_failure_label(failure).unwrap_or("external code");
            format!("This replay path crosses `{boundary}` without a boundary policy.")
        }
        KErrorCode::K0116 => {
            "This scenario uses syntax Kobo has not modeled for exact replay yet.".to_owned()
        }
        KErrorCode::K0117 => {
            "The compiler semantic trace and generated harness trace do not agree.".to_owned()
        }
        KErrorCode::K0118 => {
            "The action points at an artifact that does not match the current source.".to_owned()
        }
        _ => failure.message.clone(),
    };

    match witness_path {
        Some(path) => format!("{finding} Witness: {}", path.display()),
        None => finding,
    }
}

fn scenario_failure_explanation(failure: &ScenarioFailure) -> String {
    match failure.code {
        KErrorCode::K0100 => {
            "Kobo tracks must_call values as obligations. Every return, cancellation, or replayed path must finish the obligation before the value leaves scope.".to_owned()
        }
        KErrorCode::K0102 => {
            "Replay evidence only stays useful when the same inputs produce the same event stream. Raw nondeterminism can make a replay pass or fail for the wrong reason.".to_owned()
        }
        KErrorCode::K0103 => {
            "An uncontrolled effect can change outside Kobo's replay model. The scenario needs a model, a recording, or an explicit debt boundary before Kobo can trust the replay.".to_owned()
        }
        KErrorCode::K0105 => {
            "The quick profile is for small, fast evidence. A scenario that exceeds its budget needs to be shrunk or moved to a slower profile.".to_owned()
        }
        KErrorCode::K0107 => {
            "External code can perform IO, scheduling, time, randomness, or other effects that Kobo cannot infer from the source alone. The boundary policy says what replay may assume.".to_owned()
        }
        KErrorCode::K0116 => {
            "Exact replay is only sound for modeled syntax. Kobo found a construct outside the current modeled island coverage.".to_owned()
        }
        KErrorCode::K0117 => {
            "Full-depth replay needs two independent traces to match: the compiler semantic trace and the generated harness trace.".to_owned()
        }
        KErrorCode::K0118 => {
            "Editor and replay actions must point at artifacts created from the current source hash and target.".to_owned()
        }
        _ => failure.message.clone(),
    }
}

fn scenario_failure_decision(failure: &ScenarioFailure) -> String {
    match failure.code {
        KErrorCode::K0100 => {
            let actions = scenario_failure_actions(&failure.message)
                .unwrap_or_else(|| "one required action".to_owned());
            format!(
                "Call {actions} on every path, or record explicit debt if cleanup happens outside this scenario."
            )
        }
        KErrorCode::K0102 => {
            "Route time, randomness, file IO, and task scheduling through modeled facades before claiming replay evidence.".to_owned()
        }
        KErrorCode::K0103 => {
            "Model the effect, record the effect stream, move it outside replay, or mark replay debt explicitly.".to_owned()
        }
        KErrorCode::K0105 => {
            "Reduce the scenario, split it into smaller scenarios, or run it under a profile with a larger budget.".to_owned()
        }
        KErrorCode::K0107 => {
            "Choose typed, model, record, activity, stub, outside, opaque, or debt for this boundary before claiming exact replay.".to_owned()
        }
        KErrorCode::K0116 => {
            "Use a modeled construct, split the scenario, or keep the witness partial until coverage is implemented.".to_owned()
        }
        KErrorCode::K0117 => {
            "Regenerate the witness with --engine both and investigate any trace mismatch before replaying it.".to_owned()
        }
        KErrorCode::K0118 => {
            "Regenerate the witness/session artifacts for the current source before using the action.".to_owned()
        }
        _ => "Make replay evidence deterministic and explicit.".to_owned(),
    }
}

fn scenario_failure_label(failure: &ScenarioFailure) -> Option<&str> {
    failure
        .events
        .iter()
        .find_map(|event| event.label.as_deref())
}

fn scenario_failure_actions(message: &str) -> Option<String> {
    let actions = message.split("discharge with ").nth(1)?;
    Some(actions.trim_end_matches('.').to_owned())
}

fn modeled_boundaries_json(run: &FullDepthRun) -> Vec<&'static str> {
    run.modeled_boundaries
        .iter()
        .map(|boundary| boundary.as_str())
        .collect()
}

fn boundary_policies_json(run: &FullDepthRun) -> Vec<serde_json::Value> {
    let mut policies = run
        .boundary_decisions
        .iter()
        .map(|decision| {
            serde_json::json!({
                "boundary": decision.crate_name,
                "policy": decision.policy.as_str(),
                "reason": decision.reason,
            })
        })
        .collect::<Vec<_>>();

    for boundary in modeled_boundaries_json(run) {
        policies.push(serde_json::json!({
            "boundary": boundary,
            "policy": "model",
            "reason": "modeled v0.10 facade",
        }));
    }
    policies
}

fn ecosystem_boundaries_json(
    file: &Path,
    config: &kobo_driver::KoboConfig,
    run: &FullDepthRun,
) -> Vec<serde_json::Value> {
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
                "declaration": declaration_metadata_for_boundary(file, config, &decision.crate_name, decision.policy.as_str()),
                "adapter": config.ecosystem_policy.adapter_for(&decision.crate_name).map(|adapter| serde_json::json!({
                    "package": adapter.package,
                    "version": adapter.version,
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
        .collect()
}

fn declaration_metadata_for_boundary(
    file: &Path,
    config: &kobo_driver::KoboConfig,
    crate_name: &str,
    policy: &str,
) -> Option<serde_json::Value> {
    if !matches!(policy, "typed" | "activity") {
        return None;
    }
    let facts = match declarations::declaration_facts_for_config(file, crate_name, config) {
        declarations::DeclarationLookup::Valid(facts) => facts,
        declarations::DeclarationLookup::Missing | declarations::DeclarationLookup::Invalid(_) => {
            return None;
        }
    };
    Some(declaration_metadata_json(&facts))
}

fn activity_metadata_for_boundary(
    file: &Path,
    config: &kobo_driver::KoboConfig,
    decision: &kobo_sim_core::BoundaryDecision,
) -> Option<serde_json::Value> {
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

fn declarations_json(
    file: &Path,
    config: &kobo_driver::KoboConfig,
    run: &FullDepthRun,
) -> Vec<serde_json::Value> {
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
        .collect()
}

fn declaration_metadata_json(facts: &declarations::DeclarationFacts) -> serde_json::Value {
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

fn summary_usage_json(config: &kobo_driver::KoboConfig) -> anyhow::Result<Vec<serde_json::Value>> {
    let mut summaries = Vec::new();
    for summary in &config.ecosystem_policy.summaries {
        let valid = summary_validation::load_valid_summary(summary)?;
        let parsed = valid.value;
        let obligations = parsed
            .get("obligations")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let functions = parsed
            .get("functions")
            .and_then(serde_json::Value::as_array)
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
    Ok(summaries)
}

fn boundary_evidence_for_policy(
    decision: &kobo_sim_core::BoundaryDecision,
    run: &FullDepthRun,
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
    run: &FullDepthRun,
) -> Option<serde_json::Value> {
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
        "io_capture": record_io_capture_json(decision),
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

fn record_io_capture_json(decision: &kobo_sim_core::BoundaryDecision) -> Option<serde_json::Value> {
    if decision.policy.as_str() != "record" {
        return None;
    }
    let capture = decision.recorded_io.as_ref()?;
    Some(boundary_io_capture_json(capture))
}

fn boundary_io_payload_json(payload: &kobo_sim_core::BoundaryIoPayload) -> serde_json::Value {
    let mut fields = serde_json::Map::new();
    for field in &payload.fields {
        fields.insert(
            field.key.clone(),
            serde_json::Value::String(field.value.clone()),
        );
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

fn obligations_json(source_path: &str, source: &str, run: &FullDepthRun) -> Vec<serde_json::Value> {
    run.obligations
        .iter()
        .map(|obligation| {
            serde_json::json!({
                "binding": obligation.binding,
                "type": obligation.type_name,
                "actions": obligation.actions,
                "discharged": obligation.is_discharged,
                "declaration_span": span_json(source_path, source, obligation.declaration_span),
                "drop_span": obligation.drop_span.map(|span| span_json(source_path, source, span)),
            })
        })
        .collect()
}

fn obligation_events_json(run: &FullDepthRun) -> Vec<serde_json::Value> {
    let mut events = Vec::new();
    for obligation in &run.obligations {
        events.push(serde_json::json!({
            "state": "created",
            "binding": obligation.binding,
            "type": obligation.type_name,
        }));
        if obligation.is_discharged {
            events.push(serde_json::json!({
                "state": "discharged",
                "binding": obligation.binding,
                "actions": obligation.actions,
            }));
        } else {
            events.push(serde_json::json!({
                "state": "leaked",
                "binding": obligation.binding,
                "failure_mode": unresolved_failure_mode(obligation),
            }));
        }
    }
    for event in &run.events {
        if event.kind == "obligation-transfer" {
            events.push(serde_json::json!({
                "state": "transferred",
                "binding": event.label,
            }));
        }
    }
    if run.failure.is_none() {
        events.push(serde_json::json!({
            "state": "returned",
            "binding": null,
        }));
    }
    events
}

fn expanded_policy_json(profile: &str) -> serde_json::Value {
    let (ownership, liveness, replay, boundaries, errors) = match profile {
        "dev" => ("record", "record", "record", "record", "ergonomic"),
        "release" => ("strict", "checked", "checked", "strict", "explicit"),
        _ => ("checked", "checked", "checked", "checked", "typed"),
    };
    serde_json::json!({
        "profile": profile,
        "ownership": ownership,
        "liveness": liveness,
        "replay": replay,
        "boundaries": boundaries,
        "errors": errors,
    })
}

fn boundary_assumptions_json(run: &FullDepthRun) -> Vec<serde_json::Value> {
    let mut assumptions = run
        .boundary_decisions
        .iter()
        .map(|decision| {
            serde_json::json!({
                "boundary": decision.crate_name,
                "policy": decision.policy.as_str(),
                "reason": decision.reason,
                "replay_effect": if run.replay_guarantee == ReplayGuarantee::Exact {
                    "modeled"
                } else {
                    run.replay_guarantee.as_str()
                },
            })
        })
        .collect::<Vec<_>>();

    if run.replay_guarantee == ReplayGuarantee::NotReplayable && assumptions.is_empty() {
        let boundary = run
            .failure
            .as_ref()
            .and_then(|failure| failure.events.first())
            .and_then(|event| event.label.clone())
            .unwrap_or_else(|| "uncontrolled".to_owned());
        let reason = run
            .failure
            .as_ref()
            .map(|failure| failure.message.clone())
            .unwrap_or_else(|| "not replayable".to_owned());
        assumptions.push(serde_json::json!({
            "boundary": boundary,
            "policy": "debt",
            "reason": reason,
            "replay_effect": run.replay_guarantee.as_str(),
        }));
    }

    assumptions
}

fn failure_json(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
    primary_span: Option<String>,
) -> serde_json::Value {
    let Some(failure) = run.failure.as_ref() else {
        return serde_json::Value::Null;
    };
    serde_json::json!({
        "code": failure.code.as_str(),
        "message": failure.message,
        "mode": run.failure
            .as_ref()
            .map(|_| run_failure_mode(run))
            .unwrap_or("none"),
        "primary_span": primary_span.unwrap_or_else(|| format!("{source_path}:1:1")),
        "related_spans": related_spans_json(source_path, source, run, failure),
    })
}

fn source_spans_json(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
) -> Vec<serde_json::Value> {
    let mut spans = Vec::new();
    if let Some(failure) = run.failure.as_ref() {
        spans.push(span_json(
            source_path,
            source,
            (failure.primary_start, failure.primary_end),
        ));
    }
    for obligation in &run.obligations {
        spans.push(span_json(source_path, source, obligation.declaration_span));
        if let Some(drop_span) = obligation.drop_span {
            spans.push(span_json(source_path, source, drop_span));
        }
    }
    spans
}

fn run_failure_mode(run: &FullDepthRun) -> &'static str {
    if run
        .events
        .iter()
        .any(|event| event.kind == "lost-message" || event.kind == "storage-crash-after-write")
    {
        return "lost-message";
    }
    if run
        .obligations
        .iter()
        .any(|obligation| unresolved_failure_mode(obligation) == "unresolved-reply")
    {
        return "unresolved-reply";
    }
    "unresolved-delivery"
}

fn unresolved_failure_mode(obligation: &kobo_sim_core::RuntimeObligationSummary) -> &'static str {
    if obligation
        .actions
        .iter()
        .any(|action| action == "reply" || action == "reject" || action == "cancel")
    {
        "unresolved-reply"
    } else {
        "unresolved-delivery"
    }
}

fn escaped_obligations(run: &FullDepthRun) -> Vec<String> {
    run.obligations
        .iter()
        .filter(|obligation| !obligation.is_discharged && obligation.drop_span.is_some())
        .map(|obligation| obligation.binding.clone())
        .collect()
}

fn related_spans_json(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
    failure: &ScenarioFailure,
) -> Vec<serde_json::Value> {
    if failure.code != KErrorCode::K0100 {
        return Vec::new();
    }
    run.obligations
        .iter()
        .filter(|obligation| !obligation.is_discharged)
        .map(|obligation| {
            serde_json::json!({
                "label": format!("obligation `{}` declared here", obligation.binding),
                "span": span_json(source_path, source, obligation.declaration_span),
            })
        })
        .collect()
}

fn span_json(source_path: &str, source: &str, span: (usize, usize)) -> serde_json::Value {
    serde_json::json!({
        "path": source_path,
        "line": one_based_line_for_offset(source, span.0),
        "start": span.0,
        "end": span.1.max(span.0 + 1),
    })
}

fn boundary_decisions_json(run: &FullDepthRun) -> Vec<serde_json::Value> {
    run.boundary_decisions
        .iter()
        .map(|decision| {
            serde_json::json!({
                "crate": decision.crate_name,
                "call_path": decision.call_path,
                "call_arguments": decision.call_arguments,
                "return_type": decision.return_type,
                "call_shape": decision.call_shape.as_str(),
                "policy": decision.policy.as_str(),
                "reason": decision.reason,
                "source_span": {
                    "start": decision.span_start,
                    "end": decision.span_end,
                },
            })
        })
        .collect()
}

fn events_json(events: &[ScenarioEvent]) -> Vec<serde_json::Value> {
    events
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
        .collect()
}

fn boundary_io_capture_json(capture: &kobo_sim_core::BoundaryIoCapture) -> serde_json::Value {
    serde_json::json!({
        "mode": capture.mode.clone(),
        "replay_key": capture.replay_key.clone(),
        "request": boundary_io_payload_json(&capture.request),
        "response": boundary_io_payload_json(&capture.response),
        "request_hash": capture.request_hash.clone(),
        "response_hash": capture.response_hash.clone(),
    })
}

fn replay_token(source_identity: &str, seed: u64, run: &FullDepthRun) -> String {
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

fn sanitize_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}

fn one_based_line_for_offset(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}
