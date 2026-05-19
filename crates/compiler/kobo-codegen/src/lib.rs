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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeProfileOptions {
    pub service_buffer: usize,
    pub service_backpressure: String,
    pub scheduler: String,
    pub record: String,
    pub activity: String,
    pub cancellation: String,
    pub scenario_event_budget: u64,
}

impl Default for RuntimeProfileOptions {
    fn default() -> Self {
        Self {
            service_buffer: 64,
            service_backpressure: "block-on-full".to_owned(),
            scheduler: "small-random".to_owned(),
            record: "recorded-boundary-io".to_owned(),
            activity: "retry-idempotency".to_owned(),
            cancellation: "scheduler-history".to_owned(),
            scenario_event_budget: 64,
        }
    }
}

impl RuntimeProfileOptions {
    pub fn inspect_comment(&self) -> String {
        format!(
            "kobo: runtime profile service_buffer={} service_backpressure={} scheduler={} record={} activity={} cancellation={} scenario_event_budget={}",
            self.service_buffer,
            self.service_backpressure,
            self.scheduler,
            self.record,
            self.activity,
            self.cancellation,
            self.scenario_event_budget
        )
    }
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
    /// Unified service/scenario/runtime profile visible in generated artifacts.
    pub runtime_profile: RuntimeProfileOptions,
}

impl Default for CodegenOptions {
    fn default() -> Self {
        Self {
            diag_mode: false,
            executor_choice: executor::ExecutorChoice::None,
            runtime_profile: RuntimeProfileOptions::default(),
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
