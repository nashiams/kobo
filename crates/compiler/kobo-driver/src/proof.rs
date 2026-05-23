use std::collections::BTreeMap;
use std::path::Path;

use kobo_ir::{
    lower_core_program, CoreBlock, CoreFunction, CoreStatement, CoreStatementKind,
    CoreTerminatorKind, KoboSpan, ScenarioLifecycleTemplateSource, ScenarioOpKind, ScenarioProgram,
};
use kobo_proof::{
    certificate_material_hash, core_material_hash, stable_hash, AsyncModelEvidence,
    BoundaryAssumption, BoundaryPolicy, CancelEdgeEvidence, CoreCfgEdge, CoreCfgNode, CoreEvidence,
    CoverageLoss, FunctionSummary, FutureStateLocalEvidence, FutureStateObligationEvidence,
    HashEvidence, ObligationEvent, ObligationEventKind, ObligationState, OpaqueLedgerEntry,
    ProofCertificate, SelectPathEvidence, SourceEvidence, SourceSpan,
    SpawnedTaskObligationEvidence, SuspensionStateEvidence, TemplateVersionEvidence,
    TimeoutCancelEdgeEvidence,
};

pub use kobo_proof::{ArtifactKind, ReplayGrade};

pub struct ProofEmissionInput<'a> {
    pub source_path: &'a Path,
    pub source: &'a str,
    pub program: &'a ScenarioProgram,
    pub replay_grade: ReplayGrade,
    pub artifact_kind: ArtifactKind,
}

#[derive(Debug, thiserror::Error)]
pub enum ProofEmissionError {
    #[error("failed to serialize proof material: {0}")]
    Serialize(#[from] serde_json::Error),
}

pub fn emit_proof_certificate(
    input: ProofEmissionInput<'_>,
) -> Result<ProofCertificate, ProofEmissionError> {
    let source_path = input.source_path.display().to_string();
    let core_program = lower_core_program(input.program);
    let cfg_nodes = core_cfg_nodes(&source_path, input.source, &core_program.functions);
    let cfg_edges = core_cfg_edges(&source_path, input.source, &core_program.functions);
    let core_hash = core_material_hash(core_program.core_version, &cfg_nodes, &cfg_edges)?;
    let (template_hashes, template_versions) =
        template_evidence(&source_path, input.source, input.program)?;
    let (boundary_assumption_hashes, boundary_assumptions, opaque_edge_ledger) =
        boundary_evidence(&source_path, input.source, input.program)?;
    let (entry_env, exit_env, obligation_events) =
        obligation_evidence(&source_path, input.source, &core_program.functions);
    let async_model = async_model_evidence(
        &source_path,
        input.source,
        input.program,
        &core_program.functions,
        &exit_env,
    );
    let function_summaries = function_summaries(
        input.program,
        &entry_env,
        &exit_env,
        obligation_events.len(),
    );
    let coverage_loss = coverage_loss(input.program);

    let mut certificate = ProofCertificate {
        schema_version: 1,
        proof_target_version: "kobo-core-obligation-flow-1".to_owned(),
        semantic_schema: ".kproof".to_owned(),
        artifact_kind: input.artifact_kind,
        claim_scope: "modeled_core_obligation_flow_only".to_owned(),
        compiler_version: env!("CARGO_PKG_VERSION").to_owned(),
        source: SourceEvidence {
            path: source_path,
            hash: stable_hash(input.source),
        },
        core: CoreEvidence {
            hash: core_hash,
            version: core_program.core_version.to_owned(),
            cfg_nodes,
            cfg_edges,
            async_model,
        },
        replay_grade: input.replay_grade,
        template_hashes,
        template_versions,
        boundary_assumption_hashes,
        boundary_assumptions,
        adapter_confidence: Vec::new(),
        obligation_events,
        entry_env,
        exit_env,
        function_summaries,
        coverage_loss,
        opaque_edge_ledger,
        certificate_material_hash: String::new(),
    };
    certificate.certificate_material_hash = certificate_material_hash(&certificate)?;
    Ok(certificate)
}

fn core_cfg_nodes(source_path: &str, source: &str, functions: &[CoreFunction]) -> Vec<CoreCfgNode> {
    functions
        .iter()
        .flat_map(|function| {
            function.blocks.iter().map(move |block| CoreCfgNode {
                id: block.id.clone(),
                function: function.name.clone(),
                source_span: source_span_from_kobo(source_path, source, block_span(block)),
            })
        })
        .collect()
}

fn core_cfg_edges(source_path: &str, source: &str, functions: &[CoreFunction]) -> Vec<CoreCfgEdge> {
    functions
        .iter()
        .flat_map(|function| {
            function.blocks.iter().flat_map(move |block| {
                block.terminators.iter().flat_map(move |terminator| {
                    terminator
                        .edges
                        .iter()
                        .enumerate()
                        .map(move |(edge_index, edge)| CoreCfgEdge {
                            id: format!(
                                "{}:{}:{}:{edge_index}",
                                function.name, block.id, terminator.id
                            ),
                            function: function.name.clone(),
                            from: block.id.clone(),
                            to: edge
                                .strip_prefix("goto:")
                                .unwrap_or(edge.as_str())
                                .to_owned(),
                            kind: terminator.kind.as_str().to_owned(),
                            source_span: source_span_from_kobo(
                                source_path,
                                source,
                                terminator.source_span,
                            ),
                        })
                })
            })
        })
        .collect()
}

fn block_span(block: &CoreBlock) -> KoboSpan {
    block
        .statements
        .first()
        .map(|statement| statement.source_span)
        .or_else(|| {
            block
                .terminators
                .first()
                .map(|terminator| terminator.source_span)
        })
        .unwrap_or(KoboSpan {
            file_id: kobo_ir::FileId(0),
            start: 0,
            end: 0,
        })
}

fn template_evidence(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
) -> Result<(Vec<HashEvidence>, Vec<TemplateVersionEvidence>), serde_json::Error> {
    let mut hashes = Vec::new();
    let mut versions = Vec::new();
    for operation in &program.operations {
        let ScenarioOpKind::CreateObligation {
            template: Some(template),
            ..
        } = &operation.kind
        else {
            continue;
        };
        let template_source = match template.source {
            ScenarioLifecycleTemplateSource::Declaration => "declaration",
            ScenarioLifecycleTemplateSource::Inference => "inference",
        };
        let version = TemplateVersionEvidence {
            id: template.id.clone(),
            kind: template.kind.clone(),
            version: template.version.clone(),
            confidence: template.confidence.clone(),
            source: template_source.to_owned(),
            source_span: source_span_from_kobo(source_path, source, operation.span),
        };
        let hash = serde_json::to_string(&version).map(|material| stable_hash(&material))?;
        hashes.push(HashEvidence {
            id: template.id.clone(),
            hash,
        });
        versions.push(version);
    }
    Ok((hashes, versions))
}

fn boundary_evidence(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
) -> Result<
    (
        Vec<HashEvidence>,
        Vec<BoundaryAssumption>,
        Vec<OpaqueLedgerEntry>,
    ),
    serde_json::Error,
> {
    let mut hashes = Vec::new();
    let mut assumptions = Vec::new();
    let mut opaque_ledger = Vec::new();
    for (index, operation) in program.operations.iter().enumerate() {
        let ScenarioOpKind::ExternalBoundary {
            crate_name,
            policy,
            reason,
            ..
        } = &operation.kind
        else {
            continue;
        };
        let id = format!("boundary-{index}-{crate_name}");
        let assumption = BoundaryAssumption {
            id: id.clone(),
            boundary: crate_name.clone(),
            policy: boundary_policy(policy),
            reason: reason.clone(),
            source_span: source_span_from_kobo(source_path, source, operation.span),
        };
        let hash = serde_json::to_string(&assumption).map(|material| stable_hash(&material))?;
        hashes.push(HashEvidence {
            id: id.clone(),
            hash: hash.clone(),
        });
        if matches!(policy, kobo_ir::ScenarioBoundaryPolicy::Opaque) {
            opaque_ledger.push(OpaqueLedgerEntry {
                edge_id: id.clone(),
                boundary: crate_name.clone(),
                evidence_hash: hash,
            });
        }
        assumptions.push(assumption);
    }
    Ok((hashes, assumptions, opaque_ledger))
}

fn obligation_evidence(
    source_path: &str,
    source: &str,
    functions: &[CoreFunction],
) -> (
    Vec<ObligationState>,
    Vec<ObligationState>,
    Vec<ObligationEvent>,
) {
    let mut current_env = BTreeMap::<String, String>::new();
    let entry_env = env_states(&current_env);
    let mut events = Vec::new();
    for function in functions {
        for block in &function.blocks {
            for statement in &block.statements {
                let before = env_states(&current_env);
                apply_obligation_statement(statement, &mut current_env);
                let after = env_states(&current_env);
                if let Some(kind) = obligation_event_kind(&statement.kind) {
                    events.push(ObligationEvent {
                        id: statement.id.clone(),
                        kind,
                        binding: statement.binding.clone(),
                        action: statement.action.clone(),
                        source_span: source_span_from_kobo(
                            source_path,
                            source,
                            statement.source_span,
                        ),
                        state_before: before,
                        state_after: after,
                    });
                }
            }
        }
    }
    let exit_env = env_states(&current_env);
    (entry_env, exit_env, events)
}

fn async_model_evidence(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
    functions: &[CoreFunction],
    exit_env: &[ObligationState],
) -> AsyncModelEvidence {
    let live_locals = live_locals_across_first_await(source, &program.target);
    let select_result_hash = canonical_obligation_result_hash(exit_env);
    let mut model = AsyncModelEvidence::default();

    for function in functions {
        let mut current_env = BTreeMap::<String, String>::new();
        for block in &function.blocks {
            for statement in &block.statements {
                apply_obligation_statement(statement, &mut current_env);
            }
            for terminator in &block.terminators {
                match terminator.kind {
                    CoreTerminatorKind::Await => {
                        let suspension_id =
                            format!("{}:{}:{}", function.name, block.id, terminator.id);
                        let resume_edge = terminator
                            .edges
                            .iter()
                            .find(|edge| edge.as_str() == "await_resume")
                            .cloned()
                            .unwrap_or_else(|| "await_resume".to_owned());
                        let cancel_edge = terminator
                            .edges
                            .iter()
                            .find(|edge| edge.as_str() == "await_cancel")
                            .cloned()
                            .unwrap_or_else(|| "await_cancel".to_owned());
                        let source_span =
                            source_span_from_kobo(source_path, source, terminator.source_span);
                        model.suspension_states.push(SuspensionStateEvidence {
                            id: suspension_id.clone(),
                            function: function.name.clone(),
                            block: block.id.clone(),
                            terminator_kind: "await".to_owned(),
                            resume_edge,
                            cancel_edge: cancel_edge.clone(),
                            source_span: source_span.clone(),
                        });
                        model.cancel_edges.push(CancelEdgeEvidence {
                            id: format!("{suspension_id}:future-drop"),
                            function: function.name.clone(),
                            from: block.id.clone(),
                            to: cancel_edge.clone(),
                            reason: "future_drop".to_owned(),
                            source_span: source_span.clone(),
                        });
                        for local in &live_locals {
                            model.future_state_locals.push(FutureStateLocalEvidence {
                                binding: local.clone(),
                                suspension_state: suspension_id.clone(),
                                source_span: source_span_for_binding(source_path, source, local),
                            });
                        }
                        for state in env_states(&current_env)
                            .into_iter()
                            .filter(|state| state.state == "owned")
                        {
                            model
                                .future_state_obligations
                                .push(FutureStateObligationEvidence {
                                    binding: state.binding,
                                    state: state.state,
                                    suspension_state: suspension_id.clone(),
                                    source_span: source_span.clone(),
                                });
                        }
                        if source_span.snippet.contains("timeout")
                            || source.contains("tokio::time::timeout")
                        {
                            model.timeout_cancel_edges.push(TimeoutCancelEdgeEvidence {
                                id: format!("{suspension_id}:timeout"),
                                function: function.name.clone(),
                                suspension_state: suspension_id,
                                source: "tokio::time::timeout".to_owned(),
                                cancel_edge,
                                source_span,
                            });
                        }
                    }
                    CoreTerminatorKind::Branch => {
                        let source_span =
                            source_span_from_kobo(source_path, source, terminator.source_span);
                        for path_kind in ["winner", "loser_cancel"] {
                            model.select_paths.push(SelectPathEvidence {
                                id: format!(
                                    "{}:{}:{}:{path_kind}",
                                    function.name, block.id, terminator.id
                                ),
                                function: function.name.clone(),
                                branch_block: block.id.clone(),
                                path_kind: path_kind.to_owned(),
                                obligation_result_hash: select_result_hash.clone(),
                                source_span: source_span.clone(),
                            });
                        }
                    }
                    CoreTerminatorKind::Goto
                    | CoreTerminatorKind::Return
                    | CoreTerminatorKind::ErrorExit
                    | CoreTerminatorKind::Panic
                    | CoreTerminatorKind::OpaqueBoundary => {}
                }
            }
        }
    }

    for operation in &program.operations {
        let ScenarioOpKind::CreateObligation {
            binding,
            type_name,
            actions,
            template,
        } = &operation.kind
        else {
            continue;
        };
        let is_spawned_task = type_name == "SpawnedTask"
            || template
                .as_ref()
                .is_some_and(|template| template.kind == "spawned_task");
        if is_spawned_task {
            model
                .spawned_task_obligations
                .push(SpawnedTaskObligationEvidence {
                    binding: binding.clone(),
                    required_resolution: actions.clone(),
                    source_span: source_span_from_kobo(source_path, source, operation.span),
                });
        }
    }

    model
}

fn apply_obligation_statement(
    statement: &CoreStatement,
    current_env: &mut BTreeMap<String, String>,
) {
    match statement.kind {
        CoreStatementKind::ObligationCreate => {
            if let Some(binding) = statement.binding.as_ref() {
                current_env.insert(binding.clone(), "owned".to_owned());
            }
        }
        CoreStatementKind::ObligationDischarge => {
            if let Some(binding) = statement.binding.as_ref() {
                current_env.insert(binding.clone(), "resolved".to_owned());
            }
        }
        CoreStatementKind::ObligationTransfer => {
            if let Some(binding) = statement.binding.as_ref() {
                current_env.insert(binding.clone(), "transferred".to_owned());
            }
        }
        CoreStatementKind::ObligationMove => {
            if let Some(binding) = statement.binding.as_ref() {
                current_env.insert(binding.clone(), "moved".to_owned());
            }
        }
        CoreStatementKind::ObligationBranchUnresolved => {
            if let Some(binding) = statement.binding.as_ref() {
                current_env.insert(binding.clone(), "branch_unresolved".to_owned());
            }
        }
        CoreStatementKind::ObligationEscape => {
            if let Some(binding) = statement.binding.as_ref() {
                current_env.insert(binding.clone(), "escaped".to_owned());
            }
        }
        CoreStatementKind::UnsupportedContainer | CoreStatementKind::Call => {}
    }
}

fn obligation_event_kind(kind: &CoreStatementKind) -> Option<ObligationEventKind> {
    match kind {
        CoreStatementKind::ObligationCreate => Some(ObligationEventKind::Create),
        CoreStatementKind::ObligationDischarge => Some(ObligationEventKind::Discharge),
        CoreStatementKind::ObligationTransfer => Some(ObligationEventKind::Transfer),
        CoreStatementKind::ObligationMove => Some(ObligationEventKind::Move),
        CoreStatementKind::ObligationBranchUnresolved => {
            Some(ObligationEventKind::BranchUnresolved)
        }
        CoreStatementKind::ObligationEscape => Some(ObligationEventKind::Escape),
        CoreStatementKind::UnsupportedContainer => Some(ObligationEventKind::UnsupportedContainer),
        CoreStatementKind::Call => Some(ObligationEventKind::Call),
    }
}

fn env_states(env: &BTreeMap<String, String>) -> Vec<ObligationState> {
    env.iter()
        .map(|(binding, state)| ObligationState {
            binding: binding.clone(),
            state: state.clone(),
        })
        .collect()
}

fn function_summaries(
    program: &ScenarioProgram,
    entry_env: &[ObligationState],
    exit_env: &[ObligationState],
    event_count: usize,
) -> Vec<FunctionSummary> {
    vec![FunctionSummary {
        function: program.target.clone(),
        event_count,
        entry_env: entry_env.to_vec(),
        exit_env: exit_env.to_vec(),
    }]
}

fn coverage_loss(program: &ScenarioProgram) -> Vec<CoverageLoss> {
    let unsupported = program
        .coverage
        .unsupported_constructs
        .iter()
        .map(|label| CoverageLoss {
            kind: "unsupported_construct".to_owned(),
            label: label.clone(),
            reason: "not modeled in current Core proof certificate".to_owned(),
        });
    let opaque = program
        .coverage
        .opaque_boundaries
        .iter()
        .map(|label| CoverageLoss {
            kind: "opaque_boundary".to_owned(),
            label: label.clone(),
            reason: "opaque boundary requires ledger evidence before exact replay".to_owned(),
        });
    unsupported.chain(opaque).collect()
}

fn canonical_obligation_result_hash(exit_env: &[ObligationState]) -> String {
    let mut states = exit_env
        .iter()
        .map(|state| state.state.as_str())
        .collect::<Vec<_>>();
    states.sort_unstable();
    stable_hash(&format!("obligation-result:{states:?}"))
}

fn live_locals_across_first_await(source: &str, target: &str) -> Vec<String> {
    let Some((body_start, body_end)) = target_function_body_range(source, target) else {
        return Vec::new();
    };
    let body = &source[body_start..body_end];
    let Some(await_offset) = body.find(".await") else {
        return Vec::new();
    };
    let before_await = &body[..await_offset];
    let after_await = &body[await_offset..];
    let mut locals = extract_let_bindings(before_await)
        .into_iter()
        .filter(|binding| !binding.starts_with('_'))
        .filter(|binding| identifier_occurs(after_await, binding))
        .collect::<Vec<_>>();
    locals.sort();
    locals.dedup();
    locals
}

fn target_function_body_range(source: &str, target: &str) -> Option<(usize, usize)> {
    let needle = format!("fn {target}");
    let function_start = source.find(&needle)?;
    let body_start = source[function_start..].find('{')? + function_start + 1;
    let mut depth = 1usize;
    for (relative, ch) in source[body_start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some((body_start, body_start + relative));
                }
            }
            _ => {}
        }
    }
    None
}

fn extract_let_bindings(source: &str) -> Vec<String> {
    source
        .split(';')
        .filter_map(|statement| {
            let trimmed = statement.trim_start();
            let rest = trimmed.strip_prefix("let ")?;
            let rest = rest.strip_prefix("mut ").unwrap_or(rest).trim_start();
            let binding = rest
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                .collect::<String>();
            (!binding.is_empty()).then_some(binding)
        })
        .collect()
}

fn identifier_occurs(source: &str, identifier: &str) -> bool {
    source
        .match_indices(identifier)
        .any(|(index, _)| identifier_boundary(source, index, identifier.len()))
}

fn identifier_boundary(source: &str, start: usize, len: usize) -> bool {
    let before = source[..start].chars().next_back();
    let after = source[start + len..].chars().next();
    !before.is_some_and(is_identifier_char) && !after.is_some_and(is_identifier_char)
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn boundary_policy(policy: &kobo_ir::ScenarioBoundaryPolicy) -> BoundaryPolicy {
    match policy {
        kobo_ir::ScenarioBoundaryPolicy::Typed => BoundaryPolicy::Typed,
        kobo_ir::ScenarioBoundaryPolicy::Model => BoundaryPolicy::Model,
        kobo_ir::ScenarioBoundaryPolicy::Record => BoundaryPolicy::Record,
        kobo_ir::ScenarioBoundaryPolicy::Activity => BoundaryPolicy::Activity,
        kobo_ir::ScenarioBoundaryPolicy::Stub => BoundaryPolicy::Stub,
        kobo_ir::ScenarioBoundaryPolicy::Outside => BoundaryPolicy::Outside,
        kobo_ir::ScenarioBoundaryPolicy::Opaque => BoundaryPolicy::Opaque,
        kobo_ir::ScenarioBoundaryPolicy::Debt => BoundaryPolicy::Debt,
        kobo_ir::ScenarioBoundaryPolicy::Unselected => BoundaryPolicy::Unselected,
    }
}

fn source_span_from_kobo(source_path: &str, source: &str, span: KoboSpan) -> SourceSpan {
    source_span_from_range(source_path, source, span.start as usize, span.end as usize)
}

fn source_span_for_binding(source_path: &str, source: &str, binding: &str) -> SourceSpan {
    let let_binding = format!("let {binding}");
    let let_mut_binding = format!("let mut {binding}");
    let start = source
        .find(&let_binding)
        .or_else(|| source.find(&let_mut_binding))
        .unwrap_or(0);
    source_span_from_range(
        source_path,
        source,
        start,
        start.saturating_add(binding.len()),
    )
}

fn source_span_from_range(source_path: &str, source: &str, start: usize, end: usize) -> SourceSpan {
    let bounded_start = start.min(source.len());
    let bounded_end = end.max(bounded_start + 1).min(source.len());
    SourceSpan {
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
