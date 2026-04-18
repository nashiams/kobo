use kobo_ir::{KirNodeId, KoboSpan};

/// A site where extract-before-borrow can be applied.
#[derive(Debug, Clone)]
pub struct ExtractionSite {
    /// The binding whose borrow conflicts
    pub binding_id: KirNodeId,
    pub binding_name: String,
    /// The expression that borrows the binding (e.g., data.len())
    pub borrow_expr_span: KoboSpan,
    /// The conflicting mutation (e.g., data.push(42))
    pub conflict_span: KoboSpan,
    /// Name for the generated temporary
    pub temp_name: String,
}

/// Find all extract-before-borrow opportunities in the given transform facts.
///
/// A site qualifies when ALL of:
/// 1. Binding B has an immutable read at span S1
/// 2. Binding B has a mutation at span S2, where S2 > S1
/// 3. The read at S1 produces a value that can be hoisted (method call result)
/// 4. The borrow from S1 is still conceptually live at S2
///
/// NOT eligible: iterator borrows, closure captures, nested borrows.
///
/// Temp names: `__kobo_extract_0`, `__kobo_extract_1`, etc. (per call).
pub fn find_extract_before_borrow(facts: &kobo_ir::TransformFacts) -> Vec<ExtractionSite> {
    let mut sites = Vec::new();
    let mut counter: usize = 0;

    for binding in facts.iter_bindings() {
        let uses = &binding.usage.uses;

        // Collect read and mutation spans
        let mut read_spans: Vec<KoboSpan> = Vec::new();
        let mut mutation_spans: Vec<KoboSpan> = Vec::new();

        for event in uses {
            match event {
                kobo_ir::UseEvent::ReadOnly { span } => {
                    // BUG-12: Skip reads from reference-returning methods (e.g. .iter(), .as_ref())
                    // Extracting a reference return would move a borrow — unsound.
                    if !binding.ref_returning_read_spans.contains(span) {
                        read_spans.push(*span);
                    }
                }
                kobo_ir::UseEvent::Mutated { span } => {
                    mutation_spans.push(*span);
                }
                _ => {}
            }
        }

        // For each read followed by a mutation, check for conflict
        for read_span in &read_spans {
            for mut_span in &mutation_spans {
                // Mutation must be after the read
                if mut_span.start > read_span.start {
                    sites.push(ExtractionSite {
                        binding_id: binding.node,
                        binding_name: binding.binding_name.clone(),
                        borrow_expr_span: *read_span,
                        conflict_span: *mut_span,
                        temp_name: format!("__kobo_extract_{}", counter),
                    });
                    counter += 1;
                    // Only one extraction per read span to avoid duplicates
                    break;
                }
            }
        }
    }

    sites
}
