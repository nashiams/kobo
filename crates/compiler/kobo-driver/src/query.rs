use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Arc;

use kobo_codegen::KoboSourceMap;
use kobo_errors::KDiagnostic;
use kobo_ir::{FileId, KoboSpan};
use kobo_migrate::{solve_modular_with_evidence, SolverBudget, SolverEvidence};
use kobo_parser::{
    preprocess_bridge_blocks_mapped, preprocess_kobo_keywords_mapped,
    preprocess_spawn_blocks_mapped, v05_keyword_configs, KoboFile, PreprocessSourceMap,
    RecoveryMode,
};

use crate::config::KoboConfig;
use crate::pipeline::analysis::run_analysis_phase;
use crate::pipeline::{run_codegen_pipeline, run_kir_phase, CodegenArtifacts};
use crate::session::CompileSession;

const PREPROCESS_VERSION: u32 = 1;
const PARSER_VERSION: u32 = 1;

/// Demand-driven compiler session keyed by source and configuration fingerprints.
pub struct QuerySession {
    config: KoboConfig,
    cache: QueryCache,
    metrics: QueryMetrics,
    file_ids: HashMap<(String, u64), FileId>,
}

#[derive(Default)]
struct QueryCache {
    parse: HashMap<ParseKey, Arc<ParsedOutput>>,
    kir: HashMap<KirKey, Arc<KirOutput>>,
    diagnostics: HashMap<AnalysisKey, Arc<DiagnosticOutput>>,
    solver: HashMap<SolverKey, Arc<SolverOutput>>,
    codegen: HashMap<CodegenKey, Arc<CodegenOutput>>,
}

/// Execution counters for query cache contract tests and diagnostics.
#[derive(Default)]
pub struct QueryMetrics {
    pub parse_executions: usize,
    pub kir_executions: usize,
    pub analysis_executions: usize,
    pub solver_executions: usize,
    pub diagnostics_executions: usize,
    pub codegen_executions: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct SourceKey {
    path_identity: String,
    source_hash: u64,
    file_id: FileId,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct PreprocessKey {
    source: SourceKey,
    preprocess_version: u32,
    preprocess_config_hash: u64,
    rewritten_hash: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct ParseKey {
    preprocess: PreprocessKey,
    parser_version: u32,
    recovery_mode: RecoveryMode,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct KirKey {
    parse: ParseKey,
    transform_config_hash: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct AnalysisKey {
    kir: KirKey,
    guarantee_policy: kobo_ir::GuaranteePolicy,
    diagnostic_config_hash: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct SolverKey {
    kir: KirKey,
    solver_config_hash: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct CodegenKey {
    kir: KirKey,
    solver_evidence_hash: u64,
    codegen_config_hash: u64,
}

/// Parsed source and parser diagnostics for one query key.
pub struct ParsedOutput {
    pub file_id: FileId,
    pub file: KoboFile,
    pub diagnostics: Vec<KDiagnostic>,
    pub poisoned_spans: Vec<KoboSpan>,
    pub preprocess_source_map: PreprocessSourceMap,
}

/// KIR phase output and phase-local diagnostics for one query key.
pub struct KirOutput {
    pub kir: kobo_ir::Kir,
    pub relaxed_fn_ranges: Vec<KoboSpan>,
    pub engine_struct_names: Vec<String>,
    pub diagnostics: Vec<KDiagnostic>,
}

/// Visible and suppressed diagnostics for an analysis query.
pub struct DiagnosticOutput {
    pub visible: Vec<KDiagnostic>,
    pub suppressed: Vec<KDiagnostic>,
}

/// Solver evidence and diagnostics for one solver query.
pub struct SolverOutput {
    pub evidence: SolverEvidence,
    pub diagnostics: Vec<KDiagnostic>,
}

/// Codegen artifacts and diagnostics for one codegen query.
pub struct CodegenOutput {
    pub artifacts: CodegenArtifacts,
    pub source_map: KoboSourceMap,
    pub rustc_diagnostics: Vec<KDiagnostic>,
    pub diagnostics: Vec<KDiagnostic>,
}

impl QuerySession {
    pub fn new(config: KoboConfig) -> Self {
        Self {
            config,
            cache: QueryCache::default(),
            metrics: QueryMetrics::default(),
            file_ids: HashMap::new(),
        }
    }

    pub fn set_config(&mut self, new_config: KoboConfig) {
        self.config = new_config;
    }

    pub fn metrics(&self) -> &QueryMetrics {
        &self.metrics
    }

    pub fn parse(&mut self, path: &Path) -> Result<Arc<ParsedOutput>, ()> {
        let (source, parse_key) = self.parse_key(path)?;
        if let Some(output) = self.cache.parse.get(&parse_key) {
            return Ok(Arc::clone(output));
        }

        let mut session = CompileSession::new(self.config.clone());
        let (file, _kir) = run_kir_phase(&mut session, path)?;
        let output = Arc::new(ParsedOutput {
            file_id: file.file_id,
            file,
            diagnostics: session.diagnostics.clone(),
            poisoned_spans: session.poisoned_spans.clone(),
            preprocess_source_map: PreprocessSourceMap::identity_for(
                &source,
                parse_key.preprocess.source.file_id,
            ),
        });
        self.metrics.parse_executions += 1;
        self.cache.parse.insert(parse_key, Arc::clone(&output));
        Ok(output)
    }

    pub fn kir(&mut self, path: &Path) -> Result<Arc<KirOutput>, ()> {
        let (_source, parse_key) = self.parse_key(path)?;
        let kir_key = self.kir_key(parse_key.clone());
        if let Some(output) = self.cache.kir.get(&kir_key) {
            return Ok(Arc::clone(output));
        }

        if !self.cache.parse.contains_key(&parse_key) {
            let _ = self.parse(path)?;
        }

        let mut session = CompileSession::new(self.config.clone());
        let (_file, kir) = run_kir_phase(&mut session, path)?;
        let output = Arc::new(KirOutput {
            relaxed_fn_ranges: session.relaxed_fn_ranges.clone(),
            engine_struct_names: session.engine_struct_names.clone(),
            diagnostics: session.diagnostics.clone(),
            kir,
        });
        self.metrics.kir_executions += 1;
        self.cache.kir.insert(kir_key, Arc::clone(&output));
        Ok(output)
    }

    pub fn diagnostics(&mut self, path: &Path) -> Result<Arc<DiagnosticOutput>, ()> {
        let (_source, parse_key) = self.parse_key(path)?;
        let kir_key = self.kir_key(parse_key);
        let analysis_key = self.analysis_key(kir_key);
        if let Some(output) = self.cache.diagnostics.get(&analysis_key) {
            return Ok(Arc::clone(output));
        }

        let kir = self.kir(path)?;
        let parsed = self.parse(path)?;
        let source = fs::read_to_string(path).map_err(|error| {
            eprintln!("kobo: io error: {error}");
        })?;
        let mut session = CompileSession::new(self.config.clone());
        session.register_source_file(path.to_path_buf(), source);
        session.relaxed_fn_ranges = kir.relaxed_fn_ranges.clone();
        session.engine_struct_names = kir.engine_struct_names.clone();
        session.poisoned_spans = parsed.poisoned_spans.clone();
        session.diagnostics = parsed.diagnostics.clone();
        let _ = run_analysis_phase(&mut session, &kir.kir);

        let output = Arc::new(DiagnosticOutput {
            visible: session.diagnostics.clone(),
            suppressed: session.suppressed_diagnostics.clone(),
        });
        self.metrics.analysis_executions += 1;
        self.metrics.diagnostics_executions += 1;
        self.cache
            .diagnostics
            .insert(analysis_key, Arc::clone(&output));
        Ok(output)
    }

    pub fn solver(&mut self, path: &Path) -> Result<Arc<SolverOutput>, ()> {
        let (_source, parse_key) = self.parse_key(path)?;
        let kir_key = self.kir_key(parse_key);
        let solver_key = self.solver_key(kir_key);
        if let Some(output) = self.cache.solver.get(&solver_key) {
            return Ok(Arc::clone(output));
        }

        let kir = self.kir(path)?;
        let solved = solve_modular_with_evidence(&kir.kir, &SolverBudget::default());
        let output = Arc::new(SolverOutput {
            evidence: solved.solver_evidence,
            diagnostics: Vec::new(),
        });
        self.metrics.solver_executions += 1;
        self.cache.solver.insert(solver_key, Arc::clone(&output));
        Ok(output)
    }

    pub fn codegen(&mut self, path: &Path) -> Result<Arc<CodegenOutput>, ()> {
        let (_source, parse_key) = self.parse_key(path)?;
        let kir_key = self.kir_key(parse_key);
        let codegen_key = self.codegen_key(kir_key);
        if let Some(output) = self.cache.codegen.get(&codegen_key) {
            return Ok(Arc::clone(output));
        }

        let _ = self.kir(path)?;
        let mut session = CompileSession::new(self.config.clone());
        let artifacts = run_codegen_pipeline(&mut session, path)?;
        let output = Arc::new(CodegenOutput {
            source_map: artifacts.source_map.clone(),
            artifacts,
            rustc_diagnostics: Vec::new(),
            diagnostics: session.diagnostics.clone(),
        });
        self.metrics.codegen_executions += 1;
        self.cache.codegen.insert(codegen_key, Arc::clone(&output));
        Ok(output)
    }

    fn parse_key(&mut self, path: &Path) -> Result<(String, ParseKey), ()> {
        let source = fs::read_to_string(path).map_err(|error| {
            eprintln!("kobo: io error: {error}");
        })?;
        let path_identity = normalize_path(path);
        let source_hash = hash_value(&source);
        let next_file_id = FileId(self.file_ids.len() as u32);
        let file_id = *self
            .file_ids
            .entry((path_identity.clone(), source_hash))
            .or_insert(next_file_id);
        let source_key = SourceKey {
            path_identity,
            source_hash,
            file_id,
        };
        let rewritten_hash = preprocessed_source_hash(&source, file_id);
        let preprocess_key = PreprocessKey {
            source: source_key,
            preprocess_version: PREPROCESS_VERSION,
            preprocess_config_hash: preprocess_config_hash(&self.config),
            rewritten_hash,
        };
        let recovery_mode = if self.config.enable_parse_recovery {
            RecoveryMode::Recover
        } else {
            RecoveryMode::FailFast
        };
        Ok((
            source,
            ParseKey {
                preprocess: preprocess_key,
                parser_version: PARSER_VERSION,
                recovery_mode,
            },
        ))
    }

    fn kir_key(&self, parse: ParseKey) -> KirKey {
        KirKey {
            parse,
            transform_config_hash: transform_config_hash(&self.config),
        }
    }

    fn analysis_key(&self, kir: KirKey) -> AnalysisKey {
        AnalysisKey {
            kir,
            guarantee_policy: self.config.guarantee_policy.clone(),
            diagnostic_config_hash: diagnostic_config_hash(&self.config),
        }
    }

    fn solver_key(&self, kir: KirKey) -> SolverKey {
        SolverKey {
            kir,
            solver_config_hash: solver_config_hash(&self.config),
        }
    }

    fn codegen_key(&self, kir: KirKey) -> CodegenKey {
        CodegenKey {
            kir,
            solver_evidence_hash: solver_config_hash(&self.config),
            codegen_config_hash: codegen_config_hash(&self.config),
        }
    }
}

fn preprocessed_source_hash(source: &str, file_id: FileId) -> u64 {
    let configs = v05_keyword_configs();
    let strict_mapped = preprocess_kobo_keywords_mapped(source, file_id, &configs);
    let spawn_mapped = preprocess_spawn_blocks_mapped(&strict_mapped.rewritten, file_id);
    let bridge_mapped = preprocess_bridge_blocks_mapped(&spawn_mapped.rewritten, file_id);
    let parser_source = mask_query_field_capability_views(&bridge_mapped.rewritten);
    hash_value(&parser_source)
}

fn mask_query_field_capability_views(source: &str) -> String {
    let mut output = source.to_owned();
    let mut search_start = 0usize;
    while let Some(relative) = output[search_start..].find(" using {") {
        let start = search_start + relative;
        let Some(close_relative) = output[start..].find('}') else {
            break;
        };
        let end = start + close_relative + 1;
        output.replace_range(start..end, &" ".repeat(end - start));
        search_start = end;
    }
    output
}

fn normalize_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

fn preprocess_config_hash(config: &KoboConfig) -> u64 {
    hash_value(&(config.enable_parse_recovery, config.channel_buffer_size))
}

fn transform_config_hash(config: &KoboConfig) -> u64 {
    hash_value(&(
        config.small_struct_clone_threshold_bytes,
        &config.copy_types,
        &config.mutating_methods,
    ))
}

fn diagnostic_config_hash(config: &KoboConfig) -> u64 {
    hash_value(&(config.guarantee_policy.clone(), config.hot_borrow_threshold))
}

fn solver_config_hash(config: &KoboConfig) -> u64 {
    hash_value(&(
        config.solver_cluster_limit,
        config.solver_budget_seconds.to_bits(),
        config.lsp_solver_budget_ms,
    ))
}

fn codegen_config_hash(config: &KoboConfig) -> u64 {
    hash_value(&(
        config.diag_enabled_fingerprint(),
        config
            .output_dir
            .as_ref()
            .map(|path| path.display().to_string()),
        config.dependencies.len(),
    ))
}

fn hash_value<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

trait CodegenConfigFingerprint {
    fn diag_enabled_fingerprint(&self) -> bool;
}

impl CodegenConfigFingerprint for KoboConfig {
    fn diag_enabled_fingerprint(&self) -> bool {
        config_diag_enabled(self)
    }
}

fn config_diag_enabled(config: &KoboConfig) -> bool {
    config.guarantee_policy.diag_always_active() || std::env::var("KOBO_DIAG").as_deref() == Ok("1")
}
