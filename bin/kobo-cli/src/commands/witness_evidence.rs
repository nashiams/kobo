use std::collections::{BTreeMap, BTreeSet};

use kobo_ir::{ScenarioModeledBoundary, ScenarioOpKind, ScenarioProgram};
use kobo_sim_core::FullDepthRun;

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
                ScenarioOpKind::ExternalBoundary { crate_name } => {
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
        ScenarioOpKind::ExternalBoundary { crate_name } => {
            format!("external-boundary.{crate_name}")
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
