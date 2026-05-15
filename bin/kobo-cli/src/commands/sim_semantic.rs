use std::collections::HashMap;

use syn::spanned::Spanned;
use syn::{
    Attribute, Block, Expr, ExprCall, ExprIf, ExprLit, ExprMethodCall, ExprPath, ExprStruct, File,
    Item, ItemFn, Lit, Local, Member, Meta, Pat, PatIdent, PatType, Path, Stmt, UseTree,
};

use super::sim_model::{
    ModeledBoundary, MustCallType, Scenario, ScenarioDocument, ScenarioOperation, ScenarioProgram,
};

pub(super) const MODEL_VERSION: &str = "v0.9-production-depth-1";

pub(super) fn parse_metadata(source: &str) -> Option<(Vec<MustCallType>, Vec<Scenario>)> {
    let file = syn::parse_file(source).ok()?;
    let must_call_types = semantic_must_call_types(&file);
    let scenarios = semantic_scenarios(source, &file);
    Some((must_call_types, scenarios))
}

pub(super) fn scenario_program_from_document(
    document: &ScenarioDocument,
    scenario: &Scenario,
) -> Option<ScenarioProgram> {
    let file = syn::parse_file(&document.source).ok()?;
    let mut functions = HashMap::new();
    let mut use_crate_heads = HashMap::new();
    for item in &file.items {
        match item {
            Item::Fn(function) => {
                functions.insert(function.sig.ident.to_string(), function);
            }
            Item::Use(item_use) => {
                collect_use_crate_heads(&item_use.tree, None, &mut use_crate_heads)
            }
            _ => {}
        }
    }
    let scenario_fn = functions.get(&scenario.name).copied()?;
    let must_call_types = document
        .must_call_types
        .iter()
        .map(|item| (item.type_name.clone(), item.actions.clone()))
        .collect::<HashMap<_, _>>();
    let mut lowerer = SemanticLowerer {
        source: &document.source,
        must_call_types,
        functions,
        use_crate_heads,
        operations: Vec::new(),
        call_depth: 0,
    };
    let mut env = BindingEnv::default();
    lowerer.execute_block(&scenario_fn.block, &mut env);
    Some(ScenarioProgram {
        operations: lowerer.operations,
    })
}

fn semantic_must_call_types(file: &File) -> Vec<MustCallType> {
    let mut types = Vec::new();
    for item in &file.items {
        if let Item::Struct(item_struct) = item {
            let actions = item_struct
                .attrs
                .iter()
                .find(|attr| attr_path_ends_with(attr, &["kobo", "must_call"]))
                .and_then(must_call_actions)
                .unwrap_or_default();
            if !actions.is_empty() {
                types.push(MustCallType {
                    type_name: item_struct.ident.to_string(),
                    actions,
                });
            }
        }
    }
    types
}

fn semantic_scenarios(source: &str, file: &File) -> Vec<Scenario> {
    file.items
        .iter()
        .filter_map(|item| {
            let Item::Fn(function) = item else {
                return None;
            };
            let attr = function
                .attrs
                .iter()
                .find(|attr| attr_path_ends_with(attr, &["kobo", "scenario"]))?;
            let profile = attr_named_string(attr, "profile").unwrap_or_else(|| "async".to_owned());
            let name =
                attr_named_string(attr, "name").unwrap_or_else(|| function.sig.ident.to_string());
            let (body_start, body_end) = span_bounds(source, &function.block);
            let body = source
                .get(body_start..body_end)
                .unwrap_or_default()
                .to_owned();
            Some(Scenario {
                name,
                profile,
                body,
                body_start,
            })
        })
        .collect()
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

#[derive(Default)]
struct BindingEnv {
    bindings: HashMap<String, String>,
}

impl BindingEnv {
    fn bind(&mut self, local: String, obligation_key: String) {
        self.bindings.insert(local, obligation_key);
    }

    fn resolve(&self, local: &str) -> Option<String> {
        self.bindings.get(local).cloned()
    }
}

struct SemanticLowerer<'a> {
    source: &'a str,
    must_call_types: HashMap<String, Vec<String>>,
    functions: HashMap<String, &'a ItemFn>,
    use_crate_heads: HashMap<String, String>,
    operations: Vec<ScenarioOperation>,
    call_depth: usize,
}

impl<'a> SemanticLowerer<'a> {
    fn execute_block(&mut self, block: &'a Block, env: &mut BindingEnv) {
        for stmt in &block.stmts {
            self.execute_stmt(stmt, env);
        }
    }

    fn execute_stmt(&mut self, stmt: &'a Stmt, env: &mut BindingEnv) {
        match stmt {
            Stmt::Local(local) => self.execute_local(local, env),
            Stmt::Expr(expr, _) => self.execute_expr(expr, env),
            Stmt::Item(_) | Stmt::Macro(_) => {}
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
            _ => {}
        }
    }

    fn execute_if(&mut self, expr_if: &'a ExprIf, env: &mut BindingEnv) {
        match self.eval_bool(expr_if.cond.as_ref()) {
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
            && path_has_segment(&path.path, "process")
        {
            self.uncontrolled_effect("std::process", call);
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
            let Some(arg_binding) = expr_path_ident(arg).and_then(|name| env.resolve(&name)) else {
                continue;
            };
            helper_env.bind(param, arg_binding);
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
            let action = call.method.to_string();
            self.operations
                .push(ScenarioOperation::Discharge { binding, action });
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

    fn eval_bool(&self, expr: &'a Expr) -> Option<bool> {
        match expr {
            Expr::Lit(ExprLit {
                lit: Lit::Bool(value),
                ..
            }) => Some(value.value),
            Expr::Paren(paren) => self.eval_bool(paren.expr.as_ref()),
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
    match expr {
        Expr::MethodCall(call) => {
            call.method == method && receiver_is_ward_path(call.receiver.as_ref())
        }
        _ => false,
    }
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

fn span_bounds(source: &str, node: &impl Spanned) -> (usize, usize) {
    let span = node.span();
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
