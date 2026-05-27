use super::json_schema::events_json;
use super::*;

pub(super) struct FuzzPlan {
    base_seed: u64,
    cases: Vec<FuzzCase>,
    candidate_sources: Vec<StatefulInputSource>,
}

pub(super) struct FuzzCase {
    index: usize,
    seed: u64,
    operations: Vec<StatefulInputOperation>,
    shrink_candidates: Vec<FuzzShrinkCandidate>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StatefulInputOperation {
    source: StatefulInputSource,
    value: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FuzzShrinkCandidate {
    removed_tail_operations: usize,
    operation_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) enum StatefulInputSource {
    WardStorage { action: String },
    WardNetwork { action: String },
    WardRandom,
    WardTask,
    ObligationTransfer { binding: String, callee: String },
}

impl FuzzPlan {
    pub(super) const DEFAULT_CASES: u64 = 8;

    pub(super) fn new(
        base_seed: u64,
        program: &ScenarioProgram,
        case_count: u64,
    ) -> anyhow::Result<Self> {
        let candidate_sources = stateful_input_sources(program);
        let cases = (0..case_count)
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
    pub(super) fn new(
        base_seed: u64,
        index: u64,
        candidate_sources: &[StatefulInputSource],
    ) -> anyhow::Result<Self> {
        let seed = derive_fuzz_seed(base_seed, index);
        let mut value_tree = generated_stateful_input_tree(seed, candidate_sources.len())?;
        let generated_inputs = value_tree.current();
        let operations = stateful_operations_from_inputs(generated_inputs, candidate_sources);
        let shrink_candidates = shrink_candidates_from_tree(&mut value_tree, operations.len());
        Ok(Self {
            index: usize::try_from(index)
                .map_err(|_| anyhow::anyhow!("seed_count is too large for this platform"))?,
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

pub(super) fn run_fuzz_portfolio(
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
            model_schema: kobo_sim_core::lower::MODEL_SCHEMA.to_owned(),
            schema_version: kobo_sim_core::lower::MODEL_SCHEMA_VERSION,
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

pub(super) fn run_seed_portfolio(
    scenario_program: &kobo_ir::ScenarioProgram,
    generated_rust: &str,
    options: &kobo_sim_core::ScenarioOptions,
    engine: EngineMode,
    seed_count: u64,
) -> anyhow::Result<(FullDepthRun, u64)> {
    if seed_count <= 1 {
        let run = kobo_sim_core::run_full_depth_from_program(
            scenario_program,
            generated_rust,
            options,
            engine,
        )?;
        return Ok((run, options.seed));
    }

    let mut combined_events = Vec::new();
    let mut last_run = None;
    let mut last_seed = options.seed;
    for index in 0..seed_count {
        let seed = options.seed.wrapping_add(index);
        combined_events.push(seed_case_event(index, seed, seed_count));
        let mut case_options = options.clone();
        case_options.seed = seed;
        let mut run = kobo_sim_core::run_full_depth_from_program(
            scenario_program,
            generated_rust,
            &case_options,
            engine.clone(),
        )?;
        combined_events.extend(run.events.iter().cloned());
        if run.failure.is_some() {
            run.events = combined_events;
            refresh_seed_portfolio_digest(&mut run);
            return Ok((run, seed));
        }
        last_seed = seed;
        last_run = Some(run);
    }

    let mut run = last_run.expect("seed_count greater than one should execute at least one seed");
    run.events = combined_events;
    refresh_seed_portfolio_digest(&mut run);
    Ok((run, last_seed))
}

pub(super) fn seed_case_event(index: u64, seed: u64, seed_count: u64) -> ScenarioEvent {
    ScenarioEvent {
        kind: "scheduler-seed-case".to_owned(),
        label: Some(format!("index={index};count={seed_count}")),
        value: Some(seed),
        io: None,
    }
}

pub(super) fn refresh_seed_portfolio_digest(run: &mut FullDepthRun) {
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

pub(super) fn fuzz_driver_events_for_case(case: &FuzzCase, base_seed: u64) -> Vec<ScenarioEvent> {
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

pub(super) fn fuzz_operation_events(case: &FuzzCase) -> Vec<ScenarioEvent> {
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

pub(super) fn fuzz_shrink_candidate_events(case: &FuzzCase) -> Vec<ScenarioEvent> {
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

pub(super) fn attach_fuzz_driver_evidence(
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

pub(super) fn stateful_input_sources(program: &ScenarioProgram) -> Vec<StatefulInputSource> {
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
                kobo_ir::ScenarioModeledBoundary::WardTaskLocal => {
                    sources.push(StatefulInputSource::WardTask);
                }
                kobo_ir::ScenarioModeledBoundary::WardTime => {}
            },
            ScenarioOpKind::Transfer {
                binding, callee, ..
            } => {
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

pub(super) fn generated_stateful_input_tree(
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

pub(super) fn stateful_operations_from_inputs(
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

pub(super) fn shrink_candidates_from_tree(
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

pub(super) fn fuzz_seed_bytes(seed: u64) -> [u8; 32] {
    let mut bytes = [0_u8; 32];
    for (index, chunk) in bytes.chunks_mut(8).enumerate() {
        let material = seed
            .rotate_left((index * 11) as u32)
            .wrapping_add((index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
        chunk.copy_from_slice(&material.to_le_bytes());
    }
    bytes
}

pub(super) fn derive_fuzz_seed(base_seed: u64, index: u64) -> u64 {
    base_seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407)
        .wrapping_add(index)
}

pub(super) fn fuzz_plan_json(plan: Option<&FuzzPlan>) -> serde_json::Value {
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
