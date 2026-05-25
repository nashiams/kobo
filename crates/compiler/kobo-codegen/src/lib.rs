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
    wrap_source_map, KoboSourceMap, LoweringTraceEvent, RsSpan, SolverBudgetJson,
    SolverEvidenceJson, SourceMapEntry,
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
    pub runtime_evidence: RuntimeEvidence,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RuntimeEvidence {
    pub services: Vec<ServiceRuntimeEvidence>,
    pub handlers: Vec<HandlerLifecycleEvidence>,
    pub parallel_loops: Vec<ParallelLoopEvidence>,
    pub task_local_zones: Vec<TaskLocalEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ServiceRuntimeEvidence {
    pub name: String,
    pub buffer: usize,
    pub source_line: usize,
    pub backpressure: String,
    pub dispatch_loop: bool,
    pub client_api: bool,
    pub scenario_hooks: bool,
    pub hook_events: Vec<String>,
    pub methods: Vec<ServiceRuntimeMethodEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ServiceRuntimeMethodEvidence {
    pub name: String,
    pub variant: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HandlerLifecycleEvidence {
    pub name: String,
    pub source_line: usize,
    pub cleanup_hook: Option<String>,
    pub terminal_actions: Vec<String>,
    pub tracing_boundary: String,
    pub metrics_boundary: String,
    pub cleanup_boundary: String,
    pub cancel_cleanup: String,
    pub terminal_evidence_source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ParallelLoopEvidence {
    pub source_line: usize,
    pub lowering: String,
    pub policy: String,
    pub analysis_gate: String,
    pub proof: String,
    pub iterator: String,
    pub captured_bindings: Vec<String>,
    pub safety_checks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TaskLocalEvidence {
    pub source_line: usize,
    pub strategy: String,
    pub proof: String,
    pub captured_bindings: Vec<String>,
    pub safety_checks: Vec<String>,
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
    sourcemap::add_proof_event_source_entries(
        &mut entries,
        ast,
        &rs_source,
        kir.scenario_programs(),
    );
    let mut source_map = wrap_source_map(kobo_path, rs_path, entries);
    source_map.runtime_evidence = Some(lowered.runtime_evidence.clone());
    source_map.lowering_trace =
        sourcemap::build_lowering_trace(kir.scenario_programs(), &source_map);

    CodegenOutput {
        rs_source,
        source_map,
        error_policy_sites,
        runtime_evidence: lowered.runtime_evidence,
    }
}
