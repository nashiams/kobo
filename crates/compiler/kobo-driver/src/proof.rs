use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use crate::config::EcosystemAdapterPolicy;
use kobo_codegen::{KoboSourceMap, LoweringTraceEvent};
use kobo_ir::{
    lower_core_program, CoreBlock, CoreFunction, CoreStatement, CoreStatementKind,
    CoreTerminatorKind, KoboSpan, ScenarioLifecycleTemplateSource, ScenarioOpKind, ScenarioProgram,
};
use kobo_proof::{
    certificate_material_hash, core_material_hash, stable_hash, template_schema_hash,
    trace_material_hash, AdapterConfidence, AdapterEvidence, AsyncModelEvidence, BoundDeclaration,
    BoundDimension, BoundSource, BoundaryAssumption, BoundaryPolicy, BoundedCompleteness,
    BoundedProofEvidence, CancelEdgeEvidence, CandidateAdmissionEvidence, CandidateAdmissionFact,
    CoreCfgEdge, CoreCfgNode, CoreEvidence, CoreLoopBackEdgeFact, CoreTraceEvent, CoverageLoss,
    FunctionSummary, FutureStateLocalEvidence, FutureStateObligationEvidence, GeneratedTraceEvent,
    HashEvidence, InvariantConfidence, InvariantPreservation, InvariantTemplateEvidence,
    InvariantTemplateSource, InvariantTier, LoopInvariantEvidence, ObligationEvent,
    ObligationEventKind, ObligationState, ObligationStatus, OpaqueLedgerEntry, ProofCertificate,
    PrunedHistoryEvidence, SelectPathEvidence, SourceEvidence, SourceMapAnchorEvidence,
    SourceMapAnchorStatus, SourceSpan, SpawnedTaskObligationEvidence, SuspensionStateEvidence,
    TemplateSchemaEvidence, TimeoutCancelEdgeEvidence, TraceEventKind,
    TranslationValidationEvidence, TranslationValidationStatus, PROOF_CERTIFICATE_SCHEMA_VERSION,
    PROOF_CLAIM_SCOPE, PROOF_SEMANTIC_SCHEMA, PROOF_TARGET_VERSION,
};

pub use kobo_proof::{ArtifactKind, ReplayGrade};

pub struct ProofEmissionInput<'a> {
    pub source_path: &'a Path,
    pub source: &'a str,
    pub program: &'a ScenarioProgram,
    pub adapter_policies: &'a [EcosystemAdapterPolicy],
    pub replay_grade: ReplayGrade,
    pub artifact_kind: ArtifactKind,
    pub source_map: Option<&'a KoboSourceMap>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProofEmissionError {
    #[error("failed to serialize proof material: {0}")]
    Serialize(#[from] serde_json::Error),
}

#[derive(Clone, Debug)]
struct UserLoopInvariantDirective {
    expression: String,
    obligation_kind: Option<String>,
    source_span: SourceSpan,
}

pub fn emit_proof_certificate(
    input: ProofEmissionInput<'_>,
) -> Result<ProofCertificate, ProofEmissionError> {
    let source_path = input.source_path.display().to_string();
    let core_program = lower_core_program(input.program);
    let cfg_nodes = core_cfg_nodes(&source_path, input.source, &core_program.functions);
    let cfg_edges = core_cfg_edges(&source_path, input.source, &core_program.functions);
    let loop_facts = core_loop_facts(&cfg_edges);
    let (template_hashes, template_schemas) =
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
    );
    let core_hash = core_material_hash(
        core_program.core_version,
        &cfg_nodes,
        &cfg_edges,
        &loop_facts,
        &async_model,
    )?;
    let mut adapter_confidence = adapter_evidence(
        input.program,
        input.adapter_policies,
        input.replay_grade.clone(),
    );
    let replay_grade = adapter_adjusted_replay_grade(input.replay_grade, &adapter_confidence);
    for adapter in &mut adapter_confidence {
        adapter.replay_grade = replay_grade.clone();
    }
    let candidate_admission = candidate_admission_evidence(
        input.source_path,
        input.source,
        replay_grade.clone(),
        &adapter_confidence,
    );
    let function_summaries = function_summaries(
        input.program,
        &entry_env,
        &exit_env,
        obligation_events.len(),
    );
    let coverage_loss = coverage_loss(input.program);
    let user_loop_invariant =
        user_loop_invariant_directive(&source_path, input.source, input.program);
    let loop_invariants = loop_invariant_evidence(
        input.program,
        &core_program.functions,
        &loop_facts,
        &obligation_events,
        &template_hashes,
        user_loop_invariant.as_ref(),
    );
    let bounded_evidence = bounded_evidence(&source_path, input.source, input.program, &loop_facts);
    let core_obligation_trace = core_trace_evidence(
        input.program,
        &obligation_events,
        &source_path,
        input.source,
    );
    let generated_rust_trace = generated_trace_evidence(
        &source_path,
        input.source,
        input.program,
        &core_obligation_trace,
        input.source_map,
    );
    let translation_validation = translation_validation_evidence(
        &core_obligation_trace,
        &generated_rust_trace,
        input.source_map,
    );
    let trace_hashes = trace_hashes(&core_obligation_trace, &generated_rust_trace)?;

    let mut certificate = ProofCertificate {
        schema_version: PROOF_CERTIFICATE_SCHEMA_VERSION,
        proof_target_version: PROOF_TARGET_VERSION.to_owned(),
        semantic_schema: PROOF_SEMANTIC_SCHEMA.to_owned(),
        artifact_kind: input.artifact_kind,
        claim_scope: PROOF_CLAIM_SCOPE.to_owned(),
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
            loop_facts,
            async_model,
        },
        replay_grade,
        template_hashes,
        template_schemas,
        boundary_assumption_hashes,
        boundary_assumptions,
        adapter_confidence,
        obligation_events,
        entry_env,
        exit_env,
        function_summaries,
        coverage_loss,
        loop_invariants,
        bounded_evidence,
        core_obligation_trace,
        generated_rust_trace,
        trace_hashes,
        translation_validation,
        opaque_edge_ledger,
        candidate_admission,
        certificate_material_hash: String::new(),
    };
    certificate.certificate_material_hash = certificate_material_hash(&certificate)?;
    Ok(certificate)
}

fn core_loop_facts(edges: &[CoreCfgEdge]) -> Vec<CoreLoopBackEdgeFact> {
    edges
        .iter()
        .filter(|edge| is_back_edge(edge))
        .map(|edge| CoreLoopBackEdgeFact {
            id: format!("loop-{}-{}-{}", edge.function, edge.from, edge.to),
            function: edge.function.clone(),
            entry_block: edge.to.clone(),
            back_edge_source: edge.from.clone(),
            back_edge_target: edge.to.clone(),
            source_span: edge.source_span.clone(),
        })
        .collect()
}

fn is_back_edge(edge: &CoreCfgEdge) -> bool {
    let Some(source_index) = block_index(&edge.from) else {
        return false;
    };
    let Some(target_index) = block_index(&edge.to) else {
        return false;
    };
    target_index <= source_index
}

fn block_index(block: &str) -> Option<usize> {
    block.strip_prefix("bb")?.parse().ok()
}

fn loop_invariant_evidence(
    program: &ScenarioProgram,
    functions: &[CoreFunction],
    loop_facts: &[CoreLoopBackEdgeFact],
    obligation_events: &[ObligationEvent],
    template_hashes: &[HashEvidence],
    user_loop_invariant: Option<&UserLoopInvariantDirective>,
) -> Vec<LoopInvariantEvidence> {
    if loop_facts.is_empty() {
        return Vec::new();
    }
    let template_by_binding = template_by_binding(program);
    let type_by_binding = type_by_binding(program);
    let replay_by_function = functions
        .iter()
        .map(|function| (function.name.as_str(), function_obligation_replay(function)))
        .collect::<BTreeMap<_, _>>();
    loop_facts
        .iter()
        .filter_map(|fact| {
            let scoped_bindings = created_bindings_for_loop(fact, obligation_events);
            let relevant_bindings = scoped_bindings
                .iter()
                .filter(|binding| {
                    user_loop_invariant
                        .and_then(|directive| directive.obligation_kind.as_deref())
                        .map(|kind| {
                            type_by_binding
                                .get(binding.as_str())
                                .is_some_and(|type_name| type_name == kind)
                        })
                        .unwrap_or_else(|| template_by_binding.contains_key(binding.as_str()))
                })
                .cloned()
                .collect::<Vec<_>>();
            let first_binding = relevant_bindings.first();
            if user_loop_invariant.is_none() && first_binding.is_none() {
                return None;
            }
            let template =
                first_binding.and_then(|binding| template_by_binding.get(binding.as_str()));
            let obligation_kind = type_by_binding
                .get(first_binding.map(String::as_str).unwrap_or_default())
                .cloned()
                .or_else(|| {
                    user_loop_invariant.and_then(|directive| directive.obligation_kind.clone())
                })
                .unwrap_or_else(|| "obligation".to_owned());
            let replay = replay_by_function.get(fact.function.as_str())?;
            let back_edge_states = replay
                .block_exit_envs
                .get(&fact.back_edge_source)
                .map(env_states)
                .unwrap_or_default();
            let back_edge_leak = back_edge_states.iter().any(|state| {
                relevant_bindings.contains(&state.binding)
                    && matches!(
                        state.state,
                        ObligationStatus::Owned
                            | ObligationStatus::Moved
                            | ObligationStatus::BranchUnresolved
                    )
            });
            let malformed_user_invariant =
                user_loop_invariant.is_some_and(|directive| directive.obligation_kind.is_none());
            let unknown_user_kind = malformed_user_invariant
                || (user_loop_invariant.is_some() && relevant_bindings.is_empty());
            let preservation = if back_edge_leak || unknown_user_kind {
                InvariantPreservation::Failed
            } else {
                InvariantPreservation::Preserved
            };
            Some(LoopInvariantEvidence {
                id: fact.id.clone(),
                function: fact.function.clone(),
                entry_block: fact.entry_block.clone(),
                back_edge_source: fact.back_edge_source.clone(),
                back_edge_target: fact.back_edge_target.clone(),
                tier: user_loop_invariant
                    .map(|_| InvariantTier::User)
                    .unwrap_or(InvariantTier::Inferred),
                expression: user_loop_invariant
                    .map(|directive| directive.expression.clone())
                    .unwrap_or_else(|| format!("no_pending({obligation_kind})")),
                source_span: user_loop_invariant
                    .map(|directive| directive.source_span.clone())
                    .unwrap_or_else(|| fact.source_span.clone()),
                obligations_created: relevant_bindings,
                back_edge_states,
                preservation,
                template: template.map(|template| InvariantTemplateEvidence {
                    id: template.id.clone(),
                    version: lifecycle_template_version(template),
                    schema_hash: template_hash(template, template_hashes),
                    source: invariant_template_source(&template.source),
                    confidence: invariant_confidence(&template.confidence),
                    obligation_kind,
                    lifecycle_owner: lifecycle_owner(&template.id),
                }),
                downgrade_reason: user_invariant_downgrade_reason(
                    user_loop_invariant,
                    malformed_user_invariant,
                    unknown_user_kind,
                    back_edge_leak,
                ),
            })
        })
        .collect()
}

fn created_bindings_for_loop(
    fact: &CoreLoopBackEdgeFact,
    obligation_events: &[ObligationEvent],
) -> Vec<String> {
    let Some(entry_index) = block_index(&fact.entry_block) else {
        return Vec::new();
    };
    let Some(back_edge_index) = block_index(&fact.back_edge_source) else {
        return Vec::new();
    };
    obligation_events
        .iter()
        .filter(|event| event.kind == ObligationEventKind::Create)
        .filter(|event| {
            event
                .id
                .strip_prefix("stmt-")
                .and_then(|index| index.parse::<usize>().ok())
                .is_some_and(|index| entry_index <= index && index <= back_edge_index)
        })
        .filter_map(|event| event.binding.clone())
        .collect()
}

fn user_invariant_downgrade_reason(
    user_loop_invariant: Option<&UserLoopInvariantDirective>,
    malformed_user_invariant: bool,
    unknown_user_kind: bool,
    back_edge_leak: bool,
) -> Option<String> {
    if user_loop_invariant.is_some() && malformed_user_invariant {
        return Some("malformed user invariant expression".to_owned());
    }
    if user_loop_invariant.is_some() && unknown_user_kind {
        return Some("unknown obligation kind in user invariant".to_owned());
    }
    back_edge_leak.then(|| "unresolved obligation reaches loop back-edge".to_owned())
}

fn user_loop_invariant_directive(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
) -> Option<UserLoopInvariantDirective> {
    let file = syn::parse_file(source).ok()?;
    let function = file.items.iter().find_map(|item| match item {
        syn::Item::Fn(function) if function.sig.ident == program.target => Some(function),
        _ => None,
    })?;
    let attr = function
        .attrs
        .iter()
        .find(|attr| syn_path_ends_with(attr.path(), &["kobo", "invariant"]))?;
    let fields = attr_name_value_fields(attr)?;
    let expression = fields.get("expression")?.clone();
    let obligation_kind = no_pending_obligation_kind(&expression);
    let source_span = source_span_for_user_invariant(source_path, source, &expression);
    Some(UserLoopInvariantDirective {
        expression,
        obligation_kind,
        source_span,
    })
}

fn no_pending_obligation_kind(expression: &str) -> Option<String> {
    let obligation_kind = expression
        .strip_prefix("no_pending(")?
        .strip_suffix(')')?
        .trim();
    (!obligation_kind.is_empty()).then(|| obligation_kind.to_owned())
}

fn source_span_for_user_invariant(source_path: &str, source: &str, expression: &str) -> SourceSpan {
    let start = source.find(expression).unwrap_or(0);
    source_span_from_range(
        source_path,
        source,
        start,
        start.saturating_add(expression.len()),
    )
}

fn bounded_evidence(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
    loop_facts: &[CoreLoopBackEdgeFact],
) -> Vec<BoundedProofEvidence> {
    let Ok(file) = syn::parse_file(source) else {
        return Vec::new();
    };
    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Fn(function) if function.sig.ident == program.target => {
                bounded_attr(function).map(|fields| {
                    bounded_evidence_from_fields(source_path, source, program, loop_facts, &fields)
                })
            }
            _ => None,
        })
        .collect()
}

fn bounded_attr(function: &syn::ItemFn) -> Option<BTreeMap<String, String>> {
    function
        .attrs
        .iter()
        .find(|attr| syn_path_ends_with(attr.path(), &["kobo", "bounded"]))
        .and_then(attr_name_value_fields)
}

fn attr_name_value_fields(attr: &syn::Attribute) -> Option<BTreeMap<String, String>> {
    let syn::Meta::List(list) = &attr.meta else {
        return None;
    };
    let entries = list
        .parse_args_with(
            syn::punctuated::Punctuated::<syn::MetaNameValue, syn::Token![,]>::parse_terminated,
        )
        .ok()?;
    let mut fields = BTreeMap::new();
    for entry in entries {
        let key = entry
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())?;
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(value),
            ..
        }) = entry.value
        else {
            continue;
        };
        fields.insert(key, value.value());
    }
    Some(fields)
}

fn bounded_evidence_from_fields(
    _source_path: &str,
    _source: &str,
    program: &ScenarioProgram,
    loop_facts: &[CoreLoopBackEdgeFact],
    fields: &BTreeMap<String, String>,
) -> BoundedProofEvidence {
    let observed_history_count = numeric_field(fields, "histories").unwrap_or_default();
    let enumerated_history_count = numeric_field(fields, "unique_histories")
        .map(|unique| unique.min(observed_history_count))
        .unwrap_or(observed_history_count);
    let scheduler_dimensions = dimension_values(fields, "scheduler");
    let fault_dimensions = dimension_values(fields, "fault");
    let cancellation_points = dimension_values(fields, "cancellation");
    let expected_complete_history_count = numeric_field(fields, "expected").or_else(|| {
        derived_complete_history_count(
            &scheduler_dimensions,
            &fault_dimensions,
            &cancellation_points,
        )
    });
    let declared_completeness =
        bounded_completeness(fields.get("completeness").map(String::as_str));
    let completeness = effective_bounded_completeness(
        declared_completeness,
        enumerated_history_count,
        expected_complete_history_count,
        &scheduler_dimensions,
        &fault_dimensions,
        &cancellation_points,
    );
    let wording = bounded_wording(
        &completeness,
        enumerated_history_count,
        expected_complete_history_count,
    );
    let bounds = bound_declarations(
        expected_complete_history_count.unwrap_or(enumerated_history_count),
        &scheduler_dimensions,
        &fault_dimensions,
        &cancellation_points,
    );
    let pruned_histories = (observed_history_count > enumerated_history_count)
        .then(|| PrunedHistoryEvidence {
            id: format!("pruned-{}", program.target),
            reason: format!(
                "{} duplicate histories collapsed before completeness evaluation",
                observed_history_count - enumerated_history_count
            ),
        })
        .into_iter()
        .collect();
    BoundedProofEvidence {
        id: format!("bounded-{}", program.target),
        function: program.target.clone(),
        loop_ids: loop_facts
            .iter()
            .filter(|fact| fact.function == program.target)
            .map(|fact| fact.id.clone())
            .collect(),
        normalized_bound_hash: stable_hash(&format!(
            "{}:{enumerated_history_count}:{expected_complete_history_count:?}:{scheduler_dimensions:?}:{fault_dimensions:?}:{cancellation_points:?}",
            program.target,
        )),
        bounds,
        enumerated_history_count,
        expected_complete_history_count,
        scheduler_dimensions,
        fault_dimensions,
        cancellation_points,
        pruned_histories,
        completeness,
        wording,
    }
}

fn effective_bounded_completeness(
    declared: BoundedCompleteness,
    enumerated_history_count: u64,
    expected_complete_history_count: Option<u64>,
    scheduler_dimensions: &[String],
    fault_dimensions: &[String],
    cancellation_points: &[String],
) -> BoundedCompleteness {
    if declared != BoundedCompleteness::Complete {
        return declared;
    }
    if expected_complete_history_count != Some(enumerated_history_count)
        || scheduler_dimensions.is_empty()
        || fault_dimensions.is_empty()
        || cancellation_points.is_empty()
    {
        return BoundedCompleteness::Incomplete;
    }
    BoundedCompleteness::Complete
}

fn bound_declarations(
    history_bound: u64,
    scheduler_dimensions: &[String],
    fault_dimensions: &[String],
    cancellation_points: &[String],
) -> Vec<BoundDeclaration> {
    let mut bounds = vec![BoundDeclaration {
        dimension: BoundDimension::SchedulerHistories,
        value: history_bound,
        source: BoundSource::Ward,
        proof_relevant: true,
    }];
    if !scheduler_dimensions.is_empty() {
        bounds.push(BoundDeclaration {
            dimension: BoundDimension::LoopIterations,
            value: 1,
            source: BoundSource::Ward,
            proof_relevant: true,
        });
    }
    if !fault_dimensions.is_empty() {
        bounds.push(BoundDeclaration {
            dimension: BoundDimension::FaultInjectionChoices,
            value: fault_dimensions.len() as u64,
            source: BoundSource::Ward,
            proof_relevant: true,
        });
    }
    if !cancellation_points.is_empty() {
        bounds.push(BoundDeclaration {
            dimension: BoundDimension::CancellationPoints,
            value: cancellation_points.len() as u64,
            source: BoundSource::Ward,
            proof_relevant: true,
        });
    }
    bounds
}

fn core_trace_evidence(
    program: &ScenarioProgram,
    obligation_events: &[ObligationEvent],
    source_path: &str,
    source: &str,
) -> Vec<CoreTraceEvent> {
    let template_by_binding = template_by_binding(program);
    let events_by_statement = obligation_events
        .iter()
        .filter_map(|event| statement_index_from_event(&event.id).map(|index| (index, event)))
        .collect::<BTreeMap<_, _>>();
    let mut order = 0_u64;
    program
        .operations
        .iter()
        .enumerate()
        .filter_map(|(operation_index, operation)| {
            let event = events_by_statement.get(&operation_index).copied();
            let kind = event
                .and_then(|event| trace_event_kind(&event.kind))
                .or_else(|| trace_event_kind_from_operation(&operation.kind))?;
            let template = event
                .and_then(|event| event.binding.as_deref())
                .and_then(|binding| template_by_binding.get(binding));
            let binding = event.and_then(|event| event.binding.clone());
            let source_span = event
                .map(|event| event.source_span.clone())
                .unwrap_or_else(|| source_span_from_kobo(source_path, source, operation.span));
            let id = event
                .map(|event| format!("core-{}", event.id))
                .unwrap_or_else(|| format!("core-term-{operation_index}"));
            let trace_order = order;
            order = order.saturating_add(1);
            Some(CoreTraceEvent {
                id,
                kind,
                binding,
                order: trace_order,
                source_span,
                template_id: template.map(|template| template.id.clone()),
                template_version: template.map(|template| lifecycle_template_version(template)),
            })
        })
        .collect()
}

fn statement_index_from_event(event_id: &str) -> Option<usize> {
    event_id.strip_prefix("stmt-")?.parse().ok()
}

fn trace_event_kind_from_operation(kind: &ScenarioOpKind) -> Option<TraceEventKind> {
    match kind {
        ScenarioOpKind::CoreTerminator { kind, .. } => Some(trace_event_kind_from_terminator(kind)),
        _ => None,
    }
}

fn trace_event_kind_from_terminator(kind: &kobo_ir::ScenarioCoreTerminatorKind) -> TraceEventKind {
    match kind {
        kobo_ir::ScenarioCoreTerminatorKind::Return => TraceEventKind::Return,
        kobo_ir::ScenarioCoreTerminatorKind::ErrorExit => TraceEventKind::ErrorExit,
        kobo_ir::ScenarioCoreTerminatorKind::Panic => TraceEventKind::Panic,
        kobo_ir::ScenarioCoreTerminatorKind::Await => TraceEventKind::Cancel,
        kobo_ir::ScenarioCoreTerminatorKind::OpaqueBoundary => TraceEventKind::OpaqueBoundary,
    }
}

fn generated_trace_evidence(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
    core_trace: &[CoreTraceEvent],
    source_map: Option<&KoboSourceMap>,
) -> Vec<GeneratedTraceEvent> {
    let Some(source_map) = source_map else {
        return Vec::new();
    };
    let mut used_lowering_events = BTreeSet::new();
    core_trace
        .iter()
        .filter_map(|event| {
            let (index, lowering_event) =
                lowering_trace_event_for_core(source_map, program, event, &used_lowering_events)?;
            used_lowering_events.insert(index);
            Some(GeneratedTraceEvent {
                id: format!("generated-{}", event.id),
                core_event_id: event.id.clone(),
                kind: event.kind.clone(),
                binding: event.binding.clone(),
                order: event.order,
                source_map_anchor: SourceMapAnchorEvidence {
                    id: lowering_event.source_map_entry_id.clone(),
                    status: SourceMapAnchorStatus::Mapped,
                    generated_span: generated_source_span(source_map, lowering_event),
                    kobo_span: source_span_from_kobo_span(
                        source_path,
                        source,
                        &lowering_event.kobo_span,
                    ),
                },
                lowering_phase: lowering_event.lowering_phase.clone(),
                template_id: lowering_event
                    .template_id
                    .clone()
                    .or(event.template_id.clone()),
                template_version: lowering_event
                    .template_version
                    .clone()
                    .or(event.template_version.clone()),
            })
        })
        .collect()
}

fn translation_validation_evidence(
    core_trace: &[CoreTraceEvent],
    generated_trace: &[GeneratedTraceEvent],
    source_map: Option<&KoboSourceMap>,
) -> TranslationValidationEvidence {
    let status = if core_trace.is_empty() {
        TranslationValidationStatus::CoreOnly
    } else if source_map.is_none() {
        TranslationValidationStatus::CoreOnly
    } else if core_trace.len() == generated_trace.len() {
        TranslationValidationStatus::Validated
    } else {
        TranslationValidationStatus::Failed
    };
    TranslationValidationEvidence {
        status,
        mismatches: Vec::new(),
    }
}

fn trace_hashes(
    core_trace: &[CoreTraceEvent],
    generated_trace: &[GeneratedTraceEvent],
) -> Result<Vec<HashEvidence>, serde_json::Error> {
    Ok(vec![
        HashEvidence {
            id: "core_obligation_trace".to_owned(),
            hash: trace_material_hash(&core_trace)?,
        },
        HashEvidence {
            id: "generated_rust_trace".to_owned(),
            hash: trace_material_hash(&generated_trace)?,
        },
    ])
}

fn template_by_binding(
    program: &ScenarioProgram,
) -> BTreeMap<&str, &kobo_ir::ScenarioLifecycleTemplate> {
    program
        .operations
        .iter()
        .filter_map(|operation| match &operation.kind {
            ScenarioOpKind::CreateObligation {
                binding,
                template: Some(template),
                ..
            } => Some((binding.as_str(), template)),
            _ => None,
        })
        .collect()
}

fn type_by_binding(program: &ScenarioProgram) -> BTreeMap<&str, String> {
    program
        .operations
        .iter()
        .filter_map(|operation| match &operation.kind {
            ScenarioOpKind::CreateObligation {
                binding, type_name, ..
            } => Some((binding.as_str(), type_name.clone())),
            _ => None,
        })
        .collect()
}

fn lifecycle_template_version(template: &kobo_ir::ScenarioLifecycleTemplate) -> String {
    if matches!(template.source, ScenarioLifecycleTemplateSource::Inference) {
        return "0.1".to_owned();
    }
    template.schema_version.to_string()
}

fn template_hash(
    template: &kobo_ir::ScenarioLifecycleTemplate,
    template_hashes: &[HashEvidence],
) -> String {
    template_hashes
        .iter()
        .find(|hash| hash.id == template.id)
        .map(|hash| hash.hash.clone())
        .unwrap_or_else(|| stable_hash(&template.id))
}

fn invariant_template_source(source: &ScenarioLifecycleTemplateSource) -> InvariantTemplateSource {
    match source {
        ScenarioLifecycleTemplateSource::Declaration => InvariantTemplateSource::Declaration,
        ScenarioLifecycleTemplateSource::Inference => InvariantTemplateSource::BuiltIn,
    }
}

fn invariant_confidence(confidence: &str) -> InvariantConfidence {
    match confidence {
        "exact_template" | "exact" => InvariantConfidence::Exact,
        "sampled" => InvariantConfidence::Sampled,
        "metadata-only" | "metadata_only" => InvariantConfidence::MetadataOnly,
        _ => InvariantConfidence::Modeled,
    }
}

fn lifecycle_owner(template_id: &str) -> String {
    match template_id {
        "queue_delivery" => "queue",
        "transaction" => "transaction_manager",
        "stream_item" => "stream",
        "retry_attempt" => "retry_policy",
        "handler_reply" => "service_request",
        "spawned_task" => "task_runtime",
        "lock_permit" => "lock",
        "file_socket" => "io_resource",
        other => other,
    }
    .to_owned()
}

fn numeric_field(fields: &BTreeMap<String, String>, key: &str) -> Option<u64> {
    fields.get(key)?.parse().ok()
}

fn dimension_values(fields: &BTreeMap<String, String>, key: &str) -> Vec<String> {
    fields
        .get(key)
        .into_iter()
        .flat_map(|value| value.split(['|', ',']))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn derived_complete_history_count(
    scheduler_dimensions: &[String],
    fault_dimensions: &[String],
    cancellation_points: &[String],
) -> Option<u64> {
    if scheduler_dimensions.is_empty()
        || fault_dimensions.is_empty()
        || cancellation_points.is_empty()
    {
        return None;
    }
    Some(
        scheduler_dimensions.len() as u64
            * fault_dimensions.len() as u64
            * cancellation_points.len() as u64,
    )
}

fn bounded_completeness(value: Option<&str>) -> BoundedCompleteness {
    match value {
        Some("complete") => BoundedCompleteness::Complete,
        Some("timeout") => BoundedCompleteness::Timeout,
        Some("incomplete") => BoundedCompleteness::Incomplete,
        Some("sampled") | None => BoundedCompleteness::Sampled,
        Some(_) => BoundedCompleteness::Incomplete,
    }
}

fn bounded_wording(
    completeness: &BoundedCompleteness,
    enumerated_history_count: u64,
    expected_complete_history_count: Option<u64>,
) -> String {
    if completeness == &BoundedCompleteness::Complete
        && expected_complete_history_count == Some(enumerated_history_count)
    {
        return format!(
            "bounded proof: all {enumerated_history_count} histories explored under declared bounds"
        );
    }
    format!("evidence only: {enumerated_history_count} sampled histories, state space incomplete")
}

fn trace_event_kind(kind: &ObligationEventKind) -> Option<TraceEventKind> {
    match kind {
        ObligationEventKind::Create => Some(TraceEventKind::Create),
        ObligationEventKind::Discharge => Some(TraceEventKind::Discharge),
        ObligationEventKind::Transfer => Some(TraceEventKind::Transfer),
        ObligationEventKind::Move => Some(TraceEventKind::Move),
        ObligationEventKind::Escape => Some(TraceEventKind::Escape),
        ObligationEventKind::BranchUnresolved
        | ObligationEventKind::UnsupportedContainer
        | ObligationEventKind::Call => None,
    }
}

fn lowering_trace_event_for_core<'a>(
    source_map: &'a KoboSourceMap,
    program: &ScenarioProgram,
    event: &CoreTraceEvent,
    used: &BTreeSet<usize>,
) -> Option<(usize, &'a LoweringTraceEvent)> {
    source_map
        .lowering_trace
        .iter()
        .enumerate()
        .filter(|(index, lowering_event)| {
            !used.contains(index)
                && lowering_event.function == program.target
                && lowering_event.kind == event.kind.as_str()
                && lowering_event.binding == event.binding
        })
        .min_by_key(|(_, lowering_event)| {
            lowering_event
                .kobo_span
                .start
                .abs_diff(event.source_span.start as u32)
        })
}

fn generated_source_span(
    source_map: &KoboSourceMap,
    lowering_event: &LoweringTraceEvent,
) -> SourceSpan {
    SourceSpan {
        path: source_map.generated_file().to_owned(),
        line: lowering_event.rs_span.line,
        start: lowering_event.rs_span.column_start,
        end: lowering_event.rs_span.column_end,
        mapped: true,
        snippet: String::new(),
    }
}

fn source_span_from_kobo_span(source_path: &str, source: &str, span: &KoboSpan) -> SourceSpan {
    source_span_from_kobo(source_path, source, *span)
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
) -> Result<(Vec<HashEvidence>, Vec<TemplateSchemaEvidence>), serde_json::Error> {
    let mut hashes = Vec::new();
    let mut schemas = Vec::new();
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
        let schema = TemplateSchemaEvidence {
            id: template.id.clone(),
            kind: template.kind.clone(),
            template_schema: template.template_schema.clone(),
            schema_version: template.schema_version,
            confidence: template.confidence.clone(),
            source: template_source.to_owned(),
            source_span: source_span_from_kobo(source_path, source, operation.span),
        };
        let hash = template_schema_hash(&schema)?;
        hashes.push(HashEvidence {
            id: template.id.clone(),
            hash,
        });
        schemas.push(schema);
    }
    Ok((hashes, schemas))
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

pub fn adapter_evidence(
    program: &ScenarioProgram,
    adapter_policies: &[EcosystemAdapterPolicy],
    requested_replay_grade: ReplayGrade,
) -> Vec<AdapterEvidence> {
    let mut evidence = Vec::new();
    for operation in &program.operations {
        let ScenarioOpKind::ExternalBoundary {
            crate_name, policy, ..
        } = &operation.kind
        else {
            continue;
        };
        let Some(adapter) = adapter_policies
            .iter()
            .find(|adapter| adapter.crate_name == *crate_name)
        else {
            continue;
        };
        let confidence = adapter_confidence(adapter, policy);
        let outcome = adapter_outcome(adapter, &confidence);
        evidence.push(AdapterEvidence {
            boundary: crate_name.clone(),
            adapter: adapter.package.clone(),
            version: adapter.version.clone(),
            confidence,
            replay_grade: requested_replay_grade.clone(),
            outcome,
            reason: adapter
                .reason
                .clone()
                .unwrap_or_else(|| "configured ecosystem adapter".to_owned()),
        });
    }
    evidence.sort_by(|left, right| {
        (&left.boundary, &left.adapter, &left.version).cmp(&(
            &right.boundary,
            &right.adapter,
            &right.version,
        ))
    });
    evidence.dedup_by(|left, right| {
        left.boundary == right.boundary
            && left.adapter == right.adapter
            && left.version == right.version
    });
    evidence
}

pub fn adapter_adjusted_replay_grade(
    requested_replay_grade: ReplayGrade,
    adapters: &[AdapterEvidence],
) -> ReplayGrade {
    if matches!(
        requested_replay_grade,
        ReplayGrade::Debt | ReplayGrade::NotReplayable
    ) {
        return requested_replay_grade;
    }
    if adapters.iter().any(adapter_is_stale) {
        return ReplayGrade::Debt;
    }
    if adapters.iter().any(|adapter| {
        matches!(
            adapter.confidence,
            AdapterConfidence::Sampled | AdapterConfidence::MetadataOnly
        )
    }) {
        return ReplayGrade::NotReplayable;
    }
    if requested_replay_grade == ReplayGrade::Exact
        && adapters
            .iter()
            .any(|adapter| adapter.confidence == AdapterConfidence::Modeled)
    {
        return ReplayGrade::Partial;
    }
    requested_replay_grade
}

fn adapter_confidence(
    adapter: &EcosystemAdapterPolicy,
    policy: &kobo_ir::ScenarioBoundaryPolicy,
) -> AdapterConfidence {
    match adapter.confidence.as_deref() {
        Some("exact") => AdapterConfidence::Exact,
        Some("modeled") => AdapterConfidence::Modeled,
        Some("sampled") => AdapterConfidence::Sampled,
        Some("metadata-only") | Some("metadata_only") => AdapterConfidence::MetadataOnly,
        _ if !adapter.validated => AdapterConfidence::MetadataOnly,
        _ if adapter.capture.as_deref() == Some("boundary-io")
            && matches!(policy, kobo_ir::ScenarioBoundaryPolicy::Record) =>
        {
            AdapterConfidence::Exact
        }
        _ if adapter.adapter_runtime.is_some() => AdapterConfidence::Modeled,
        _ => AdapterConfidence::MetadataOnly,
    }
}

fn adapter_outcome(adapter: &EcosystemAdapterPolicy, confidence: &AdapterConfidence) -> String {
    if adapter_version_is_stale(adapter.version.as_deref()) {
        return "debt".to_owned();
    }
    match confidence {
        AdapterConfidence::Exact | AdapterConfidence::Modeled => "proof".to_owned(),
        AdapterConfidence::Sampled => "probing_pass".to_owned(),
        AdapterConfidence::MetadataOnly => "metadata_only".to_owned(),
    }
}

fn adapter_is_stale(adapter: &AdapterEvidence) -> bool {
    adapter_version_is_stale(adapter.version.as_deref()) || adapter.outcome == "debt"
}

fn adapter_version_is_stale(version: Option<&str>) -> bool {
    let Some(version) = version else {
        return true;
    };
    version == "0.0.0" || version.contains("stale")
}

pub fn candidate_admission_evidence(
    source_path: &Path,
    source: &str,
    replay_grade: ReplayGrade,
    adapter_confidence: &[AdapterEvidence],
) -> Vec<CandidateAdmissionEvidence> {
    let Ok(file) = syn::parse_file(source) else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    for item in &file.items {
        for attr in candidate_attrs(item) {
            let Some(fields) = candidate_attr_fields(attr) else {
                continue;
            };
            let replay_related = bool_field(&fields, "replay_related").unwrap_or(false);
            candidates.push(CandidateAdmissionEvidence {
                id: string_field(&fields, "id").unwrap_or_else(|| "unknown".to_owned()),
                track: string_field(&fields, "track").unwrap_or_else(|| "unknown".to_owned()),
                status: string_field(&fields, "status").unwrap_or_else(|| "research".to_owned()),
                evidence: candidate_evidence_facts(source_path, &file, &fields),
                inspect_visibility: string_field(&fields, "inspect"),
                manual_rust_equivalent: string_field(&fields, "manual_rust"),
                strict_compatible: bool_field(&fields, "strict").unwrap_or(false),
                whole_ecosystem_modeling_required: bool_field(&fields, "whole_ecosystem")
                    .unwrap_or(true),
                diagnostic_snapshots: string_field(&fields, "diagnostic_snapshot")
                    .into_iter()
                    .collect(),
                replay_related,
                replay_grade: replay_related.then_some(replay_grade.clone()),
                adapter_confidence: replay_related
                    .then(|| adapter_confidence.to_vec())
                    .unwrap_or_default(),
            });
        }
    }
    candidates
}

fn candidate_attrs(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Fn(item) => &item.attrs,
        syn::Item::Impl(item) => &item.attrs,
        syn::Item::Struct(item) => &item.attrs,
        syn::Item::Mod(item) => &item.attrs,
        syn::Item::Trait(item) => &item.attrs,
        _ => &[],
    }
}

fn candidate_attr_fields(attr: &syn::Attribute) -> Option<BTreeMap<String, String>> {
    if !syn_path_ends_with(attr.path(), &["kobo", "candidate_track"]) {
        return None;
    }
    let syn::Meta::List(list) = &attr.meta else {
        return None;
    };
    let entries = list
        .parse_args_with(
            syn::punctuated::Punctuated::<syn::MetaNameValue, syn::Token![,]>::parse_terminated,
        )
        .ok()?;
    let mut fields = BTreeMap::new();
    for entry in entries {
        let key = entry
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())?;
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(value),
            ..
        }) = entry.value
        else {
            continue;
        };
        fields.insert(key, value.value());
    }
    Some(fields)
}

fn string_field(fields: &BTreeMap<String, String>, key: &str) -> Option<String> {
    fields.get(key).filter(|value| !value.is_empty()).cloned()
}

fn bool_field(fields: &BTreeMap<String, String>, key: &str) -> Option<bool> {
    match fields.get(key)?.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn candidate_evidence_facts(
    source_path: &Path,
    file: &syn::File,
    fields: &BTreeMap<String, String>,
) -> Vec<CandidateAdmissionFact> {
    const RESERVED: &[&str] = &[
        "id",
        "track",
        "status",
        "inspect",
        "manual_rust",
        "strict",
        "whole_ecosystem",
        "diagnostic_snapshot",
        "replay_related",
    ];
    const DERIVED: &[&str] = &[
        "target_rust",
        "avoids_nightly",
        "no_hidden_heap",
        "allocation_report",
        "memory_budget",
        "hidden_heap_sites",
        "cast_policy",
        "debt_casts",
        "strict_casts",
        "transform_set",
        "desugaring",
        "coherence",
        "hidden_impls",
        "context_threading",
        "hidden_globals",
        "stable_semantics",
        "temporal_extension",
        "adapter_scope",
        "adapter_treadmill",
        "minimization_proof",
        "backend_user_theory",
        "backend_assumptions",
    ];
    let mut facts = fields
        .iter()
        .filter(|(key, _)| !RESERVED.contains(&key.as_str()))
        .filter(|(key, _)| !DERIVED.contains(&key.as_str()))
        .map(|(key, value)| CandidateAdmissionFact {
            key: key.clone(),
            value: value.clone(),
        })
        .collect::<Vec<_>>();
    facts.extend(derived_candidate_facts(source_path, file, fields));
    facts.sort_by(|left, right| left.key.cmp(&right.key));
    facts.dedup_by(|left, right| left.key == right.key);
    facts
}

fn derived_candidate_facts(
    source_path: &Path,
    file: &syn::File,
    fields: &BTreeMap<String, String>,
) -> Vec<CandidateAdmissionFact> {
    let mut facts = Vec::new();
    let candidate_id = fields.get("id").map(String::as_str).unwrap_or("");
    let config = read_project_config(source_path);
    match candidate_id {
        "S-32" => {
            if let Some(target_rust) = config_string(&config, &["output", "target_rust"]) {
                facts.push(candidate_fact("target_rust", target_rust));
                facts.push(candidate_fact(
                    "avoids_nightly",
                    (!file_uses_nightly_features(file)).to_string(),
                ));
            }
        }
        "S-49" => {
            if config_bool(&config, &["output", "no_std"]).unwrap_or(false) {
                let hidden_heap_sites = file_hidden_heap_site_count(file);
                facts.push(candidate_fact(
                    "no_hidden_heap",
                    (hidden_heap_sites == 0).to_string(),
                ));
                facts.push(candidate_fact("allocation_report", "structural"));
                facts.push(candidate_fact(
                    "hidden_heap_sites",
                    hidden_heap_sites.to_string(),
                ));
                if let Some(memory_budget) = config_string(&config, &["output", "memory_budget"]) {
                    facts.push(candidate_fact("memory_budget", memory_budget));
                }
            }
        }
        "S-37+" => {
            if let Some(policy) = config_string(&config, &["casts", "policy"]) {
                let cast_count = file_cast_site_count(file);
                facts.push(candidate_fact("cast_policy", policy));
                if cast_count > 0 {
                    facts.push(candidate_fact("debt_casts", "source_spans"));
                    facts.push(candidate_fact("strict_casts", "raw-casts-present"));
                } else {
                    facts.push(candidate_fact("debt_casts", "none"));
                    facts.push(candidate_fact("strict_casts", "explicit"));
                }
            }
        }
        "S-46" => {
            let transform_set = file_attrs_with_path(file, &["kobo", "sugar"])
                .into_iter()
                .flat_map(attr_path_arguments)
                .filter(|argument| {
                    matches!(
                        argument.as_str(),
                        "builder" | "visitor" | "state_machine" | "event_enum"
                    )
                })
                .collect::<Vec<_>>();
            if !transform_set.is_empty() {
                let transform_set = ["builder", "visitor", "state_machine", "event_enum"]
                    .into_iter()
                    .filter(|token| transform_set.iter().any(|argument| argument == token))
                    .collect::<Vec<_>>()
                    .join("|");
                facts.push(candidate_fact("transform_set", transform_set));
                facts.push(candidate_fact("desugaring", "inspectable"));
            }
        }
        "S-47" => {
            if !file_attrs_with_path(file, &["kobo", "newtype_scaffold"]).is_empty() {
                facts.push(candidate_fact("coherence", "newtype_forwarding"));
                facts.push(candidate_fact(
                    "hidden_impls",
                    file_has_hidden_impls(file).to_string(),
                ));
            }
        }
        "S-48" => {
            let hidden_globals = file_has_hidden_globals(file);
            if file_has_context_binding(file) || hidden_globals {
                facts.push(candidate_fact("context_threading", "explicit"));
                facts.push(candidate_fact("hidden_globals", hidden_globals.to_string()));
            }
        }
        "research-smt-temporal" => {
            if file_has_attr(file, &["kobo", "ward"]) && file_has_attr(file, &["kobo", "invariant"])
            {
                facts.push(candidate_fact("stable_semantics", "ward|invariant"));
            }
            if file_attrs_with_path(file, &["kobo", "temporal"])
                .into_iter()
                .any(|attr| {
                    attr_string_field(attr, "query").as_deref()
                        == Some("beyond_always_eventually_never")
                })
            {
                facts.push(candidate_fact(
                    "temporal_extension",
                    "beyond_always_eventually_never",
                ));
            }
        }
        "research-broad-adapters" => {
            if let Some(scope) = config_string(&config, &["adapters", "scope"]) {
                facts.push(candidate_fact("adapter_scope", scope));
            }
            if let Some(treadmill) = config_string(&config, &["adapters", "treadmill"]) {
                facts.push(candidate_fact("adapter_treadmill", treadmill));
            }
        }
        "research-model-checking" => {
            if file_attrs_with_path(file, &["kobo", "witness_minimization"])
                .into_iter()
                .any(|attr| attr_has_path_argument(attr, "labeled_trace"))
            {
                facts.push(candidate_fact("minimization_proof", "labeled_trace"));
            }
            for attr in file_attrs_with_path(file, &["kobo", "model_check"]) {
                if let Some(theory) = attr_string_field(attr, "backend_user_theory") {
                    if theory == "kobo_core_loop" || theory == "z3_smt" {
                        facts.push(candidate_fact("backend_user_theory", theory));
                    }
                }
                if attr_string_field(attr, "backend_assumptions").as_deref() == Some("ledger") {
                    facts.push(candidate_fact("backend_assumptions", "ledger"));
                }
            }
        }
        _ => {}
    }
    facts
}

fn candidate_fact(key: &str, value: impl Into<String>) -> CandidateAdmissionFact {
    CandidateAdmissionFact {
        key: key.to_owned(),
        value: value.into(),
    }
}

fn read_project_config(source_path: &Path) -> Option<toml::Value> {
    for directory in source_path.parent().into_iter().flat_map(Path::ancestors) {
        let config_path = directory.join("Kobo.toml");
        let Ok(source) = std::fs::read_to_string(&config_path) else {
            continue;
        };
        if let Ok(config) = source.parse::<toml::Value>() {
            return Some(config);
        }
    }
    None
}

fn config_string(config: &Option<toml::Value>, path: &[&str]) -> Option<String> {
    let mut current = config.as_ref()?;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(str::to_owned)
}

fn config_bool(config: &Option<toml::Value>, path: &[&str]) -> Option<bool> {
    let mut current = config.as_ref()?;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_bool()
}

fn file_uses_nightly_features(file: &syn::File) -> bool {
    file.attrs
        .iter()
        .any(|attr| syn_path_ends_with(attr.path(), &["feature"]))
}

fn file_hidden_heap_site_count(file: &syn::File) -> usize {
    struct HiddenHeapVisitor {
        count: usize,
    }

    impl<'ast> syn::visit::Visit<'ast> for HiddenHeapVisitor {
        fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
            if let syn::Expr::Path(function) = call.func.as_ref() {
                if path_is_heap_constructor(&function.path) {
                    self.count += 1;
                }
            }
            syn::visit::visit_expr_call(self, call);
        }

        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            if matches!(
                call.method.to_string().as_str(),
                "with_capacity" | "try_with_capacity" | "collect" | "to_vec" | "to_string"
            ) {
                self.count += 1;
            }
            syn::visit::visit_expr_method_call(self, call);
        }

        fn visit_macro(&mut self, mac: &'ast syn::Macro) {
            if path_last_ident_is(&mac.path, "vec") || path_last_ident_is(&mac.path, "format") {
                self.count += 1;
            }
            syn::visit::visit_macro(self, mac);
        }
    }

    let mut visitor = HiddenHeapVisitor { count: 0 };
    syn::visit::visit_file(&mut visitor, file);
    visitor.count
}

fn file_cast_site_count(file: &syn::File) -> usize {
    struct CastVisitor {
        count: usize,
    }

    impl<'ast> syn::visit::Visit<'ast> for CastVisitor {
        fn visit_expr_cast(&mut self, cast: &'ast syn::ExprCast) {
            self.count += 1;
            syn::visit::visit_expr_cast(self, cast);
        }
    }

    let mut visitor = CastVisitor { count: 0 };
    syn::visit::visit_file(&mut visitor, file);
    visitor.count
}

fn file_has_hidden_impls(file: &syn::File) -> bool {
    file.items.iter().any(item_has_hidden_impl)
}

fn item_has_hidden_impl(item: &syn::Item) -> bool {
    match item {
        syn::Item::Impl(item) => {
            item.attrs
                .iter()
                .any(|attr| syn_path_ends_with(attr.path(), &["hidden_impl"]))
                || item.trait_.as_ref().is_some_and(|(_, trait_path, _)| {
                    path_has_segment(trait_path, "external")
                        || path_has_segment(trait_path, "ExternalTrait")
                })
                || type_has_segment(item.self_ty.as_ref(), "external")
                || type_has_segment(item.self_ty.as_ref(), "ExternalType")
        }
        syn::Item::Mod(item) => item
            .content
            .as_ref()
            .is_some_and(|(_, items)| items.iter().any(item_has_hidden_impl)),
        _ => false,
    }
}

fn file_has_hidden_globals(file: &syn::File) -> bool {
    struct HiddenGlobalVisitor {
        found: bool,
    }

    impl<'ast> syn::visit::Visit<'ast> for HiddenGlobalVisitor {
        fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
            if matches!(item.mutability, syn::StaticMutability::Mut(_)) {
                self.found = true;
            }
            syn::visit::visit_item_static(self, item);
        }

        fn visit_macro(&mut self, mac: &'ast syn::Macro) {
            if path_last_ident_is(&mac.path, "lazy_static")
                || path_last_ident_is(&mac.path, "thread_local")
            {
                self.found = true;
            }
            syn::visit::visit_macro(self, mac);
        }
    }

    let mut visitor = HiddenGlobalVisitor { found: false };
    syn::visit::visit_file(&mut visitor, file);
    visitor.found
}

fn file_has_context_binding(file: &syn::File) -> bool {
    struct ContextBindingVisitor {
        found: bool,
    }

    impl<'ast> syn::visit::Visit<'ast> for ContextBindingVisitor {
        fn visit_pat_ident(&mut self, pat: &'ast syn::PatIdent) {
            if pat.ident == "ctx" {
                self.found = true;
            }
            syn::visit::visit_pat_ident(self, pat);
        }
    }

    let mut visitor = ContextBindingVisitor { found: false };
    syn::visit::visit_file(&mut visitor, file);
    visitor.found
}

fn file_has_attr(file: &syn::File, suffix: &[&str]) -> bool {
    !file_attrs_with_path(file, suffix).is_empty()
}

fn file_attrs_with_path<'a>(file: &'a syn::File, suffix: &[&str]) -> Vec<&'a syn::Attribute> {
    let mut attrs = Vec::new();
    attrs.extend(
        file.attrs
            .iter()
            .filter(|attr| syn_path_ends_with(attr.path(), suffix)),
    );
    for item in &file.items {
        collect_item_attrs_with_path(item, suffix, &mut attrs);
    }
    attrs
}

fn collect_item_attrs_with_path<'a>(
    item: &'a syn::Item,
    suffix: &[&str],
    attrs: &mut Vec<&'a syn::Attribute>,
) {
    attrs.extend(
        item_attributes(item)
            .iter()
            .filter(|attr| syn_path_ends_with(attr.path(), suffix)),
    );
    match item {
        syn::Item::Impl(item) => {
            for impl_item in &item.items {
                attrs.extend(
                    impl_item_attributes(impl_item)
                        .iter()
                        .filter(|attr| syn_path_ends_with(attr.path(), suffix)),
                );
            }
        }
        syn::Item::Mod(item) => {
            if let Some((_, items)) = &item.content {
                for item in items {
                    collect_item_attrs_with_path(item, suffix, attrs);
                }
            }
        }
        _ => {}
    }
}

fn item_attributes(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Const(item) => &item.attrs,
        syn::Item::Enum(item) => &item.attrs,
        syn::Item::Fn(item) => &item.attrs,
        syn::Item::Impl(item) => &item.attrs,
        syn::Item::Mod(item) => &item.attrs,
        syn::Item::Static(item) => &item.attrs,
        syn::Item::Struct(item) => &item.attrs,
        syn::Item::Trait(item) => &item.attrs,
        syn::Item::Type(item) => &item.attrs,
        syn::Item::Union(item) => &item.attrs,
        _ => &[],
    }
}

fn impl_item_attributes(item: &syn::ImplItem) -> &[syn::Attribute] {
    match item {
        syn::ImplItem::Const(item) => &item.attrs,
        syn::ImplItem::Fn(item) => &item.attrs,
        syn::ImplItem::Type(item) => &item.attrs,
        _ => &[],
    }
}

fn attr_path_arguments(attr: &syn::Attribute) -> Vec<String> {
    let syn::Meta::List(list) = &attr.meta else {
        return Vec::new();
    };
    list.parse_args_with(syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)
        .map(|entries| {
            entries
                .into_iter()
                .filter_map(|entry| match entry {
                    syn::Meta::Path(path) => path
                        .segments
                        .last()
                        .map(|segment| segment.ident.to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn attr_has_path_argument(attr: &syn::Attribute, argument: &str) -> bool {
    attr_path_arguments(attr)
        .iter()
        .any(|candidate| candidate == argument)
}

fn attr_string_field(attr: &syn::Attribute, key: &str) -> Option<String> {
    let syn::Meta::List(list) = &attr.meta else {
        return None;
    };
    let entries = list
        .parse_args_with(
            syn::punctuated::Punctuated::<syn::MetaNameValue, syn::Token![,]>::parse_terminated,
        )
        .ok()?;
    for entry in entries {
        let field = entry
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())?;
        if field != key {
            continue;
        }
        if let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(value),
            ..
        }) = entry.value
        {
            return Some(value.value());
        }
    }
    None
}

fn path_is_heap_constructor(path: &syn::Path) -> bool {
    let last = path
        .segments
        .last()
        .map(|segment| segment.ident.to_string());
    let Some(last) = last else {
        return false;
    };
    matches!(
        last.as_str(),
        "new" | "from" | "with_capacity" | "try_with_capacity"
    ) && (path_has_segment(path, "Vec")
        || path_has_segment(path, "Box")
        || path_has_segment(path, "String")
        || path_has_segment(path, "alloc"))
}

fn path_last_ident_is(path: &syn::Path, expected: &str) -> bool {
    path.segments
        .last()
        .is_some_and(|segment| segment.ident == expected)
}

fn path_has_segment(path: &syn::Path, expected: &str) -> bool {
    path.segments
        .iter()
        .any(|segment| segment.ident == expected)
}

fn type_has_segment(ty: &syn::Type, expected: &str) -> bool {
    match ty {
        syn::Type::Path(ty) => path_has_segment(&ty.path, expected),
        syn::Type::Reference(ty) => type_has_segment(&ty.elem, expected),
        syn::Type::Group(ty) => type_has_segment(&ty.elem, expected),
        syn::Type::Paren(ty) => type_has_segment(&ty.elem, expected),
        _ => false,
    }
}

fn syn_path_ends_with(path: &syn::Path, suffix: &[&str]) -> bool {
    let segments = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    segments.len() >= suffix.len()
        && segments[segments.len() - suffix.len()..]
            .iter()
            .zip(suffix)
            .all(|(left, right)| left == right)
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
    let entry_env = env_states(&BTreeMap::new());
    let mut events = Vec::new();
    let mut terminal_envs = Vec::new();
    for function in functions {
        let replay = function_obligation_replay(function);
        for block in &function.blocks {
            let mut current_env = replay
                .block_entry_envs
                .get(&block.id)
                .cloned()
                .unwrap_or_default();
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
        terminal_envs.extend(replay.terminal_envs);
    }
    let exit_env = env_states(&merge_terminal_envs(&terminal_envs));
    (entry_env, exit_env, events)
}

struct FunctionObligationReplay {
    block_entry_envs: BTreeMap<String, BTreeMap<String, ObligationStatus>>,
    block_exit_envs: BTreeMap<String, BTreeMap<String, ObligationStatus>>,
    terminal_envs: Vec<BTreeMap<String, ObligationStatus>>,
}

fn function_obligation_replay(function: &CoreFunction) -> FunctionObligationReplay {
    let blocks = function
        .blocks
        .iter()
        .map(|block| (block.id.as_str(), block))
        .collect::<BTreeMap<_, _>>();
    let Some(entry_block) = function.blocks.first() else {
        return FunctionObligationReplay {
            block_entry_envs: BTreeMap::new(),
            block_exit_envs: BTreeMap::new(),
            terminal_envs: Vec::new(),
        };
    };

    let mut block_entry_envs = BTreeMap::<String, BTreeMap<String, ObligationStatus>>::new();
    let mut queued = VecDeque::new();
    block_entry_envs.insert(entry_block.id.clone(), BTreeMap::new());
    queued.push_back(entry_block.id.clone());

    while let Some(block_id) = queued.pop_front() {
        let Some(block) = blocks.get(block_id.as_str()).copied() else {
            continue;
        };
        let entry_env = block_entry_envs.get(&block.id).cloned().unwrap_or_default();
        let exit_env = apply_block_obligation_statements(block, entry_env);
        for target in block_successor_targets(block) {
            if is_back_edge_between_blocks(&block.id, &target) {
                continue;
            }
            let Some(target_block) = blocks.get(target.as_str()) else {
                continue;
            };
            let changed = merge_block_entry_env(
                block_entry_envs
                    .entry(target_block.id.clone())
                    .or_insert_with(BTreeMap::new),
                &exit_env,
            );
            if changed {
                queued.push_back(target_block.id.clone());
            }
        }
    }

    let block_exit_envs = function
        .blocks
        .iter()
        .filter_map(|block| {
            block_entry_envs.get(&block.id).cloned().map(|entry_env| {
                (
                    block.id.clone(),
                    apply_block_obligation_statements(block, entry_env),
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    let terminal_envs = function
        .blocks
        .iter()
        .filter(|block| block_successor_targets(block).is_empty())
        .filter_map(|block| block_exit_envs.get(&block.id).cloned())
        .collect::<Vec<_>>();

    FunctionObligationReplay {
        block_entry_envs,
        block_exit_envs,
        terminal_envs,
    }
}

fn is_back_edge_between_blocks(source: &str, target: &str) -> bool {
    let Some(source_index) = block_index(source) else {
        return false;
    };
    let Some(target_index) = block_index(target) else {
        return false;
    };
    target_index <= source_index
}

fn apply_block_obligation_statements(
    block: &CoreBlock,
    mut env: BTreeMap<String, ObligationStatus>,
) -> BTreeMap<String, ObligationStatus> {
    for statement in &block.statements {
        apply_obligation_statement(statement, &mut env);
    }
    env
}

fn block_successor_targets(block: &CoreBlock) -> Vec<String> {
    block
        .terminators
        .iter()
        .flat_map(|terminator| terminator.edges.iter())
        .filter_map(|edge| edge.strip_prefix("goto:").map(str::to_owned))
        .collect()
}

fn merge_block_entry_env(
    current: &mut BTreeMap<String, ObligationStatus>,
    incoming: &BTreeMap<String, ObligationStatus>,
) -> bool {
    let before = current.clone();
    for (binding, incoming_status) in incoming {
        match current.get(binding) {
            Some(current_status) if current_status == incoming_status => {}
            Some(_) => {
                current.insert(binding.clone(), ObligationStatus::BranchUnresolved);
            }
            None => {
                current.insert(binding.clone(), incoming_status.clone());
            }
        }
    }
    *current != before
}

fn merge_terminal_envs(
    terminal_envs: &[BTreeMap<String, ObligationStatus>],
) -> BTreeMap<String, ObligationStatus> {
    let mut merged = BTreeMap::new();
    for env in terminal_envs {
        merge_block_entry_env(&mut merged, env);
    }
    merged
}

fn async_model_evidence(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
    functions: &[CoreFunction],
) -> AsyncModelEvidence {
    let live_locals_by_await = parsed_live_locals_by_await(source, &program.target);
    let mut model = AsyncModelEvidence::default();

    for function in functions {
        let replay = function_obligation_replay(function);
        let mut await_index = 0usize;
        for block in &function.blocks {
            let block_exit_env = replay
                .block_exit_envs
                .get(&block.id)
                .cloned()
                .unwrap_or_default();
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
                            boundary: terminator.boundary.clone(),
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
                        let live_locals = live_locals_by_await
                            .get(await_index)
                            .cloned()
                            .unwrap_or_default();
                        await_index += 1;
                        for local in &live_locals {
                            model.future_state_locals.push(FutureStateLocalEvidence {
                                binding: local.clone(),
                                suspension_state: suspension_id.clone(),
                                source_span: source_span_for_binding(source_path, source, local),
                            });
                        }
                        for state in env_states(&block_exit_env)
                            .into_iter()
                            .filter(|state| state.state == ObligationStatus::Owned)
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
                        if terminator.boundary.as_deref() == Some("tokio::time::timeout") {
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
                        let cancelled_obligations = env_states(&block_exit_env)
                            .into_iter()
                            .filter(|state| state.state == ObligationStatus::Owned)
                            .collect::<Vec<_>>();
                        for branch_target in terminator
                            .edges
                            .iter()
                            .filter_map(|edge| edge.strip_prefix("goto:"))
                        {
                            let obligation_results = replay
                                .block_exit_envs
                                .get(branch_target)
                                .map(env_states)
                                .unwrap_or_default();
                            for path_kind in ["winner", "loser_cancel"] {
                                let cancelled_obligations = (path_kind == "loser_cancel")
                                    .then(|| cancelled_obligations.clone())
                                    .unwrap_or_default();
                                model.select_paths.push(SelectPathEvidence {
                                    id: format!(
                                        "{}:{}:{}:{branch_target}:{path_kind}",
                                        function.name, block.id, terminator.id
                                    ),
                                    function: function.name.clone(),
                                    branch_block: block.id.clone(),
                                    branch_target: branch_target.to_owned(),
                                    path_kind: path_kind.to_owned(),
                                    obligation_results: obligation_results.clone(),
                                    cancelled_obligations: cancelled_obligations.clone(),
                                    obligation_result_hash: canonical_select_result_hash(
                                        branch_target,
                                        path_kind,
                                        &obligation_results,
                                        &cancelled_obligations,
                                    ),
                                    source_span: source_span.clone(),
                                });
                            }
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
    current_env: &mut BTreeMap<String, ObligationStatus>,
) {
    match statement.kind {
        CoreStatementKind::ObligationCreate => {
            if let Some(binding) = statement.binding.as_ref() {
                current_env.insert(binding.clone(), ObligationStatus::Owned);
            }
        }
        CoreStatementKind::ObligationDischarge => {
            if let Some(binding) = statement.binding.as_ref() {
                current_env.insert(binding.clone(), ObligationStatus::Resolved);
            }
        }
        CoreStatementKind::ObligationTransfer => {
            if let Some(binding) = statement.binding.as_ref() {
                current_env.insert(binding.clone(), ObligationStatus::Transferred);
            }
        }
        CoreStatementKind::ObligationMove => {
            if let Some(binding) = statement.binding.as_ref() {
                current_env.insert(binding.clone(), ObligationStatus::Moved);
            }
        }
        CoreStatementKind::ObligationBranchUnresolved => {
            if let Some(binding) = statement.binding.as_ref() {
                current_env.insert(binding.clone(), ObligationStatus::BranchUnresolved);
            }
        }
        CoreStatementKind::ObligationEscape => {
            if let Some(binding) = statement.binding.as_ref() {
                current_env.insert(binding.clone(), ObligationStatus::Escaped);
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

fn env_states(env: &BTreeMap<String, ObligationStatus>) -> Vec<ObligationState> {
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

fn canonical_select_result_hash(
    branch_target: &str,
    path_kind: &str,
    results: &[ObligationState],
    cancelled_obligations: &[ObligationState],
) -> String {
    let mut states = results
        .iter()
        .map(|state| format!("{}:{}", state.binding, state.state.as_str()))
        .collect::<Vec<_>>();
    states.sort_unstable();
    let mut cancelled = cancelled_obligations
        .iter()
        .map(|state| format!("{}:{}", state.binding, state.state.as_str()))
        .collect::<Vec<_>>();
    cancelled.sort_unstable();
    stable_hash(&format!(
        "select-result:{branch_target}:{path_kind}:{states:?}:cancelled:{cancelled:?}"
    ))
}

fn parsed_live_locals_by_await(source: &str, target: &str) -> Vec<Vec<String>> {
    let Ok(file) = syn::parse_file(source) else {
        return Vec::new();
    };
    let Some(function) = file.items.iter().find_map(|item| match item {
        syn::Item::Fn(function) if function.sig.ident == target => Some(function),
        _ => None,
    }) else {
        return Vec::new();
    };

    let mut locals_before_statement = Vec::<BTreeSet<String>>::new();
    let mut declared = BTreeSet::<String>::new();
    for statement in &function.block.stmts {
        locals_before_statement.push(declared.clone());
        collect_pat_bindings_in_stmt(statement, &mut declared);
    }

    let mut later_statement_uses = vec![BTreeSet::<String>::new(); function.block.stmts.len()];
    let mut suffix_uses = BTreeSet::<String>::new();
    for (index, statement) in function.block.stmts.iter().enumerate().rev() {
        later_statement_uses[index] = suffix_uses.clone();
        collect_ident_uses_in_stmt(statement, &mut suffix_uses);
    }

    function
        .block
        .stmts
        .iter()
        .enumerate()
        .flat_map(|(index, statement)| {
            let declared_before = locals_before_statement
                .get(index)
                .cloned()
                .unwrap_or_default();
            await_live_locals_in_stmt(statement, &later_statement_uses[index], &declared_before)
                .into_iter()
                .map(|locals| locals.into_iter().collect::<Vec<_>>())
        })
        .collect()
}

fn await_live_locals_in_stmt(
    statement: &syn::Stmt,
    later_statement_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> Vec<BTreeSet<String>> {
    match statement {
        syn::Stmt::Local(local) => local
            .init
            .as_ref()
            .map(|init| {
                await_live_locals_in_expr(init.expr.as_ref(), later_statement_uses, declared_before)
                    .0
            })
            .unwrap_or_default(),
        syn::Stmt::Expr(expr, _) => {
            await_live_locals_in_expr(expr, later_statement_uses, declared_before).0
        }
        syn::Stmt::Macro(_) | syn::Stmt::Item(_) => Vec::new(),
    }
}

fn await_live_locals_in_expr(
    expr: &syn::Expr,
    later_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    match expr {
        syn::Expr::Await(await_expr) => {
            let (mut awaits, uses) =
                await_live_locals_in_expr(await_expr.base.as_ref(), later_uses, declared_before);
            awaits.push(live_declared_locals(later_uses, declared_before));
            (awaits, uses)
        }
        syn::Expr::Array(array) => {
            await_live_locals_in_expr_sequence(array.elems.iter(), later_uses, declared_before)
        }
        syn::Expr::Assign(assign) => await_live_locals_in_expr_sequence(
            [assign.left.as_ref(), assign.right.as_ref()].into_iter(),
            later_uses,
            declared_before,
        ),
        syn::Expr::Binary(binary) => await_live_locals_in_expr_sequence(
            [binary.left.as_ref(), binary.right.as_ref()].into_iter(),
            later_uses,
            declared_before,
        ),
        syn::Expr::Block(block) => {
            await_live_locals_in_block(&block.block, later_uses, declared_before)
        }
        syn::Expr::Break(expr_break) => expr_break
            .expr
            .as_deref()
            .map(|value| await_live_locals_in_expr(value, later_uses, declared_before))
            .unwrap_or_default(),
        syn::Expr::Call(call) => await_live_locals_in_expr_sequence(
            std::iter::once(call.func.as_ref()).chain(call.args.iter()),
            later_uses,
            declared_before,
        ),
        syn::Expr::Cast(cast) => {
            await_live_locals_in_expr(cast.expr.as_ref(), later_uses, declared_before)
        }
        syn::Expr::Closure(closure) => {
            await_live_locals_in_expr(closure.body.as_ref(), later_uses, declared_before)
        }
        syn::Expr::Field(field) => {
            await_live_locals_in_expr(field.base.as_ref(), later_uses, declared_before)
        }
        syn::Expr::ForLoop(expr_for) => {
            let mut body_declared = declared_before.clone();
            collect_pat_bindings(&expr_for.pat, &mut body_declared);
            let (body_awaits, body_uses) =
                await_live_locals_in_block(&expr_for.body, later_uses, &body_declared);
            let mut iter_later_uses = later_uses.clone();
            iter_later_uses.extend(body_uses.iter().cloned());
            let (mut awaits, iter_uses) = await_live_locals_in_expr(
                expr_for.expr.as_ref(),
                &iter_later_uses,
                declared_before,
            );
            awaits.extend(body_awaits);
            let mut uses = iter_uses;
            uses.extend(body_uses);
            (awaits, uses)
        }
        syn::Expr::Group(group) => {
            await_live_locals_in_expr(group.expr.as_ref(), later_uses, declared_before)
        }
        syn::Expr::If(expr_if) => await_live_locals_in_if(expr_if, later_uses, declared_before),
        syn::Expr::Index(index) => await_live_locals_in_expr_sequence(
            [index.expr.as_ref(), index.index.as_ref()].into_iter(),
            later_uses,
            declared_before,
        ),
        syn::Expr::Let(expr_let) => {
            await_live_locals_in_expr(expr_let.expr.as_ref(), later_uses, declared_before)
        }
        syn::Expr::Match(expr_match) => {
            await_live_locals_in_match(expr_match, later_uses, declared_before)
        }
        syn::Expr::MethodCall(call) => await_live_locals_in_expr_sequence(
            std::iter::once(call.receiver.as_ref()).chain(call.args.iter()),
            later_uses,
            declared_before,
        ),
        syn::Expr::Paren(paren) => {
            await_live_locals_in_expr(paren.expr.as_ref(), later_uses, declared_before)
        }
        syn::Expr::Range(range) => {
            let children = range
                .start
                .iter()
                .chain(range.end.iter())
                .map(|expr| expr.as_ref());
            await_live_locals_in_expr_sequence(children, later_uses, declared_before)
        }
        syn::Expr::Reference(reference) => {
            await_live_locals_in_expr(reference.expr.as_ref(), later_uses, declared_before)
        }
        syn::Expr::Repeat(repeat) => await_live_locals_in_expr_sequence(
            [repeat.expr.as_ref(), repeat.len.as_ref()].into_iter(),
            later_uses,
            declared_before,
        ),
        syn::Expr::Return(expr_return) => expr_return
            .expr
            .as_deref()
            .map(|value| await_live_locals_in_expr(value, later_uses, declared_before))
            .unwrap_or_default(),
        syn::Expr::Struct(expr_struct) => {
            let children = expr_struct
                .fields
                .iter()
                .map(|field| &field.expr)
                .chain(expr_struct.rest.iter().map(|expr| expr.as_ref()));
            await_live_locals_in_expr_sequence(children, later_uses, declared_before)
        }
        syn::Expr::Try(expr_try) => {
            await_live_locals_in_expr(expr_try.expr.as_ref(), later_uses, declared_before)
        }
        syn::Expr::TryBlock(try_block) => {
            await_live_locals_in_block(&try_block.block, later_uses, declared_before)
        }
        syn::Expr::Tuple(tuple) => {
            await_live_locals_in_expr_sequence(tuple.elems.iter(), later_uses, declared_before)
        }
        syn::Expr::Unary(unary) => {
            await_live_locals_in_expr(unary.expr.as_ref(), later_uses, declared_before)
        }
        syn::Expr::Unsafe(expr_unsafe) => {
            await_live_locals_in_block(&expr_unsafe.block, later_uses, declared_before)
        }
        syn::Expr::While(expr_while) => {
            let (body_awaits, body_uses) =
                await_live_locals_in_block(&expr_while.body, later_uses, declared_before);
            let mut condition_later_uses = later_uses.clone();
            condition_later_uses.extend(body_uses.iter().cloned());
            let (mut awaits, condition_uses) = await_live_locals_in_expr(
                expr_while.cond.as_ref(),
                &condition_later_uses,
                declared_before,
            );
            awaits.extend(body_awaits);
            let mut uses = condition_uses;
            uses.extend(body_uses);
            (awaits, uses)
        }
        syn::Expr::Yield(expr_yield) => expr_yield
            .expr
            .as_deref()
            .map(|value| await_live_locals_in_expr(value, later_uses, declared_before))
            .unwrap_or_default(),
        _ => {
            let uses = ident_uses_in_expr(expr);
            (
                vec![live_declared_locals(later_uses, declared_before); expr_await_count(expr)],
                uses,
            )
        }
    }
}

fn live_declared_locals(
    live_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> BTreeSet<String> {
    declared_before
        .iter()
        .filter(|binding| !binding.starts_with('_'))
        .filter(|binding| live_uses.contains(*binding))
        .cloned()
        .collect()
}

fn await_live_locals_in_block(
    block: &syn::Block,
    later_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    let mut later_statement_uses = vec![BTreeSet::<String>::new(); block.stmts.len()];
    let mut suffix_uses = later_uses.clone();
    for (index, statement) in block.stmts.iter().enumerate().rev() {
        later_statement_uses[index] = suffix_uses.clone();
        collect_ident_uses_in_stmt(statement, &mut suffix_uses);
    }
    let mut declared = declared_before.clone();
    let mut awaits = Vec::new();
    let mut uses = BTreeSet::new();
    for (index, statement) in block.stmts.iter().enumerate() {
        awaits.extend(await_live_locals_in_stmt(
            statement,
            &later_statement_uses[index],
            &declared,
        ));
        collect_ident_uses_in_stmt(statement, &mut uses);
        collect_pat_bindings_in_stmt(statement, &mut declared);
    }
    (awaits, uses)
}

fn await_live_locals_in_if(
    expr_if: &syn::ExprIf,
    later_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    let (then_awaits, then_uses) =
        await_live_locals_in_block(&expr_if.then_branch, later_uses, declared_before);
    let (else_awaits, else_uses) = expr_if
        .else_branch
        .as_ref()
        .map(|(_, else_expr)| {
            await_live_locals_in_expr(else_expr.as_ref(), later_uses, declared_before)
        })
        .unwrap_or_default();
    let mut condition_later_uses = later_uses.clone();
    condition_later_uses.extend(then_uses.iter().cloned());
    condition_later_uses.extend(else_uses.iter().cloned());
    let (condition_awaits, condition_uses) = await_live_locals_in_expr(
        expr_if.cond.as_ref(),
        &condition_later_uses,
        declared_before,
    );
    let mut awaits = condition_awaits;
    awaits.extend(then_awaits);
    awaits.extend(else_awaits);
    let mut uses = condition_uses;
    uses.extend(then_uses);
    uses.extend(else_uses);
    (awaits, uses)
}

fn await_live_locals_in_match(
    expr_match: &syn::ExprMatch,
    later_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    let mut arm_awaits = Vec::new();
    let mut arm_uses = BTreeSet::new();
    for arm in &expr_match.arms {
        let mut arm_declared = declared_before.clone();
        collect_pat_bindings(&arm.pat, &mut arm_declared);
        let (body_awaits, body_uses) =
            await_live_locals_in_expr(arm.body.as_ref(), later_uses, &arm_declared);
        let mut guard_later_uses = later_uses.clone();
        guard_later_uses.extend(body_uses.iter().cloned());
        let (guard_awaits, guard_uses) = arm
            .guard
            .as_ref()
            .map(|(_, guard)| {
                await_live_locals_in_expr(guard.as_ref(), &guard_later_uses, &arm_declared)
            })
            .unwrap_or_default();
        arm_awaits.extend(guard_awaits);
        arm_awaits.extend(body_awaits);
        arm_uses.extend(guard_uses);
        arm_uses.extend(body_uses);
    }
    let mut scrutinee_later_uses = later_uses.clone();
    scrutinee_later_uses.extend(arm_uses.iter().cloned());
    let (mut awaits, scrutinee_uses) = await_live_locals_in_expr(
        expr_match.expr.as_ref(),
        &scrutinee_later_uses,
        declared_before,
    );
    awaits.extend(arm_awaits);
    let mut uses = scrutinee_uses;
    uses.extend(arm_uses);
    (awaits, uses)
}

fn await_live_locals_in_expr_sequence<'a>(
    children: impl Iterator<Item = &'a syn::Expr>,
    later_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    let children = children.collect::<Vec<_>>();
    let mut suffix = later_uses.clone();
    let mut awaits_reversed = Vec::<BTreeSet<String>>::new();
    let mut uses = BTreeSet::<String>::new();
    for child in children.into_iter().rev() {
        let (child_awaits, child_uses) = await_live_locals_in_expr(child, &suffix, declared_before);
        suffix.extend(child_uses.iter().cloned());
        uses.extend(child_uses);
        awaits_reversed.extend(child_awaits.into_iter().rev());
    }
    awaits_reversed.reverse();
    (awaits_reversed, uses)
}

fn ident_uses_in_expr(expr: &syn::Expr) -> BTreeSet<String> {
    let mut uses = BTreeSet::new();
    collect_ident_uses_in_expr(expr, &mut uses);
    uses
}

fn expr_await_count(expr: &syn::Expr) -> usize {
    struct AwaitVisitor {
        count: usize,
    }

    impl<'ast> syn::visit::Visit<'ast> for AwaitVisitor {
        fn visit_expr_await(&mut self, expr: &'ast syn::ExprAwait) {
            self.count += 1;
            syn::visit::visit_expr_await(self, expr);
        }
    }

    let mut visitor = AwaitVisitor { count: 0 };
    syn::visit::visit_expr(&mut visitor, expr);
    visitor.count
}

fn collect_pat_bindings_in_stmt(statement: &syn::Stmt, bindings: &mut BTreeSet<String>) {
    let syn::Stmt::Local(local) = statement else {
        return;
    };
    collect_pat_bindings(&local.pat, bindings);
}

fn collect_pat_bindings(pattern: &syn::Pat, bindings: &mut BTreeSet<String>) {
    match pattern {
        syn::Pat::Ident(ident) => {
            bindings.insert(ident.ident.to_string());
        }
        syn::Pat::Tuple(tuple) => {
            for element in &tuple.elems {
                collect_pat_bindings(element, bindings);
            }
        }
        syn::Pat::Struct(pattern) => {
            for field in &pattern.fields {
                collect_pat_bindings(&field.pat, bindings);
            }
        }
        syn::Pat::TupleStruct(pattern) => {
            for element in &pattern.elems {
                collect_pat_bindings(element, bindings);
            }
        }
        syn::Pat::Slice(pattern) => {
            for element in &pattern.elems {
                collect_pat_bindings(element, bindings);
            }
        }
        syn::Pat::Reference(pattern) => collect_pat_bindings(&pattern.pat, bindings),
        syn::Pat::Type(pattern) => collect_pat_bindings(&pattern.pat, bindings),
        syn::Pat::Or(pattern) => {
            for case in &pattern.cases {
                collect_pat_bindings(case, bindings);
            }
        }
        _ => {}
    }
}

fn collect_ident_uses_in_stmt(statement: &syn::Stmt, uses: &mut BTreeSet<String>) {
    struct UseVisitor<'a> {
        uses: &'a mut BTreeSet<String>,
    }

    impl<'a, 'ast> syn::visit::Visit<'ast> for UseVisitor<'a> {
        fn visit_expr_path(&mut self, expr: &'ast syn::ExprPath) {
            if expr.qself.is_none() && expr.path.segments.len() == 1 {
                if let Some(segment) = expr.path.segments.first() {
                    self.uses.insert(segment.ident.to_string());
                }
            }
            syn::visit::visit_expr_path(self, expr);
        }
    }

    let mut visitor = UseVisitor { uses };
    syn::visit::visit_stmt(&mut visitor, statement);
}

fn collect_ident_uses_in_expr(expr: &syn::Expr, uses: &mut BTreeSet<String>) {
    struct UseVisitor<'a> {
        uses: &'a mut BTreeSet<String>,
    }

    impl<'a, 'ast> syn::visit::Visit<'ast> for UseVisitor<'a> {
        fn visit_expr_path(&mut self, expr: &'ast syn::ExprPath) {
            if expr.qself.is_none() && expr.path.segments.len() == 1 {
                if let Some(segment) = expr.path.segments.first() {
                    self.uses.insert(segment.ident.to_string());
                }
            }
            syn::visit::visit_expr_path(self, expr);
        }
    }

    let mut visitor = UseVisitor { uses };
    syn::visit::visit_expr(&mut visitor, expr);
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
