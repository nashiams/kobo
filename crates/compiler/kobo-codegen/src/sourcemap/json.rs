use std::path::Path;

use super::{KoboSourceMap, SourceMapEntry};
pub fn wrap_source_map(
    kobo_path: &Path,
    rs_path: &Path,
    entries: Vec<SourceMapEntry>,
) -> KoboSourceMap {
    KoboSourceMap {
        version: 3,
        file: rs_path.display().to_string(),
        sources: vec![kobo_path.display().to_string()],
        x_kobo_mappings: entries,
        runtime_evidence: None,
        solver_evidence: None,
        lowering_trace: Vec::new(),
    }
}
