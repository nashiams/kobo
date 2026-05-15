use std::collections::HashMap;

use kobo_ir::{FileId, NodeIdGen};
use kobo_parser::parse_file;
use proc_macro2::Span;
use syn::spanned::Spanned;
use syn::{
    Attribute, Block, Expr, ExprCall, ExprIf, ExprLit, ExprMatch, ExprMethodCall,
    ExprPath, ExprStruct, File, Item, ItemFn, Lit, Local, Macro, Member, Meta, Pat, PatIdent,
    PatType, Path, Stmt, UseTree,
};

use crate::core::{
    LoweredScenario, ModeledBoundary, ScenarioCoverage, ScenarioOperation,
};

pub const MODEL_VERSION: &str = "v0.9-driver-kir-full-depth-1";

pub fn lower_from_parser(
    source: &str,
    target: &str,
    fallback_profile: &str,
) -> anyhow::Result<LoweredScenario> {
    let mut id_gen = NodeIdGen::new();
    let parsed = parse_file(source, FileId(0), &mut id_gen)
        .map_err(|error| anyhow::anyhow!("failed to parse Kobo source for scenario semantics: {error}"))?;
    let file = parsed.syn_file();
    let mut functions = HashMap::new();
    let mut use_crate_heads = HashMap::new();
    let mut must_call_types = HashMap::new();

    for item in &file.items {
        match item {
            Item::Fn(function) => {
                functions.insert(function.sig.ident.to_string(), function);
            }
            Item::Struct(item_struct) => {
                if let Some(actions) = item_struct
                    .attrs
                    .iter()
                    .find(|attr| attr_path_ends_with(attr, &["kobo", "must_call"]))
                    .and_then(must_call_actions)
                {
                    must_call_types.insert(item_struct.ident.to_string(), actions);
                }
            }
            Item::Use(item_use) => {
                collect_use_crate_heads(&item_use.tree, None, &mut use_crate_heads);
            }
            _ => {}
        }
    }

    let scenario = functions
        .get(target)
        .copied()
        .or_else(|| first_scenario(file))
        .ok_or_else(|| anyhow::anyhow!("no #[kobo::scenario] function found"))?;
    let profile = scenario
        .attrs
        .iter()
        .find(|attr| attr_path_ends_with(attr, &["kobo", "scenario"]))
        .and_then(|attr| attr_named_string(attr, "profile"))
        .unwrap_or_else(|| fallback_profile.to_owned());
    let mut lowerer = Lowerer {
        source,
        must_call_types,
        functions,
        use_crate_heads,
        operations: Vec::new(),
        unsupported_constructs: Vec::new(),
        call_depth: 0,
    };
    let mut env = BindingEnv::default();
    lowerer.execute_block(&scenario.block, &mut env);
    Ok(LoweredScenario {
        profile,
        operations: lowerer.operations,
        coverage: ScenarioCoverage {
            unsupported_constructs: lowerer.unsupported_constructs,
            reason: None,
        },
    })
}

fn first_scenario<'a>(file: &'a File) -> Option<&'a ItemFn> {
    file.items.iter().find_map(|item| {
        let Item::Fn(function) = item else {
            return None;
        };
        function
            .attrs
            .iter()
            .any(|attr| attr_path_ends_with(attr, &["kobo", "scenario"]))
            .then_some(function)
    })
}

#[derive(Default)]
struct BindingEnv {
    bindings: HashMap<String, String>,
    bools: HashMap<String, bool>,
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

struct Lowerer<'a> {
    source: &'a str,
    must_call_types: HashMap<String, Vec<String>>,
    functions: HashMap<String, &'a ItemFn>,
    use_crate_heads: HashMap<String, String>,
    operations: Vec<ScenarioOperation>,
    unsupported_constructs: Vec<String>,
    call_depth: usize,
}

impl<'a> Lowerer<'a> {
    fn execute_block(&mut self, block: &'a Block, env: &mut BindingEnv) {
        for stmt in &block.stmts {
            self.execute_stmt(stmt, env);
        }
    }

    fn execute_stmt(&mut self, stmt: &'a Stmt, env: &mut BindingEnv) {
        match stmt {
            Stmt::Local(local) => self.execute_local(local, env),
            Stmt::Expr(expr, _) => self.execute_expr(expr, env),
            Stmt::Item(_) => {}
            Stmt::Macro(stmt_macro) => self.unsupported_macro(&stmt_macro.mac),
        }
    }

    fn execute_local(&mut self, local: &'a Local, env: &mut BindingEnv) {
        let Some(init) = &local.init else {
            return;
        };
        if let Some((binding, type_name, span)) =
            self.local_must_call_creation(local, init.expr.as_ref())
        {
            let Some(actions) = self.must_call_types.get(&type_name).cloned() else {
                self.execute_expr(init.expr.as_ref(), env);
                return;
            };
            env.bind(binding.clone(), binding.clone());
            self.operations.push(ScenarioOperation::CreateObligation {
                binding,
                type_name,
                actions,
                span_start: span.0,
                span_end: span.1,
            });
            return;
        }
        if let Some(binding) =
            expr_path_ident(init.expr.as_ref()).and_then(|name| env.resolve(&name))
        {
            let span = span_bounds(self.source, init.expr.as_ref());
            self.operations.push(ScenarioOperation::MoveBinding {
                binding,
                span_start: span.0,
                span_end: span.1,
            });
            return;
        }
        if let Some((binding, value)) = pat_ident(&local.pat).zip(self.eval_bool(init.expr.as_ref(), env)) {
            env.bind_bool(binding, value);
            return;
        }
        self.execute_expr(init.expr.as_ref(), env);
    }

    fn local_must_call_creation(
        &self,
        local: &'a Local,
        expr: &'a Expr,
    ) -> Option<(String, String, (usize, usize))> {
        let binding = pat_ident(&local.pat)?;
        let Expr::Struct(ExprStruct { path, .. }) = expr else {
            return None;
        };
        let type_name = path_last_ident(path)?;
        self.must_call_types.contains_key(&type_name).then(|| {
            let span = span_bounds(self.source, expr);
            (binding, type_name, span)
        })
    }

    fn execute_expr(&mut self, expr: &'a Expr, env: &mut BindingEnv) {
        match expr {
            Expr::MethodCall(call) => self.execute_method_call(call, env),
            Expr::Call(call) => self.execute_call(call, env),
            Expr::If(expr_if) => self.execute_if(expr_if, env),
            Expr::Match(expr_match) => self.execute_match(expr_match, env),
            Expr::Await(await_expr) => self.execute_expr(await_expr.base.as_ref(), env),
            Expr::Block(block) => self.execute_block(&block.block, env),
            Expr::Loop(expr_loop) => {
                let span = span_bounds(self.source, expr_loop);
                self.operations.push(ScenarioOperation::Loop {
                    span_start: span.0,
                    span_end: span.1,
                });
            }
            Expr::Unsafe(expr_unsafe) => self.execute_block(&expr_unsafe.block, env),
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
            if self.record_known_call(path, call, env) {
                return;
            }
            if self.execute_helper_call(path, call, env) {
                return;
            }
        }
        for arg in &call.args {
            self.execute_expr(arg, env);
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
                let span = span_bounds(self.source, call);
                self.operations.push(ScenarioOperation::MoveBinding {
                    binding,
                    span_start: span.0,
                    span_end: span.1,
                });
                return true;
            }
        }
        if path_ends_with(&path.path, &["SystemTime", "now"]) {
            self.raw_nondeterminism("SystemTime::now", call);
            return true;
        }
        if path_ends_with(&path.path, &["Instant", "now"]) {
            self.raw_nondeterminism("Instant::now", call);
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
            let crate_name = self.external_crate_name(&path.path);
            let span = span_bounds(self.source, call);
            self.operations.push(ScenarioOperation::ExternalBoundary {
                crate_name,
                span_start: span.0,
                span_end: span.1,
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
            let span = span_bounds(self.source, call);
            self.operations.push(ScenarioOperation::UncontrolledEffect {
                operation: "helper-call recursion depth exceeded".to_owned(),
                span_start: span.0,
                span_end: span.1,
            });
            return true;
        }

        let mut helper_env = BindingEnv::default();
        for (input, arg) in function.sig.inputs.iter().zip(call.args.iter()) {
            let Some(param) = fn_arg_ident(input) else {
                continue;
            };
            if let Some(arg_binding) = expr_path_ident(arg).and_then(|name| env.resolve(&name)) {
                helper_env.bind(param.clone(), arg_binding);
            }
            if let Some(value) = self.eval_bool(arg, env) {
                helper_env.bind_bool(param, value);
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
            self.operations.push(ScenarioOperation::Discharge {
                binding,
                action: call.method.to_string(),
            });
            return;
        }
        if let Some(boundary) = modeled_boundary(call) {
            let span = span_bounds(self.source, call);
            self.operations.push(ScenarioOperation::ModeledEffect {
                boundary,
                span_start: span.0,
                span_end: span.1,
            });
            return;
        }
        self.execute_expr(call.receiver.as_ref(), env);
        for arg in &call.args {
            self.execute_expr(arg, env);
        }
    }

    fn eval_bool(&self, expr: &'a Expr, env: &BindingEnv) -> Option<bool> {
        match expr {
            Expr::Lit(ExprLit {
                lit: Lit::Bool(value),
                ..
            }) => Some(value.value),
            Expr::Path(path) => path_last_ident(&path.path).and_then(|name| env.resolve_bool(&name)),
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
        let name = mac.path.segments.iter().map(|segment| segment.ident.to_string()).collect::<Vec<_>>().join("::");
        let label = if name.ends_with("select") {
            format!("{name}!")
        } else {
            format!("macro:{name}")
        };
        if !self.unsupported_constructs.contains(&label) {
            self.unsupported_constructs.push(label);
        }
    }

    fn raw_nondeterminism(&mut self, operation: &str, expr: &impl Spanned) {
        let span = span_bounds(self.source, expr);
        self.operations.push(ScenarioOperation::RawNondeterminism {
            operation: operation.to_owned(),
            span_start: span.0,
            span_end: span.1,
        });
    }

    fn uncontrolled_effect(&mut self, operation: &str, expr: &impl Spanned) {
        let span = span_bounds(self.source, expr);
        self.operations.push(ScenarioOperation::UncontrolledEffect {
            operation: operation.to_owned(),
            span_start: span.0,
            span_end: span.1,
        });
    }

    fn external_crate_name(&self, path: &Path) -> String {
        let first = path_first_ident(path).unwrap_or_else(|| "external".to_owned());
        if first == "Client" {
            return self
                .use_crate_heads
                .get("Client")
                .cloned()
                .unwrap_or_else(|| "Client".to_owned());
        }
        first
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
    function.block.stmts.iter().find_map(|stmt| match stmt {
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

fn modeled_boundary(call: &ExprMethodCall) -> Option<ModeledBoundary> {
    if receiver_has_ward_member(call.receiver.as_ref(), "time") {
        return Some(ModeledBoundary::WardTime);
    }
    if receiver_has_ward_member(call.receiver.as_ref(), "random") {
        return Some(ModeledBoundary::WardRandom);
    }
    if receiver_has_ward_member(call.receiver.as_ref(), "task")
        || receiver_is_ward_method(call.receiver.as_ref(), "task")
    {
        return Some(ModeledBoundary::WardTask);
    }
    if receiver_is_ward_path(call.receiver.as_ref()) && call.method == "task" {
        return Some(ModeledBoundary::WardTask);
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

fn member_name(member: &Member) -> Option<String> {
    match member {
        Member::Named(ident) => Some(ident.to_string()),
        Member::Unnamed(_) => None,
    }
}

fn attr_path_ends_with(attr: &Attribute, suffix: &[&str]) -> bool {
    path_ends_with(attr.path(), suffix)
}

fn must_call_actions(attr: &Attribute) -> Option<Vec<String>> {
    let Meta::List(list) = &attr.meta else {
        return None;
    };
    let text = list.tokens.to_string();
    let actions = text
        .split('|')
        .map(str::trim)
        .map(|part| part.trim_matches(','))
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    (!actions.is_empty()).then_some(actions)
}

fn attr_named_string(attr: &Attribute, key: &str) -> Option<String> {
    let Meta::List(list) = &attr.meta else {
        return None;
    };
    let text = list.tokens.to_string();
    let key_start = text.find(key)?;
    let after_key = &text[key_start + key.len()..];
    let quote_start = after_key.find('"')?;
    let rest = &after_key[quote_start + 1..];
    let quote_end = rest.find('"')?;
    Some(rest[..quote_end].to_owned())
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
    path.segments.first().map(|segment| segment.ident.to_string())
}

fn path_last_ident(path: &Path) -> Option<String> {
    path.segments.last().map(|segment| segment.ident.to_string())
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

fn span_bounds(source: &str, node: &impl Spanned) -> (usize, usize) {
    let span = node.span();
    span_to_bounds(source, span)
}

fn span_to_bounds(source: &str, span: Span) -> (usize, usize) {
    let start = line_col_to_offset(source, span.start().line, span.start().column).unwrap_or(0);
    let end = line_col_to_offset(source, span.end().line, span.end().column)
        .unwrap_or_else(|| (start + 1).min(source.len()));
    (start, end.max(start + 1).min(source.len()))
}

fn line_col_to_offset(source: &str, line: usize, column: usize) -> Option<usize> {
    if line == 0 {
        return None;
    }
    let mut offset = 0usize;
    for (index, text) in source.lines().enumerate() {
        if index + 1 == line {
            return Some((offset + column).min(source.len()));
        }
        offset += text.len() + 1;
    }
    None
}

fn collect_use_crate_heads(
    tree: &UseTree,
    crate_head: Option<String>,
    output: &mut HashMap<String, String>,
) {
    match tree {
        UseTree::Path(path) => {
            let head = crate_head.unwrap_or_else(|| path.ident.to_string());
            collect_use_crate_heads(&path.tree, Some(head), output);
        }
        UseTree::Name(name) => {
            if let Some(head) = crate_head {
                output.insert(name.ident.to_string(), head);
            }
        }
        UseTree::Rename(rename) => {
            if let Some(head) = crate_head {
                output.insert(rename.rename.to_string(), head);
            }
        }
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use_crate_heads(item, crate_head.clone(), output);
            }
        }
        UseTree::Glob(_) => {}
    }
}
