use std::collections::{BTreeMap, BTreeSet};

use crate::{
    KoboSpan, ScenarioCoreCfgFacts, ScenarioCoreTerminatorKind, ScenarioOp, ScenarioOpKind,
    ScenarioProgram,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreProgram {
    pub source: &'static str,
    pub cfg_source: &'static str,
    pub core_version: &'static str,
    pub functions: Vec<CoreFunction>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreFunction {
    pub name: String,
    pub source_span: KoboSpan,
    pub blocks: Vec<CoreBlock>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreBlock {
    pub id: String,
    pub kir_cfg_block: Option<u32>,
    pub successor_source: &'static str,
    pub statements: Vec<CoreStatement>,
    pub terminators: Vec<CoreTerminator>,
    pub successors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreStatement {
    pub id: String,
    pub kind: CoreStatementKind,
    pub binding: Option<String>,
    pub action: Option<String>,
    pub boundary: Option<String>,
    pub source_span: KoboSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoreStatementKind {
    ObligationCreate,
    ObligationDischarge,
    ObligationTransfer,
    ObligationMove,
    ObligationBranchUnresolved,
    ObligationEscape,
    UnsupportedContainer,
    Call,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreTerminator {
    pub id: String,
    pub kind: CoreTerminatorKind,
    pub boundary: Option<String>,
    pub policy: Option<String>,
    pub edges: Vec<String>,
    pub source_span: KoboSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoreTerminatorKind {
    Goto,
    Branch,
    Return,
    ErrorExit,
    Panic,
    Await,
    OpaqueBoundary,
}

pub fn lower_program(program: &ScenarioProgram) -> CoreProgram {
    let mut blocks = Vec::new();
    let modeled_ops = program
        .operations
        .iter()
        .filter(|operation| !matches!(operation.kind, ScenarioOpKind::Return))
        .collect::<Vec<_>>();

    for (index, operation) in modeled_ops.iter().enumerate() {
        let next = (index + 1 < modeled_ops.len()).then(|| format!("bb{}", index + 1));
        let statements = statement_from_operation(index, operation)
            .into_iter()
            .collect();
        let mut terminators = terminators_from_operation(index, operation, next.as_deref());
        let successors = if terminators.is_empty() {
            next.iter().cloned().collect::<Vec<_>>()
        } else {
            terminators
                .iter()
                .flat_map(|terminator| terminator.edges.iter())
                .filter_map(|edge| edge.strip_prefix("goto:").map(str::to_owned))
                .collect::<Vec<_>>()
        };
        if terminators.is_empty() {
            if let Some(next) = next {
                terminators.push(CoreTerminator {
                    id: format!("term-{index}-goto"),
                    kind: CoreTerminatorKind::Goto,
                    boundary: None,
                    policy: None,
                    edges: vec![format!("goto:{next}")],
                    source_span: operation.span,
                });
            }
        }
        blocks.push(CoreBlock {
            id: format!("bb{index}"),
            kir_cfg_block: None,
            successor_source: "scenario_linear",
            statements,
            terminators,
            successors,
        });
    }

    if blocks.is_empty() {
        blocks.push(CoreBlock {
            id: "bb0".to_owned(),
            kir_cfg_block: None,
            successor_source: "scenario_linear",
            statements: Vec::new(),
            terminators: Vec::new(),
            successors: Vec::new(),
        });
    }

    let cfg_source = if let Some(core_cfg) = program.coverage.core_cfg.as_ref() {
        apply_kir_cfg_successors(&mut blocks, &modeled_ops, core_cfg);
        "kir_cfg"
    } else {
        "scenario_linear"
    };

    CoreProgram {
        source: "compiler_core_ir",
        cfg_source,
        core_version: "core-1",
        functions: vec![CoreFunction {
            name: program.target.clone(),
            source_span: function_span(program),
            blocks,
        }],
    }
}

fn apply_kir_cfg_successors(
    blocks: &mut [CoreBlock],
    modeled_ops: &[&ScenarioOp],
    core_cfg: &ScenarioCoreCfgFacts,
) {
    let op_to_cfg_block = modeled_ops
        .iter()
        .map(|operation| cfg_block_for_span(operation.span, core_cfg))
        .collect::<Vec<_>>();

    let mut cfg_to_core_blocks: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (index, cfg_block) in op_to_cfg_block.iter().enumerate() {
        if let Some(cfg_block) = cfg_block {
            cfg_to_core_blocks
                .entry(*cfg_block)
                .or_default()
                .push(index);
            if let Some(block) = blocks.get_mut(index) {
                block.kir_cfg_block = Some(*cfg_block);
                block.successor_source = "kir_cfg";
            }
        }
    }

    let cfg_edges = core_cfg
        .edges
        .iter()
        .map(|edge| (edge.from, edge.to))
        .collect::<BTreeSet<_>>();

    for (index, block) in blocks.iter_mut().enumerate() {
        let Some(cfg_block) = op_to_cfg_block.get(index).and_then(|block| *block) else {
            continue;
        };
        let mut successors = Vec::new();
        for (from, to) in &cfg_edges {
            if *from != cfg_block {
                continue;
            }
            if let Some(core_indexes) = cfg_to_core_blocks.get(to) {
                for core_index in core_indexes {
                    let successor = format!("bb{core_index}");
                    if successor != block.id && !successors.contains(&successor) {
                        successors.push(successor);
                    }
                }
            }
        }
        if !successors.is_empty() {
            block.successors = successors;
        }
    }
}

fn cfg_block_for_span(span: KoboSpan, core_cfg: &ScenarioCoreCfgFacts) -> Option<u32> {
    core_cfg
        .blocks
        .iter()
        .filter(|block| {
            ranges_overlap(
                span.start as usize,
                span.end as usize,
                block.span_start,
                block.span_end,
            )
        })
        .min_by_key(|block| block.span_end.saturating_sub(block.span_start))
        .map(|block| block.id)
}

fn ranges_overlap(
    left_start: usize,
    left_end: usize,
    right_start: usize,
    right_end: usize,
) -> bool {
    let left_end = left_end.max(left_start.saturating_add(1));
    let right_end = right_end.max(right_start.saturating_add(1));
    left_start < right_end && right_start < left_end
}

fn statement_from_operation(index: usize, operation: &ScenarioOp) -> Option<CoreStatement> {
    let source_span = operation.span;
    match &operation.kind {
        ScenarioOpKind::CreateObligation { binding, .. } => Some(CoreStatement {
            id: format!("stmt-{index}"),
            kind: CoreStatementKind::ObligationCreate,
            binding: Some(binding.clone()),
            action: None,
            boundary: None,
            source_span,
        }),
        ScenarioOpKind::Discharge { binding, action } => Some(CoreStatement {
            id: format!("stmt-{index}"),
            kind: CoreStatementKind::ObligationDischarge,
            binding: Some(binding.clone()),
            action: Some(action.clone()),
            boundary: None,
            source_span,
        }),
        ScenarioOpKind::Transfer { binding, callee } => Some(CoreStatement {
            id: format!("stmt-{index}"),
            kind: CoreStatementKind::ObligationTransfer,
            binding: Some(binding.clone()),
            action: Some(callee.clone()),
            boundary: None,
            source_span,
        }),
        ScenarioOpKind::MoveBinding { binding } => Some(CoreStatement {
            id: format!("stmt-{index}"),
            kind: CoreStatementKind::ObligationMove,
            binding: Some(binding.clone()),
            action: None,
            boundary: None,
            source_span,
        }),
        ScenarioOpKind::BranchUnresolved { binding } => Some(CoreStatement {
            id: format!("stmt-{index}"),
            kind: CoreStatementKind::ObligationBranchUnresolved,
            binding: Some(binding.clone()),
            action: None,
            boundary: None,
            source_span,
        }),
        ScenarioOpKind::UnsupportedContainer {
            binding, container, ..
        } => Some(CoreStatement {
            id: format!("stmt-{index}"),
            kind: CoreStatementKind::UnsupportedContainer,
            binding: Some(binding.clone()),
            action: Some(container.clone()),
            boundary: None,
            source_span,
        }),
        ScenarioOpKind::ExternalBoundary {
            crate_name, policy, ..
        } => Some(CoreStatement {
            id: format!("stmt-{index}"),
            kind: CoreStatementKind::ObligationEscape,
            binding: None,
            action: Some(policy.as_str().to_owned()),
            boundary: Some(crate_name.clone()),
            source_span,
        }),
        ScenarioOpKind::ModeledEffect { .. }
        | ScenarioOpKind::StorageEvent { .. }
        | ScenarioOpKind::NetworkEvent { .. } => Some(CoreStatement {
            id: format!("stmt-{index}"),
            kind: CoreStatementKind::Call,
            binding: None,
            action: None,
            boundary: None,
            source_span,
        }),
        ScenarioOpKind::CoreTerminator { .. }
        | ScenarioOpKind::Select { .. }
        | ScenarioOpKind::RawNondeterminism { .. }
        | ScenarioOpKind::UncontrolledEffect { .. }
        | ScenarioOpKind::Loop
        | ScenarioOpKind::Return => None,
    }
}

fn terminators_from_operation(
    index: usize,
    operation: &ScenarioOp,
    next: Option<&str>,
) -> Vec<CoreTerminator> {
    match &operation.kind {
        ScenarioOpKind::CoreTerminator {
            kind,
            boundary,
            policy,
            edges,
        } => {
            let mut edges = edges.clone();
            if matches!(
                kind,
                ScenarioCoreTerminatorKind::Await | ScenarioCoreTerminatorKind::OpaqueBoundary
            ) {
                if let Some(next) = next {
                    edges.push(format!("goto:{next}"));
                }
            }
            vec![CoreTerminator {
                id: format!("term-{index}"),
                kind: core_terminator_kind(kind),
                boundary: boundary.clone(),
                policy: policy.as_ref().map(|policy| policy.as_str().to_owned()),
                edges,
                source_span: operation.span,
            }]
        }
        ScenarioOpKind::Select { branch_count } => {
            let edges = next
                .map(|next| {
                    let count = (*branch_count).max(1);
                    (0..count)
                        .map(|_| format!("goto:{next}"))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            vec![CoreTerminator {
                id: format!("term-{index}"),
                kind: CoreTerminatorKind::Branch,
                boundary: None,
                policy: None,
                edges,
                source_span: operation.span,
            }]
        }
        ScenarioOpKind::Loop => vec![CoreTerminator {
            id: format!("term-{index}"),
            kind: CoreTerminatorKind::Goto,
            boundary: None,
            policy: None,
            edges: vec![format!("goto:bb{index}")],
            source_span: operation.span,
        }],
        _ => Vec::new(),
    }
}

fn core_terminator_kind(kind: &ScenarioCoreTerminatorKind) -> CoreTerminatorKind {
    match kind {
        ScenarioCoreTerminatorKind::Return => CoreTerminatorKind::Return,
        ScenarioCoreTerminatorKind::ErrorExit => CoreTerminatorKind::ErrorExit,
        ScenarioCoreTerminatorKind::Panic => CoreTerminatorKind::Panic,
        ScenarioCoreTerminatorKind::Await => CoreTerminatorKind::Await,
        ScenarioCoreTerminatorKind::OpaqueBoundary => CoreTerminatorKind::OpaqueBoundary,
    }
}

fn function_span(program: &ScenarioProgram) -> KoboSpan {
    let mut spans = program
        .operations
        .iter()
        .filter(|operation| !matches!(operation.kind, ScenarioOpKind::Return))
        .map(|operation| operation.span);
    let Some(first) = spans.next() else {
        return KoboSpan::generated(program.file_id);
    };
    spans.fold(first, |acc, span| KoboSpan {
        file_id: acc.file_id,
        start: acc.start.min(span.start),
        end: acc.end.max(span.end),
    })
}

impl CoreStatementKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::ObligationCreate => "obligation_create",
            Self::ObligationDischarge => "obligation_discharge",
            Self::ObligationTransfer => "obligation_transfer",
            Self::ObligationMove => "obligation_move",
            Self::ObligationBranchUnresolved => "obligation_branch_unresolved",
            Self::ObligationEscape => "obligation_escape",
            Self::UnsupportedContainer => "unsupported_container",
            Self::Call => "call",
        }
    }
}

impl CoreTerminatorKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Goto => "goto",
            Self::Branch => "branch",
            Self::Return => "return",
            Self::ErrorExit => "error_exit",
            Self::Panic => "panic",
            Self::Await => "await",
            Self::OpaqueBoundary => "opaque_boundary",
        }
    }
}
