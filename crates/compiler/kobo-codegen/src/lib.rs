mod annotate;
mod emit;
mod lower;
mod sourcemap;

use std::path::Path;

pub use annotate::annotate;
pub use emit::emit_file;
pub use lower::{lower, LoweredFile};
pub use sourcemap::{build_source_map_entries, wrap_source_map, KoboSourceMap, RsSpan, SourceMapEntry};

pub struct CodegenOutput {
    pub rs_source: String,
    pub source_map: KoboSourceMap,
}

pub fn codegen_file(
    kir: &kobo_ir::Kir,
    ast: &kobo_parser::KoboFile,
    solution: &kobo_ir::SolutionMap,
    kobo_path: &Path,
    rs_path: &Path,
) -> CodegenOutput {
    let lowered = lower(kir, ast, solution);
    let formatted = emit_file(&lowered.file);
    let mut entries = build_source_map_entries(&formatted, &lowered.sites);
    let rs_source = annotate(&formatted, &lowered.sites, &mut entries);
    let source_map = wrap_source_map(kobo_path, rs_path, entries);

    CodegenOutput {
        rs_source,
        source_map,
    }
}
