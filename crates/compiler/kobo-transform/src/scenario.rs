use std::collections::HashMap;

use kobo_ir::{
    KoboSpan, MustCallObligation, ScenarioBoundary, ScenarioCoverageFacts, ScenarioModeledBoundary,
    ScenarioOp, ScenarioOpKind, ScenarioProgram,
};
use kobo_parser::KoboFile;
use syn::spanned::Spanned;
use syn::{
    Block, Expr, ExprCall, ExprIf, ExprLit, ExprMatch, ExprMethodCall, ExprPath, ExprStruct, File,
    Item, ItemFn, Lit, Local, Macro, Pat, PatIdent, PatType, Path, Stmt,
};

type BindingMap = HashMap<String, String>;
type BoolMap = HashMap<String, bool>;

#[derive(Default)]
struct BindingEnv {
    bindings: BindingMap,
    bools: BoolMap,
}

struct ScenarioLowerer<'a> {
    ast: &'a KoboFile,
    must_call_types: &'a HashMap<String, Vec<String>>,
    functions: &'a HashMap<String, &'a ItemFn>,
    operations: Vec<ScenarioOp>,
    coverage: ScenarioCoverageFacts,
    call_depth: usize,
}

pub fn build_scenario_programs(
    ast: &KoboFile,
    must_call_obligations: &[MustCallObligation],
    fallback_profile: &str,
) -> Vec<ScenarioProgram> {
    let _ = fallback_profile;
    let file = ast.syn_file();
    let functions = collect_functions(file);
    let must_call_types = must_call_type_map(file, must_call_obligations);

    functions
        .values()
        .map(|function| lower_function(ast, &functions, &must_call_types, function))
        .collect()
}

fn lower_function(
    ast: &KoboFile,
    functions: &HashMap<String, &ItemFn>,
    must_call_types: &HashMap<String, Vec<String>>,
    function: &ItemFn,
) -> ScenarioProgram {
    let mut lowerer = ScenarioLowerer {
        ast,
        must_call_types,
        functions,
        operations: Vec::new(),
        coverage: ScenarioCoverageFacts::default(),
        call_depth: 0,
    };
    let mut env = BindingEnv::default();
    lowerer.execute_block(&function.block, &mut env);
    lowerer.operations.push(ScenarioOp {
        span: KoboSpan::generated(ast.file_id),
        kind: ScenarioOpKind::Return,
    });

    ScenarioProgram {
        file_id: ast.file_id,
        target: function.sig.ident.to_string(),
        source_hash: String::new(),
        operations: lowerer.operations,
        boundaries: lowerer
            .coverage
            .opaque_boundaries
            .iter()
            .map(|name| ScenarioBoundary {
                span: KoboSpan::generated(ast.file_id),
                name: name.clone(),
                decision: "unselected".to_owned(),
            })
            .collect(),
        coverage: lowerer.coverage,
    }
}

fn collect_functions(file: &File) -> HashMap<String, &ItemFn> {
    file.items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(function) => Some((function.sig.ident.to_string(), function)),
            _ => None,
        })
        .collect()
}

fn must_call_type_map(
    file: &File,
    obligations: &[MustCallObligation],
) -> HashMap<String, Vec<String>> {
    if !obligations.is_empty() {
        return obligations
            .iter()
            .map(|obligation| {
                (
                    obligation.owner_type.clone(),
                    obligation
                        .actions
                        .iter()
                        .map(|action| action.name.clone())
                        .collect(),
                )
            })
            .collect();
    }

    file.items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(item_struct) => item_struct
                .attrs
                .iter()
                .find(|attr| path_ends_with(attr.path(), &["kobo", "must_call"]))
                .and_then(must_call_actions)
                .map(|actions| (item_struct.ident.to_string(), actions)),
            _ => None,
        })
        .collect()
}

impl BindingEnv {
    fn bind(&mut self, local: String, obligation_key: String) {
        self.bindings.insert(local, obligation_key);
    }

    fn bind_bool(&mut self, local: String, value: bool) {
        self.bools.insert(local, value);
    }

    fn resolve(&self, local: &str) -> Option<String> {
        self.bindings.get(local).cloned()
    }

    fn resolve_bool(&self, local: &str) -> Option<bool> {
        self.bools.get(local).copied()
    }
}

impl<'a> ScenarioLowerer<'a> {
    fn execute_block(&mut self, block: &'a Block, env: &mut BindingEnv) {
        for statement in &block.stmts {
            self.execute_statement(statement, env);
        }
    }

    fn execute_statement(&mut self, statement: &'a Stmt, env: &mut BindingEnv) {
        match statement {
            Stmt::Local(local) => self.execute_local(local, env),
            Stmt::Expr(expr, _) => self.execute_expr(expr, env),
            Stmt::Item(_) => {}
            Stmt::Macro(statement_macro) => self.unsupported_macro(&statement_macro.mac),
        }
    }

    fn execute_local(&mut self, local: &'a Local, env: &mut BindingEnv) {
        let Some(init) = &local.init else {
            return;
        };
        if let Some((binding, type_name, span)) =
            self.local_must_call_creation(local, init.expr.as_ref())
        {
            if let Some(actions) = self.must_call_types.get(&type_name).cloned() {
                env.bind(binding.clone(), binding.clone());
                self.operations.push(ScenarioOp {
                    span,
                    kind: ScenarioOpKind::CreateObligation {
                        binding,
                        type_name,
                        actions,
                    },
                });
                return;
            }
        }
        if let Some(binding) =
            expr_path_ident(init.expr.as_ref()).and_then(|name| env.resolve(&name))
        {
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
        self.execute_expr(init.expr.as_ref(), env);
    }

    fn local_must_call_creation(
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

    fn execute_expr(&mut self, expr: &'a Expr, env: &mut BindingEnv) {
        match expr {
            Expr::MethodCall(call) => self.execute_method_call(call, env),
            Expr::Call(call) => self.execute_call(call, env),
            Expr::If(expr_if) => self.execute_if(expr_if, env),
            Expr::Match(expr_match) => self.execute_match(expr_match, env),
            Expr::Await(await_expr) => self.execute_expr(await_expr.base.as_ref(), env),
            Expr::Block(block) => self.execute_block(&block.block, env),
            Expr::Loop(expr_loop) => self.operations.push(ScenarioOp {
                span: self.span(expr_loop),
                kind: ScenarioOpKind::Loop,
            }),
            Expr::Paren(paren) => self.execute_expr(paren.expr.as_ref(), env),
            Expr::Macro(expr_macro) => self.unsupported_macro(&expr_macro.mac),
            _ => {}
        }
    }

    fn execute_if(&mut self, expr_if: &'a ExprIf, env: &mut BindingEnv) {
        match self.eval_bool(expr_if.cond.as_ref(), env) {
            Some(true) => self.execute_block(&expr_if.then_branch, env),
            Some(false) => {
                if let Some((_, else_expr)) = expr_if.else_branch.as_ref() {
                    self.execute_expr(else_expr.as_ref(), env);
                }
            }
            None => {
                self.execute_block(&expr_if.then_branch, env);
                if let Some((_, else_expr)) = expr_if.else_branch.as_ref() {
                    self.execute_expr(else_expr.as_ref(), env);
                }
            }
        }
    }

    fn execute_match(&mut self, expr_match: &'a ExprMatch, env: &mut BindingEnv) {
        let discriminant = self.eval_bool(expr_match.expr.as_ref(), env);
        for arm in &expr_match.arms {
            if matches_bool_pat(&arm.pat, discriminant) || discriminant.is_none() {
                self.execute_expr(arm.body.as_ref(), env);
                if discriminant.is_some() {
                    break;
                }
            }
        }
    }

    fn execute_call(&mut self, call: &'a ExprCall, env: &mut BindingEnv) {
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

    fn record_known_call(
        &mut self,
        path: &'a ExprPath,
        call: &'a ExprCall,
        env: &mut BindingEnv,
    ) -> bool {
        if path_ends_with(&path.path, &["drop"]) {
            if let Some(binding) = call
                .args
                .first()
                .and_then(expr_path_ident)
                .and_then(|name| env.resolve(&name))
            {
                self.operations.push(ScenarioOp {
                    span: self.span(call),
                    kind: ScenarioOpKind::MoveBinding { binding },
                });
                return true;
            }
        }
        if path_ends_with(&path.path, &["SystemTime", "now"])
            || path_ends_with(&path.path, &["Instant", "now"])
        {
            self.raw_nondeterminism("time", call);
            return true;
        }
        if path_ends_with(&path.path, &["thread_rng"])
            || path_first_ident(&path.path).as_deref() == Some("rand")
        {
            self.raw_nondeterminism("random", call);
            return true;
        }
        if path_first_ident(&path.path).as_deref() == Some("std")
            && path_has_segment(&path.path, "fs")
        {
            self.uncontrolled_effect("std::fs", call);
            return true;
        }
        if path_first_ident(&path.path).as_deref() == Some("std")
            && path_has_segment(&path.path, "net")
        {
            self.uncontrolled_effect("std::net", call);
            return true;
        }
        if path_ends_with(&path.path, &["Client", "new"]) {
            self.operations.push(ScenarioOp {
                span: self.span(call),
                kind: ScenarioOpKind::ExternalBoundary {
                    crate_name: path_first_ident(&path.path)
                        .unwrap_or_else(|| "external".to_owned()),
                },
            });
            return true;
        }
        false
    }

    fn execute_helper_call(
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
        if self.call_depth > 16 {
            self.uncontrolled_effect("helper-call recursion depth exceeded", call);
            return true;
        }

        let mut helper_env = BindingEnv::default();
        for (input, argument) in function.sig.inputs.iter().zip(call.args.iter()) {
            let Some(parameter) = fn_arg_ident(input) else {
                continue;
            };
            if let Some(argument_binding) =
                expr_path_ident(argument).and_then(|name| env.resolve(&name))
            {
                self.operations.push(ScenarioOp {
                    span: self.span(call),
                    kind: ScenarioOpKind::Transfer {
                        binding: argument_binding.clone(),
                        callee: function_name.clone(),
                    },
                });
                helper_env.bind(parameter.clone(), argument_binding);
            }
            if let Some(value) = self.eval_bool(argument, env) {
                helper_env.bind_bool(parameter, value);
            }
        }

        self.call_depth += 1;
        self.execute_block(&function.block, &mut helper_env);
        self.call_depth -= 1;
        true
    }

    fn execute_method_call(&mut self, call: &'a ExprMethodCall, env: &mut BindingEnv) {
        if let Some(binding) =
            receiver_ident(call.receiver.as_ref()).and_then(|name| env.resolve(&name))
        {
            self.operations.push(ScenarioOp {
                span: self.span(call),
                kind: ScenarioOpKind::Discharge {
                    binding,
                    action: call.method.to_string(),
                },
            });
            return;
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
        self.execute_expr(call.receiver.as_ref(), env);
        for argument in &call.args {
            self.execute_expr(argument, env);
        }
    }

    fn record_storage_or_network_event(&mut self, call: &'a ExprMethodCall) -> bool {
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

    fn eval_bool(&self, expr: &'a Expr, env: &BindingEnv) -> Option<bool> {
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

    fn unsupported_macro(&mut self, mac: &Macro) {
        let name = mac
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::");
        let label = if name.ends_with("select") {
            format!("{name}!")
        } else {
            format!("macro:{name}")
        };
        if !self.coverage.unsupported_constructs.contains(&label) {
            self.coverage.unsupported_constructs.push(label);
        }
    }

    fn raw_nondeterminism(&mut self, operation: &str, expr: &impl Spanned) {
        self.operations.push(ScenarioOp {
            span: self.span(expr),
            kind: ScenarioOpKind::RawNondeterminism {
                operation: operation.to_owned(),
            },
        });
    }

    fn uncontrolled_effect(&mut self, operation: &str, expr: &impl Spanned) {
        self.operations.push(ScenarioOp {
            span: self.span(expr),
            kind: ScenarioOpKind::UncontrolledEffect {
                operation: operation.to_owned(),
            },
        });
    }

    fn span(&self, node: &impl Spanned) -> KoboSpan {
        let span = self.ast.span_from_syn(node.span());
        if span.is_empty() {
            KoboSpan::new(span.start, span.start.saturating_add(1), span.file_id)
        } else {
            span
        }
    }
}

fn matches_bool_pat(pat: &Pat, value: Option<bool>) -> bool {
    match (pat, value) {
        (
            Pat::Lit(ExprLit {
                lit: Lit::Bool(candidate),
                ..
            }),
            Some(value),
        ) => candidate.value == value,
        (Pat::Wild(_), _) => true,
        _ => false,
    }
}

fn function_returns_bool_literal(function: &ItemFn) -> Option<bool> {
    function
        .block
        .stmts
        .iter()
        .find_map(|statement| match statement {
            Stmt::Expr(
                Expr::Lit(ExprLit {
                    lit: Lit::Bool(value),
                    ..
                }),
                _,
            ) => Some(value.value),
            _ => None,
        })
}

fn modeled_boundary(call: &ExprMethodCall) -> Option<ScenarioModeledBoundary> {
    if receiver_has_ward_member(call.receiver.as_ref(), "time") {
        return Some(ScenarioModeledBoundary::WardTime);
    }
    if receiver_has_ward_member(call.receiver.as_ref(), "random") {
        return Some(ScenarioModeledBoundary::WardRandom);
    }
    if receiver_has_ward_member(call.receiver.as_ref(), "task")
        || receiver_is_ward_method(call.receiver.as_ref(), "task")
    {
        return Some(ScenarioModeledBoundary::WardTask);
    }
    if receiver_is_ward_path(call.receiver.as_ref()) && call.method == "task" {
        return Some(ScenarioModeledBoundary::WardTask);
    }
    None
}

fn receiver_has_ward_member(expr: &Expr, member: &str) -> bool {
    match expr {
        Expr::Field(field) => {
            member_name(&field.member).as_deref() == Some(member)
                && receiver_is_ward_path(field.base.as_ref())
        }
        Expr::MethodCall(call) => receiver_has_ward_member(call.receiver.as_ref(), member),
        _ => false,
    }
}

fn receiver_is_ward_method(expr: &Expr, method: &str) -> bool {
    matches!(expr, Expr::MethodCall(call) if call.method == method && receiver_is_ward_path(call.receiver.as_ref()))
}

fn receiver_is_ward_path(expr: &Expr) -> bool {
    expr_path_ident(expr).as_deref() == Some("ward")
}

fn member_name(member: &syn::Member) -> Option<String> {
    match member {
        syn::Member::Named(ident) => Some(ident.to_string()),
        syn::Member::Unnamed(_) => None,
    }
}

fn must_call_actions(attr: &syn::Attribute) -> Option<Vec<String>> {
    let syn::Meta::List(list) = &attr.meta else {
        return None;
    };
    let actions = list
        .tokens
        .to_string()
        .split('|')
        .map(str::trim)
        .map(|part| part.trim_matches(','))
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    (!actions.is_empty()).then_some(actions)
}

fn pat_ident(pat: &Pat) -> Option<String> {
    match pat {
        Pat::Ident(PatIdent { ident, .. }) => Some(ident.to_string()),
        Pat::Type(PatType { pat, .. }) => pat_ident(pat),
        Pat::Wild(_) => None,
        _ => None,
    }
}

fn fn_arg_ident(arg: &syn::FnArg) -> Option<String> {
    match arg {
        syn::FnArg::Typed(pat_type) => pat_ident(pat_type.pat.as_ref()),
        syn::FnArg::Receiver(_) => None,
    }
}

fn expr_path_ident(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Path(path) => path_last_ident(&path.path),
        Expr::Reference(reference) => expr_path_ident(reference.expr.as_ref()),
        Expr::Paren(paren) => expr_path_ident(paren.expr.as_ref()),
        _ => None,
    }
}

fn receiver_ident(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Path(path) => path_last_ident(&path.path),
        Expr::Reference(reference) => receiver_ident(reference.expr.as_ref()),
        Expr::Paren(paren) => receiver_ident(paren.expr.as_ref()),
        _ => None,
    }
}

fn path_first_ident(path: &Path) -> Option<String> {
    path.segments
        .first()
        .map(|segment| segment.ident.to_string())
}

fn path_last_ident(path: &Path) -> Option<String> {
    path.segments
        .last()
        .map(|segment| segment.ident.to_string())
}

fn path_has_segment(path: &Path, segment: &str) -> bool {
    path.segments
        .iter()
        .any(|path_segment| path_segment.ident == segment)
}

fn path_ends_with(path: &Path, suffix: &[&str]) -> bool {
    let segments = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    if suffix.len() > segments.len() {
        return false;
    }
    segments[segments.len() - suffix.len()..]
        .iter()
        .zip(suffix)
        .all(|(segment, expected)| segment == expected)
}
