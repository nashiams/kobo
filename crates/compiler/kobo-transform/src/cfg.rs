use std::collections::{HashMap, HashSet};

use kobo_ir::{Kir, KirNodeId, NodeKind};

// --- Types ---

/// Unique identifier for a basic block in the CFG.
pub type CfgBlockId = usize;

/// A basic block in the control-flow graph.
pub struct CfgBlock {
    pub id: CfgBlockId,
    pub kir_nodes: Vec<KirNodeId>,
}

/// Control-flow graph over KIR nodes.
///
/// Built from the frozen KIR. Each basic block contains a sequence of KIR
/// nodes; edges represent control-flow transitions (branches, loops, etc.).
///
/// The CFG is scope-based: each `ScopeStart`/`ScopeEnd` pair in the KIR
/// defines a new basic block. Sibling scopes at the same depth are treated
/// as parallel branches from their parent block, with a merge block after.
///
/// Conservative: cycles are inferred when a scope block contains uses of
/// bindings declared before it AND the scope is a repeated sibling (suggesting
/// a loop body). In v0.7, all repeated-sibling scopes are conservatively
/// treated as potential loops.
pub struct CfgGraph {
    blocks: Vec<CfgBlock>,
    edges: Vec<(CfgBlockId, CfgBlockId)>,
    /// For each merge block, the set of child blocks whose borrows die there.
    /// Filled during CFG construction: when a scope ends, borrows created in
    /// the scope's child blocks are killed at the merge point.
    scope_merges: HashMap<CfgBlockId, Vec<CfgBlockId>>,
}

// --- Functions ---

/// Constructs the CFG from the frozen KIR.
pub fn build_cfg(kir: &Kir) -> CfgGraph {
    let mut builder = CfgBuilder::new();
    builder.build_from_kir(kir);
    builder.finish()
}

// --- Builder internals ---

struct CfgBuilder {
    blocks: Vec<Vec<KirNodeId>>,
    edges: Vec<(CfgBlockId, CfgBlockId)>,
    current: CfgBlockId,
    /// Records merge_block → [child_blocks]. Used to kill borrows at scope exits.
    scope_merges: HashMap<CfgBlockId, Vec<CfgBlockId>>,
}

impl CfgBuilder {
    fn new() -> Self {
        Self {
            blocks: vec![Vec::new()],
            edges: Vec::new(),
            current: 0,
            scope_merges: HashMap::new(),
        }
    }

    fn new_block(&mut self) -> CfgBlockId {
        let id = self.blocks.len();
        self.blocks.push(Vec::new());
        id
    }

    fn add_edge(&mut self, from: CfgBlockId, to: CfgBlockId) {
        if !self.edges.contains(&(from, to)) {
            self.edges.push((from, to));
        }
    }

    fn build_from_kir(&mut self, kir: &Kir) {
        // Walk KIR nodes and build blocks at scope boundaries.
        //
        // scope_stack entries: (parent_block, Vec<pending_merge_sources>)
        //
        // ScopeStart: create child block, edge parent→child, push entry.
        //   If parent matches top entry's parent, it's a sibling — reuse.
        //
        // ScopeEnd:
        //   If next is ScopeStart (sibling): save current to pending, reset
        //     current to parent (don't pop).
        //   Otherwise: save current to pending, pop entry, create merge block,
        //     wire all pending → merge. For single inner scopes, add a
        //     conservative self-edge (potential loop).
        let nodes: Vec<_> = kir.iter_nodes().collect();
        let mut scope_stack: Vec<(CfgBlockId, Vec<CfgBlockId>)> = Vec::new();

        let mut i = 0;
        while i < nodes.len() {
            match nodes[i].kind {
                NodeKind::ScopeStart => {
                    let parent = self.current;
                    let child = self.new_block();
                    self.add_edge(parent, child);
                    self.current = child;

                    // Check if this is a sibling of an existing scope at the
                    // same parent. If so, don't push a new entry.
                    let is_sibling = scope_stack
                        .last()
                        .map_or(false, |(p, _)| *p == parent);
                    if !is_sibling {
                        scope_stack.push((parent, Vec::new()));
                    }
                }
                NodeKind::ScopeEnd => {
                    let next_is_sibling = i + 1 < nodes.len()
                        && nodes[i + 1].kind == NodeKind::ScopeStart;

                    if let Some(top) = scope_stack.last_mut() {
                        top.1.push(self.current);

                        if next_is_sibling {
                            // Return to parent for next sibling branch.
                            self.current = top.0;
                        } else {
                            // Final scope end: pop and create merge.
                            let (_, pending) = scope_stack.pop().unwrap();

                            // For single inner scopes, add a conservative
                            // self-edge (could be a loop body). Skip for the
                            // outermost scope (stack empty after pop).
                            if pending.len() == 1 && !scope_stack.is_empty() {
                                self.add_edge(pending[0], pending[0]);
                            }

                            let merge = self.new_block();
                            for &src in &pending {
                                self.add_edge(src, merge);
                            }
                            self.scope_merges.insert(merge, pending);
                            self.current = merge;
                        }
                    }
                }
                _ => {
                    self.blocks[self.current].push(nodes[i].id);
                }
            }
            i += 1;
        }
    }

    fn finish(self) -> CfgGraph {
        let blocks = self
            .blocks
            .into_iter()
            .enumerate()
            .map(|(id, kir_nodes)| CfgBlock { id, kir_nodes })
            .collect();
        CfgGraph {
            blocks,
            edges: self.edges,
            scope_merges: self.scope_merges,
        }
    }
}

// --- CfgGraph public API ---

impl CfgGraph {
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    pub fn blocks(&self) -> &[CfgBlock] {
        &self.blocks
    }

    pub fn block(&self, id: CfgBlockId) -> &CfgBlock {
        &self.blocks[id]
    }

    /// Returns the set of child block IDs whose borrows die at `merge_block`.
    pub fn scope_merge_children(&self, merge_block: CfgBlockId) -> Option<&Vec<CfgBlockId>> {
        self.scope_merges.get(&merge_block)
    }

    pub fn has_edge(&self, from: usize, to: usize) -> bool {
        self.edges.contains(&(from, to))
    }

    pub fn has_cycle(&self) -> bool {
        let adj = self.adjacency_list();
        let mut visited = HashSet::new();
        let mut on_stack = HashSet::new();
        for start in 0..self.blocks.len() {
            if self.dfs_has_cycle(start, &adj, &mut visited, &mut on_stack) {
                return true;
            }
        }
        false
    }

    pub fn predecessors(&self, block: CfgBlockId) -> Vec<CfgBlockId> {
        self.edges
            .iter()
            .filter(|(_, to)| *to == block)
            .map(|(from, _)| *from)
            .collect()
    }

    pub fn successors(&self, block: CfgBlockId) -> Vec<CfgBlockId> {
        self.edges
            .iter()
            .filter(|(from, _)| *from == block)
            .map(|(_, to)| *to)
            .collect()
    }

    pub fn post_order(&self) -> Vec<CfgBlockId> {
        let adj = self.adjacency_list();
        let mut visited = HashSet::new();
        let mut order = Vec::new();
        if !self.blocks.is_empty() {
            self.dfs_post_order(0, &adj, &mut visited, &mut order);
        }
        for i in 0..self.blocks.len() {
            if !visited.contains(&i) {
                self.dfs_post_order(i, &adj, &mut visited, &mut order);
            }
        }
        order
    }

    fn adjacency_list(&self) -> HashMap<usize, Vec<usize>> {
        let mut adj: HashMap<usize, Vec<usize>> = HashMap::new();
        for &(from, to) in &self.edges {
            adj.entry(from).or_default().push(to);
        }
        adj
    }

    fn dfs_has_cycle(
        &self,
        node: usize,
        adj: &HashMap<usize, Vec<usize>>,
        visited: &mut HashSet<usize>,
        on_stack: &mut HashSet<usize>,
    ) -> bool {
        if on_stack.contains(&node) {
            return true;
        }
        if visited.contains(&node) {
            return false;
        }
        visited.insert(node);
        on_stack.insert(node);
        if let Some(neighbors) = adj.get(&node) {
            for &next in neighbors {
                if self.dfs_has_cycle(next, adj, visited, on_stack) {
                    return true;
                }
            }
        }
        on_stack.remove(&node);
        false
    }

    fn dfs_post_order(
        &self,
        node: usize,
        adj: &HashMap<usize, Vec<usize>>,
        visited: &mut HashSet<usize>,
        order: &mut Vec<CfgBlockId>,
    ) {
        if !visited.insert(node) {
            return;
        }
        if let Some(neighbors) = adj.get(&node) {
            for &next in neighbors {
                self.dfs_post_order(next, adj, visited, order);
            }
        }
        order.push(node);
    }
}

// --- Binding Liveness Analysis ---

/// Result of binding liveness analysis on the CFG.
///
/// Tracks which declarations (by KirNodeId) are "live" — i.e. might still be
/// used in the future — at the entry and exit of each basic block. This feeds
/// Phase 6 (Rc elision) to determine whether a binding escapes its scope.
///
/// Standard backward dataflow:
///   GEN[b]  = decl_ids of bindings USED or MOVED in block b
///   KILL[b] = decl_ids of bindings DECLARED in block b
///   live_in[b]  = GEN[b] ∪ (live_out[b] \ KILL[b])
///   live_out[b] = ∪ live_in[s] for all successors s of b
pub struct BindingLiveness {
    live_at_entry: Vec<HashSet<KirNodeId>>,
    live_at_exit: Vec<HashSet<KirNodeId>>,
}

impl BindingLiveness {
    /// Returns `true` if the declaration `decl_id` is live at entry of `block`.
    pub fn is_live_at(&self, block: CfgBlockId, decl_id: KirNodeId) -> bool {
        self.live_at_entry
            .get(block)
            .map_or(false, |s| s.contains(&decl_id))
    }

    /// Returns all decl_ids live at entry of `block`.
    pub fn live_at(&self, block: CfgBlockId) -> &HashSet<KirNodeId> {
        static EMPTY: std::sync::LazyLock<HashSet<KirNodeId>> =
            std::sync::LazyLock::new(HashSet::new);
        self.live_at_entry.get(block).unwrap_or(&EMPTY)
    }
}

/// Compute which bindings are live at each CFG block.
///
/// Backward fixed-point iteration. A binding is "live" at a block if it might
/// be used (Read, Write, Move) before being re-declared.
pub fn compute_binding_liveness(cfg: &CfgGraph, kir: &Kir) -> BindingLiveness {
    let n = cfg.block_count();
    let mut gen: Vec<HashSet<KirNodeId>> = vec![HashSet::new(); n];
    let mut kill: Vec<HashSet<KirNodeId>> = vec![HashSet::new(); n];

    // Build GEN and KILL sets per block.
    for block in cfg.blocks() {
        for &node_id in &block.kir_nodes {
            if let Some(node) = kir.get_node(node_id) {
                match node.kind {
                    NodeKind::Use(_) | NodeKind::Move | NodeKind::Borrow(_) => {
                        if let Some(decl_id) = node.decl_id {
                            gen[block.id].insert(decl_id);
                        }
                    }
                    NodeKind::Decl => {
                        kill[block.id].insert(node.id);
                    }
                    _ => {}
                }
            }
        }
    }

    let mut live_in: Vec<HashSet<KirNodeId>> = vec![HashSet::new(); n];
    let mut live_out: Vec<HashSet<KirNodeId>> = vec![HashSet::new(); n];

    // Backward fixed-point iteration.
    let mut changed = true;
    while changed {
        changed = false;
        for block_id in (0..n).rev() {
            // live_out[b] = ∪ live_in[s] for all successors s
            let mut new_out = HashSet::new();
            for &succ in &cfg.successors(block_id) {
                new_out.extend(&live_in[succ]);
            }

            // live_in[b] = GEN[b] ∪ (live_out[b] \ KILL[b])
            let mut new_in = gen[block_id].clone();
            for item in &new_out {
                if !kill[block_id].contains(item) {
                    new_in.insert(*item);
                }
            }

            if new_in != live_in[block_id] || new_out != live_out[block_id] {
                live_in[block_id] = new_in;
                live_out[block_id] = new_out;
                changed = true;
            }
        }
    }

    BindingLiveness {
        live_at_entry: live_in,
        live_at_exit: live_out,
    }
}

/// Send requirements for bindings in async contexts.
///
/// Any binding inside an async function conservatively needs `Send`,
/// because it may be alive across an `.await` point. Without explicit
/// await-point tracking in KIR (deferred to v0.8+), we mark ALL bindings
/// inside async functions as needing Send.
pub struct SendRequirements {
    needs_send: HashSet<KirNodeId>,
}

impl SendRequirements {
    /// Returns true if the binding with the given Decl node ID needs Send.
    pub fn needs_send(&self, decl_id: KirNodeId) -> bool {
        self.needs_send.contains(&decl_id)
    }
}

/// Compute which bindings need `Send` for async contexts.
///
/// Conservative v0.7 approach: any binding whose `is_async` flag is set
/// in transform facts is marked as needing Send. This over-approximates
/// (marks ALL bindings in async fns, not just those crossing await points).
pub fn compute_send_requirements(kir: &Kir) -> SendRequirements {
    let mut needs_send = HashSet::new();
    for binding in &kir.transform_facts().bindings {
        if binding.is_async {
            needs_send.insert(binding.node);
        }
    }
    SendRequirements { needs_send }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::TransformOptions;
    use crate::transform::build_kir;
    use kobo_ir::{FileId, NodeIdGen};
    use kobo_parser::parse_file;

    fn build_test_cfg(source: &str) -> CfgGraph {
        let mut id_gen = NodeIdGen::new();
        let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
        let kir = build_kir(&ast, &mut id_gen, TransformOptions::default());
        build_cfg(&kir)
    }

    #[test]
    fn test_cfg_basic_function() {
        let source = r#"
fn main() {
    let x = 1;
    if true {
        let y = x;
    } else {
        let z = x;
    }
}
"#;
        let mut id_gen = NodeIdGen::new();
        let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
        let kir = build_kir(&ast, &mut id_gen, TransformOptions::default());
        let cfg = build_cfg(&kir);
        assert!(
            cfg.block_count() >= 3,
            "expected at least 3 blocks, got {}",
            cfg.block_count()
        );
        // The fn body block (1) branches to the if-true block and else block.
        assert!(cfg.has_edge(1, 2), "fn body → if-true branch");
        assert!(cfg.has_edge(1, 3), "fn body → else branch");
        // Both branches merge.
        assert!(cfg.has_edge(2, 4), "if-true → merge");
        assert!(cfg.has_edge(3, 4), "else → merge");
    }

    #[test]
    fn test_cfg_loop() {
        let source = r#"
fn main() {
    let x = vec![1, 2, 3];
    for item in x.iter() {
        println!("{}", item);
    }
}
"#;
        let mut id_gen = NodeIdGen::new();
        let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
        let kir = build_kir(&ast, &mut id_gen, TransformOptions::default());
        let cfg = build_cfg(&kir);
        assert!(cfg.has_cycle(), "loop must create cycle in CFG");
    }

    #[test]
    fn test_borrow_liveness_dead_after_scope() {
        // Binding `data` is used in the if-block (read) and after (write).
        // After all uses, data should be dead.
        let source = r#"
fn main() {
    let data = vec![1, 2, 3];
    if true {
        let r = &data;
        println!("{:?}", r);
    }
    data.push(4);
}
"#;
        let mut id_gen = NodeIdGen::new();
        let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
        let kir = build_kir(&ast, &mut id_gen, TransformOptions::default());
        let cfg = build_cfg(&kir);
        let liveness = compute_binding_liveness(&cfg, &kir);

        // Find the Decl node for `data` — it's the first Decl.
        let data_decl = kir
            .iter_nodes()
            .find(|n| n.kind == NodeKind::Decl)
            .unwrap()
            .id;

        // `data` is used in block 2 (if-body, Read) and block 3 (after-if, Write).
        // Block 1 is where data is DECLARED, so data is NOT live at entry of
        // block 1 (it's killed by the Decl). But it IS live at blocks 2 and 3.
        assert!(
            liveness.is_live_at(2, data_decl),
            "data should be live in the if-body block (used there)"
        );
        assert!(
            liveness.is_live_at(3, data_decl),
            "data should be live at after-if block (used there)"
        );
        // Block 4 is the fn-exit merge — no more uses → dead.
        assert!(
            !liveness.is_live_at(4, data_decl),
            "data should be dead at fn exit"
        );
    }

    #[test]
    fn test_borrow_liveness_alive_in_outer_scope() {
        // Binding `data` is used both in an inner scope (write) and after (read).
        // data should be live at entry of the inner scope block.
        let source = r#"
fn main() {
    let data = vec![1, 2, 3];
    if true {
        data.push(4);
    }
    println!("{:?}", data);
}
"#;
        let mut id_gen = NodeIdGen::new();
        let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
        let kir = build_kir(&ast, &mut id_gen, TransformOptions::default());
        let cfg = build_cfg(&kir);
        let liveness = compute_binding_liveness(&cfg, &kir);

        // Find Decl for `data`.
        let data_decl = kir
            .iter_nodes()
            .find(|n| n.kind == NodeKind::Decl)
            .unwrap()
            .id;

        // data.push(4) is Use(Write) inside the if-body block.
        // println uses data AFTER the if block.
        // So data should be live at the if-body block.
        assert!(
            liveness.is_live_at(2, data_decl),
            "data should be live in inner scope (used there + after)"
        );
    }

    #[test]
    fn test_send_propagation_async_boundary() {
        // In an async fn, all bindings conservatively need Send.
        let source = r#"
async fn handler(data: Vec<i32>) {
    let shared = data;
    println!("{:?}", shared);
}
"#;
        let mut id_gen = NodeIdGen::new();
        let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
        let kir = build_kir(&ast, &mut id_gen, TransformOptions::default());
        let send_facts = compute_send_requirements(&kir);

        // Find the Decl node for `shared` (second Decl — first is `data` param).
        let shared_decl = kir
            .iter_nodes()
            .filter(|n| n.kind == NodeKind::Decl)
            .nth(1)
            .expect("should have a second Decl for `shared`")
            .id;

        assert!(
            send_facts.needs_send(shared_decl),
            "binding in async fn should need Send"
        );
    }

    #[test]
    fn test_send_not_needed_sync_fn() {
        // In a non-async fn, no bindings need Send.
        let source = r#"
fn main() {
    let data = vec![1, 2, 3];
    println!("{:?}", data);
}
"#;
        let mut id_gen = NodeIdGen::new();
        let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
        let kir = build_kir(&ast, &mut id_gen, TransformOptions::default());
        let send_facts = compute_send_requirements(&kir);

        let data_decl = kir
            .iter_nodes()
            .find(|n| n.kind == NodeKind::Decl)
            .unwrap()
            .id;

        assert!(
            !send_facts.needs_send(data_decl),
            "binding in sync fn should NOT need Send"
        );
    }
}
