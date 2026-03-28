mod annotate;
mod emit;
mod lower;
mod sourcemap;

use std::path::Path;

pub use emit::emit_file;
pub use sourcemap::{wrap_source_map, KoboSourceMap, RsSpan, SourceMapEntry};

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
    let lowered = lower::lower(kir, ast, solution);
    let formatted = emit_file(&lowered.file);
    let anchors = lowered.anchors.resolve(&formatted);
    let mut entries = sourcemap::build_source_map_entries(&lowered.sites, &anchors);
    let rs_source = annotate::annotate(
        &formatted,
        &lowered.sites,
        &lowered.notes,
        &anchors,
        &mut entries,
    );
    let source_map = wrap_source_map(kobo_path, rs_path, entries);

    CodegenOutput {
        rs_source,
        source_map,
    }
}
