use super::{
    expr_path_ident, local_suppression_reason, pat_ident, path_ends_with, path_ends_with_segments,
    path_last_ident, path_to_string, BindingEnv, Expr, ExprCall, ExprMethodCall, ExprStruct,
    HashMap, InferredLifecycleCreation, KoboSpan, Local, MethodShape, MethodShapeMap, Pat, PatType,
    Path, ProtocolTemplateRegistry, ScenarioLifecycleTemplate, ScenarioLowerer, ScenarioOp,
    ScenarioOpKind, UnsupportedContainerShape,
};

struct AwaitVisitor {
    found: bool,
}

impl<'ast> syn::visit::Visit<'ast> for AwaitVisitor {
    fn visit_expr_await(&mut self, _expr: &'ast syn::ExprAwait) {
        self.found = true;
    }
}

impl<'a> ScenarioLowerer<'a> {
    pub(super) fn execute_local(&mut self, local: &'a Local, env: &mut BindingEnv) {
        let Some(init) = &local.init else {
            return;
        };
        if let Some((binding, type_name, span)) =
            self.local_must_call_creation(local, init.expr.as_ref())
        {
            if let Some(actions) = self.must_call_types.get(&type_name).cloned() {
                let template = ScenarioLifecycleTemplate::declared(&type_name);
                env.bind_obligation(binding.clone(), binding.clone(), actions.clone());
                self.operations.push(ScenarioOp {
                    span,
                    kind: ScenarioOpKind::CreateObligation {
                        binding: binding.clone(),
                        type_name,
                        actions,
                        template: Some(template),
                    },
                });
                self.record_reasoned_suppression(local, &binding);
                return;
            }
        }
        if let Some(creation) = self.local_lifecycle_creation(local, init.expr.as_ref(), env) {
            if expr_contains_await(init.expr.as_ref()) {
                self.record_unsupported_construct("lifecycle_method_await_initializer");
                self.execute_expr(init.expr.as_ref(), env);
            }
            env.bind_obligation(
                creation.binding.clone(),
                creation.binding.clone(),
                creation.actions.clone(),
            );
            self.operations.push(ScenarioOp {
                span: creation.span,
                kind: ScenarioOpKind::CreateObligation {
                    binding: creation.binding.clone(),
                    type_name: creation.type_name,
                    actions: creation.actions,
                    template: Some(creation.template),
                },
            });
            self.record_reasoned_suppression(local, &creation.binding);
            return;
        }
        if let Some((binding, type_name, container, span)) =
            self.unsupported_obligation_container(local, init.expr.as_ref())
        {
            self.operations.push(ScenarioOp {
                span,
                kind: ScenarioOpKind::UnsupportedContainer {
                    binding,
                    type_name,
                    container,
                },
            });
            return;
        }
        if let Some((binding, type_name)) = local_static_type(local, init.expr.as_ref()) {
            env.bind_type(binding, type_name);
            return;
        }
        if let Some(binding) =
            expr_path_ident(init.expr.as_ref()).and_then(|name| env.resolve(&name))
        {
            if let Some(local_binding) = pat_ident(&local.pat) {
                let records_terminal_move = local_binding.starts_with('_');
                env.bind(local_binding, binding.clone());
                if !records_terminal_move {
                    return;
                }
            }
            self.operations.push(ScenarioOp {
                span: self.span(init.expr.as_ref()),
                kind: ScenarioOpKind::MoveBinding { binding },
            });
            return;
        }
        if let Some((binding, value)) =
            pat_ident(&local.pat).zip(self.eval_bool(init.expr.as_ref(), env))
        {
            env.bind_bool(binding, value);
            return;
        }
        if let Some(binding) = pat_ident(&local.pat) {
            if let Some(value) = self.record_external_boundary_expr(init.expr.as_ref(), env) {
                env.bind_external(binding, value);
                return;
            }
        }
        if let Some(binding) = pat_ident(&local.pat) {
            env.clear_local(&binding);
        }
        self.execute_expr(init.expr.as_ref(), env);
    }

    pub(super) fn local_must_call_creation(
        &self,
        local: &'a Local,
        expr: &'a Expr,
    ) -> Option<(String, String, KoboSpan)> {
        let binding = pat_ident(&local.pat)?;
        let Expr::Struct(ExprStruct { path, .. }) = expr else {
            return None;
        };
        let type_name = path_last_ident(path)?;
        self.must_call_types
            .contains_key(&type_name)
            .then(|| (binding, type_name, self.span(expr)))
    }

    pub(super) fn local_lifecycle_creation(
        &self,
        local: &'a Local,
        expr: &'a Expr,
        env: &BindingEnv,
    ) -> Option<InferredLifecycleCreation> {
        let binding = pat_ident(&local.pat)?;
        if tokio_spawn_call(expr).is_some() {
            return Some(InferredLifecycleCreation {
                binding,
                type_name: "SpawnedTask".to_owned(),
                actions: spawned_task_actions(),
                template: ScenarioLifecycleTemplate::inferred("spawned_task", "spawned_task"),
                span: self.span(expr),
            });
        }
        if let Some(span) = self.builtin_file_socket_create_span(expr, env) {
            return Some(InferredLifecycleCreation {
                binding,
                type_name: "FileSocket".to_owned(),
                actions: file_socket_actions(),
                template: ScenarioLifecycleTemplate::inferred("file_socket", "file_socket"),
                span,
            });
        }

        let call = lifecycle_method_call(expr)?;
        let receiver_type = expr_static_type(call.receiver.as_ref(), env, self.method_shapes)?;
        let method_name = call.method.to_string();
        let method_shape = self
            .method_shapes
            .get(&receiver_type)
            .and_then(|methods| methods.get(&method_name))?;
        lifecycle_template_from_shape(&method_name, method_shape, self.method_shapes).map(
            |template| InferredLifecycleCreation {
                binding,
                type_name: template.obligation_kind.to_owned(),
                actions: template.terminal_action_strings(),
                template: ScenarioLifecycleTemplate::from_protocol_definition(template),
                span: self.span(call),
            },
        )
    }

    pub(super) fn builtin_file_socket_create_span(
        &self,
        expr: &'a Expr,
        env: &BindingEnv,
    ) -> Option<KoboSpan> {
        match peel_paren_expr(expr) {
            Expr::Call(call) => {
                let Expr::Path(path) = call.func.as_ref() else {
                    return None;
                };
                let resolved_path = self.resolved_path_segments(&path.path, env);
                (is_std_file_open_path(&resolved_path) || is_std_tcp_connect_path(&resolved_path))
                    .then(|| self.span(call))
            }
            Expr::Await(await_expr) => {
                self.builtin_file_socket_create_span(await_expr.base.as_ref(), env)
            }
            Expr::MethodCall(call) => {
                let method = call.method.to_string();
                let receiver_type =
                    expr_static_type(call.receiver.as_ref(), env, self.method_shapes)?;
                (method == "accept" && type_name_ends_with(&receiver_type, "TcpListener"))
                    .then(|| self.span(call))
            }
            _ => None,
        }
    }

    pub(super) fn unsupported_obligation_container(
        &self,
        local: &'a Local,
        expr: &'a Expr,
    ) -> Option<(String, String, String, KoboSpan)> {
        let binding = pat_ident(&local.pat)?;
        let container = obligation_container_shape(expr, self.must_call_types)?;
        Some((
            binding,
            container.type_name,
            container.container,
            self.span(expr),
        ))
    }

    pub(super) fn record_reasoned_suppression(&mut self, local: &'a Local, binding: &str) {
        let Some(reason) = local_suppression_reason(local) else {
            return;
        };
        self.operations.push(ScenarioOp {
            span: self.span(local),
            kind: ScenarioOpKind::Discharge {
                binding: binding.to_owned(),
                action: format!("suppressed:{reason}"),
            },
        });
    }
}

pub(super) fn is_tokio_spawn(path: &Path) -> bool {
    path_ends_with(path, &["tokio", "spawn"])
}

pub(super) fn tokio_spawn_call(expr: &Expr) -> Option<&ExprCall> {
    match expr {
        Expr::Call(call) => match call.func.as_ref() {
            Expr::Path(path) if is_tokio_spawn(&path.path) => Some(call),
            _ => None,
        },
        Expr::Paren(paren) => tokio_spawn_call(paren.expr.as_ref()),
        _ => None,
    }
}

pub(super) fn local_static_type(local: &Local, expr: &Expr) -> Option<(String, String)> {
    let binding = pat_ident(&local.pat)?;
    type_annotation_name(&local.pat)
        .or_else(|| expr_static_type_from_initializer(expr))
        .map(|type_name| (binding, type_name))
}

pub(super) fn type_annotation_name(pat: &Pat) -> Option<String> {
    let Pat::Type(PatType { ty, .. }) = pat else {
        return None;
    };
    match ty.as_ref() {
        syn::Type::Path(path) => Some(path_to_string(&path.path)),
        _ => None,
    }
}

pub(super) fn expr_static_type_from_initializer(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Struct(struct_expr) => path_last_ident(&struct_expr.path),
        Expr::Paren(paren) => expr_static_type_from_initializer(paren.expr.as_ref()),
        _ => None,
    }
}

pub(super) fn expr_contains_await(expr: &Expr) -> bool {
    let mut visitor = AwaitVisitor { found: false };
    syn::visit::visit_expr(&mut visitor, expr);
    visitor.found
}

pub(super) fn obligation_container_shape(
    expr: &Expr,
    must_call_types: &HashMap<String, Vec<String>>,
) -> Option<UnsupportedContainerShape> {
    let expr = peel_paren_expr(expr);
    let Expr::Call(arc_call) = expr else {
        return None;
    };
    let Expr::Path(arc_path) = arc_call.func.as_ref() else {
        return None;
    };
    if !path_ends_with(&arc_path.path, &["Arc", "new"]) {
        return None;
    }
    let mutex_expr = arc_call.args.first()?;
    let Expr::Call(mutex_call) = peel_paren_expr(mutex_expr) else {
        return None;
    };
    let Expr::Path(mutex_path) = mutex_call.func.as_ref() else {
        return None;
    };
    if !path_ends_with(&mutex_path.path, &["Mutex", "new"]) {
        return None;
    }
    let inner_expr = mutex_call.args.first()?;
    let Expr::Struct(inner_struct) = peel_paren_expr(inner_expr) else {
        return None;
    };
    let type_name = path_last_ident(&inner_struct.path)?;
    must_call_types
        .contains_key(&type_name)
        .then(|| UnsupportedContainerShape {
            type_name,
            container: "Arc<Mutex>".to_owned(),
        })
}

pub(super) fn peel_paren_expr(expr: &Expr) -> &Expr {
    match expr {
        Expr::Paren(paren) => peel_paren_expr(paren.expr.as_ref()),
        other => other,
    }
}

pub(super) fn expr_static_type(
    expr: &Expr,
    env: &BindingEnv,
    method_shapes: &MethodShapeMap,
) -> Option<String> {
    match expr {
        Expr::Path(path) => path_last_ident(&path.path)
            .and_then(|local| env.resolve_type(&local).map(str::to_owned)),
        Expr::Struct(struct_expr) => path_last_ident(&struct_expr.path),
        Expr::MethodCall(call) => {
            let receiver_type = expr_static_type(call.receiver.as_ref(), env, method_shapes)?;
            let method_name = call.method.to_string();
            method_shapes
                .get(&receiver_type)
                .and_then(|methods| methods.get(&method_name))
                .and_then(|shape| shape.return_type.clone())
        }
        Expr::Await(await_expr) => expr_static_type(await_expr.base.as_ref(), env, method_shapes),
        Expr::Paren(paren) => expr_static_type(paren.expr.as_ref(), env, method_shapes),
        _ => None,
    }
}

pub(super) fn lifecycle_method_call(expr: &Expr) -> Option<&ExprMethodCall> {
    match expr {
        Expr::MethodCall(call) => Some(call),
        Expr::Await(await_expr) => lifecycle_method_call(await_expr.base.as_ref()),
        Expr::Paren(paren) => lifecycle_method_call(paren.expr.as_ref()),
        _ => None,
    }
}

pub(super) fn lifecycle_template_from_shape(
    method_name: &str,
    method_shape: &MethodShape,
    method_shapes: &MethodShapeMap,
) -> Option<kobo_ir::ProtocolTemplateDefinition> {
    let return_type = method_shape.return_type.as_deref()?;
    ProtocolTemplateRegistry::builtin_templates()
        .iter()
        .copied()
        .find(|template| {
            template.has_create_method(method_name)
                && type_has_terminal_action(
                    return_type,
                    &template.terminal_action_strings(),
                    method_shapes,
                )
        })
}

pub(super) fn is_std_file_open_path(path: &[String]) -> bool {
    path_ends_with_segments(path, &["std", "fs", "File", "open"])
        || path_ends_with_segments(path, &["std", "fs", "OpenOptions", "open"])
}

pub(super) fn is_std_tcp_connect_path(path: &[String]) -> bool {
    path_ends_with_segments(path, &["std", "net", "TcpStream", "connect"])
        || path_ends_with_segments(path, &["std", "os", "unix", "net", "UnixStream", "connect"])
}

pub(super) fn type_name_ends_with(type_name: &str, suffix: &str) -> bool {
    type_name == suffix || type_name.ends_with(&format!("::{suffix}"))
}
pub(super) fn type_has_terminal_action(
    type_name: &str,
    actions: &[String],
    method_shapes: &MethodShapeMap,
) -> bool {
    let Some(methods) = method_shapes.get(type_name) else {
        return false;
    };
    actions.iter().any(|action| {
        rust_method_name(action)
            .and_then(|method| methods.get(method))
            .is_some_and(|shape| shape.consumes_receiver)
    })
}

pub(super) fn rust_method_name(action: &str) -> Option<&str> {
    match action {
        "drop-at-safe-boundary" | "opaque-boundary" => None,
        "detach-with-policy" => Some("detach_with_policy"),
        other => Some(other),
    }
}

#[cfg(test)]
pub(super) fn queue_delivery_actions() -> Vec<String> {
    ["ack", "nack", "requeue"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
pub(super) fn transaction_actions() -> Vec<String> {
    ["commit", "rollback"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

pub(super) fn handler_reply_actions() -> Vec<String> {
    ["reply", "reject", "cancel"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

pub(super) fn spawned_task_actions() -> Vec<String> {
    ["await", "abort", "detach-with-policy"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

pub(super) fn file_socket_actions() -> Vec<String> {
    ["close", "transfer", "opaque-boundary"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

pub(super) fn drop_discharge_action(actions: &[String]) -> Option<String> {
    if actions
        .iter()
        .any(|action| action == "drop-at-safe-boundary")
    {
        return Some("drop-at-safe-boundary".to_owned());
    }
    if actions.iter().any(|action| action == "close")
        && actions.iter().any(|action| action == "opaque-boundary")
    {
        return Some("close".to_owned());
    }
    None
}

pub(super) fn terminal_action_name(method: &str) -> String {
    match method {
        "detach_with_policy" => "detach-with-policy".to_owned(),
        other => other.to_owned(),
    }
}
