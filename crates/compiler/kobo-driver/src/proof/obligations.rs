use super::*;

pub(super) fn obligation_evidence(
    source_path: &str,
    source: &str,
    functions: &[CoreFunction],
    loop_regions: &LoopRegionIndex,
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
                        loop_regions: loop_regions
                            .loop_regions_for_block(&function.name, &block.id),
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

pub(super) struct FunctionObligationReplay {
    pub(super) block_entry_envs: BTreeMap<String, BTreeMap<String, ObligationStatus>>,
    pub(super) block_exit_envs: BTreeMap<String, BTreeMap<String, ObligationStatus>>,
    pub(super) terminal_envs: Vec<BTreeMap<String, ObligationStatus>>,
}

pub(super) fn function_obligation_replay(function: &CoreFunction) -> FunctionObligationReplay {
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
            let discovered = !block_entry_envs.contains_key(&target_block.id);
            let changed = merge_block_entry_env(
                block_entry_envs
                    .entry(target_block.id.clone())
                    .or_insert_with(BTreeMap::new),
                &exit_env,
            );
            if discovered || changed {
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
        .filter_map(|block| {
            let targets = block_successor_targets(block);
            let is_terminal =
                targets.is_empty() || targets.iter().any(|target| modeled_exit_target(target));
            is_terminal
                .then(|| block_exit_envs.get(&block.id).cloned())
                .flatten()
        })
        .collect::<Vec<_>>();

    FunctionObligationReplay {
        block_entry_envs,
        block_exit_envs,
        terminal_envs,
    }
}

pub(super) fn is_back_edge_between_blocks(source: &str, target: &str) -> bool {
    let Some(source_index) = block_index(source) else {
        return false;
    };
    let Some(target_index) = block_index(target) else {
        return false;
    };
    target_index <= source_index
}

pub(super) fn apply_block_obligation_statements(
    block: &CoreBlock,
    mut env: BTreeMap<String, ObligationStatus>,
) -> BTreeMap<String, ObligationStatus> {
    for statement in &block.statements {
        apply_obligation_statement(statement, &mut env);
    }
    env
}

pub(super) fn block_successor_targets(block: &CoreBlock) -> Vec<String> {
    block
        .terminators
        .iter()
        .flat_map(|terminator| terminator.edges.iter())
        .map(|edge| core_successor_target(edge))
        .collect()
}

pub(super) fn core_successor_target(edge: &str) -> String {
    parse_core_edge(edge).target
}

pub(super) fn modeled_exit_target(target: &str) -> bool {
    matches!(target, "return" | "error_exit" | "panic" | "break_exit")
}

pub(super) fn merge_block_entry_env(
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

pub(super) fn merge_terminal_envs(
    terminal_envs: &[BTreeMap<String, ObligationStatus>],
) -> BTreeMap<String, ObligationStatus> {
    let mut merged = BTreeMap::new();
    for env in terminal_envs {
        merge_block_entry_env(&mut merged, env);
    }
    merged
}

pub(super) fn apply_obligation_statement(
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

pub(super) fn obligation_event_kind(kind: &CoreStatementKind) -> Option<ObligationEventKind> {
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

pub(super) fn env_states(env: &BTreeMap<String, ObligationStatus>) -> Vec<ObligationState> {
    env.iter()
        .map(|(binding, state)| ObligationState {
            binding: binding.clone(),
            state: state.clone(),
        })
        .collect()
}

pub(super) fn env_states_for_bindings(
    env: &BTreeMap<String, ObligationStatus>,
    bindings: &[String],
) -> Vec<ObligationState> {
    env.iter()
        .filter(|(binding, _)| bindings.contains(binding))
        .map(|(binding, state)| ObligationState {
            binding: binding.clone(),
            state: state.clone(),
        })
        .collect()
}

pub(super) fn function_summaries(
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

pub(super) fn coverage_loss(program: &ScenarioProgram) -> Vec<CoverageLoss> {
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
