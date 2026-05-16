mod annotate;
pub mod cargo_gen;
pub mod clean;
mod emit;
mod error_policy;
pub mod executor;
pub mod hard_rules;
mod lower;
mod sourcemap;

use std::path::Path;

pub use emit::emit_file;
pub use error_policy::ErrorPolicySite;
pub use sourcemap::{
    wrap_source_map, KoboSourceMap, RsSpan, SolverBudgetJson, SolverEvidenceJson, SourceMapEntry,
};

/// Annotate lock acquisition order in generated Rust source for inspect output.
///
/// Returns the source with a leading `// kobo: lock order: ...` comment if
/// two or more lock sites are detected, otherwise returns the source unchanged.
pub fn annotate_lock_order(source: &str) -> String {
    let sites = lower::rewrite::lock_order::detect_lock_sites(source);
    if let Some(comment) = lower::rewrite::lock_order::lock_order_comment(&sites) {
        format!("{comment}\n{source}")
    } else {
        source.to_owned()
    }
}

pub struct CodegenOutput {
    pub rs_source: String,
    pub source_map: KoboSourceMap,
    pub error_policy_sites: Vec<ErrorPolicySite>,
}

/// Options that control code generation behaviour.
///
/// Passed into `codegen_file()` from the driver. The driver reads `KOBO_DIAG`
/// from the environment and sets `diag_mode` accordingly.
#[derive(Debug, Clone)]
pub struct CodegenOptions {
    /// When `true`, `RcMutShared` bindings are wrapped in `DiagOwner::new(…)`
    /// instead of bare `Rc::new(RefCell::new(…))`.  Requires the generated
    /// crate to depend on `kobo-diag` with `features = ["diag"]`.
    pub diag_mode: bool,
    /// Async executor selected from direct dependencies for `async fn main()`.
    pub executor_choice: executor::ExecutorChoice,
}

impl Default for CodegenOptions {
    fn default() -> Self {
        Self {
            diag_mode: false,
            executor_choice: executor::ExecutorChoice::None,
        }
    }
}

pub fn codegen_file(
    kir: &kobo_ir::Kir,
    ast: &kobo_parser::KoboFile,
    solution: &kobo_ir::SolutionMap,
    kobo_path: &Path,
    rs_path: &Path,
    options: &CodegenOptions,
) -> CodegenOutput {
    let lowered = lower::lower(kir, ast, solution, kobo_path, options);
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
    let (rs_source, error_policy_sites) =
        error_policy::resolve_marked_error_policy_sites(rs_source, &lowered.error_policy_markers);
    let source_map = wrap_source_map(kobo_path, rs_path, entries);

    CodegenOutput {
        rs_source,
        source_map,
        error_policy_sites,
    }
}
