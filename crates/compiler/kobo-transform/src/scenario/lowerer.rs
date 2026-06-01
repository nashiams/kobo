use super::{
    boundaries_from_operations, collect_boundary_policies, collect_functions,
    collect_method_shapes, collect_use_crate_aliases, handler_reply_actions,
    is_handler_obligation_argument, must_call_type_map, pat_ident, path_ends_with, BindingEnv,
    Block, BlockFlow, BoundaryPolicyMap, FunctionMap, HashMap, ImportMap, Item, ItemFn, KoboFile,
    MethodShapeMap, MustCallObligation, ScenarioCallGraph, ScenarioCoverageFacts,
    ScenarioLifecycleTemplate, ScenarioLowerer, ScenarioOp, ScenarioOpKind, ScenarioProgram, Stmt,
};
pub fn build_scenario_programs(
    ast: &KoboFile,
    must_call_obligations: &[MustCallObligation],
) -> Vec<ScenarioProgram> {
    let file = ast.syn_file();
    let functions = collect_functions(file);
    let call_graph = ScenarioCallGraph::build(&functions);
    let imports = collect_use_crate_aliases(file);
    let boundary_policies = collect_boundary_policies(file);
    let method_shapes = collect_method_shapes(file);
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
                &method_shapes,
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
    method_shapes: &MethodShapeMap,
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
        method_shapes,
        operations: Vec::new(),
        coverage: ScenarioCoverageFacts {
            call_graph_sccs: call_graph.coverage_facts(),
            ..ScenarioCoverageFacts::default()
        },
        active_functions: Vec::new(),
        loop_stack: Vec::new(),
        next_loop_id: 0,
    };
    let mut env = BindingEnv::with_imports(imports.clone());
    lowerer.execute_function(&function.sig.ident.to_string(), function, &mut env);
    lowerer.operations.push(ScenarioOp {
        span: ast.span_from_syn(function.sig.ident.span()),
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

impl<'a> ScenarioLowerer<'a> {
    pub(super) fn execute_function(
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

    pub(super) fn seed_handler_obligations(&mut self, function: &'a ItemFn, env: &mut BindingEnv) {
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
            env.bind_obligation(binding.clone(), binding.clone(), handler_reply_actions());
            self.operations.push(ScenarioOp {
                span: self.span(argument),
                kind: ScenarioOpKind::CreateObligation {
                    binding,
                    type_name: "HandlerReply".to_owned(),
                    actions: handler_reply_actions(),
                    template: Some(ScenarioLifecycleTemplate::inferred(
                        "handler_reply",
                        "handler_reply",
                    )),
                },
            });
        }
    }

    pub(super) fn execute_block(&mut self, block: &'a Block, env: &mut BindingEnv) -> BlockFlow {
        let saved_imports = env.imports.clone();
        for statement in &block.stmts {
            if let Stmt::Item(Item::Use(item_use)) = statement {
                env.bind_imports_from_use(item_use);
                continue;
            }
            let flow = self.execute_statement(statement, env);
            if flow != BlockFlow::Fallthrough {
                env.imports = saved_imports;
                return flow;
            }
        }
        env.imports = saved_imports;
        BlockFlow::Fallthrough
    }

    pub(super) fn execute_statement(
        &mut self,
        statement: &'a Stmt,
        env: &mut BindingEnv,
    ) -> BlockFlow {
        match statement {
            Stmt::Local(local) => {
                self.execute_local(local, env);
                BlockFlow::Fallthrough
            }
            Stmt::Expr(expr, _) => self.execute_expr(expr, env),
            Stmt::Item(_) => BlockFlow::Fallthrough,
            Stmt::Macro(statement_macro) => {
                if self.record_panic_macro(&statement_macro.mac) {
                    return BlockFlow::Return;
                }
                if !self.record_spawn_macro(&statement_macro.mac) {
                    self.unsupported_macro(&statement_macro.mac);
                }
                BlockFlow::Fallthrough
            }
        }
    }
}
