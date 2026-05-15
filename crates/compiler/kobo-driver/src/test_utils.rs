//! Shared test helpers for v0.8 phases.
//!
//! Provides in-memory source→KIR→codegen pipelines so tests do not
//! touch the filesystem. Every phase's test suite imports these helpers.

use std::path::Path;
use std::path::PathBuf;

use kobo_codegen::{codegen_file, CodegenOptions};
use kobo_ir::{
    FileId, Kir, KoboMode, OwnershipTier, SharedBindingFacts, SolutionMap, TierDecision,
    TransformBindingFacts,
};
use kobo_parser::{
    collect_strict_items_from_syn, parse_file, postprocess_strict_markers,
    preprocess_kobo_keywords, preprocess_spawn_blocks, preprocess_strict_reject_invalid,
    v05_keyword_configs, KoboFile,
};
use kobo_transform::{build_kir, TransformOptions};

use crate::config::KoboConfig;
use crate::pipeline::codegen::collect_error_policy_sites;
use crate::pipeline::{apply_error_policy_sites, CodegenArtifacts};
use crate::session::CompileSession;

// --- Types first ---

/// Result of an in-memory compilation attempt.
pub struct CompileResult {
    pub kir: Kir,
    pub ast: KoboFile,
    pub session: CompileSession,
}

// --- In-memory pipeline helpers ---

/// Parse and transform `.kobo` source in memory, returning KIR + AST.
///
/// Runs: preprocess → parse → collect strict items → build_kir.
/// Does NOT run analysis or codegen — use those directly on the result.
pub fn compile_to_kir(source: &str, config: &KoboConfig) -> Result<CompileResult, String> {
    let mut session = CompileSession::new(config.clone());
    let file_id = session.register_source_file("test.kobo".into(), source.to_owned());

    // Preprocess @strict keywords.
    let configs = v05_keyword_configs();
    if let Err(e) = preprocess_strict_reject_invalid(source, &configs) {
        return Err(format!("preprocess error: {e}"));
    }
    let (rewritten, markers) = preprocess_kobo_keywords(source, &configs);

    // v0.8: rewrite spawn { ... } blocks.
    let (rewritten, _spawn_infos) = preprocess_spawn_blocks(&rewritten, file_id);

    // Parse.
    let mut kobo_file = parse_file(&rewritten, file_id, &mut session.id_gen)
        .map_err(|e| format!("parse error: {e}"))?;

    // Validate #[kobo::handler] usage.
    kobo_parser::validate_handler_attributes(kobo_file.syn_file())
        .map_err(|e| format!("handler error: {e}"))?;

    // Collect @strict blocks/fns and strip markers.
    let (strict_blocks, strict_fns) =
        collect_strict_items_from_syn(kobo_file.syn_file(), &rewritten, file_id);
    postprocess_strict_markers(kobo_file.syn_file_mut(), &markers)
        .map_err(|e| format!("postprocess error: {e}"))?;
    kobo_file.set_strict_items(strict_blocks, strict_fns);

    // Transform → KIR.
    let kir = build_kir(
        &kobo_file,
        &mut session.id_gen,
        TransformOptions {
            small_struct_clone_threshold_bytes: config.small_struct_clone_threshold_bytes,
            copy_types: config.copy_types.clone(),
            mutating_methods: config.mutating_methods.clone(),
        },
    );

    session.relaxed_fn_ranges = kir.relaxed_fn_ranges().to_vec();

    Ok(CompileResult {
        kir,
        ast: kobo_file,
        session,
    })
}

/// Compile `.kobo` source in Script mode and return the generated `.rs` source.
///
/// Runs the full pipeline: preprocess → parse → transform → codegen.
pub fn compile_and_inspect(source: &str) -> String {
    compile_and_inspect_with_config(source, KoboConfig::default())
}

/// Compile in Script mode specifically (for lifetime erasure tests).
pub fn compile_and_inspect_script_mode(source: &str) -> String {
    let config = KoboConfig {
        mode: KoboMode::Script,
        ..Default::default()
    };
    compile_and_inspect_with_config(source, config)
}

/// Compile with a specific config and return the generated `.rs` source.
pub fn compile_and_inspect_with_config(source: &str, config: KoboConfig) -> String {
    let result = compile_to_kir(source, &config).expect("compile_to_kir failed");
    codegen_to_string(&result.kir, &result.ast, &config)
}

pub fn run_codegen_for_source_with_policy(
    source: &str,
    policy_name: &str,
) -> Result<CodegenArtifacts, String> {
    let config = KoboConfig::default();
    let result = compile_to_kir(source, &config)?;
    let solution = SolutionMap::new();
    let kobo_path = Path::new("test.kobo");
    let rs_path = Path::new("test.rs");
    let executor_choice = kobo_codegen::executor::select_executor(&config.dependencies);
    let output = codegen_file(
        &result.kir,
        &result.ast,
        &solution,
        kobo_path,
        rs_path,
        &CodegenOptions {
            diag_mode: false,
            executor_choice,
        },
    );
    let error_policy_sites = collect_error_policy_sites(&result.ast, &output.rs_source, FileId(0));
    let artifacts = CodegenArtifacts {
        file_id: FileId(0),
        kobo_file: result.ast,
        rs_source: output.rs_source,
        rs_path: PathBuf::from("test.rs"),
        map_path: PathBuf::from("test.kobo.map"),
        source_map: output.source_map,
        must_call_obligations: result.kir.must_call_obligations().to_vec(),
        error_policy_sites,
    };
    Ok(apply_error_policy_sites(artifacts, policy_name))
}

/// Like `compile_and_inspect` but returns `Result` — for tests expecting compile errors.
pub fn try_compile(source: &str) -> Result<String, String> {
    let config = KoboConfig::default();
    let result = compile_to_kir(source, &config)?;

    // Run analysis to collect diagnostics.
    let facts = kobo_analysis::run_analysis(&result.kir, result.session.file_set());
    let diags = kobo_analysis::facts_to_diagnostics(
        &facts,
        result.kir.transform_facts(),
        result.session.file_set(),
        &result.kir,
        result.session.mode(),
    );

    let has_errors = diags
        .iter()
        .any(|d| d.severity == kobo_errors::Severity::Error);
    if has_errors {
        let msgs: Vec<String> = diags
            .iter()
            .filter(|d| d.severity == kobo_errors::Severity::Error)
            .map(|d| d.primary.text.clone())
            .collect();
        return Err(msgs.join("\n"));
    }

    Ok(codegen_to_string(&result.kir, &result.ast, &config))
}

/// Run tier analysis and return all tier decisions.
pub fn analyze_tiers(source: &str) -> Vec<TierDecision> {
    let config = KoboConfig::default();
    let result = compile_to_kir(source, &config).expect("compile_to_kir failed");
    result.kir.iter_tier_decisions().cloned().collect()
}

/// Find a specific binding's tier decision by name.
///
/// Panics if the binding is not found — use `analyze_tiers_named` + `find_tier_named` instead.
#[allow(dead_code)]
pub fn find_tier(decisions: &[TierDecision], _name: &str) -> OwnershipTier {
    // TierDecision has `node` but not the binding name directly.
    // Use analyze_tiers_named + find_tier_named for name-based lookups.
    let _ = decisions;
    panic!("find_tier: use analyze_tiers_named + find_tier_named instead");
}

/// Analyze tiers and return a map of binding_name → OwnershipTier.
pub fn analyze_tiers_named(source: &str) -> Vec<(String, OwnershipTier)> {
    let config = KoboConfig::default();
    let result = compile_to_kir(source, &config).expect("compile_to_kir failed");
    result
        .kir
        .transform_facts()
        .iter_bindings()
        .map(|b| {
            let tier = result
                .kir
                .iter_tier_decisions()
                .find(|d| d.node == b.node)
                .map(|d| d.tier)
                .unwrap_or(OwnershipTier::Undecided);
            (b.binding_name.clone(), tier)
        })
        .collect()
}

/// Find a binding's tier by name from the named tiers list.
pub fn find_tier_named(tiers: &[(String, OwnershipTier)], name: &str) -> OwnershipTier {
    tiers
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no tier decision for binding '{name}'"))
        .1
}

/// Build `SharedBindingFacts` with sensible defaults for test construction.
pub fn make_shared_binding_facts() -> SharedBindingFacts {
    SharedBindingFacts::default()
}

/// Build `TransformBindingFacts` wrapping shared facts with zeroed-out fields.
pub fn make_transform_binding_facts(shared: SharedBindingFacts) -> TransformBindingFacts {
    use kobo_ir::{BindingUsage, KirNodeId, KoboAstNodeId, KoboSpan};
    TransformBindingFacts {
        node: KirNodeId(0),
        ast_id: KoboAstNodeId(0),
        binding_name: String::new(),
        span: KoboSpan::new(0, 0, FileId(0)),
        resource_kind: None,
        hint: None,
        hint_span: None,
        is_copy_known: false,
        is_generic: false,
        is_async: false,
        async_shared: false,
        usage: BindingUsage {
            declaration: KoboSpan::new(0, 0, FileId(0)),
            uses: Vec::new(),
        },
        shared_facts: shared,
        clone_elision: None,
        elision_fallback: None,
        plain_clone_alias: false,
        plain_clone_source: None,
        plain_clone_move_span: None,
        elision_skip_reason: None,
        decl_scope_depth: 0,
        method_read_spans: Vec::new(),
        ref_returning_read_spans: Vec::new(),
    }
}

/// Generate a cluster of N interconnected bindings for solver budget tests.
pub fn generate_large_cluster(n: usize) -> String {
    let mut code = String::from("fn test_fn() {\n");
    for i in 0..n {
        code.push_str(&format!("    let v{i} = create();\n"));
        if i > 0 {
            code.push_str(&format!("    use_both(&v{}, &v{i});\n", i - 1));
        }
    }
    code.push_str("}\n");
    code
}

/// Generate a moderate cluster (same shape, fewer bindings).
pub fn generate_moderate_cluster(n: usize) -> String {
    generate_large_cluster(n)
}

// --- Internal helpers ---

fn codegen_to_string(kir: &Kir, ast: &KoboFile, config: &KoboConfig) -> String {
    let solution = SolutionMap::new();
    let kobo_path = Path::new("test.kobo");
    let rs_path = Path::new("test.rs");
    let executor_choice = kobo_codegen::executor::select_executor(&config.dependencies);
    let output = codegen_file(
        kir,
        ast,
        &solution,
        kobo_path,
        rs_path,
        &CodegenOptions {
            diag_mode: false,
            executor_choice,
        },
    );
    output.rs_source
}
