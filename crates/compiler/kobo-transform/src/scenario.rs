use std::collections::{HashMap, HashSet};

use kobo_ir::{
    KoboSpan, MustCallObligation, ScenarioBoundary, ScenarioBoundaryCallArgument,
    ScenarioBoundaryPolicy, ScenarioCallGraphScc, ScenarioCoverageFacts, ScenarioExternalCallShape,
    ScenarioModeledBoundary, ScenarioOp, ScenarioOpKind, ScenarioProgram,
};
use kobo_parser::KoboFile;
use quote::ToTokens;
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
type ExternalBindingMap = HashMap<String, ExternalBoundaryValue>;
type ImportMap = HashMap<String, Vec<String>>;
type BoundaryPolicyMap = HashMap<String, BoundaryPolicyFact>;
type FunctionMap<'a> = HashMap<String, &'a ItemFn>;
type FunctionSccMap = HashMap<String, usize>;

#[derive(Default)]
struct BindingEnv {
    bindings: BindingMap,
    bools: BoolMap,
    external_values: ExternalBindingMap,
    imports: ImportMap,
}

#[derive(Clone, Debug)]
struct ExternalBoundaryValue {
    crate_name: String,
    type_path: String,
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

struct InferredLifecycleCreation {
    binding: String,
    type_name: String,
    actions: Vec<String>,
    span: KoboSpan,
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
    let mut env = BindingEnv::with_imports(imports.clone());
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
        collect_use_tree_aliases(&item_use.tree, Vec::new(), &mut imports);
    }
    imports
}

fn collect_use_tree_aliases(tree: &UseTree, prefix: Vec<String>, imports: &mut ImportMap) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            collect_use_tree_aliases(&path.tree, next, imports);
        }
        UseTree::Name(name) => {
            let mut full_path = prefix;
            full_path.push(name.ident.to_string());
            imports.insert(name.ident.to_string(), full_path);
        }
        UseTree::Rename(rename) => {
            let mut full_path = prefix;
            full_path.push(rename.ident.to_string());
            imports.insert(rename.rename.to_string(), full_path);
        }
        UseTree::Group(group) => {
            for tree in &group.items {
                collect_use_tree_aliases(tree, prefix.clone(), imports);
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
    fn with_imports(imports: ImportMap) -> Self {
        Self {
            imports,
            ..Self::default()
        }
    }

    fn bind(&mut self, local: String, obligation_key: String) {
        self.bindings.insert(local, obligation_key);
    }

    fn bind_bool(&mut self, local: String, value: bool) {
        self.bools.insert(local, value);
    }

    fn bind_external(&mut self, local: String, value: ExternalBoundaryValue) {
        self.external_values.insert(local, value);
    }

    fn resolve(&self, local: &str) -> Option<String> {
        self.bindings.get(local).cloned()
    }

    fn resolve_bool(&self, local: &str) -> Option<bool> {
        self.bools.get(local).copied()
    }

    fn resolve_external(&self, local: &str) -> Option<&ExternalBoundaryValue> {
        self.external_values.get(local)
    }

    fn bind_imports_from_use(&mut self, item_use: &ItemUse) {
        collect_use_tree_aliases(&item_use.tree, Vec::new(), &mut self.imports);
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
        self.seed_handler_obligations(function, env);
        self.execute_block(&function.block, env);
        let _ = self.active_functions.pop();
    }

    fn seed_handler_obligations(&mut self, function: &'a ItemFn, env: &mut BindingEnv) {
        if !function
            .attrs
            .iter()
            .any(|attr| path_ends_with(attr.path(), &["kobo", "handler"]))
        {
            return;
        }

        for input in &function.sig.inputs {
            let syn::FnArg::Typed(argument) = input else {
                continue;
            };
            if !is_handler_obligation_argument(argument) {
                continue;
            }
            let Some(binding) = pat_ident(argument.pat.as_ref()) else {
                continue;
            };
            env.bind(binding.clone(), binding.clone());
            self.operations.push(ScenarioOp {
                span: self.span(argument),
                kind: ScenarioOpKind::CreateObligation {
                    binding,
                    type_name: "HandlerReply".to_owned(),
                    actions: handler_reply_actions(),
                },
            });
        }
    }

    fn execute_block(&mut self, block: &'a Block, env: &mut BindingEnv) {
        let saved_imports = env.imports.clone();
        for statement in &block.stmts {
            if let Stmt::Item(Item::Use(item_use)) = statement {
                env.bind_imports_from_use(item_use);
                continue;
            }
            self.execute_statement(statement, env);
        }
        env.imports = saved_imports;
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
        if let Some(creation) = self.local_lifecycle_creation(local, init.expr.as_ref()) {
            env.bind(creation.binding.clone(), creation.binding.clone());
            self.operations.push(ScenarioOp {
                span: creation.span,
                kind: ScenarioOpKind::CreateObligation {
                    binding: creation.binding,
                    type_name: creation.type_name,
                    actions: creation.actions,
                },
            });
            self.execute_expr(init.expr.as_ref(), env);
            return;
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
        if let Some(binding) = pat_ident(&local.pat) {
            if let Some(value) = self.record_external_boundary_expr(init.expr.as_ref(), env) {
                env.bind_external(binding, value);
                return;
            }
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

    fn local_lifecycle_creation(
        &self,
        local: &'a Local,
        expr: &'a Expr,
    ) -> Option<InferredLifecycleCreation> {
        let binding = pat_ident(&local.pat)?;
        if tokio_spawn_call(expr).is_some() {
            return Some(InferredLifecycleCreation {
                binding,
                type_name: "SpawnedTask".to_owned(),
                actions: spawned_task_actions(),
                span: self.span(expr),
            });
        }

        let call = lifecycle_method_call(expr)?;
        match call.method.to_string().as_str() {
            "recv" => Some(InferredLifecycleCreation {
                binding,
                type_name: "Delivery".to_owned(),
                actions: queue_delivery_actions(),
                span: self.span(call),
            }),
            "begin" => Some(InferredLifecycleCreation {
                binding,
                type_name: "Transaction".to_owned(),
                actions: transaction_actions(),
                span: self.span(call),
            }),
            _ => None,
        }
    }

    fn execute_expr(&mut self, expr: &'a Expr, env: &mut BindingEnv) {
        match expr {
            Expr::MethodCall(call) => self.execute_method_call(call, env),
            Expr::Call(call) => self.execute_call(call, env),
            Expr::If(expr_if) => self.execute_if(expr_if, env),
            Expr::Match(expr_match) => self.execute_match(expr_match, env),
            Expr::Async(expr_async) => self.execute_async(expr_async, env),
            Expr::Await(await_expr) => {
                if let Some(binding) =
                    expr_path_ident(await_expr.base.as_ref()).and_then(|name| env.resolve(&name))
                {
                    self.operations.push(ScenarioOp {
                        span: self.span(expr),
                        kind: ScenarioOpKind::Discharge {
                            binding,
                            action: "join".to_owned(),
                        },
                    });
                    return;
                }
                self.execute_expr(await_expr.base.as_ref(), env);
            }
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

    fn record_external_boundary_expr(
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

    fn record_external_path_call(
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

    fn record_external_method_call(
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

    fn record_external_boundary_operation(
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
    }

    fn execute_async(&mut self, expr_async: &'a ExprAsync, env: &mut BindingEnv) {
        self.execute_block(&expr_async.block, env);
    }

    fn external_crate_name(&self, path: &Path, env: &BindingEnv) -> String {
        let Some(first_ident) = path_first_ident(path) else {
            return "external".to_owned();
        };
        self.lookup_import(env, &first_ident)
            .and_then(|path| path.first().cloned())
            .unwrap_or(first_ident)
    }

    fn resolved_path_segments(&self, path: &Path, env: &BindingEnv) -> Vec<String> {
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

    fn external_call_path(&self, path: &Path, crate_name: &str, env: &BindingEnv) -> String {
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

    fn external_call_shape(&self, path: &Path, env: &BindingEnv) -> ScenarioExternalCallShape {
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

    fn imported_external_crate(&self, path: &Path, env: &BindingEnv) -> Option<String> {
        let first_ident = path_first_ident(path)?;
        let import_path = self.lookup_import(env, &first_ident)?;
        let crate_name = import_path.first()?;
        (!matches!(
            crate_name.as_str(),
            "std" | "core" | "alloc" | "crate" | "self" | "super" | "kobo" | "ward"
        ))
        .then(|| crate_name.clone())
    }

    fn lookup_import<'b>(&'b self, env: &'b BindingEnv, local: &str) -> Option<&'b Vec<String>> {
        env.imports.get(local).or_else(|| self.imports.get(local))
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

    fn record_opaque_boundary(&mut self, boundary: &str) {
        if !self
            .coverage
            .opaque_boundaries
            .iter()
            .any(|existing| existing == boundary)
        {
            self.coverage.opaque_boundaries.push(boundary.to_owned());
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

        let mut helper_env = BindingEnv::with_imports(self.imports.clone());
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
                helper_env.bind_bool(parameter.clone(), value);
            }
            if let Some(external_value) =
                expr_path_ident(argument).and_then(|name| env.resolve_external(&name).cloned())
            {
                helper_env.bind_external(parameter, external_value);
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
                    action: terminal_action_name(&call.method.to_string()),
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
        if self.record_external_method_call(call, env).is_some() {
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
            span: self.operation_span(expr, operation),
            kind: ScenarioOpKind::RawNondeterminism {
                operation: operation.to_owned(),
            },
        });
    }

    fn uncontrolled_effect(&mut self, operation: &str, expr: &impl Spanned) {
        self.operations.push(ScenarioOp {
            span: self.operation_span(expr, operation),
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

    fn operation_span(&self, node: &impl Spanned, needle: &str) -> KoboSpan {
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

fn nearest_occurrence(source: &str, needle: &str, anchor: usize) -> Option<usize> {
    source
        .match_indices(needle)
        .map(|(index, _)| index)
        .min_by_key(|index| index.abs_diff(anchor))
}

fn imported_path_looks_like_module(import_path: &[String]) -> bool {
    import_path
        .last()
        .is_some_and(|segment| starts_with_module_identifier(segment))
}

fn starts_with_module_identifier(segment: &str) -> bool {
    segment
        .chars()
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_lowercase())
}

fn boundaries_from_operations(operations: &[ScenarioOp]) -> Vec<ScenarioBoundary> {
    let mut boundaries = Vec::new();
    for operation in operations {
        let ScenarioOpKind::ExternalBoundary {
            crate_name,
            call_path,
            policy,
            ..
        } = &operation.kind
        else {
            continue;
        };
        let boundary_name = call_path.as_ref().unwrap_or(crate_name);
        if boundaries
            .iter()
            .any(|boundary: &ScenarioBoundary| boundary.name == *boundary_name)
        {
            continue;
        }
        boundaries.push(ScenarioBoundary {
            span: operation.span,
            name: boundary_name.clone(),
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

fn boundary_call_arguments(
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

fn associated_return_type_path(call_path: &str) -> Option<String> {
    let mut segments = call_path
        .split("::")
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    (segments.len() >= 2).then(|| {
        let _method = segments.pop();
        segments.join("::")
    })
}

fn external_method_return_type(crate_name: &str, receiver_type: &str, method: &str) -> String {
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

fn tokio_spawn_call(expr: &Expr) -> Option<&ExprCall> {
    match expr {
        Expr::Call(call) => match call.func.as_ref() {
            Expr::Path(path) if is_tokio_spawn(&path.path) => Some(call),
            _ => None,
        },
        Expr::Paren(paren) => tokio_spawn_call(paren.expr.as_ref()),
        _ => None,
    }
}

fn lifecycle_method_call(expr: &Expr) -> Option<&ExprMethodCall> {
    match expr {
        Expr::MethodCall(call) => Some(call),
        Expr::Await(await_expr) => lifecycle_method_call(await_expr.base.as_ref()),
        Expr::Paren(paren) => lifecycle_method_call(paren.expr.as_ref()),
        _ => None,
    }
}

fn queue_delivery_actions() -> Vec<String> {
    ["ack", "nack", "requeue"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn transaction_actions() -> Vec<String> {
    ["commit", "rollback"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn handler_reply_actions() -> Vec<String> {
    ["reply", "reject", "cancel"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn spawned_task_actions() -> Vec<String> {
    ["join", "abort", "detach-with-policy"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn terminal_action_name(method: &str) -> String {
    match method {
        "detach_with_policy" => "detach-with-policy".to_owned(),
        other => other.to_owned(),
    }
}

fn is_replay_owned_policy(policy: &ScenarioBoundaryPolicy) -> bool {
    matches!(
        policy,
        ScenarioBoundaryPolicy::Model
            | ScenarioBoundaryPolicy::Record
            | ScenarioBoundaryPolicy::Activity
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

fn is_handler_obligation_argument(argument: &PatType) -> bool {
    handler_argument_type_name(argument)
        .as_deref()
        .is_some_and(|type_name| !is_non_obligation_handler_type(type_name))
}

fn handler_argument_type_name(argument: &PatType) -> Option<String> {
    let syn::Type::Path(type_path) = argument.ty.as_ref() else {
        return None;
    };
    type_path
        .path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

fn is_non_obligation_handler_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "bool"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "f32"
            | "f64"
            | "String"
            | "str"
    )
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

fn path_starts_with(path: &[String], prefix: &[&str]) -> bool {
    path.len() >= prefix.len()
        && path
            .iter()
            .zip(prefix.iter())
            .all(|(segment, expected)| segment == expected)
}

fn path_ends_with_segments(path: &[String], suffix: &[&str]) -> bool {
    path.len() >= suffix.len()
        && path[path.len() - suffix.len()..]
            .iter()
            .zip(suffix.iter())
            .all(|(segment, expected)| segment == expected)
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

#[cfg(test)]
mod tests {
    use super::*;
    use kobo_ir::{FileId, NodeIdGen};
    use kobo_parser::parse_file;

    #[test]
    fn infers_normal_lifecycle_templates_from_scenario_ast() {
        let source = r#"
struct Queue {}
struct Delivery {}
struct Db {}
struct Tx {}
struct Request {}

#[kobo::handler]
#[kobo::scenario(profile = "async")]
async fn service(request: Request) {
    let message = Queue {}.recv().await;
    message.ack();

    let tx = Db {}.begin();
    tx.commit();

    let task = tokio::spawn(async {});
    task.abort();

    request.reply();
}
"#;
        let mut id_gen = NodeIdGen::new();
        let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
        let program = build_scenario_programs(&ast, &[], "async")
            .into_iter()
            .find(|program| program.target == "service")
            .expect("service scenario should lower");

        let creates = program
            .operations
            .iter()
            .filter_map(|operation| match &operation.kind {
                ScenarioOpKind::CreateObligation {
                    binding,
                    type_name,
                    actions,
                } => Some((binding.as_str(), type_name.as_str(), actions.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();
        for (binding, type_name, actions) in [
            ("request", "HandlerReply", handler_reply_actions()),
            ("message", "Delivery", queue_delivery_actions()),
            ("tx", "Transaction", transaction_actions()),
            ("task", "SpawnedTask", spawned_task_actions()),
        ] {
            assert!(
                creates
                    .iter()
                    .any(|candidate| candidate == &(binding, type_name, actions.clone())),
                "missing inferred obligation {binding}/{type_name}/{actions:?} in {creates:?}"
            );
        }

        let discharges = program
            .operations
            .iter()
            .filter_map(|operation| match &operation.kind {
                ScenarioOpKind::Discharge { binding, action } => {
                    Some((binding.as_str(), action.as_str()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for discharge in [
            ("request", "reply"),
            ("message", "ack"),
            ("tx", "commit"),
            ("task", "abort"),
        ] {
            assert!(
                discharges.contains(&discharge),
                "missing lifecycle discharge {discharge:?} in {discharges:?}"
            );
        }
    }
}
