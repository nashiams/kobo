//! Stage: Function summaries for inter-procedural analysis.
//!
//! A `FunctionSummary` captures the ownership constraints that flow
//! in/out of a single function — a compact representation that lets the
//! constraint solver scale across call boundaries.

use kobo_ir::{KirNodeId, OwnershipTier};

/// A compact summary of one function's ownership behaviour.
#[derive(Clone, Debug)]
pub struct FunctionSummary {
    /// Function name (for diagnostics).
    pub name: String,
    /// Parameters with inferred tier requirements.
    pub params: Vec<ParamSummary>,
    /// The return tier, if the function produces an owned value.
    pub return_tier: Option<OwnershipTier>,
    /// Whether this function captures variables across an await boundary.
    pub captures_across_await: bool,
    /// Whether this function sends owned data to another thread/task.
    pub sends_owned: bool,
    /// Internal cluster size — helps budget decisions.
    pub internal_node_count: usize,
}

/// Ownership summary for one parameter.
#[derive(Clone, Debug)]
pub struct ParamSummary {
    pub name: String,
    pub node_id: KirNodeId,
    /// Minimum tier required by the function's body.
    pub floor: OwnershipTier,
    /// Whether the parameter escapes the function scope.
    pub escapes: bool,
    /// Whether the parameter is mutated inside the body.
    pub mutated: bool,
}

/// A mapping from function name to summary.
#[derive(Clone, Debug, Default)]
pub struct SummaryTable {
    entries: Vec<(String, FunctionSummary)>,
}

impl SummaryTable {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn insert(&mut self, name: String, summary: FunctionSummary) {
        self.entries.push((name, summary));
    }

    pub fn get(&self, name: &str) -> Option<&FunctionSummary> {
        self.entries.iter().find(|(n, _)| n == name).map(|(_, s)| s)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &FunctionSummary)> {
        self.entries.iter().map(|(n, s)| (n.as_str(), s))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Build function summaries from KIR.
///
/// Walks declaration nodes, collects sharing facts, and builds parameter
/// summaries.  Returns a table that the solver can query during
/// inter-procedural constraint propagation.
pub fn build_summaries(kir: &kobo_ir::Kir) -> SummaryTable {
    let mut table = SummaryTable::new();
    let facts = kir.transform_facts();

    // Group bindings by function scope (approximated by contiguous decl_scope_depth).
    // For now, each binding with is_async or shared_facts data contributes to its
    // enclosing function's summary.
    for binding in &facts.bindings {
        let name = binding.binding_name.clone();
        let sf = &binding.shared_facts;

        let floor = if sf.needs_send {
            if sf.needs_mutable_wrapper {
                OwnershipTier::ArcMutShared
            } else {
                OwnershipTier::ArcShared
            }
        } else if sf.needs_sharing {
            if sf.needs_mutable_wrapper {
                OwnershipTier::RcMutShared
            } else {
                OwnershipTier::RcShared
            }
        } else {
            OwnershipTier::PlainOwned
        };

        let param = ParamSummary {
            name: name.clone(),
            node_id: binding.node,
            floor,
            escapes: sf.has_escape,
            mutated: sf.mutation_required,
        };

        // Upsert: if we already have a summary for this binding name, add param.
        if let Some(existing) = table
            .entries
            .iter_mut()
            .find(|(n, _)| *n == name)
            .map(|(_, s)| s)
        {
            existing.params.push(param);
            existing.internal_node_count += 1;
        } else {
            table.insert(
                name,
                FunctionSummary {
                    name: binding.binding_name.clone(),
                    params: vec![param],
                    return_tier: None,
                    captures_across_await: binding.is_async,
                    sends_owned: sf.needs_send,
                    internal_node_count: 1,
                },
            );
        }
    }

    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_kir_yields_empty_summaries() {
        let kir = kobo_ir::Kir::default();
        let table = build_summaries(&kir);
        assert!(table.is_empty());
    }

    #[test]
    fn summary_table_round_trip() {
        let mut table = SummaryTable::new();
        table.insert(
            "foo".into(),
            FunctionSummary {
                name: "foo".into(),
                params: Vec::new(),
                return_tier: Some(OwnershipTier::RcShared),
                captures_across_await: false,
                sends_owned: false,
                internal_node_count: 0,
            },
        );
        assert!(table.get("foo").is_some());
        assert!(table.get("bar").is_none());
        assert_eq!(table.len(), 1);
    }
}
