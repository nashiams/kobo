use std::collections::{HashMap, HashSet};

use kobo_ir::{
    KoboSpan, MustCallObligation, ScenarioBoundary, ScenarioBoundaryPolicy, ScenarioCallGraphScc,
    ScenarioCoverageFacts, ScenarioModeledBoundary, ScenarioOp, ScenarioOpKind, ScenarioProgram,
};
use kobo_parser::KoboFile;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{
    Block, Expr, ExprAsync, ExprCall, ExprIf, ExprLit, ExprMatch, ExprMethodCall, ExprPath,
    ExprStruct, File, Item, ItemFn, ItemUse, Lit, Local, Macro, MetaNameValue, Pat, PatIdent,
    PatType, Path, Stmt, UseTree,
};

type BindingMap = HashMap<String, String>;
type BoolMap = HashMap<String, bool>;
type ImportMap = HashMap<String, String>;
type BoundaryPolicyMap = HashMap<String, BoundaryPolicyFact>;
type FunctionMap<'a> = HashMap<String, &'a ItemFn>;
type FunctionSccMap = HashMap<String, usize>;

#[derive(Default)]
struct BindingEnv {
    bindings: BindingMap,
    bools: BoolMap,
}

#[derive(Clone, Debug)]
struct BoundaryPolicyFact {
    policy: ScenarioBoundaryPolicy,
    reason: Option<String>,
}

#[derive(Clone, Debug)]
struct FunctionScc {
    functions: Vec<String>,
    is_recursive: bool,
}

#[derive(Clone, Debug)]
struct ScenarioCallGraph {
    sccs: Vec<FunctionScc>,
    function_sccs: FunctionSccMap,
}

#[derive(Default)]
struct TarjanState {
    next_index: usize,
    stack: Vec<String>,
    indices: HashMap<String, usize>,
    lowlinks: HashMap<String, usize>,
    on_stack: HashSet<String>,
    components: Vec<Vec<String>>,
}

struct DirectCallVisitor<'a> {
    known_functions: &'a HashSet<String>,
    calls: Vec<String>,
}

struct ScenarioLowerer<'a> {
    ast: &'a KoboFile,
    must_call_types: &'a HashMap<String, Vec<String>>,
    functions: &'a FunctionMap<'a>,
    call_graph: &'a ScenarioCallGraph,
    imports: &'a ImportMap,
    boundary_policies: &'a BoundaryPolicyMap,
    operations: Vec<ScenarioOp>,
    coverage: ScenarioCoverageFacts,
    active_functions: Vec<String>,
}

pub fn build_scenario_programs(
    ast: &KoboFile,
    must_call_obligations: &[MustCallObligation],
    fallback_profile: &str,
) -> Vec<ScenarioProgram> {
    let _ = fallback_profile;
    let file = ast.syn_file();
    let functions = collect_functions(file);
    let call_graph = ScenarioCallGraph::build(&functions);
    let imports = collect_use_crate_aliases(file);
    let boundary_policies = collect_boundary_policies(file);
    let must_call_types = must_call_type_map(file, must_call_obligations);

    functions
        .values()
        .map(|function| {
            lower_function(
                ast,
                &functions,
                &call_graph,
                &imports,
                &boundary_policies,
                &must_call_types,
                function,
            )
        })
        .collect()
}

fn lower_function(
    ast: &KoboFile,
    functions: &FunctionMap<'_>,
    call_graph: &ScenarioCallGraph,
    imports: &ImportMap,
    boundary_policies: &BoundaryPolicyMap,
    must_call_types: &HashMap<String, Vec<String>>,
    function: &ItemFn,
) -> ScenarioProgram {
    let mut lowerer = ScenarioLowerer {
        ast,
        must_call_types,
        functions,
        call_graph,
        imports,
        boundary_policies,
        operations: Vec::new(),
        coverage: ScenarioCoverageFacts {
            call_graph_sccs: call_graph.coverage_facts(),
            ..ScenarioCoverageFacts::default()
        },
        active_functions: Vec::new(),
    };
    let mut env = BindingEnv::default();
    lowerer.execute_function(&function.sig.ident.to_string(), function, &mut env);
    lowerer.operations.push(ScenarioOp {
        span: KoboSpan::generated(ast.file_id),
        kind: ScenarioOpKind::Return,
    });
    let boundaries = boundaries_from_operations(&lowerer.operations);
    let operations = lowerer.operations;
    let coverage = lowerer.coverage;

    ScenarioProgram {
        file_id: ast.file_id,
        target: function.sig.ident.to_string(),
        source_hash: String::new(),
        operations,
        boundaries,
        coverage,
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

fn collect_use_crate_aliases(file: &File) -> ImportMap {
    let mut imports = HashMap::new();
    for item in &file.items {
        let Item::Use(item_use) = item else {
            continue;
        };
        collect_use_tree_aliases(&item_use.tree, None, &mut imports);
    }
    imports
}

fn collect_use_tree_aliases(tree: &UseTree, crate_name: Option<String>, imports: &mut ImportMap) {
    match tree {
        UseTree::Path(path) => {
            let root = crate_name.unwrap_or_else(|| path.ident.to_string());
            collect_use_tree_aliases(&path.tree, Some(root), imports);
        }
        UseTree::Name(name) => {
            if let Some(root) = crate_name {
                imports.insert(name.ident.to_string(), root);
            }
        }
        UseTree::Rename(rename) => {
            imports.insert(
                rename.rename.to_string(),
                crate_name.unwrap_or_else(|| rename.ident.to_string()),
            );
        }
        UseTree::Group(group) => {
            for tree in &group.items {
                collect_use_tree_aliases(tree, crate_name.clone(), imports);
            }
        }
        UseTree::Glob(_) => {}
    }
}

fn collect_boundary_policies(file: &File) -> BoundaryPolicyMap {
    let mut policies = HashMap::new();
    for item in &file.items {
        let Item::Use(item_use) = item else {
            continue;
        };
        if let Some(policy) = boundary_policy_from_use(item_use) {
            policies.insert(policy.0, policy.1);
        }
    }
    policies
}

fn boundary_policy_from_use(item_use: &ItemUse) -> Option<(String, BoundaryPolicyFact)> {
    item_use.attrs.iter().find_map(|attr| {
        if !path_ends_with(attr.path(), &["kobo", "boundary"]) {
            return None;
        }
        parse_boundary_attr(attr)
    })
}

fn parse_boundary_attr(attr: &syn::Attribute) -> Option<(String, BoundaryPolicyFact)> {
    let syn::Meta::List(list) = &attr.meta else {
        return None;
    };
    let entries = list
        .parse_args_with(Punctuated::<MetaNameValue, syn::Token![,]>::parse_terminated)
        .ok()?;
    let mut crate_name = None;
    let mut policy = ScenarioBoundaryPolicy::Unselected;
    let mut reason = None;

    for entry in entries {
        let Some(key) = path_last_ident(&entry.path) else {
            continue;
        };
        let syn::Expr::Lit(ExprLit {
            lit: Lit::Str(value),
            ..
        }) = entry.value
        else {
            continue;
        };
        match key.as_str() {
            "crate" => crate_name = Some(value.value()),
            "policy" => policy = ScenarioBoundaryPolicy::from_str(&value.value()),
            "reason" => reason = Some(value.value()),
            _ => {}
        }
    }

    let crate_name = crate_name?;
    Some((crate_name, BoundaryPolicyFact { policy, reason }))
}

impl ScenarioCallGraph {
    fn build(functions: &FunctionMap<'_>) -> Self {
        let edges = collect_call_graph_edges(functions);
        let mut state = TarjanState::default();
        let mut names = functions.keys().cloned().collect::<Vec<_>>();
        names.sort();
        for name in names {
            if !state.indices.contains_key(&name) {
                state.connect(&name, &edges);
            }
        }

        let mut sccs = state
            .components
            .into_iter()
            .map(|mut functions| {
                functions.sort();
                let is_recursive = functions.len() > 1
                    || functions
                        .first()
                        .and_then(|function| {
                            edges.get(function).map(|calls| calls.contains(function))
                        })
                        .unwrap_or(false);
                FunctionScc {
                    functions,
                    is_recursive,
                }
            })
            .collect::<Vec<_>>();
        sccs.sort_by(|left, right| left.functions.cmp(&right.functions));

        let mut function_sccs = HashMap::new();
        for (index, component) in sccs.iter().enumerate() {
            for function in &component.functions {
                function_sccs.insert(function.clone(), index);
            }
        }

        Self {
            sccs,
            function_sccs,
        }
    }

    fn coverage_facts(&self) -> Vec<ScenarioCallGraphScc> {
        self.sccs
            .iter()
            .map(|component| ScenarioCallGraphScc {
                functions: component.functions.clone(),
                is_recursive: component.is_recursive,
            })
            .collect()
    }

    fn is_recursive_function(&self, function: &str) -> bool {
        self.function_sccs
            .get(function)
            .and_then(|index| self.sccs.get(*index))
            .map(|component| component.is_recursive)
            .unwrap_or(false)
    }
}

impl TarjanState {
    fn connect(&mut self, node: &str, edges: &HashMap<String, Vec<String>>) {
        let node_index = self.next_index;
        self.next_index += 1;
        self.indices.insert(node.to_owned(), node_index);
        self.lowlinks.insert(node.to_owned(), node_index);
        self.stack.push(node.to_owned());
        self.on_stack.insert(node.to_owned());

        for callee in edges.get(node).into_iter().flatten() {
            if !self.indices.contains_key(callee) {
                self.connect(callee, edges);
                let child_lowlink = self.lowlinks.get(callee).copied().unwrap_or(node_index);
                let node_lowlink = self.lowlinks.get(node).copied().unwrap_or(node_index);
                self.lowlinks
                    .insert(node.to_owned(), node_lowlink.min(child_lowlink));
            } else if self.on_stack.contains(callee) {
                let callee_index = self.indices.get(callee).copied().unwrap_or(node_index);
                let node_lowlink = self.lowlinks.get(node).copied().unwrap_or(node_index);
                self.lowlinks
                    .insert(node.to_owned(), node_lowlink.min(callee_index));
            }
        }

        if self.indices.get(node) == self.lowlinks.get(node) {
            self.finish_component(node);
        }
    }

    fn finish_component(&mut self, root: &str) {
        let mut component = Vec::new();
        while let Some(function) = self.stack.pop() {
            self.on_stack.remove(&function);
            let is_root = function == root;
            component.push(function);
            if is_root {
                break;
            }
        }
        if !component.is_empty() {
            self.components.push(component);
        }
    }
}

impl<'ast> Visit<'ast> for DirectCallVisitor<'_> {
    fn visit_expr_call(&mut self, node: &'ast ExprCall) {
        if let Expr::Path(path) = node.func.as_ref() {
            if let Some(function_name) = path_last_ident(&path.path) {
                if self.known_functions.contains(&function_name) {
                    self.calls.push(function_name);
                }
            }
        }
        visit::visit_expr_call(self, node);
    }
}

fn collect_call_graph_edges(functions: &FunctionMap<'_>) -> HashMap<String, Vec<String>> {
    let known_functions = functions.keys().cloned().collect::<HashSet<_>>();
    let mut edges = HashMap::new();
    for (function_name, function) in functions {
        let mut visitor = DirectCallVisitor {
            known_functions: &known_functions,
            calls: Vec::new(),
        };
        visitor.visit_block(&function.block);
        visitor.calls.sort();
        visitor.calls.dedup();
        edges.insert(function_name.clone(), visitor.calls);
    }
    edges
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
    fn execute_function(
        &mut self,
        function_name: &str,
        function: &'a ItemFn,
        env: &mut BindingEnv,
    ) {
        self.active_functions.push(function_name.to_owned());
        self.execute_block(&function.block, env);
        let _ = self.active_functions.pop();
    }

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
            Expr::Async(expr_async) => self.execute_async(expr_async, env),
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
            let crate_name = self.external_crate_name(&path.path);
            let policy = self.boundary_policy_for(&crate_name);
            if !is_replay_owned_policy(&policy.policy) {
                self.record_opaque_boundary(&crate_name);
            }
            self.operations.push(ScenarioOp {
                span: self.span(call),
                kind: ScenarioOpKind::ExternalBoundary {
                    crate_name,
                    policy: policy.policy,
                    reason: policy.reason,
                },
            });
            return true;
        }
        false
    }

    fn execute_async(&mut self, expr_async: &'a ExprAsync, env: &mut BindingEnv) {
        self.execute_block(&expr_async.block, env);
    }

    fn external_crate_name(&self, path: &Path) -> String {
        let Some(first_ident) = path_first_ident(path) else {
            return "external".to_owned();
        };
        self.imports
            .get(&first_ident)
            .cloned()
            .unwrap_or(first_ident)
    }

    fn boundary_policy_for(&self, crate_name: &str) -> BoundaryPolicyFact {
        self.boundary_policies
            .get(crate_name)
            .cloned()
            .unwrap_or_else(|| BoundaryPolicyFact {
                policy: ScenarioBoundaryPolicy::Unselected,
                reason: None,
            })
    }

    fn record_opaque_boundary(&mut self, crate_name: &str) {
        if !self
            .coverage
            .opaque_boundaries
            .iter()
            .any(|boundary| boundary == crate_name)
        {
            self.coverage.opaque_boundaries.push(crate_name.to_owned());
        }
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

        if self.is_recursive_reentry(&function_name) {
            self.operations.push(ScenarioOp {
                span: self.span(call),
                kind: ScenarioOpKind::Loop,
            });
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

        self.execute_function(&function_name, function, &mut helper_env);
        true
    }

    fn is_recursive_reentry(&self, function_name: &str) -> bool {
        self.call_graph.is_recursive_function(function_name)
            && self
                .active_functions
                .iter()
                .any(|active_function| active_function == function_name)
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

    fn record_unsupported_construct(&mut self, label: &str) {
        if !self
            .coverage
            .unsupported_constructs
            .iter()
            .any(|construct| construct == label)
        {
            self.coverage.unsupported_constructs.push(label.to_owned());
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

fn boundaries_from_operations(operations: &[ScenarioOp]) -> Vec<ScenarioBoundary> {
    let mut boundaries = Vec::new();
    for operation in operations {
        let ScenarioOpKind::ExternalBoundary {
            crate_name, policy, ..
        } = &operation.kind
        else {
            continue;
        };
        if boundaries
            .iter()
            .any(|boundary: &ScenarioBoundary| boundary.name == *crate_name)
        {
            continue;
        }
        boundaries.push(ScenarioBoundary {
            span: operation.span,
            name: crate_name.clone(),
            decision: policy.as_str().to_owned(),
        });
    }
    boundaries
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

fn is_tokio_spawn(path: &Path) -> bool {
    path_ends_with(path, &["tokio", "spawn"])
}

fn is_replay_owned_policy(policy: &ScenarioBoundaryPolicy) -> bool {
    matches!(
        policy,
        ScenarioBoundaryPolicy::Model
            | ScenarioBoundaryPolicy::Record
            | ScenarioBoundaryPolicy::Stub
    )
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

fn select_branch_count(mac: &Macro) -> u32 {
    let branch_count = mac.tokens.to_string().matches("=>").count();
    branch_count.max(1) as u32
}
