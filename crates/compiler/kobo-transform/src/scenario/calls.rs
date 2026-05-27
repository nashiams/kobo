use super::*;

impl<'a> ScenarioLowerer<'a> {
    pub(super) fn execute_call(&mut self, call: &'a ExprCall, env: &mut BindingEnv) {
        if let Expr::Path(path) = call.func.as_ref() {
            if self.record_known_call(path, call, env) || self.execute_helper_call(path, call, env)
            {
                return;
            }
        }
        for argument in &call.args {
            self.execute_expr(argument, env);
        }
    }

    pub(super) fn record_known_call(
        &mut self,
        path: &'a ExprPath,
        call: &'a ExprCall,
        env: &mut BindingEnv,
    ) -> bool {
        if is_tokio_spawn(&path.path) {
            self.operations.push(ScenarioOp {
                span: self.span(call),
                kind: ScenarioOpKind::ModeledEffect {
                    boundary: ScenarioModeledBoundary::WardTask,
                },
            });
            if let Some(argument) = call.args.first() {
                self.execute_expr(argument, env);
            }
            return true;
        }
        if path_ends_with(&path.path, &["drop"]) {
            if let Some(binding) = call
                .args
                .first()
                .and_then(expr_path_ident)
                .and_then(|name| env.resolve(&name))
            {
                if let Some(action) = env
                    .terminal_actions(&binding)
                    .and_then(|actions| drop_discharge_action(&actions))
                {
                    self.operations.push(ScenarioOp {
                        span: self.span(call),
                        kind: ScenarioOpKind::Discharge { binding, action },
                    });
                    return true;
                }
                self.operations.push(ScenarioOp {
                    span: self.span(call),
                    kind: ScenarioOpKind::MoveBinding { binding },
                });
                return true;
            }
        }
        let resolved_path = self.resolved_path_segments(&path.path, env);
        if path_ends_with_segments(&resolved_path, &["SystemTime", "now"]) {
            self.raw_nondeterminism("SystemTime::now", call);
            return true;
        }
        if path_ends_with_segments(&resolved_path, &["Instant", "now"]) {
            self.raw_nondeterminism("Instant::now", call);
            return true;
        }
        if path_ends_with_segments(&resolved_path, &["thread_rng"]) {
            self.raw_nondeterminism("thread_rng", call);
            return true;
        }
        if resolved_path
            .first()
            .is_some_and(|segment| segment == "rand")
        {
            self.raw_nondeterminism("rand::", call);
            return true;
        }
        if path_starts_with(&resolved_path, &["std", "env"]) {
            self.raw_nondeterminism("std::env", call);
            return true;
        }
        if path_starts_with(&resolved_path, &["std", "process"]) {
            self.raw_nondeterminism("std::process", call);
            return true;
        }
        if path_ends_with_segments(&resolved_path, &["std", "thread", "sleep"]) {
            self.raw_nondeterminism("std::thread::sleep", call);
            return true;
        }
        if path_starts_with(&resolved_path, &["std", "fs"]) {
            self.uncontrolled_effect("std::fs", call);
            return true;
        }
        if path_starts_with(&resolved_path, &["std", "net"]) {
            self.uncontrolled_effect("std::net", call);
            return true;
        }
        if self.record_external_path_call(path, call, env).is_some() {
            return true;
        }
        false
    }

    pub(super) fn execute_helper_call(
        &mut self,
        path: &'a ExprPath,
        call: &'a ExprCall,
        env: &mut BindingEnv,
    ) -> bool {
        let Some(function_name) = path_last_ident(&path.path) else {
            return false;
        };
        let Some(function) = self.functions.get(function_name.as_str()).copied() else {
            return false;
        };

        if self.is_recursive_reentry(&function_name) {
            self.operations.push(ScenarioOp {
                span: self.span(call),
                kind: ScenarioOpKind::Loop,
            });
            return true;
        }

        for argument in &call.args {
            self.execute_expr(argument, env);
        }

        let mut helper_env = BindingEnv::with_imports(self.imports.clone());
        let mut transfer_ops = Vec::new();
        for (input, argument) in function.sig.inputs.iter().zip(call.args.iter()) {
            let Some(parameter) = fn_arg_ident(input) else {
                continue;
            };
            if let Some(argument_binding) =
                expr_path_ident(argument).and_then(|name| env.resolve(&name))
            {
                let transfer_index = self.operations.len();
                self.operations.push(ScenarioOp {
                    span: self.span(call),
                    kind: ScenarioOpKind::Transfer {
                        binding: argument_binding.clone(),
                        callee: function_name.clone(),
                        proven: false,
                    },
                });
                if let Some(actions) = env.terminal_actions(&argument_binding) {
                    helper_env.bind_obligation(
                        parameter.clone(),
                        argument_binding.clone(),
                        actions,
                    );
                } else {
                    helper_env.bind(parameter.clone(), argument_binding.clone());
                }
                transfer_ops.push((
                    transfer_index,
                    argument_binding.clone(),
                    function_name.clone(),
                ));
            }
            if let Some(value) = self.eval_bool(argument, env) {
                helper_env.bind_bool(parameter.clone(), value);
            }
            if let Some(external_value) =
                expr_path_ident(argument).and_then(|name| env.resolve_external(&name).cloned())
            {
                helper_env.bind_external(parameter, external_value);
            }
        }

        self.execute_function(&function_name, function, &mut helper_env);
        for (index, obligation_key, _) in transfer_ops {
            let proven = !helper_env.has_active_obligation(&obligation_key);
            if let Some(ScenarioOp {
                kind: ScenarioOpKind::Transfer { proven: slot, .. },
                ..
            }) = self.operations.get_mut(index)
            {
                *slot = proven;
            }
        }
        true
    }

    pub(super) fn is_recursive_reentry(&self, function_name: &str) -> bool {
        self.call_graph.is_recursive_function(function_name)
            && self
                .active_functions
                .iter()
                .any(|active_function| active_function == function_name)
    }

    pub(super) fn execute_method_call(&mut self, call: &'a ExprMethodCall, env: &mut BindingEnv) {
        if let Some(binding) =
            receiver_ident(call.receiver.as_ref()).and_then(|name| env.resolve(&name))
        {
            let action = terminal_action_name(&call.method.to_string());
            if env
                .terminal_actions(&binding)
                .is_some_and(|actions| actions.iter().any(|candidate| candidate == &action))
            {
                self.operations.push(ScenarioOp {
                    span: self.span(call),
                    kind: ScenarioOpKind::Discharge {
                        binding: binding.clone(),
                        action,
                    },
                });
                env.discharge_obligation(&binding);
                return;
            }
        }
        if self.record_storage_or_network_event(call) {
            return;
        }
        if let Some(boundary) = modeled_boundary(call) {
            self.operations.push(ScenarioOp {
                span: self.span(call),
                kind: ScenarioOpKind::ModeledEffect { boundary },
            });
            return;
        }
        if self.record_external_method_call(call, env).is_some() {
            return;
        }
        self.execute_expr(call.receiver.as_ref(), env);
        for argument in &call.args {
            self.execute_expr(argument, env);
        }
    }

    pub(super) fn record_storage_or_network_event(&mut self, call: &'a ExprMethodCall) -> bool {
        let receiver = call.receiver.as_ref();
        if receiver_has_ward_member(receiver, "storage") {
            self.operations.push(ScenarioOp {
                span: self.span(call),
                kind: ScenarioOpKind::StorageEvent {
                    action: call.method.to_string(),
                },
            });
            return true;
        }
        if receiver_has_ward_member(receiver, "network") {
            self.operations.push(ScenarioOp {
                span: self.span(call),
                kind: ScenarioOpKind::NetworkEvent {
                    action: call.method.to_string(),
                },
            });
            return true;
        }
        false
    }
}
