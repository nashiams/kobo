use super::*;

impl ScenarioCallGraph {
    pub(super) fn build(functions: &FunctionMap<'_>) -> Self {
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

    pub(super) fn coverage_facts(&self) -> Vec<ScenarioCallGraphScc> {
        self.sccs
            .iter()
            .map(|component| ScenarioCallGraphScc {
                functions: component.functions.clone(),
                is_recursive: component.is_recursive,
            })
            .collect()
    }

    pub(super) fn is_recursive_function(&self, function: &str) -> bool {
        self.function_sccs
            .get(function)
            .and_then(|index| self.sccs.get(*index))
            .map(|component| component.is_recursive)
            .unwrap_or(false)
    }
}

impl TarjanState {
    pub(super) fn connect(&mut self, node: &str, edges: &HashMap<String, Vec<String>>) {
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

    pub(super) fn finish_component(&mut self, root: &str) {
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

pub(super) fn collect_call_graph_edges(
    functions: &FunctionMap<'_>,
) -> HashMap<String, Vec<String>> {
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
