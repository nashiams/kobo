use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ParsedLoopEdgeKind {
    BackEdge,
    Continue,
    Break,
    Condition,
}

#[derive(Clone, Debug)]
pub(super) struct ParsedCoreEdge {
    pub(super) target: String,
    pub(super) loop_id: Option<String>,
    pub(super) loop_edge_kind: Option<ParsedLoopEdgeKind>,
    pub(super) loop_entry_block: Option<String>,
}

#[derive(Default)]
pub(super) struct LoopRegionIndex {
    block_regions: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

impl ParsedLoopEdgeKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::BackEdge => "back_edge",
            Self::Continue => "continue",
            Self::Break => "break",
            Self::Condition => "condition",
        }
    }
}

pub(super) fn core_loop_facts(edges: &[CoreCfgEdge]) -> Vec<CoreLoopBackEdgeFact> {
    edges
        .iter()
        .filter(|edge| is_loop_back_edge(edge))
        .map(|edge| CoreLoopBackEdgeFact {
            id: format!("loop-{}-{}", edge.function, edge.id),
            function: edge.function.clone(),
            loop_id: edge
                .loop_id
                .clone()
                .unwrap_or_else(|| format!("legacy-{}-{}", edge.function, edge.to)),
            loop_label: edge.loop_label.clone(),
            entry_block: edge
                .loop_entry_block
                .clone()
                .unwrap_or_else(|| edge.to.clone()),
            back_edge_source: edge.from.clone(),
            back_edge_target: edge.to.clone(),
            source_span: edge.source_span.clone(),
        })
        .collect()
}

pub(super) fn core_loop_exit_facts(edges: &[CoreCfgEdge]) -> Vec<CoreLoopExitFact> {
    edges
        .iter()
        .filter_map(|edge| {
            let exit_kind = loop_exit_kind(edge)?;
            let entry_block = edge.loop_entry_block.clone()?;
            Some(CoreLoopExitFact {
                id: format!("loop-exit-{}-{}", edge.function, edge.id),
                function: edge.function.clone(),
                loop_id: edge
                    .loop_id
                    .clone()
                    .unwrap_or_else(|| format!("legacy-{}-{entry_block}", edge.function)),
                loop_label: edge.loop_label.clone(),
                entry_block,
                exit_source: edge.from.clone(),
                exit_kind: exit_kind.to_owned(),
                exit_target: edge.to.clone(),
                source_span: edge.source_span.clone(),
            })
        })
        .collect()
}

pub(super) fn is_loop_back_edge(edge: &CoreCfgEdge) -> bool {
    match edge.loop_edge_kind.as_deref() {
        Some("back_edge" | "continue") => true,
        Some(_) => false,
        None => is_legacy_back_edge(edge),
    }
}

pub(super) fn loop_exit_kind(edge: &CoreCfgEdge) -> Option<&'static str> {
    match edge.loop_edge_kind.as_deref() {
        Some("break") => Some("break"),
        Some("condition") => Some("condition"),
        _ => None,
    }
}

pub(super) fn is_legacy_back_edge(edge: &CoreCfgEdge) -> bool {
    let Some(source_index) = block_index(&edge.from) else {
        return false;
    };
    let Some(target_index) = block_index(&edge.to) else {
        return false;
    };
    target_index <= source_index
}

pub(super) fn block_index(block: &str) -> Option<usize> {
    block.strip_prefix("bb")?.parse().ok()
}

pub(super) fn core_cfg_nodes(
    source_path: &str,
    source: &str,
    functions: &[CoreFunction],
) -> Vec<CoreCfgNode> {
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

pub(super) fn core_cfg_edges(
    source_path: &str,
    source: &str,
    functions: &[CoreFunction],
    loop_labels: &BTreeMap<String, Option<String>>,
) -> Vec<CoreCfgEdge> {
    functions
        .iter()
        .flat_map(|function| {
            function.blocks.iter().flat_map(move |block| {
                block.terminators.iter().flat_map(move |terminator| {
                    terminator
                        .edges
                        .iter()
                        .enumerate()
                        .map(move |(edge_index, edge)| {
                            let parsed_edge = parse_core_edge(edge);
                            let loop_label = parsed_edge
                                .loop_id
                                .as_deref()
                                .and_then(|loop_id| loop_labels.get(loop_id))
                                .cloned()
                                .flatten();
                            CoreCfgEdge {
                                id: format!(
                                    "{}:{}:{}:{edge_index}",
                                    function.name, block.id, terminator.id
                                ),
                                function: function.name.clone(),
                                from: block.id.clone(),
                                to: parsed_edge.target,
                                kind: terminator.kind.as_str().to_owned(),
                                loop_id: parsed_edge.loop_id,
                                loop_label,
                                loop_edge_kind: parsed_edge
                                    .loop_edge_kind
                                    .map(|kind| kind.as_str().to_owned()),
                                loop_entry_block: parsed_edge.loop_entry_block,
                                source_span: source_span_from_kobo(
                                    source_path,
                                    source,
                                    terminator.source_span,
                                ),
                            }
                        })
                })
            })
        })
        .collect()
}

pub(super) fn loop_labels_by_id(program: &ScenarioProgram) -> BTreeMap<String, Option<String>> {
    program
        .operations
        .iter()
        .filter_map(|operation| match &operation.kind {
            ScenarioOpKind::LoopStart { loop_id, label } => Some((loop_id.clone(), label.clone())),
            _ => None,
        })
        .collect()
}

pub(super) fn parse_core_edge(edge: &str) -> ParsedCoreEdge {
    if let Some(target) = edge.strip_prefix("goto:") {
        return ParsedCoreEdge::plain(target);
    }
    if let Some(payload) = edge.strip_prefix("loop_back:") {
        return parse_loop_target_edge(payload, ParsedLoopEdgeKind::BackEdge, edge);
    }
    if let Some(payload) = edge.strip_prefix("loop_continue:") {
        return parse_loop_target_edge(payload, ParsedLoopEdgeKind::Continue, edge);
    }
    if let Some(payload) = edge.strip_prefix("loop_exit:") {
        return parse_loop_exit_edge(payload, edge);
    }
    ParsedCoreEdge::plain(edge)
}

pub(super) fn parse_loop_target_edge(
    payload: &str,
    kind: ParsedLoopEdgeKind,
    fallback: &str,
) -> ParsedCoreEdge {
    let Some((loop_id, target)) = payload.rsplit_once(':') else {
        return ParsedCoreEdge::plain(fallback);
    };
    ParsedCoreEdge {
        target: target.to_owned(),
        loop_id: Some(loop_id.to_owned()),
        loop_edge_kind: Some(kind),
        loop_entry_block: Some(target.to_owned()),
    }
}

pub(super) fn parse_loop_exit_edge(payload: &str, fallback: &str) -> ParsedCoreEdge {
    let parts = payload.splitn(4, ':').collect::<Vec<_>>();
    if parts.len() == 4 {
        let loop_edge_kind = parsed_exit_kind(parts[0]);
        return ParsedCoreEdge {
            target: parts[3].to_owned(),
            loop_id: Some(parts[1].to_owned()),
            loop_edge_kind,
            loop_entry_block: Some(parts[2].to_owned()),
        };
    }
    parse_legacy_loop_exit_edge(payload).unwrap_or_else(|| ParsedCoreEdge::plain(fallback))
}

pub(super) fn parse_legacy_loop_exit_edge(payload: &str) -> Option<ParsedCoreEdge> {
    let mut parts = payload.splitn(3, ':');
    let kind = parts.next()?;
    let entry_block = parts.next()?.to_owned();
    let target = parts
        .next()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{kind}_exit"));
    Some(ParsedCoreEdge {
        target,
        loop_id: None,
        loop_edge_kind: parsed_exit_kind(kind),
        loop_entry_block: Some(entry_block),
    })
}

pub(super) fn parsed_exit_kind(kind: &str) -> Option<ParsedLoopEdgeKind> {
    match kind {
        "break" => Some(ParsedLoopEdgeKind::Break),
        "condition" => Some(ParsedLoopEdgeKind::Condition),
        _ => None,
    }
}

impl ParsedCoreEdge {
    fn plain(target: &str) -> Self {
        Self {
            target: target.to_owned(),
            loop_id: None,
            loop_edge_kind: None,
            loop_entry_block: None,
        }
    }
}

impl LoopRegionIndex {
    pub(super) fn from_core(
        functions: &[CoreFunction],
        edges: &[CoreCfgEdge],
        loop_facts: &[CoreLoopBackEdgeFact],
    ) -> Self {
        let mut index = Self::default();
        for function in functions {
            for fact in loop_facts
                .iter()
                .filter(|fact| fact.function == function.name.as_str())
            {
                let region_blocks = loop_region_blocks(function, edges, fact);
                for block_id in region_blocks {
                    push_unique_loop_region(
                        index
                            .block_regions
                            .entry(function.name.clone())
                            .or_default()
                            .entry(block_id)
                            .or_default(),
                        fact.loop_id.clone(),
                    );
                }
            }
        }
        index
    }

    pub(super) fn loop_regions_for_block(&self, function: &str, block: &str) -> Vec<String> {
        self.block_regions
            .get(function)
            .and_then(|regions| regions.get(block))
            .cloned()
            .unwrap_or_default()
    }
}

pub(super) fn loop_region_blocks(
    function: &CoreFunction,
    edges: &[CoreCfgEdge],
    fact: &CoreLoopBackEdgeFact,
) -> BTreeSet<String> {
    let known_blocks = function
        .blocks
        .iter()
        .map(|block| block.id.as_str())
        .collect::<BTreeSet<_>>();
    let function_edges = edges
        .iter()
        .filter(|edge| edge.function == function.name.as_str())
        .collect::<Vec<_>>();
    let mut visited = BTreeSet::new();
    let mut queued = VecDeque::from([fact.entry_block.clone()]);
    while let Some(block_id) = queued.pop_front() {
        if !known_blocks.contains(block_id.as_str()) || !visited.insert(block_id.clone()) {
            continue;
        }
        for edge in function_edges.iter().filter(|edge| edge.from == block_id) {
            if !edge.to.starts_with("bb") || !known_blocks.contains(edge.to.as_str()) {
                continue;
            }
            if edge.loop_id.as_deref() == Some(fact.loop_id.as_str())
                && edge.loop_edge_kind.is_some()
            {
                continue;
            }
            queued.push_back(edge.to.clone());
        }
    }
    visited
}

pub(super) fn push_unique_loop_region(regions: &mut Vec<String>, loop_id: String) {
    if !regions.contains(&loop_id) {
        regions.push(loop_id);
    }
}

pub(super) fn block_span(block: &CoreBlock) -> KoboSpan {
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
