use kobo_ir::Kir;

// --- Types ---

/// Control-flow graph over KIR nodes.
///
/// Empty stub at v0.1. Populated during the async ownership passes in v0.7.
/// The build function is called by the pipeline from day one so the shape
/// is correct before the implementation arrives.
pub struct CfgGraph;

// --- Functions ---

/// Constructs the CFG from the frozen KIR.
///
/// Stub: returns an empty graph. The pipeline calls this unconditionally
/// so the hook exists for v0.7 to fill in.
pub fn build_cfg(_kir: &Kir) -> CfgGraph {
    CfgGraph
}
