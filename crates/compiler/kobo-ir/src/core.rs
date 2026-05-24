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
    let branch_arm_joins = branch_arm_join_targets(&modeled_ops);

    for (index, operation) in modeled_ops.iter().enumerate() {
        let next = match branch_arm_joins.get(&index) {
            Some(join) => join.clone(),
            None => (index + 1 < modeled_ops.len()).then(|| format!("bb{}", index + 1)),
        };
        let statements = statement_from_operation(index, operation)
            .into_iter()
            .collect();
        let mut terminators = terminators_from_operation(
            index,
            operation,
            next.as_deref(),
            modeled_ops.len(),
            loop_entry_index(&modeled_ops, index),
        );
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

fn branch_arm_join_targets(modeled_ops: &[&ScenarioOp]) -> BTreeMap<usize, Option<String>> {
    let mut joins = BTreeMap::new();
    for (index, operation) in modeled_ops.iter().enumerate() {
        let ScenarioOpKind::Select { branch_count } = operation.kind else {
            continue;
        };
        let branch_count = branch_count.max(1) as usize;
        let join = (index + branch_count + 1 < modeled_ops.len())
            .then(|| format!("bb{}", index + branch_count + 1));
        for branch_index in index + 1..=index + branch_count {
            if branch_index < modeled_ops.len() {
                joins.insert(branch_index, join.clone());
            }
        }
    }
    joins
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
        if modeled_ops
            .get(index)
            .is_some_and(|operation| uses_scenario_control_successors(&operation.kind))
        {
            continue;
        }
        let Some(cfg_block) = op_to_cfg_block.get(index).and_then(|block| *block) else {
            continue;
        };
        let mut successors = Vec::new();
        for (from, to) in &cfg_edges {
            if *from != cfg_block {
                continue;
            }
            if from == to {
                if let Some(successor) = same_cfg_successor(index, &cfg_to_core_blocks, *to) {
                    push_unique_successor(&mut successors, successor);
                }
                continue;
            }
            if let Some(core_indexes) = cfg_to_core_blocks.get(to) {
                for core_index in core_indexes {
                    push_unique_successor(&mut successors, format!("bb{core_index}"));
                }
            }
        }
        if !successors.is_empty() {
            block.successors = successors;
            sync_terminator_successor_edges(block);
        }
    }
}

fn uses_scenario_control_successors(kind: &ScenarioOpKind) -> bool {
    matches!(
        kind,
        ScenarioOpKind::LoopStart
            | ScenarioOpKind::Loop
            | ScenarioOpKind::LoopBackEdge
            | ScenarioOpKind::LoopContinue
            | ScenarioOpKind::LoopBreak
    )
}

fn same_cfg_successor(
    index: usize,
    cfg_to_core_blocks: &BTreeMap<u32, Vec<usize>>,
    cfg_block: u32,
) -> Option<String> {
    let core_indexes = cfg_to_core_blocks.get(&cfg_block)?;
    let position = core_indexes
        .iter()
        .position(|core_index| *core_index == index)?;
    let target = core_indexes
        .get(position + 1)
        .or_else(|| core_indexes.first())?;
    Some(format!("bb{target}"))
}

fn push_unique_successor(successors: &mut Vec<String>, successor: String) {
    if !successors.contains(&successor) {
        successors.push(successor);
    }
}

fn sync_terminator_successor_edges(block: &mut CoreBlock) {
    if block.successors.is_empty() {
        return;
    }
    let successor_edges = block
        .successors
        .iter()
        .map(|successor| format!("goto:{successor}"))
        .collect::<Vec<_>>();
    for terminator in &mut block.terminators {
        if matches!(
            terminator.kind,
            CoreTerminatorKind::Branch | CoreTerminatorKind::Goto
        ) {
            terminator.edges = successor_edges.clone();
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
        ScenarioOpKind::Transfer {
            binding,
            callee,
            proven,
        } => Some(CoreStatement {
            id: format!("stmt-{index}"),
            kind: CoreStatementKind::ObligationTransfer,
            binding: Some(binding.clone()),
            action: Some(if *proven {
                callee.clone()
            } else {
                format!("unproven:{callee}")
            }),
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
        | ScenarioOpKind::LoopStart
        | ScenarioOpKind::LoopBackEdge
        | ScenarioOpKind::LoopContinue
        | ScenarioOpKind::LoopBreak
        | ScenarioOpKind::Loop
        | ScenarioOpKind::Return => None,
    }
}

fn terminators_from_operation(
    index: usize,
    operation: &ScenarioOp,
    next: Option<&str>,
    block_count: usize,
    loop_entry_index: usize,
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
            let edges = select_branch_edges(index, *branch_count, block_count, next);
            vec![CoreTerminator {
                id: format!("term-{index}"),
                kind: CoreTerminatorKind::Branch,
                boundary: None,
                policy: None,
                edges,
                source_span: operation.span,
            }]
        }
        ScenarioOpKind::Loop => {
            let mut edges = vec![format!("goto:bb{loop_entry_index}")];
            if let Some(next) = next {
                edges.push(format!("goto:{next}"));
            }
            vec![CoreTerminator {
                id: format!("term-{index}"),
                kind: CoreTerminatorKind::Goto,
                boundary: None,
                policy: None,
                edges,
                source_span: operation.span,
            }]
        }
        ScenarioOpKind::LoopBackEdge | ScenarioOpKind::LoopContinue => vec![CoreTerminator {
            id: format!("term-{index}"),
            kind: CoreTerminatorKind::Goto,
            boundary: None,
            policy: None,
            edges: vec![format!("goto:bb{loop_entry_index}")],
            source_span: operation.span,
        }],
        ScenarioOpKind::LoopBreak => {
            let mut edges = vec![format!("goto:bb{loop_entry_index}")];
            edges.push(
                next.map(|next| format!("goto:{next}"))
                    .unwrap_or_else(|| "break_exit".to_owned()),
            );
            vec![CoreTerminator {
                id: format!("term-{index}-break"),
                kind: CoreTerminatorKind::Goto,
                boundary: None,
                policy: None,
                edges,
                source_span: operation.span,
            }]
        }
        _ => Vec::new(),
    }
}

fn loop_entry_index(modeled_ops: &[&ScenarioOp], loop_index: usize) -> usize {
    modeled_ops[..loop_index]
        .iter()
        .rposition(|operation| {
            matches!(
                operation.kind,
                ScenarioOpKind::LoopStart | ScenarioOpKind::Loop
            )
        })
        .map(|index| index + 1)
        .unwrap_or(0)
}

fn select_branch_edges(
    index: usize,
    branch_count: u32,
    block_count: usize,
    fallback_next: Option<&str>,
) -> Vec<String> {
    let branch_count = branch_count.max(1) as usize;
    let mut edges = (1..=branch_count)
        .filter_map(|offset| {
            let target = index + offset;
            (target < block_count).then(|| format!("goto:bb{target}"))
        })
        .collect::<Vec<_>>();
    if edges.is_empty() {
        if let Some(next) = fallback_next {
            edges.push(format!("goto:{next}"));
        }
    }
    edges
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

#[cfg(test)]
mod tests {
    use crate::{
        FileId, ScenarioCoreCfgBlock, ScenarioCoreCfgEdge, ScenarioCoreCfgFacts,
        ScenarioCoverageFacts,
    };

    use super::*;

    #[test]
    fn kir_self_loop_cfg_preserves_operation_order_instead_of_complete_graph() {
        let program = ScenarioProgram {
            file_id: FileId(0),
            target: "queue_loop_case".to_owned(),
            source_hash: "hash".to_owned(),
            operations: vec![
                ScenarioOp {
                    span: span(10, 20),
                    kind: ScenarioOpKind::CreateObligation {
                        binding: "delivery".to_owned(),
                        type_name: "Delivery".to_owned(),
                        actions: vec!["ack".to_owned()],
                        template: None,
                    },
                },
                ScenarioOp {
                    span: span(21, 30),
                    kind: ScenarioOpKind::Discharge {
                        binding: "delivery".to_owned(),
                        action: "ack".to_owned(),
                    },
                },
                ScenarioOp {
                    span: span(31, 40),
                    kind: ScenarioOpKind::Loop,
                },
            ],
            boundaries: Vec::new(),
            coverage: ScenarioCoverageFacts {
                core_cfg: Some(ScenarioCoreCfgFacts {
                    blocks: vec![ScenarioCoreCfgBlock {
                        id: 6,
                        kir_nodes: vec![20, 21, 22],
                        span_start: 10,
                        span_end: 40,
                    }],
                    edges: vec![ScenarioCoreCfgEdge { from: 6, to: 6 }],
                }),
                ..ScenarioCoverageFacts::default()
            },
        };

        let core = lower_program(&program);
        let blocks = &core.functions[0].blocks;
        assert_eq!(blocks[0].successors, vec!["bb1"]);
        assert_eq!(blocks[1].successors, vec!["bb2"]);
        assert_eq!(blocks[2].successors, vec!["bb0"]);
        assert_eq!(blocks[0].terminators[0].edges, vec!["goto:bb1"]);
        assert_eq!(blocks[1].terminators[0].edges, vec!["goto:bb2"]);
        assert_eq!(blocks[2].terminators[0].edges, vec!["goto:bb0"]);
    }

    fn span(start: u32, end: u32) -> KoboSpan {
        KoboSpan::new(start, end, FileId(0))
    }
}
