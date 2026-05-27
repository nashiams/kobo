use super::*;

impl<'a> ScenarioLowerer<'a> {
    pub(super) fn push_loop_frame(&mut self, label: Option<String>) -> LoopFrame {
        let index = self.next_loop_id;
        self.next_loop_id += 1;
        let id = match label.as_deref() {
            Some(label) => format!("loop-{label}-{index}"),
            None => format!("loop-{index}"),
        };
        let frame = LoopFrame { id, label };
        self.loop_stack.push(frame.clone());
        frame
    }

    pub(super) fn pop_loop_frame(&mut self, loop_id: &str) {
        if self
            .loop_stack
            .last()
            .is_some_and(|frame| frame.id == loop_id)
        {
            self.loop_stack.pop();
        }
    }

    pub(super) fn resolve_loop_frame(&self, label: Option<&syn::Lifetime>) -> Option<LoopFrame> {
        match label.map(lifetime_label_name) {
            Some(label) => self
                .loop_stack
                .iter()
                .rev()
                .find(|frame| frame.label.as_deref() == Some(label.as_str()))
                .cloned(),
            None => self.loop_stack.last().cloned(),
        }
    }

    pub(super) fn finish_loop_flow(
        &mut self,
        flow: BlockFlow,
        frame: &LoopFrame,
        can_exit: bool,
        node: &impl Spanned,
    ) -> BlockFlow {
        match flow {
            BlockFlow::Fallthrough => {
                self.operations.push(ScenarioOp {
                    span: self.span(node),
                    kind: ScenarioOpKind::LoopBackEdge {
                        loop_id: frame.id.clone(),
                        can_exit,
                    },
                });
                BlockFlow::Fallthrough
            }
            BlockFlow::Continue { loop_id } if loop_id == frame.id => BlockFlow::Fallthrough,
            BlockFlow::Break { loop_id } if loop_id == frame.id => BlockFlow::Fallthrough,
            BlockFlow::Mixed { flows } => {
                let remaining = flows
                    .into_iter()
                    .filter(|flow| !loop_consumes_flow(flow, frame))
                    .collect::<Vec<_>>();
                combined_non_fallthrough_flow(remaining)
            }
            flow => flow,
        }
    }

    pub(super) fn execute_if(&mut self, expr_if: &'a ExprIf, env: &mut BindingEnv) -> BlockFlow {
        let condition_value = self.eval_bool(expr_if.cond.as_ref(), env);
        self.execute_expr(expr_if.cond.as_ref(), env);
        match condition_value {
            Some(true) => self.execute_block(&expr_if.then_branch, env),
            Some(false) => {
                if let Some((_, else_expr)) = expr_if.else_branch.as_ref() {
                    return self.execute_expr(else_expr.as_ref(), env);
                }
                BlockFlow::Fallthrough
            }
            None => {
                self.operations.push(ScenarioOp {
                    span: self.span(expr_if),
                    kind: ScenarioOpKind::Select { branch_count: 2 },
                });
                let mut then_env = env.clone();
                let mut else_env = env.clone();
                let then_flow = self.execute_block(&expr_if.then_branch, &mut then_env);
                let else_flow = if let Some((_, else_expr)) = expr_if.else_branch.as_ref() {
                    self.execute_expr(else_expr.as_ref(), &mut else_env)
                } else {
                    BlockFlow::Fallthrough
                };
                self.record_control_flow_unresolved(expr_if, &then_env, &then_flow, "then");
                self.record_control_flow_unresolved(expr_if, &else_env, &else_flow, "else");
                self.record_branch_unresolved(expr_if, env, &then_env, &else_env);
                let mut fallthrough_envs = Vec::new();
                if then_flow == BlockFlow::Fallthrough {
                    fallthrough_envs.push(then_env);
                }
                if else_flow == BlockFlow::Fallthrough {
                    fallthrough_envs.push(else_env);
                }
                if !fallthrough_envs.is_empty() {
                    merge_branch_envs(env, fallthrough_envs);
                    return BlockFlow::Fallthrough;
                }
                common_branch_flow(then_flow, else_flow)
            }
        }
    }

    pub(super) fn record_branch_unresolved(
        &mut self,
        expr_if: &'a ExprIf,
        before: &BindingEnv,
        then_env: &BindingEnv,
        else_env: &BindingEnv,
    ) {
        for binding in before.active_obligations() {
            let then_active = then_env.has_active_obligation(&binding);
            let else_active = else_env.has_active_obligation(&binding);
            if then_active != else_active {
                self.operations.push(ScenarioOp {
                    span: self.span(expr_if),
                    kind: ScenarioOpKind::BranchUnresolved { binding },
                });
            }
        }
    }

    pub(super) fn record_control_flow_unresolved(
        &mut self,
        node: &impl Spanned,
        branch_env: &BindingEnv,
        flow: &BlockFlow,
        _edge: &str,
    ) {
        if *flow == BlockFlow::Fallthrough {
            return;
        }
        for binding in branch_env.active_obligations() {
            self.operations.push(ScenarioOp {
                span: self.span(node),
                kind: ScenarioOpKind::BranchUnresolved { binding },
            });
        }
    }

    pub(super) fn record_loop_control_unresolved(
        &mut self,
        node: &impl Spanned,
        env: &BindingEnv,
        _edge: &str,
    ) {
        for binding in env.active_obligations() {
            self.operations.push(ScenarioOp {
                span: self.span(node),
                kind: ScenarioOpKind::BranchUnresolved { binding },
            });
        }
    }

    pub(super) fn execute_match(
        &mut self,
        expr_match: &'a ExprMatch,
        env: &mut BindingEnv,
    ) -> BlockFlow {
        let discriminant = self.eval_bool(expr_match.expr.as_ref(), env);
        self.execute_expr(expr_match.expr.as_ref(), env);
        if discriminant.is_some() {
            for arm in &expr_match.arms {
                if matches_bool_pat(&arm.pat, discriminant) {
                    if let Some((_, guard)) = arm.guard.as_ref() {
                        self.execute_expr(guard.as_ref(), env);
                    }
                    return self.execute_expr(arm.body.as_ref(), env);
                }
            }
            return BlockFlow::Fallthrough;
        }

        self.operations.push(ScenarioOp {
            span: self.span(expr_match),
            kind: ScenarioOpKind::Select {
                branch_count: expr_match.arms.len() as u32,
            },
        });
        let before = env.clone();
        let mut arm_envs = Vec::new();
        let mut fallthrough_envs = Vec::new();
        let mut flows = Vec::new();
        for arm in &expr_match.arms {
            let mut arm_env = before.clone();
            if let Some((_, guard)) = arm.guard.as_ref() {
                self.execute_expr(guard.as_ref(), &mut arm_env);
            }
            let flow = self.execute_expr(arm.body.as_ref(), &mut arm_env);
            self.record_control_flow_unresolved(expr_match, &arm_env, &flow, "match arm");
            if flow == BlockFlow::Fallthrough {
                fallthrough_envs.push(arm_env.clone());
            }
            flows.push(flow);
            arm_envs.push(arm_env);
        }
        self.record_multi_branch_unresolved(expr_match, &before, &arm_envs);
        if !fallthrough_envs.is_empty() {
            merge_branch_envs(env, fallthrough_envs);
            return BlockFlow::Fallthrough;
        }
        common_multi_branch_flow(&flows)
    }

    pub(super) fn record_multi_branch_unresolved(
        &mut self,
        expr_match: &'a ExprMatch,
        before: &BindingEnv,
        branch_envs: &[BindingEnv],
    ) {
        for binding in before.active_obligations() {
            let mut active_states = branch_envs
                .iter()
                .map(|branch_env| branch_env.has_active_obligation(&binding));
            let Some(first) = active_states.next() else {
                continue;
            };
            if active_states.any(|active| active != first) {
                self.operations.push(ScenarioOp {
                    span: self.span(expr_match),
                    kind: ScenarioOpKind::BranchUnresolved { binding },
                });
            }
        }
    }
}

pub(super) fn common_branch_flow(left: BlockFlow, right: BlockFlow) -> BlockFlow {
    if left == right {
        left
    } else {
        combined_non_fallthrough_flow([left, right])
    }
}

pub(super) fn common_multi_branch_flow(flows: &[BlockFlow]) -> BlockFlow {
    let Some(first) = flows.first().cloned() else {
        return BlockFlow::Fallthrough;
    };
    if flows.iter().all(|flow| flow == &first) {
        first
    } else {
        combined_non_fallthrough_flow(flows.iter().cloned())
    }
}

pub(super) fn combined_non_fallthrough_flow(
    flows: impl IntoIterator<Item = BlockFlow>,
) -> BlockFlow {
    let mut unique_flows = Vec::new();
    for flow in flows {
        push_unique_non_fallthrough_flow(&mut unique_flows, flow);
    }
    match unique_flows.len() {
        0 => BlockFlow::Fallthrough,
        1 => unique_flows.pop().unwrap_or(BlockFlow::Fallthrough),
        _ => BlockFlow::Mixed {
            flows: unique_flows,
        },
    }
}

pub(super) fn push_unique_non_fallthrough_flow(unique_flows: &mut Vec<BlockFlow>, flow: BlockFlow) {
    match flow {
        BlockFlow::Fallthrough => {}
        BlockFlow::Mixed { flows } => {
            for flow in flows {
                push_unique_non_fallthrough_flow(unique_flows, flow);
            }
        }
        flow => {
            if !unique_flows.contains(&flow) {
                unique_flows.push(flow);
            }
        }
    }
}

pub(super) fn loop_consumes_flow(flow: &BlockFlow, frame: &LoopFrame) -> bool {
    match flow {
        BlockFlow::Continue { loop_id } | BlockFlow::Break { loop_id } => loop_id == &frame.id,
        _ => false,
    }
}

pub(super) fn loop_label(label: Option<&syn::Label>) -> Option<String> {
    label.map(loop_label_name)
}

pub(super) fn loop_label_name(label: &syn::Label) -> String {
    lifetime_label_name(&label.name)
}

pub(super) fn lifetime_label_name(label: &syn::Lifetime) -> String {
    label.ident.to_string()
}

pub(super) fn merge_branch_envs(env: &mut BindingEnv, branch_envs: Vec<BindingEnv>) {
    let mut merged_actions = HashMap::new();
    let bindings = branch_envs
        .iter()
        .flat_map(|branch_env| branch_env.terminal_actions.keys())
        .cloned()
        .collect::<HashSet<_>>();
    for binding in bindings {
        if let Some(actions) = branch_envs
            .iter()
            .find_map(|branch_env| branch_env.terminal_actions.get(&binding))
        {
            merged_actions.insert(binding, actions.clone());
        }
    }
    env.terminal_actions = merged_actions;
}
