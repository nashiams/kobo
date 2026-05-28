use super::{
    expr_path_ident, path_ends_with, path_first_ident, receiver_ident, BindingEnv,
    BoundaryPolicyFact, Expr, ExprCall, ExprMethodCall, ExprPath, ExternalBoundaryValue, Path,
    Punctuated, ScenarioBoundaryCallArgument, ScenarioBoundaryPolicy, ScenarioCoreTerminatorKind,
    ScenarioExternalCallShape, ScenarioLowerer, ScenarioOp, ScenarioOpKind, Spanned, ToTokens,
};
impl<'a> ScenarioLowerer<'a> {
    pub(super) fn record_external_boundary_expr(
        &mut self,
        expr: &'a Expr,
        env: &mut BindingEnv,
    ) -> Option<ExternalBoundaryValue> {
        match expr {
            Expr::Call(call) => {
                let Expr::Path(path) = call.func.as_ref() else {
                    return None;
                };
                self.record_external_path_call(path, call, env)
            }
            Expr::MethodCall(call) => self.record_external_method_call(call, env),
            Expr::Await(await_expr) => {
                self.record_external_boundary_expr(await_expr.base.as_ref(), env)
            }
            Expr::Paren(paren) => self.record_external_boundary_expr(paren.expr.as_ref(), env),
            _ => None,
        }
    }

    pub(super) fn record_external_path_call(
        &mut self,
        path: &'a ExprPath,
        call: &'a ExprCall,
        env: &BindingEnv,
    ) -> Option<ExternalBoundaryValue> {
        if path_ends_with(&path.path, &["Client", "new"]) {
            let crate_name = self.external_crate_name(&path.path, env);
            let call_path = self.external_call_path(&path.path, &crate_name, env);
            let return_type = associated_return_type_path(&call_path)
                .unwrap_or_else(|| format!("{crate_name}::__KoboBoundaryValue"));
            self.record_boundary_argument_escapes(&call.args, env, &crate_name);
            self.record_external_boundary_operation(
                call,
                crate_name.clone(),
                Some(call_path),
                boundary_call_arguments(&call.args),
                Some(return_type.clone()),
                ScenarioExternalCallShape::AssociatedFunction,
            );
            return Some(ExternalBoundaryValue {
                crate_name,
                type_path: return_type,
            });
        }
        if let Some(crate_name) = self.imported_external_crate(&path.path, env) {
            let call_path = self.external_call_path(&path.path, &crate_name, env);
            let call_shape = self.external_call_shape(&path.path, env);
            let return_type = match call_shape {
                ScenarioExternalCallShape::AssociatedFunction => {
                    associated_return_type_path(&call_path)
                        .unwrap_or_else(|| format!("{crate_name}::__KoboBoundaryValue"))
                }
                ScenarioExternalCallShape::FreeFunction => {
                    format!("{crate_name}::__KoboBoundaryValue")
                }
                ScenarioExternalCallShape::Method => {
                    format!("{crate_name}::__KoboBoundaryValue")
                }
            };
            self.record_boundary_argument_escapes(&call.args, env, &crate_name);
            self.record_external_boundary_operation(
                call,
                crate_name.clone(),
                Some(call_path),
                boundary_call_arguments(&call.args),
                Some(return_type.clone()),
                call_shape,
            );
            return Some(ExternalBoundaryValue {
                crate_name,
                type_path: return_type,
            });
        }
        None
    }

    pub(super) fn record_external_method_call(
        &mut self,
        call: &'a ExprMethodCall,
        env: &BindingEnv,
    ) -> Option<ExternalBoundaryValue> {
        let receiver_name = receiver_ident(call.receiver.as_ref())?;
        let receiver = env.resolve_external(&receiver_name)?.clone();
        let call_path = format!("{}::{}", receiver.type_path, call.method);
        let return_type = external_method_return_type(
            &receiver.crate_name,
            &receiver.type_path,
            &call.method.to_string(),
        );
        self.record_boundary_argument_escapes(&call.args, env, &receiver.crate_name);
        self.record_external_boundary_operation(
            call,
            receiver.crate_name.clone(),
            Some(call_path),
            boundary_call_arguments(&call.args),
            Some(return_type.clone()),
            ScenarioExternalCallShape::Method,
        );
        Some(ExternalBoundaryValue {
            crate_name: receiver.crate_name,
            type_path: return_type,
        })
    }

    pub(super) fn record_boundary_argument_escapes(
        &mut self,
        args: &'a Punctuated<Expr, syn::token::Comma>,
        env: &BindingEnv,
        crate_name: &str,
    ) {
        let policy = self.boundary_policy_for(crate_name);
        if matches!(policy.policy, ScenarioBoundaryPolicy::Unselected) {
            return;
        }
        for argument in args {
            if let Some(binding) = expr_path_ident(argument).and_then(|name| env.resolve(&name)) {
                self.operations.push(ScenarioOp {
                    span: self.span(argument),
                    kind: ScenarioOpKind::Discharge {
                        binding,
                        action: format!("escape:{crate_name}"),
                    },
                });
            }
        }
    }

    pub(super) fn record_external_boundary_operation(
        &mut self,
        node: &impl Spanned,
        crate_name: String,
        call_path: Option<String>,
        call_arguments: Vec<ScenarioBoundaryCallArgument>,
        return_type: Option<String>,
        call_shape: ScenarioExternalCallShape,
    ) {
        let policy = self.boundary_policy_for(&crate_name);
        if !is_replay_owned_policy(&policy.policy) {
            self.record_opaque_boundary(&crate_name);
        }
        let terminator_policy = policy.policy.clone();
        let terminator_boundary = crate_name.clone();
        self.operations.push(ScenarioOp {
            span: self.span(node),
            kind: ScenarioOpKind::ExternalBoundary {
                call_path,
                call_arguments,
                return_type,
                call_shape,
                crate_name,
                policy: policy.policy,
                reason: policy.reason,
            },
        });
        if !is_replay_owned_policy(&terminator_policy) {
            self.operations.push(ScenarioOp {
                span: self.span(node),
                kind: ScenarioOpKind::CoreTerminator {
                    kind: ScenarioCoreTerminatorKind::OpaqueBoundary,
                    boundary: Some(terminator_boundary),
                    policy: Some(terminator_policy),
                    edges: vec!["opaque_boundary".to_owned()],
                },
            });
        }
    }

    pub(super) fn external_crate_name(&self, path: &Path, env: &BindingEnv) -> String {
        let Some(first_ident) = path_first_ident(path) else {
            return "external".to_owned();
        };
        self.lookup_import(env, &first_ident)
            .and_then(|path| path.first().cloned())
            .unwrap_or(first_ident)
    }

    pub(super) fn resolved_path_segments(&self, path: &Path, env: &BindingEnv) -> Vec<String> {
        let segments = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        let Some(first) = segments.first().cloned() else {
            return segments;
        };
        if let Some(imported_path) = self.lookup_import(env, &first) {
            let mut resolved = imported_path.clone();
            resolved.extend(segments.into_iter().skip(1));
            return resolved;
        }
        segments
    }

    pub(super) fn external_call_path(
        &self,
        path: &Path,
        crate_name: &str,
        env: &BindingEnv,
    ) -> String {
        let mut segments = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        if let Some(first) = segments.first() {
            if first == crate_name {
                return segments.join("::");
            }
            if let Some(imported_path) = self.lookup_import(env, first) {
                if imported_path.first().is_some_and(|root| root == crate_name) {
                    let mut resolved = imported_path.clone();
                    resolved.extend(segments.into_iter().skip(1));
                    return resolved.join("::");
                }
            }
        }
        if let Some(first) = segments.first_mut() {
            *first = crate_name.to_owned();
        }
        segments.join("::")
    }

    pub(super) fn external_call_shape(
        &self,
        path: &Path,
        env: &BindingEnv,
    ) -> ScenarioExternalCallShape {
        let segments = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        let Some(first) = segments.first() else {
            return ScenarioExternalCallShape::FreeFunction;
        };
        if let Some(imported_path) = self.lookup_import(env, first) {
            if segments.len() == 1 || imported_path_looks_like_module(imported_path) {
                return ScenarioExternalCallShape::FreeFunction;
            }
            return ScenarioExternalCallShape::AssociatedFunction;
        }
        if segments.len() == 1 {
            return ScenarioExternalCallShape::FreeFunction;
        }
        ScenarioExternalCallShape::AssociatedFunction
    }

    pub(super) fn imported_external_crate(&self, path: &Path, env: &BindingEnv) -> Option<String> {
        let first_ident = path_first_ident(path)?;
        let import_path = self.lookup_import(env, &first_ident)?;
        let crate_name = import_path.first()?;
        (!matches!(
            crate_name.as_str(),
            "std" | "core" | "alloc" | "crate" | "self" | "super" | "kobo" | "ward"
        ))
        .then(|| crate_name.clone())
    }

    pub(super) fn lookup_import<'b>(
        &'b self,
        env: &'b BindingEnv,
        local: &str,
    ) -> Option<&'b Vec<String>> {
        env.imports.get(local).or_else(|| self.imports.get(local))
    }

    pub(super) fn boundary_policy_for(&self, crate_name: &str) -> BoundaryPolicyFact {
        self.boundary_policies
            .get(crate_name)
            .cloned()
            .unwrap_or_else(|| BoundaryPolicyFact {
                policy: ScenarioBoundaryPolicy::Unselected,
                reason: None,
            })
    }

    pub(super) fn record_opaque_boundary(&mut self, boundary: &str) {
        if !self
            .coverage
            .opaque_boundaries
            .iter()
            .any(|existing| existing == boundary)
        {
            self.coverage.opaque_boundaries.push(boundary.to_owned());
        }
    }
}

pub(super) fn imported_path_looks_like_module(import_path: &[String]) -> bool {
    import_path
        .last()
        .is_some_and(|segment| starts_with_module_identifier(segment))
}

pub(super) fn starts_with_module_identifier(segment: &str) -> bool {
    segment
        .chars()
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_lowercase())
}
pub(super) fn boundary_call_arguments(
    arguments: &Punctuated<Expr, syn::token::Comma>,
) -> Vec<ScenarioBoundaryCallArgument> {
    arguments
        .iter()
        .enumerate()
        .map(|(index, argument)| ScenarioBoundaryCallArgument {
            index,
            source: argument.to_token_stream().to_string(),
        })
        .collect()
}
pub(super) fn associated_return_type_path(call_path: &str) -> Option<String> {
    let mut segments = call_path
        .split("::")
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    (segments.len() >= 2).then(|| {
        let _method = segments.pop();
        segments.join("::")
    })
}

pub(super) fn external_method_return_type(
    crate_name: &str,
    receiver_type: &str,
    method: &str,
) -> String {
    match (crate_name, receiver_type, method) {
        ("reqwest", "reqwest::Client", "get" | "post" | "put" | "delete" | "patch" | "request") => {
            "reqwest::RequestBuilder".to_owned()
        }
        ("reqwest", "reqwest::RequestBuilder", "send") => "reqwest::Response".to_owned(),
        ("sqlx", "sqlx::Pool", "begin") => "sqlx::Transaction".to_owned(),
        ("sqlx", "sqlx::Pool", "acquire") => "sqlx::PoolConnection".to_owned(),
        ("sqlx", "sqlx::Transaction", "commit" | "rollback") => {
            "sqlx::__KoboBoundaryValue".to_owned()
        }
        _ => format!("{crate_name}::__KoboBoundaryValue"),
    }
}
pub(super) fn is_replay_owned_policy(policy: &ScenarioBoundaryPolicy) -> bool {
    matches!(
        policy,
        ScenarioBoundaryPolicy::Model
            | ScenarioBoundaryPolicy::Record
            | ScenarioBoundaryPolicy::Activity
            | ScenarioBoundaryPolicy::Stub
    )
}
