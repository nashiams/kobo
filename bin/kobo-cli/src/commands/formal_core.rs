use std::collections::{BTreeMap, BTreeSet, VecDeque};

use kobo_errors::KErrorCode;
use kobo_ir::{
    lower_core_program, CoreBlock, CoreFunction, CoreStatement, CoreStatementKind, CoreTerminator,
    CoreTerminatorKind, KoboSpan, ScenarioProgram,
};
use kobo_sim_core::{FullDepthRun, ScenarioEvent, ScenarioFailure};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
struct CoreSourceSpan {
    path: String,
    line: usize,
    start: usize,
    end: usize,
    mapped: bool,
    snippet: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActiveObligation {
    binding: String,
    source_span: CoreSourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedObligation {
    binding: String,
    resolution: String,
    source_span: CoreSourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ObligationState {
    Owned(ActiveObligation),
    Resolved(ResolvedObligation),
}

type ObligationEnv = BTreeMap<String, ObligationState>;

#[derive(Clone, Debug)]
struct StrictLivenessError {
    exit_kind: String,
    binding: String,
    source_span: CoreSourceSpan,
    obligation_span: CoreSourceSpan,
    message: String,
}

#[derive(Clone, Debug)]
struct StrictResolvedPath {
    binding: String,
    resolution: String,
    reason: Option<String>,
    source_span: CoreSourceSpan,
}

#[derive(Clone, Debug, Default)]
struct StrictLivenessAnalysis {
    errors: Vec<StrictLivenessError>,
    resolved_paths: Vec<StrictResolvedPath>,
}

pub(super) fn formal_core_json(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
) -> Value {
    let core = lower_core_program(program);
    serde_json::json!({
        "source": core.source,
        "cfg_source": core.cfg_source,
        "core_version": core.core_version,
        "functions": core
            .functions
            .into_iter()
            .map(|function| function_json(source_path, source, function))
            .collect::<Vec<_>>(),
    })
}

pub(super) fn apply_strict_liveness(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
    run: &mut FullDepthRun,
) {
    let analysis = analyze_strict_liveness(source_path, source, program, run);
    if run.failure.is_none() {
        if let Some(error) = analysis.errors.first() {
            run.failure = Some(ScenarioFailure {
                code: KErrorCode::K0100,
                message: error.message.clone(),
                primary_start: error.source_span.start,
                primary_end: error.source_span.end,
                events: vec![ScenarioEvent {
                    kind: format!("strict-liveness-{}", error.exit_kind),
                    label: Some(error.binding.clone()),
                    value: None,
                    io: None,
                }],
            });
        }
    }
}

pub(super) fn strict_liveness_json(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
    run: &FullDepthRun,
) -> Value {
    let analysis = analyze_strict_liveness(source_path, source, program, run);
    serde_json::json!({
        "source": "compiler_core_forward_dataflow",
        "theorem_target": "no_unresolved_local_obligation_on_modeled_exit",
        "dataflow": {
            "engine": "core_cfg_forward",
            "join_semantics": "semantic_obligation_state",
            "states": [
                "owned",
                "discharged",
                "returned",
                "transferred",
                "escaped",
                "suppressed",
                "proof_failure",
            ],
        },
        "status": if analysis.errors.is_empty() { "passed" } else { "failed" },
        "join_conflicts": analysis.errors.iter()
            .filter(|error| error.exit_kind == "semantic_join")
            .cloned()
            .map(strict_error_json)
            .collect::<Vec<_>>(),
        "errors": analysis.errors.into_iter().map(strict_error_json).collect::<Vec<_>>(),
        "resolved_paths": analysis.resolved_paths.iter().map(resolved_path_json).collect::<Vec<_>>(),
        "reasoned_suppressions": analysis.resolved_paths.iter()
            .filter(|path| path.resolution == "suppressed")
            .map(resolved_path_json)
            .collect::<Vec<_>>(),
    })
}

pub(super) fn proof_seed_json(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
    run: &FullDepthRun,
) -> Value {
    let core = lower_core_program(program);
    let core_edges = core
        .functions
        .iter()
        .flat_map(|function| {
            function.blocks.iter().flat_map(move |block| {
                block.terminators.iter().map(move |terminator| {
                    serde_json::json!({
                        "function": function.name,
                        "block": block.id,
                        "kind": terminator.kind.as_str(),
                        "edges": terminator.edges,
                        "source_span": source_span_json(&source_span_from_kobo(
                            source_path,
                            source,
                            terminator.source_span,
                        )),
                    })
                })
            })
        })
        .collect::<Vec<_>>();
    let obligation_states = run
        .obligations
        .iter()
        .map(|obligation| {
            let state = if obligation.is_discharged {
                "discharged"
            } else {
                "owned"
            };
            serde_json::json!({
                "binding": obligation.binding,
                "actions": obligation.actions,
                "state": state,
                "source_span": source_span_from_range(
                    source_path,
                    source,
                    obligation.declaration_span.0,
                    obligation.declaration_span.1,
                )
                .as_json(),
            })
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "source": "compiler_core",
        "theorem_target": "no_unresolved_local_obligation_on_modeled_exit",
        "core_edges": core_edges,
        "modeled_exit_obligation_states": obligation_states,
    })
}

pub(super) fn summary_json(program: &ScenarioProgram) -> Value {
    serde_json::json!({
        "crate": "formal_core",
        "path": null,
        "schema_version": 1,
        "summary_hash": program.source_hash,
        "solver_metadata": {
            "engine": "kobo-compiler-core",
            "outcome": "evidence-only",
        },
        "core_fact_count": program.operations.len(),
        "template_fact_source": "scenario_program.typed_lifecycle_template",
        "functions": [program.target.clone()],
    })
}

pub(super) fn validate_formal_core_witness(witness: &Value) -> anyhow::Result<()> {
    if witness["schema_version"].as_u64() != Some(1) || witness["formal_core"].is_null() {
        return Ok(());
    }

    if witness["formal_core"]["source"].as_str() != Some("compiler_core_ir") {
        return formal_core_error("formal_core.source must be compiler_core_ir");
    }

    let Some(functions) = witness["formal_core"]["functions"].as_array() else {
        return formal_core_error("formal_core.functions must be an array");
    };
    for function in functions {
        validate_source_span(&function["source_span"], "Core function")?;
        let Some(blocks) = function["blocks"].as_array() else {
            return formal_core_error("formal_core function blocks must be an array");
        };
        for block in blocks {
            let Some(statements) = block["statements"].as_array() else {
                return formal_core_error("formal_core block statements must be an array");
            };
            for statement in statements {
                validate_source_span(&statement["source_span"], "Core statement")?;
            }
            let Some(terminators) = block["terminators"].as_array() else {
                return formal_core_error("formal_core block terminators must be an array");
            };
            for terminator in terminators {
                validate_source_span(&terminator["source_span"], "Core terminator")?;
            }
        }
    }
    Ok(())
}

fn analyze_strict_liveness(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
    run: &FullDepthRun,
) -> StrictLivenessAnalysis {
    let core = lower_core_program(program);
    let recursive_functions = recursive_functions(program);
    let mut analysis = StrictLivenessAnalysis::default();
    for function in &core.functions {
        analyze_function_liveness(
            source_path,
            source,
            function,
            &recursive_functions,
            &mut analysis,
        );
    }
    add_runtime_failure_error(source_path, source, run, &mut analysis);
    analysis
}

fn analyze_function_liveness(
    source_path: &str,
    source: &str,
    function: &CoreFunction,
    recursive_functions: &BTreeSet<String>,
    analysis: &mut StrictLivenessAnalysis,
) {
    let blocks = function
        .blocks
        .iter()
        .map(|block| (block.id.clone(), block))
        .collect::<BTreeMap<_, _>>();
    let Some(entry) = function.blocks.first() else {
        return;
    };
    let mut in_states: BTreeMap<String, ObligationEnv> = BTreeMap::new();
    let mut worklist = VecDeque::from([entry.id.clone()]);
    in_states.insert(entry.id.clone(), BTreeMap::new());

    while let Some(block_id) = worklist.pop_front() {
        let Some(block) = blocks.get(&block_id) else {
            continue;
        };
        let mut active = in_states.get(&block_id).cloned().unwrap_or_default();
        apply_block_liveness(
            source_path,
            source,
            block,
            recursive_functions,
            &mut active,
            analysis,
        );

        if block.successors.is_empty() {
            let span = source_span_from_kobo(source_path, source, function.source_span);
            record_active_exit_errors("normal_exit", &span, &active, analysis);
            continue;
        }

        for successor in &block.successors {
            if successor == &block.id {
                continue;
            }
            let changed = merge_obligation_env(
                source_path,
                source,
                in_states.entry(successor.clone()).or_default(),
                active.clone(),
                analysis,
            );
            if changed && !worklist.iter().any(|candidate| candidate == successor) {
                worklist.push_back(successor.clone());
            }
        }
    }
}

fn apply_block_liveness(
    source_path: &str,
    source: &str,
    block: &CoreBlock,
    recursive_functions: &BTreeSet<String>,
    active: &mut ObligationEnv,
    analysis: &mut StrictLivenessAnalysis,
) {
    for statement in &block.statements {
        apply_statement_liveness(
            source_path,
            source,
            statement,
            recursive_functions,
            active,
            analysis,
        );
    }
    for terminator in &block.terminators {
        apply_terminator_liveness(source_path, source, terminator, active, analysis);
    }
}

fn apply_statement_liveness(
    source_path: &str,
    source: &str,
    statement: &CoreStatement,
    recursive_functions: &BTreeSet<String>,
    active: &mut ObligationEnv,
    analysis: &mut StrictLivenessAnalysis,
) {
    match statement.kind {
        CoreStatementKind::ObligationCreate => {
            if let Some(binding) = statement.binding.as_ref() {
                active.insert(
                    binding.clone(),
                    ObligationState::Owned(ActiveObligation {
                        binding: binding.clone(),
                        source_span: source_span_from_kobo(
                            source_path,
                            source,
                            statement.source_span,
                        ),
                    }),
                );
            }
        }
        CoreStatementKind::ObligationDischarge => {
            let Some(binding) = statement.binding.as_ref() else {
                return;
            };
            let Some(ObligationState::Owned(obligation)) = active.get(binding).cloned() else {
                return;
            };
            let action = statement.action.as_deref().unwrap_or_default();
            let (resolution, reason) = resolution_from_action(action);
            let source_span = source_span_from_kobo(source_path, source, statement.source_span);
            active.insert(
                binding.clone(),
                ObligationState::Resolved(ResolvedObligation {
                    binding: obligation.binding.clone(),
                    resolution: resolution.clone(),
                    source_span: source_span.clone(),
                }),
            );
            analysis.resolved_paths.push(StrictResolvedPath {
                binding: obligation.binding,
                resolution,
                reason,
                source_span,
            });
        }
        CoreStatementKind::ObligationTransfer => {
            let Some(binding) = statement.binding.as_ref() else {
                return;
            };
            let Some(ObligationState::Owned(obligation)) = active.get(binding).cloned() else {
                return;
            };
            let callee = statement.action.as_deref().unwrap_or_default();
            let resolution = if recursive_functions.contains(callee) {
                "pending_recursive_transfer"
            } else if callee.starts_with("unproven:") {
                "pending_unproven_transfer"
            } else {
                "summary_proved_discharge"
            };
            let source_span = source_span_from_kobo(source_path, source, statement.source_span);
            if resolution == "summary_proved_discharge" {
                active.insert(
                    binding.clone(),
                    ObligationState::Resolved(ResolvedObligation {
                        binding: obligation.binding.clone(),
                        resolution: resolution.to_owned(),
                        source_span: source_span.clone(),
                    }),
                );
            }
            analysis.resolved_paths.push(StrictResolvedPath {
                binding: obligation.binding,
                resolution: resolution.to_owned(),
                reason: transfer_reason(callee),
                source_span,
            });
        }
        CoreStatementKind::ObligationEscape => {
            if let Some(binding) = statement.binding.as_ref() {
                let source_span = source_span_from_kobo(source_path, source, statement.source_span);
                active.insert(
                    binding.clone(),
                    ObligationState::Resolved(ResolvedObligation {
                        binding: binding.clone(),
                        resolution: "escaped".to_owned(),
                        source_span,
                    }),
                );
            }
        }
        CoreStatementKind::ObligationBranchUnresolved => {
            let Some(binding) = statement.binding.as_ref() else {
                return;
            };
            let obligation = active
                .get(binding)
                .and_then(ObligationState::owned)
                .cloned()
                .unwrap_or(ActiveObligation {
                    binding: binding.clone(),
                    source_span: source_span_from_kobo(source_path, source, statement.source_span),
                });
            let source_span = source_span_from_kobo(source_path, source, statement.source_span);
            if !has_error(analysis, "branch_exit", binding, source_span.start) {
                analysis.errors.push(StrictLivenessError {
                    exit_kind: "branch_exit".to_owned(),
                    binding: binding.clone(),
                    obligation_span: obligation.source_span,
                    source_span,
                    message: format!(
                        "strict liveness: unresolved obligation `{binding}` reaches one branch exit"
                    ),
                });
            }
        }
        CoreStatementKind::UnsupportedContainer => {
            let binding = statement
                .binding
                .as_deref()
                .filter(|binding| !binding.is_empty())
                .unwrap_or("_shared");
            let container = statement
                .action
                .as_deref()
                .unwrap_or("unsupported container");
            let source_span = source_span_from_kobo(source_path, source, statement.source_span);
            analysis.errors.push(StrictLivenessError {
                exit_kind: "unsupported_container".to_owned(),
                binding: binding.to_owned(),
                obligation_span: source_span.clone(),
                source_span,
                message: format!(
                    "strict liveness: {container} containing an obligation needs an obligation-aware wrapper or declaration"
                ),
            });
        }
        CoreStatementKind::ObligationMove | CoreStatementKind::Call => {}
    }
}

fn apply_terminator_liveness(
    source_path: &str,
    source: &str,
    terminator: &CoreTerminator,
    active: &ObligationEnv,
    analysis: &mut StrictLivenessAnalysis,
) {
    let source_span = source_span_from_kobo(source_path, source, terminator.source_span);
    match terminator.kind {
        CoreTerminatorKind::Return
        | CoreTerminatorKind::ErrorExit
        | CoreTerminatorKind::Panic
        | CoreTerminatorKind::Await
        | CoreTerminatorKind::OpaqueBoundary => {
            record_active_exit_errors(terminator.kind.as_str(), &source_span, active, analysis);
        }
        CoreTerminatorKind::Goto => {
            if terminator.edges.iter().any(|edge| {
                edge.strip_prefix("goto:")
                    .is_some_and(|target| target == "bb0")
            }) {
                record_active_exit_errors("recursive_scc", &source_span, active, analysis);
            }
        }
        CoreTerminatorKind::Branch => {}
    }
}

fn merge_obligation_env(
    source_path: &str,
    source: &str,
    current: &mut ObligationEnv,
    incoming: ObligationEnv,
    analysis: &mut StrictLivenessAnalysis,
) -> bool {
    let mut changed = false;
    for (binding, incoming_state) in incoming {
        match current.get_mut(&binding) {
            Some(current_state) => {
                if current_state == &incoming_state {
                    continue;
                }
                record_join_conflict_if_needed(
                    source_path,
                    source,
                    &binding,
                    current_state,
                    &incoming_state,
                    analysis,
                );
                let joined = join_obligation_state(current_state.clone(), incoming_state);
                if *current_state != joined {
                    *current_state = joined;
                    changed = true;
                }
            }
            None => {
                current.insert(binding, incoming_state);
                changed = true;
            }
        }
    }
    changed
}

fn join_obligation_state(current: ObligationState, incoming: ObligationState) -> ObligationState {
    match (&current, &incoming) {
        (ObligationState::Owned(_), _) => current,
        (_, ObligationState::Owned(_)) => incoming,
        (ObligationState::Resolved(current), ObligationState::Resolved(incoming))
            if current.resolution == incoming.resolution =>
        {
            ObligationState::Resolved(current.clone())
        }
        (ObligationState::Resolved(_), ObligationState::Resolved(incoming)) => {
            ObligationState::Resolved(incoming.clone())
        }
    }
}

fn record_join_conflict_if_needed(
    source_path: &str,
    source: &str,
    binding: &str,
    current: &ObligationState,
    incoming: &ObligationState,
    analysis: &mut StrictLivenessAnalysis,
) {
    let (owned, resolved) = match (current.owned(), incoming.resolved()) {
        (Some(owned), Some(resolved)) => (owned, resolved),
        _ => match (incoming.owned(), current.resolved()) {
            (Some(owned), Some(resolved)) => (owned, resolved),
            _ => return,
        },
    };
    if has_error(
        analysis,
        "semantic_join",
        binding,
        resolved.source_span.start,
    ) {
        return;
    }
    let source_span = source_span_from_range(
        source_path,
        source,
        resolved.source_span.start,
        resolved.source_span.end,
    );
    analysis.errors.push(StrictLivenessError {
        exit_kind: "semantic_join".to_owned(),
        binding: binding.to_owned(),
        source_span,
        obligation_span: owned.source_span.clone(),
        message: format!(
            "strict liveness: obligation `{binding}` is resolved on one Core path and still owned on another"
        ),
    });
}

fn recursive_functions(program: &ScenarioProgram) -> BTreeSet<String> {
    program
        .coverage
        .call_graph_sccs
        .iter()
        .filter(|scc| scc.is_recursive)
        .flat_map(|scc| scc.functions.iter().cloned())
        .collect()
}

fn record_active_exit_errors(
    exit_kind: &str,
    source_span: &CoreSourceSpan,
    active: &ObligationEnv,
    analysis: &mut StrictLivenessAnalysis,
) {
    for obligation in active.values().filter_map(ObligationState::owned) {
        if has_error(analysis, exit_kind, &obligation.binding, source_span.start) {
            continue;
        }
        analysis.errors.push(StrictLivenessError {
            exit_kind: exit_kind.to_owned(),
            binding: obligation.binding.clone(),
            source_span: source_span.clone(),
            obligation_span: obligation.source_span.clone(),
            message: format!(
                "strict liveness: unresolved obligation `{}` reaches {exit_kind}",
                obligation.binding
            ),
        });
    }
}

fn has_error(
    analysis: &StrictLivenessAnalysis,
    exit_kind: &str,
    binding: &str,
    start: usize,
) -> bool {
    analysis.errors.iter().any(|error| {
        error.exit_kind == exit_kind && error.binding == binding && error.source_span.start == start
    })
}

fn resolution_from_action(action: &str) -> (String, Option<String>) {
    if action == "return" {
        return ("returned".to_owned(), None);
    }
    if let Some(boundary) = action.strip_prefix("escape:") {
        return ("escaped".to_owned(), Some(boundary.to_owned()));
    }
    if let Some(reason) = action.strip_prefix("suppressed:") {
        return ("suppressed".to_owned(), Some(reason.to_owned()));
    }
    ("discharged".to_owned(), Some(action.to_owned()))
}

fn transfer_reason(callee: &str) -> Option<String> {
    if callee.is_empty() {
        return None;
    }
    Some(
        callee
            .strip_prefix("unproven:")
            .unwrap_or(callee)
            .to_owned(),
    )
}

fn add_runtime_failure_error(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
    analysis: &mut StrictLivenessAnalysis,
) {
    let Some(failure) = run.failure.as_ref() else {
        return;
    };
    if failure.code != KErrorCode::K0100 {
        return;
    }
    let binding = failure
        .events
        .first()
        .and_then(|event| event.label.clone())
        .unwrap_or_else(|| "unknown".to_owned());
    let exit_kind = if failure
        .events
        .iter()
        .any(|event| event.kind.contains("cancel"))
        || failure.message.contains("cancel")
    {
        "cancel"
    } else {
        "normal_exit"
    };
    if analysis
        .errors
        .iter()
        .any(|error| error.binding == binding && error.exit_kind == exit_kind)
    {
        return;
    }
    let source_span = source_span_from_range(
        source_path,
        source,
        failure.primary_start,
        failure.primary_end,
    );
    analysis.errors.push(StrictLivenessError {
        exit_kind: exit_kind.to_owned(),
        binding,
        obligation_span: source_span.clone(),
        source_span,
        message: failure.message.clone(),
    });
}

fn function_json(source_path: &str, source: &str, function: CoreFunction) -> Value {
    serde_json::json!({
        "name": function.name,
        "source_span": source_span_json(&source_span_from_kobo(source_path, source, function.source_span)),
        "blocks": function
            .blocks
            .into_iter()
            .map(|block| block_json(source_path, source, block))
            .collect::<Vec<_>>(),
    })
}

fn block_json(source_path: &str, source: &str, block: CoreBlock) -> Value {
    serde_json::json!({
        "id": block.id,
        "kir_cfg_block": block.kir_cfg_block,
        "successor_source": block.successor_source,
        "statements": block
            .statements
            .into_iter()
            .map(|statement| statement_json(source_path, source, statement))
            .collect::<Vec<_>>(),
        "terminators": block
            .terminators
            .into_iter()
            .map(|terminator| terminator_json(source_path, source, terminator))
            .collect::<Vec<_>>(),
        "successors": block.successors,
    })
}

fn statement_json(source_path: &str, source: &str, statement: CoreStatement) -> Value {
    serde_json::json!({
        "id": statement.id,
        "kind": statement.kind.as_str(),
        "binding": statement.binding,
        "action": statement.action,
        "boundary": statement.boundary,
        "source_span": source_span_json(&source_span_from_kobo(source_path, source, statement.source_span)),
    })
}

fn terminator_json(source_path: &str, source: &str, terminator: CoreTerminator) -> Value {
    serde_json::json!({
        "id": terminator.id,
        "kind": terminator.kind.as_str(),
        "boundary": terminator.boundary,
        "policy": terminator.policy,
        "edges": terminator.edges,
        "source_span": source_span_json(&source_span_from_kobo(source_path, source, terminator.source_span)),
    })
}

fn strict_error_json(error: StrictLivenessError) -> Value {
    serde_json::json!({
        "exit_kind": error.exit_kind,
        "binding": error.binding,
        "message": error.message,
        "source_span": source_span_json(&error.source_span),
        "obligation_span": source_span_json(&error.obligation_span),
    })
}

fn resolved_path_json(path: &StrictResolvedPath) -> Value {
    serde_json::json!({
        "binding": &path.binding,
        "resolution": &path.resolution,
        "reason": &path.reason,
        "source_span": source_span_json(&path.source_span),
    })
}

fn source_span_json(span: &CoreSourceSpan) -> Value {
    span.as_json()
}

fn source_span_from_kobo(source_path: &str, source: &str, span: KoboSpan) -> CoreSourceSpan {
    source_span_from_range(source_path, source, span.start as usize, span.end as usize)
}

fn source_span_from_range(
    source_path: &str,
    source: &str,
    start: usize,
    end: usize,
) -> CoreSourceSpan {
    let bounded_start = start.min(source.len());
    let bounded_end = end.max(bounded_start + 1).min(source.len());
    CoreSourceSpan {
        path: source_path.to_owned(),
        line: one_based_line_for_offset(source, bounded_start),
        start: bounded_start,
        end: bounded_end,
        mapped: bounded_end > bounded_start,
        snippet: line_snippet(source, bounded_start),
    }
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

impl CoreSourceSpan {
    fn as_json(&self) -> Value {
        serde_json::json!({
            "path": self.path,
            "line": self.line,
            "start": self.start,
            "end": self.end,
            "mapped": self.mapped,
            "snippet": self.snippet,
        })
    }
}

impl ObligationState {
    fn owned(&self) -> Option<&ActiveObligation> {
        match self {
            Self::Owned(obligation) => Some(obligation),
            Self::Resolved(_) => None,
        }
    }

    fn resolved(&self) -> Option<&ResolvedObligation> {
        match self {
            Self::Owned(_) => None,
            Self::Resolved(obligation) => Some(obligation),
        }
    }
}

fn validate_source_span(span: &Value, context: &str) -> anyhow::Result<()> {
    let mapped = span["mapped"].as_bool() == Some(true);
    let start = span["start"].as_u64();
    let end = span["end"].as_u64();
    let non_empty = start.zip(end).is_some_and(|(start, end)| end > start);
    let has_original = span["path"].as_str().is_some()
        && span["line"].as_u64().is_some()
        && span["snippet"]
            .as_str()
            .is_some_and(|snippet| !snippet.is_empty());
    if mapped && non_empty && has_original {
        return Ok(());
    }
    formal_core_error(&format!(
        "unmapped Core node in {context}: source_span must carry original path, line, snippet, and non-empty byte range"
    ))
}

fn formal_core_error<T>(message: &str) -> anyhow::Result<T> {
    eprintln!("error[K0115]: {message}");
    eprintln!("help: Core evidence must be source-mapped to the original .kobo file");
    anyhow::bail!("invalid formal Core evidence")
}
