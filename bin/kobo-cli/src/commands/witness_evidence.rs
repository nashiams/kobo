use std::collections::{BTreeMap, BTreeSet};

use kobo_ir::{ScenarioModeledBoundary, ScenarioOpKind, ScenarioProgram};
use kobo_sim_core::{BoundaryPolicyChoice, FullDepthRun, ReplayGuarantee};

#[derive(Debug, Default)]
struct FunctionSummary {
    function: String,
    creates: Vec<String>,
    transfers_in: Vec<String>,
    transfers: Vec<String>,
    discharges: Vec<String>,
    leaks: Vec<String>,
    returns: Vec<String>,
    escapes: Vec<String>,
    suppressed: Vec<String>,
}

#[derive(Debug, Default)]
struct FunctionSummaryBuilder {
    summaries: BTreeMap<String, FunctionSummary>,
    binding_owner: BTreeMap<String, String>,
    discharged_bindings: BTreeSet<String>,
}

pub(super) fn operation_coverage_json(
    program: &ScenarioProgram,
    run: &FullDepthRun,
) -> serde_json::Value {
    let mut modeled = program
        .operations
        .iter()
        .map(operation_coverage_label)
        .collect::<Vec<_>>();
    modeled.sort();
    modeled.dedup();

    serde_json::json!({
        "source": "scenario_program",
        "operation_count": program.operations.len(),
        "modeled": modeled,
        "unsupported": program.coverage.unsupported_constructs,
        "boundary_owned": run.opaque_boundaries,
    })
}

pub(super) fn function_summaries_json(
    program: &ScenarioProgram,
    run: &FullDepthRun,
) -> serde_json::Value {
    let summaries = FunctionSummaryBuilder::from_program(program, run).finish();
    serde_json::Value::Array(
        summaries
            .into_iter()
            .map(function_summary_json)
            .collect::<Vec<_>>(),
    )
}

pub(super) fn call_graph_obligation_summaries_json(
    program: &ScenarioProgram,
    run: &FullDepthRun,
) -> serde_json::Value {
    let summaries = FunctionSummaryBuilder::from_program(program, run).finish();
    let summary_values = summaries
        .into_iter()
        .map(function_summary_json)
        .collect::<Vec<_>>();
    let sccs = program
        .coverage
        .call_graph_sccs
        .iter()
        .map(|scc| {
            serde_json::json!({
                "functions": scc.functions.clone(),
                "is_recursive": scc.is_recursive,
            })
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "source": "scenario_program.call_graph_sccs",
        "target": program.target,
        "sccs": sccs,
        "summaries": summary_values,
    })
}

pub(super) fn boundary_ledger_json(
    program: &ScenarioProgram,
    run: &FullDepthRun,
) -> serde_json::Value {
    let mut entries = run
        .boundary_decisions
        .iter()
        .map(|decision| {
            serde_json::json!({
                "boundary": decision.crate_name,
                "status": boundary_status(&decision.policy),
                "policy": decision.policy.as_str(),
                "reason": decision.reason,
                "source": "scenario_program",
            })
        })
        .collect::<Vec<_>>();

    for boundary in &run.modeled_boundaries {
        entries.push(serde_json::json!({
            "boundary": boundary.as_str(),
            "status": "modeled",
            "policy": "model",
            "reason": "modeled v0.10 facade",
            "source": "scenario_program",
        }));
    }

    if entries.is_empty() && !program.boundaries.is_empty() {
        entries.extend(program.boundaries.iter().map(|boundary| {
            serde_json::json!({
                "boundary": boundary.name.clone(),
                "status": "debt",
                "policy": boundary.decision.clone(),
                "reason": null,
                "source": "scenario_program",
            })
        }));
    }

    entries.sort_by(|left, right| {
        let left_key = format!(
            "{}:{}",
            left["boundary"].as_str().unwrap_or_default(),
            left["policy"].as_str().unwrap_or_default()
        );
        let right_key = format!(
            "{}:{}",
            right["boundary"].as_str().unwrap_or_default(),
            right["policy"].as_str().unwrap_or_default()
        );
        left_key.cmp(&right_key)
    });
    serde_json::Value::Array(entries)
}

pub(super) fn inferred_obligations_json(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
) -> serde_json::Value {
    serde_json::Value::Array(
        run.obligations
            .iter()
            .map(|obligation| {
                let template = lifecycle_template(&obligation.actions);
                let state = if obligation.is_discharged {
                    "discharged"
                } else {
                    "leaked"
                };
                let coverage_loss = if obligation.is_discharged {
                    serde_json::Value::Null
                } else {
                    serde_json::Value::String("unresolved_terminal_action".to_owned())
                };

                serde_json::json!({
                    "id": format!("{}:{}", template.id, obligation.binding),
                    "kind": template.kind,
                    "template_id": template.id,
                    "template_version": "v0.10.1",
                    "binding": obligation.binding,
                    "state": state,
                    "terminal_actions": obligation.actions.clone(),
                    "source_span": span_json(source_path, source, obligation.declaration_span),
                    "confidence": template.confidence,
                    "coverage_loss": coverage_loss,
                })
            })
            .collect::<Vec<_>>(),
    )
}

pub(super) fn replay_grade_json(run: &FullDepthRun, fuzz_enabled: bool) -> &'static str {
    if fuzz_enabled && run.failure.is_some() {
        return "counterexample";
    }
    if run.failure.is_some() {
        return "diagnostic_failure";
    }
    if fuzz_enabled {
        return "probing_pass";
    }
    match run.replay_guarantee {
        ReplayGuarantee::Exact => "exact",
        ReplayGuarantee::Partial => "partial",
        ReplayGuarantee::NotReplayable => "not_replayable",
    }
}

impl FunctionSummaryBuilder {
    fn from_program(program: &ScenarioProgram, run: &FullDepthRun) -> Self {
        let mut builder = Self::default();
        builder.ensure_summary(&program.target);

        for operation in &program.operations {
            match &operation.kind {
                ScenarioOpKind::CreateObligation { binding, .. } => {
                    builder
                        .binding_owner
                        .insert(binding.clone(), program.target.clone());
                    builder
                        .summary_mut(&program.target)
                        .creates
                        .push(binding.clone());
                }
                ScenarioOpKind::Transfer { binding, callee } => {
                    let owner = builder
                        .binding_owner
                        .get(binding)
                        .cloned()
                        .unwrap_or_else(|| program.target.clone());
                    builder
                        .summary_mut(&owner)
                        .transfers
                        .push(format!("{binding}->{callee}"));
                    builder
                        .summary_mut(callee)
                        .transfers_in
                        .push(binding.clone());
                    builder
                        .binding_owner
                        .insert(binding.clone(), callee.clone());
                }
                ScenarioOpKind::Discharge { binding, .. } => {
                    let owner = builder
                        .binding_owner
                        .get(binding)
                        .cloned()
                        .unwrap_or_else(|| program.target.clone());
                    builder.summary_mut(&owner).discharges.push(binding.clone());
                    builder.discharged_bindings.insert(binding.clone());
                }
                ScenarioOpKind::ExternalBoundary {
                    crate_name, policy, ..
                } => {
                    if matches!(
                        policy,
                        kobo_ir::ScenarioBoundaryPolicy::Model
                            | kobo_ir::ScenarioBoundaryPolicy::Record
                            | kobo_ir::ScenarioBoundaryPolicy::Stub
                    ) {
                        continue;
                    }
                    builder
                        .summary_mut(&program.target)
                        .escapes
                        .push(crate_name.clone());
                }
                ScenarioOpKind::Return => {
                    builder
                        .summary_mut(&program.target)
                        .returns
                        .push("return".to_owned());
                }
                ScenarioOpKind::RawNondeterminism { operation }
                | ScenarioOpKind::UncontrolledEffect { operation } => {
                    builder
                        .summary_mut(&program.target)
                        .escapes
                        .push(operation.clone());
                }
                ScenarioOpKind::MoveBinding { .. }
                | ScenarioOpKind::ModeledEffect { .. }
                | ScenarioOpKind::StorageEvent { .. }
                | ScenarioOpKind::NetworkEvent { .. }
                | ScenarioOpKind::Select { .. }
                | ScenarioOpKind::Loop => {}
            }
        }

        for obligation in &run.obligations {
            if obligation.is_discharged || builder.discharged_bindings.contains(&obligation.binding)
            {
                continue;
            }
            let owner = builder
                .binding_owner
                .get(&obligation.binding)
                .cloned()
                .unwrap_or_else(|| program.target.clone());
            builder
                .summary_mut(&owner)
                .leaks
                .push(obligation.binding.clone());
        }

        builder
    }

    fn ensure_summary(&mut self, function: &str) {
        self.summaries
            .entry(function.to_owned())
            .or_insert_with(|| FunctionSummary {
                function: function.to_owned(),
                ..FunctionSummary::default()
            });
    }

    fn summary_mut(&mut self, function: &str) -> &mut FunctionSummary {
        self.ensure_summary(function);
        self.summaries
            .get_mut(function)
            .expect("summary should exist after ensure_summary")
    }

    fn finish(mut self) -> Vec<FunctionSummary> {
        for summary in self.summaries.values_mut() {
            sort_unique(&mut summary.creates);
            sort_unique(&mut summary.transfers_in);
            sort_unique(&mut summary.transfers);
            sort_unique(&mut summary.discharges);
            sort_unique(&mut summary.leaks);
            sort_unique(&mut summary.returns);
            sort_unique(&mut summary.escapes);
            sort_unique(&mut summary.suppressed);
        }
        self.summaries.into_values().collect()
    }
}

fn operation_coverage_label(operation: &kobo_ir::ScenarioOp) -> String {
    match &operation.kind {
        ScenarioOpKind::CreateObligation { .. } => "obligation-create".to_owned(),
        ScenarioOpKind::Discharge { .. } => "obligation-discharge".to_owned(),
        ScenarioOpKind::Transfer { .. } => "obligation-transfer".to_owned(),
        ScenarioOpKind::MoveBinding { .. } => "obligation-move".to_owned(),
        ScenarioOpKind::ModeledEffect { boundary } => {
            format!("modeled.{}", modeled_boundary_label(boundary))
        }
        ScenarioOpKind::StorageEvent { action } => format!("storage.{action}"),
        ScenarioOpKind::NetworkEvent { action } => format!("network.{action}"),
        ScenarioOpKind::Select { .. } => "select".to_owned(),
        ScenarioOpKind::RawNondeterminism { operation } => {
            format!("raw-nondeterminism.{operation}")
        }
        ScenarioOpKind::UncontrolledEffect { operation } => {
            format!("uncontrolled-effect.{operation}")
        }
        ScenarioOpKind::ExternalBoundary {
            crate_name, policy, ..
        } => {
            format!("external-boundary.{}.{}", policy.as_str(), crate_name)
        }
        ScenarioOpKind::Loop => "loop".to_owned(),
        ScenarioOpKind::Return => "return".to_owned(),
    }
}

fn modeled_boundary_label(boundary: &ScenarioModeledBoundary) -> &'static str {
    match boundary {
        ScenarioModeledBoundary::WardTime => "ward.time",
        ScenarioModeledBoundary::WardRandom => "ward.random",
        ScenarioModeledBoundary::WardTask => "ward.task",
    }
}

struct LifecycleTemplate {
    kind: &'static str,
    id: &'static str,
    confidence: &'static str,
}

fn lifecycle_template(actions: &[String]) -> LifecycleTemplate {
    if actions
        .iter()
        .any(|action| matches!(action.as_str(), "ack" | "nack" | "requeue"))
    {
        return LifecycleTemplate {
            kind: "queue_delivery",
            id: "queue_delivery",
            confidence: "exact_template",
        };
    }
    if actions
        .iter()
        .any(|action| matches!(action.as_str(), "commit" | "rollback"))
    {
        return LifecycleTemplate {
            kind: "transaction",
            id: "transaction",
            confidence: "exact_template",
        };
    }
    if actions
        .iter()
        .any(|action| matches!(action.as_str(), "reply" | "reject" | "cancel"))
    {
        return LifecycleTemplate {
            kind: "handler_reply",
            id: "handler_reply",
            confidence: "exact_template",
        };
    }
    if actions
        .iter()
        .any(|action| matches!(action.as_str(), "join" | "abort" | "detach-with-policy"))
    {
        return LifecycleTemplate {
            kind: "spawned_task",
            id: "spawned_task",
            confidence: "exact_template",
        };
    }
    LifecycleTemplate {
        kind: "lifecycle_obligation",
        id: "custom_lifecycle_obligation",
        confidence: "heuristic",
    }
}

fn boundary_status(policy: &BoundaryPolicyChoice) -> &'static str {
    match policy {
        BoundaryPolicyChoice::Model | BoundaryPolicyChoice::Stub => "modeled",
        BoundaryPolicyChoice::Record => "recordable",
        BoundaryPolicyChoice::Outside => "outside",
        BoundaryPolicyChoice::Opaque => "opaque",
        BoundaryPolicyChoice::Debt | BoundaryPolicyChoice::Unselected => "debt",
    }
}

fn span_json(source_path: &str, source: &str, span: (usize, usize)) -> serde_json::Value {
    serde_json::json!({
        "path": source_path,
        "line": one_based_line_for_offset(source, span.0),
        "start": span.0,
        "end": span.1.max(span.0 + 1),
    })
}

fn one_based_line_for_offset(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn function_summary_json(summary: FunctionSummary) -> serde_json::Value {
    serde_json::json!({
        "function": summary.function,
        "creates": summary.creates,
        "transfers_in": summary.transfers_in,
        "transfers": summary.transfers,
        "discharges": summary.discharges,
        "leaks": summary.leaks,
        "returns": summary.returns,
        "escapes": summary.escapes,
        "suppressed": summary.suppressed,
    })
}

fn sort_unique(values: &mut Vec<String>) {
    values.sort();
    values.dedup();
}
