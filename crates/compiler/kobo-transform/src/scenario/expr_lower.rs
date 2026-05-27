use super::{
    expr_path_ident, function_returns_bool_literal, loop_label, path_ends_with,
    path_ends_with_segments, path_last_ident, peel_paren_expr, select_branch_count, BindingEnv,
    BlockFlow, Expr, ExprAsync, ExprLit, ExprTry, KoboSpan, Lit, Macro, ScenarioBoundaryPolicy,
    ScenarioCoreTerminatorKind, ScenarioLowerer, ScenarioModeledBoundary, ScenarioOp,
    ScenarioOpKind, Spanned,
};
impl<'a> ScenarioLowerer<'a> {
    pub(super) fn execute_expr(&mut self, expr: &'a Expr, env: &mut BindingEnv) -> BlockFlow {
        match expr {
            Expr::MethodCall(call) => {
                self.execute_method_call(call, env);
                BlockFlow::Fallthrough
            }
            Expr::Call(call) => {
                self.execute_call(call, env);
                BlockFlow::Fallthrough
            }
            Expr::If(expr_if) => self.execute_if(expr_if, env),
            Expr::Match(expr_match) => self.execute_match(expr_match, env),
            Expr::Async(expr_async) => {
                self.execute_async(expr_async, env);
                BlockFlow::Fallthrough
            }
            Expr::Await(await_expr) => self.execute_await_expr(await_expr, env),
            Expr::Try(expr_try) => {
                self.execute_try(expr_try, env);
                BlockFlow::Fallthrough
            }
            Expr::Block(block) => self.execute_block(&block.block, env),
            Expr::Break(expr_break) => self.execute_break_expr(expr_break, env),
            Expr::ForLoop(expr_for) => self.execute_for_loop_expr(expr_for, env),
            Expr::Loop(expr_loop) => self.execute_loop_expr(expr_loop, env),
            Expr::TryBlock(try_block) => self.execute_block(&try_block.block, env),
            Expr::While(expr_while) => self.execute_while_expr(expr_while, env),
            Expr::Macro(expr_macro) => self.execute_macro_expr(expr_macro),
            Expr::Return(expr_return) => self.execute_return_expr(expr_return, env),
            Expr::Continue(expr_continue) => self.execute_continue_expr(expr_continue, env),
            _ => self.execute_structural_expr(expr, env),
        }
    }

    fn execute_await_expr(
        &mut self,
        await_expr: &'a syn::ExprAwait,
        env: &mut BindingEnv,
    ) -> BlockFlow {
        let timeout_boundary = self.timeout_await_boundary(await_expr.base.as_ref(), env);
        self.record_core_terminator(
            ScenarioCoreTerminatorKind::Await,
            timeout_boundary,
            None,
            vec!["await_resume".to_owned(), "await_cancel".to_owned()],
            await_expr,
        );
        if let Some(binding) =
            expr_path_ident(await_expr.base.as_ref()).and_then(|name| env.resolve(&name))
        {
            self.operations.push(ScenarioOp {
                span: self.span(await_expr),
                kind: ScenarioOpKind::Discharge {
                    binding,
                    action: "await".to_owned(),
                },
            });
            return BlockFlow::Fallthrough;
        }
        self.execute_expr(await_expr.base.as_ref(), env);
        BlockFlow::Fallthrough
    }

    fn execute_break_expr(
        &mut self,
        expr_break: &'a syn::ExprBreak,
        env: &mut BindingEnv,
    ) -> BlockFlow {
        if let Some(value) = expr_break.expr.as_deref() {
            self.execute_expr(value, env);
        }
        self.record_loop_control_unresolved(expr_break, env, "break");
        let frame = self.resolve_loop_frame(expr_break.label.as_ref());
        let loop_id = frame
            .as_ref()
            .map(|frame| frame.id.clone())
            .unwrap_or_else(|| "loop-unknown".to_owned());
        self.operations.push(ScenarioOp {
            span: self.span(expr_break),
            kind: ScenarioOpKind::LoopBreak {
                loop_id: loop_id.clone(),
            },
        });
        BlockFlow::Break { loop_id }
    }

    fn execute_for_loop_expr(
        &mut self,
        expr_for: &'a syn::ExprForLoop,
        env: &mut BindingEnv,
    ) -> BlockFlow {
        self.execute_expr(expr_for.expr.as_ref(), env);
        let frame = self.push_loop_frame(loop_label(expr_for.label.as_ref()));
        self.record_loop_start(&frame, expr_for);
        let flow = self.execute_block(&expr_for.body, env);
        self.pop_loop_frame(&frame.id);
        self.finish_loop_flow(flow, &frame, true, expr_for)
    }

    fn execute_loop_expr(
        &mut self,
        expr_loop: &'a syn::ExprLoop,
        env: &mut BindingEnv,
    ) -> BlockFlow {
        let frame = self.push_loop_frame(loop_label(expr_loop.label.as_ref()));
        self.record_loop_start(&frame, expr_loop);
        let flow = self.execute_block(&expr_loop.body, env);
        self.pop_loop_frame(&frame.id);
        self.finish_loop_flow(flow, &frame, false, expr_loop)
    }

    fn execute_while_expr(
        &mut self,
        expr_while: &'a syn::ExprWhile,
        env: &mut BindingEnv,
    ) -> BlockFlow {
        self.execute_expr(expr_while.cond.as_ref(), env);
        let frame = self.push_loop_frame(loop_label(expr_while.label.as_ref()));
        self.record_loop_start(&frame, expr_while);
        let flow = self.execute_block(&expr_while.body, env);
        self.pop_loop_frame(&frame.id);
        self.finish_loop_flow(flow, &frame, true, expr_while)
    }

    fn execute_macro_expr(&mut self, expr_macro: &'a syn::ExprMacro) -> BlockFlow {
        if self.record_panic_macro(&expr_macro.mac) {
            return BlockFlow::Return;
        }
        if !self.record_spawn_macro(&expr_macro.mac) {
            self.unsupported_macro(&expr_macro.mac);
        }
        BlockFlow::Fallthrough
    }

    fn execute_return_expr(
        &mut self,
        expr_return: &'a syn::ExprReturn,
        env: &mut BindingEnv,
    ) -> BlockFlow {
        if let Some(binding) = expr_return
            .expr
            .as_deref()
            .and_then(expr_path_ident)
            .and_then(|name| env.resolve(&name))
        {
            self.operations.push(ScenarioOp {
                span: self.span(expr_return),
                kind: ScenarioOpKind::Discharge {
                    binding,
                    action: "return".to_owned(),
                },
            });
        }
        self.record_core_terminator(
            ScenarioCoreTerminatorKind::Return,
            None,
            None,
            vec!["return".to_owned()],
            expr_return,
        );
        BlockFlow::Return
    }

    fn execute_continue_expr(
        &mut self,
        expr_continue: &'a syn::ExprContinue,
        env: &mut BindingEnv,
    ) -> BlockFlow {
        self.record_loop_control_unresolved(expr_continue, env, "continue");
        let frame = self.resolve_loop_frame(expr_continue.label.as_ref());
        let loop_id = frame
            .as_ref()
            .map(|frame| frame.id.clone())
            .unwrap_or_else(|| "loop-unknown".to_owned());
        self.operations.push(ScenarioOp {
            span: self.span(expr_continue),
            kind: ScenarioOpKind::LoopContinue {
                loop_id: loop_id.clone(),
            },
        });
        BlockFlow::Continue { loop_id }
    }

    fn record_loop_start(&mut self, frame: &super::LoopFrame, node: &impl Spanned) {
        self.operations.push(ScenarioOp {
            span: self.span(node),
            kind: ScenarioOpKind::LoopStart {
                loop_id: frame.id.clone(),
                label: frame.label.clone(),
            },
        });
    }

    pub(super) fn execute_try(&mut self, expr_try: &'a ExprTry, env: &mut BindingEnv) {
        self.record_core_terminator(
            ScenarioCoreTerminatorKind::ErrorExit,
            None,
            None,
            vec!["error_exit".to_owned()],
            expr_try,
        );
        self.execute_expr(expr_try.expr.as_ref(), env);
    }

    pub(super) fn execute_async(&mut self, expr_async: &'a ExprAsync, env: &mut BindingEnv) {
        self.execute_block(&expr_async.block, env);
    }

    pub(super) fn eval_bool(&self, expr: &'a Expr, env: &BindingEnv) -> Option<bool> {
        match expr {
            Expr::Lit(ExprLit {
                lit: Lit::Bool(value),
                ..
            }) => Some(value.value),
            Expr::Path(path) => {
                path_last_ident(&path.path).and_then(|name| env.resolve_bool(&name))
            }
            Expr::Paren(paren) => self.eval_bool(paren.expr.as_ref(), env),
            Expr::Call(call) => {
                let Expr::Path(path) = call.func.as_ref() else {
                    return None;
                };
                if !call.args.is_empty() {
                    return None;
                }
                let function = self.functions.get(&path_last_ident(&path.path)?)?;
                function_returns_bool_literal(function)
            }
            _ => None,
        }
    }

    pub(super) fn unsupported_macro(&mut self, mac: &Macro) {
        let name = mac
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::");
        let label = if name.ends_with("select") {
            self.operations.push(ScenarioOp {
                span: self.span(mac),
                kind: ScenarioOpKind::Select {
                    branch_count: select_branch_count(mac),
                },
            });
            format!("{name}!")
        } else {
            format!("macro:{name}")
        };
        self.record_unsupported_construct(&label);
    }

    pub(super) fn record_spawn_macro(&mut self, mac: &Macro) -> bool {
        let Some(name) = mac.path.get_ident().map(|ident| ident.to_string()) else {
            return false;
        };
        let boundary = match name.as_str() {
            "__kobo_spawn_block" => ScenarioModeledBoundary::WardTask,
            "__kobo_spawn_local_block" => ScenarioModeledBoundary::WardTaskLocal,
            _ => return false,
        };
        self.operations.push(ScenarioOp {
            span: self.span(mac),
            kind: ScenarioOpKind::ModeledEffect { boundary },
        });
        true
    }

    pub(super) fn record_panic_macro(&mut self, mac: &Macro) -> bool {
        if !path_ends_with(&mac.path, &["panic"]) {
            return false;
        }
        self.record_core_terminator(
            ScenarioCoreTerminatorKind::Panic,
            None,
            None,
            vec!["panic".to_owned()],
            mac,
        );
        true
    }

    pub(super) fn timeout_await_boundary(
        &self,
        expr: &'a Expr,
        env: &BindingEnv,
    ) -> Option<String> {
        match peel_paren_expr(expr) {
            Expr::Call(call) => {
                let Expr::Path(path) = call.func.as_ref() else {
                    return None;
                };
                let resolved_path = self.resolved_path_segments(&path.path, env);
                path_ends_with_segments(&resolved_path, &["tokio", "time", "timeout"])
                    .then(|| "tokio::time::timeout".to_owned())
            }
            Expr::Await(await_expr) => self.timeout_await_boundary(await_expr.base.as_ref(), env),
            _ => None,
        }
    }

    pub(super) fn record_core_terminator(
        &mut self,
        kind: ScenarioCoreTerminatorKind,
        boundary: Option<String>,
        policy: Option<ScenarioBoundaryPolicy>,
        edges: Vec<String>,
        node: &impl Spanned,
    ) {
        self.operations.push(ScenarioOp {
            span: self.span(node),
            kind: ScenarioOpKind::CoreTerminator {
                kind,
                boundary,
                policy,
                edges,
            },
        });
    }

    pub(super) fn record_unsupported_construct(&mut self, label: &str) {
        if !self
            .coverage
            .unsupported_constructs
            .iter()
            .any(|construct| construct == label)
        {
            self.coverage.unsupported_constructs.push(label.to_owned());
        }
    }

    pub(super) fn raw_nondeterminism(&mut self, operation: &str, expr: &impl Spanned) {
        self.operations.push(ScenarioOp {
            span: self.operation_span(expr, operation),
            kind: ScenarioOpKind::RawNondeterminism {
                operation: operation.to_owned(),
            },
        });
    }

    pub(super) fn uncontrolled_effect(&mut self, operation: &str, expr: &impl Spanned) {
        self.operations.push(ScenarioOp {
            span: self.operation_span(expr, operation),
            kind: ScenarioOpKind::UncontrolledEffect {
                operation: operation.to_owned(),
            },
        });
    }

    pub(super) fn span(&self, node: &impl Spanned) -> KoboSpan {
        let span = self.ast.span_from_syn(node.span());
        if span.is_empty() {
            KoboSpan::new(span.start, span.start.saturating_add(1), span.file_id)
        } else {
            span
        }
    }

    pub(super) fn operation_span(&self, node: &impl Spanned, needle: &str) -> KoboSpan {
        let span = self.span(node);
        let source = self.ast.source();
        let start = span.start as usize;
        let end = span.end as usize;
        if source
            .get(start..end.min(source.len()))
            .is_some_and(|snippet| snippet.contains(needle))
        {
            return span;
        }
        let Some(start) = nearest_occurrence(source, needle, start) else {
            return span;
        };
        KoboSpan::new(
            start as u32,
            (start + needle.len()).max(start + 1) as u32,
            self.ast.file_id,
        )
    }
}

pub(super) fn nearest_occurrence(source: &str, needle: &str, anchor: usize) -> Option<usize> {
    source
        .match_indices(needle)
        .map(|(index, _)| index)
        .min_by_key(|index| index.abs_diff(anchor))
}
