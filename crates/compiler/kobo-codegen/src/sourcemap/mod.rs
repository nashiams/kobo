mod entries;
mod json;
mod lookup;
mod lowering_trace;
mod proof_anchors;
mod spans;
mod syntax_anchors;

#[cfg(test)]
mod lowering_trace_tests;
#[cfg(test)]
mod tests;

pub use entries::{KoboSourceMap, RsSpan, SolverBudgetJson, SolverEvidenceJson, SourceMapEntry};
pub use json::wrap_source_map;
pub use lowering_trace::LoweringTraceEvent;

pub(crate) use entries::build_source_map_entries;
pub(crate) use lowering_trace::build_lowering_trace;
pub(crate) use proof_anchors::add_proof_event_source_entries;

#[cfg(test)]
use lowering_trace::core_event_id_for_operation;
