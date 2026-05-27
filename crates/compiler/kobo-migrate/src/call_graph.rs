//! Stage: Call-graph construction with Tarjan SCC detection.
//!
//! Builds a directed call graph from KIR, then finds strongly connected
//! components (SCCs) for topo-order iteration by the solver.

use std::collections::{BTreeMap, BTreeSet};

/// A directed call graph.
#[derive(Clone, Debug, Default)]
pub struct CallGraph {
    /// Adjacency list: caller → callees.
    pub edges: BTreeMap<String, BTreeSet<String>>,
    /// All known function names.
    pub nodes: BTreeSet<String>,
}

impl CallGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, name: String) {
        self.nodes.insert(name);
    }

    pub fn add_edge(&mut self, caller: String, callee: String) {
        self.nodes.insert(caller.clone());
        self.nodes.insert(callee.clone());
        self.edges.entry(caller).or_default().insert(callee);
    }

    pub fn callees(&self, name: &str) -> impl Iterator<Item = &str> {
        self.edges
            .get(name)
            .into_iter()
            .flat_map(|s| s.iter().map(|e| e.as_str()))
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.values().map(|s| s.len()).sum()
    }
}

/// One strongly connected component.
#[derive(Clone, Debug)]
pub struct Scc {
    pub members: Vec<String>,
    pub is_recursive: bool,
}

/// Compute topological order of SCCs via Tarjan's algorithm.
///
/// Returns SCCs in reverse topological order (leaves first),
/// suitable for bottom-up propagation.
pub fn tarjan_scc(graph: &CallGraph) -> Vec<Scc> {
    let mut state = TarjanState::new(&graph.nodes);
    let nodes_sorted: Vec<String> = graph.nodes.iter().cloned().collect();
    for node in &nodes_sorted {
        if !state.visited.contains(node.as_str()) {
            strongconnect(&mut state, node, graph);
        }
    }
    state.result
}

struct TarjanState {
    index_counter: u32,
    index: BTreeMap<String, u32>,
    lowlink: BTreeMap<String, u32>,
    on_stack: BTreeSet<String>,
    stack: Vec<String>,
    visited: BTreeSet<String>,
    result: Vec<Scc>,
}

impl TarjanState {
    fn new(_nodes: &BTreeSet<String>) -> Self {
        Self {
            index_counter: 0,
            index: BTreeMap::new(),
            lowlink: BTreeMap::new(),
            on_stack: BTreeSet::new(),
            stack: Vec::new(),
            visited: BTreeSet::new(),
            result: Vec::new(),
        }
    }
}

fn strongconnect(state: &mut TarjanState, v: &str, graph: &CallGraph) {
    state.index.insert(v.to_owned(), state.index_counter);
    state.lowlink.insert(v.to_owned(), state.index_counter);
    state.index_counter += 1;
    state.stack.push(v.to_owned());
    state.on_stack.insert(v.to_owned());
    state.visited.insert(v.to_owned());

    // Visit successors.
    if let Some(callees) = graph.edges.get(v) {
        for w in callees {
            if !state.visited.contains(w.as_str()) {
                strongconnect(state, w, graph);
                let lw = state.lowlink[w];
                let lv = state.lowlink[v];
                if lw < lv {
                    state.lowlink.insert(v.to_owned(), lw);
                }
            } else if state.on_stack.contains(w.as_str()) {
                let iw = state.index[w];
                let lv = state.lowlink[v];
                if iw < lv {
                    state.lowlink.insert(v.to_owned(), iw);
                }
            }
        }
    }

    // Root of an SCC?
    if state.lowlink[v] == state.index[v] {
        let mut members = Vec::new();
        loop {
            let w = state.stack.pop().expect("stack should not be empty");
            state.on_stack.remove(&w);
            members.push(w.clone());
            if w == v {
                break;
            }
        }
        members.sort(); // Deterministic ordering.
        let is_recursive =
            members.len() > 1 || graph.edges.get(v).map(|c| c.contains(v)).unwrap_or(false);
        state.result.push(Scc {
            members,
            is_recursive,
        });
    }
}

/// Build a call graph from KIR (simplified: uses binding names as proxies).
pub fn build_call_graph(kir: &kobo_ir::Kir) -> CallGraph {
    let mut cg = CallGraph::new();
    let facts = kir.transform_facts();

    for binding in &facts.bindings {
        cg.add_node(binding.binding_name.clone());

        // If a binding has a plain_clone_source, that implies a flow edge.
        if let Some(source_id) = binding.plain_clone_source {
            if let Some(source_binding) = facts.bindings.iter().find(|b| b.node == source_id) {
                cg.add_edge(
                    source_binding.binding_name.clone(),
                    binding.binding_name.clone(),
                );
            }
        }
    }

    cg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_graph_no_sccs() {
        let cg = CallGraph::new();
        let sccs = tarjan_scc(&cg);
        assert!(sccs.is_empty());
    }

    #[test]
    fn dag_yields_singleton_sccs() {
        let mut cg = CallGraph::new();
        cg.add_edge("a".into(), "b".into());
        cg.add_edge("b".into(), "c".into());
        let sccs = tarjan_scc(&cg);
        assert_eq!(sccs.len(), 3);
        for scc in &sccs {
            assert!(!scc.is_recursive);
        }
    }

    #[test]
    fn cycle_yields_single_scc() {
        let mut cg = CallGraph::new();
        cg.add_edge("a".into(), "b".into());
        cg.add_edge("b".into(), "c".into());
        cg.add_edge("c".into(), "a".into());
        let sccs = tarjan_scc(&cg);
        assert_eq!(sccs.len(), 1);
        assert!(sccs[0].is_recursive);
        assert_eq!(sccs[0].members.len(), 3);
    }

    #[test]
    fn self_loop_detected() {
        let mut cg = CallGraph::new();
        cg.add_edge("a".into(), "a".into());
        let sccs = tarjan_scc(&cg);
        assert_eq!(sccs.len(), 1);
        assert!(sccs[0].is_recursive);
    }

    #[test]
    fn scc_order_is_deterministic() {
        let mut cg = CallGraph::new();
        cg.add_edge("z".into(), "y".into());
        cg.add_edge("y".into(), "z".into());
        cg.add_edge("a".into(), "b".into());
        let r1 = tarjan_scc(&cg);
        let r2 = tarjan_scc(&cg);
        assert_eq!(r1.len(), r2.len());
        for (a, b) in r1.iter().zip(r2.iter()) {
            assert_eq!(a.members, b.members);
        }
    }
}
