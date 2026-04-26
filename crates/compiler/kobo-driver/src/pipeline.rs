use std::path::{Path, PathBuf};

use kobo_analysis::{facts_to_diagnostics, run_analysis};
use kobo_codegen::{codegen_file, CodegenOptions, CodegenOutput, KoboSourceMap, SolverEvidenceJson, SolverBudgetJson};
use kobo_errors::{resolve_severity, DiagDecision, DiagLabel, KDiagnostic, KErrorCode, Severity};
use kobo_ir::{AsyncViolationKind, FileId, Kir, KoboMode, RelaxAttrError, SolutionMap, WarnEarlyPattern};
use kobo_migrate::{build_kir_constraint_graph, solve_with_evidence, SolveOutcome, SolverBudget, SolverEvidence};
use kobo_parser::{
    collect_strict_items_from_syn, parse_file, postprocess_strict_markers,
    preprocess_kobo_keywords, preprocess_spawn_blocks, preprocess_strict_reject_invalid,
    v05_keyword_configs, mode_parse::parse_file_mode, KoboFile,
};
use kobo_transform::{build_kir, TransformOptions, strict_async::check_strict_async};

use crate::filesystem::{
    map_path_for, output_path_for, read_kobo_file, write_map_file, write_rs_file,
};
use crate::rustc::{compile_and_remap, extract_kobo_regions, filter_wrapper_noise, remap_warnings_to_diagnostics};
use crate::session::CompileSession;

pub struct CodegenArtifacts {
    pub file_id: FileId,
    pub rs_source: String,
    pub rs_path: PathBuf,
    pub map_path: PathBuf,
    pub source_map: KoboSourceMap,
}

pub fn run_kir_phase(session: &mut CompileSession, input: &Path) -> Result<(KoboFile, Kir), ()> {
    let source = match read_kobo_file(input) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("kobo: io error: {error}");
            return Err(());
        }
    };
    let file_id = session.register_source_file(input.to_path_buf(), source.clone());

    // S-26: Per-module mode — file attribute overrides Kobo.toml, CLI overrides both.
    if !session.cli_mode_override {
        match parse_file_mode(&source) {
            Ok(Some(file_mode)) => {
                session.config.mode = file_mode;
            }
            Ok(None) => {} // no file-level attribute, keep current mode
            Err(e) => {
                eprintln!("kobo: {e}");
                return Err(());
            }
        }
    }

    // v0.5 preprocessing: rewrite @strict → marker attributes before syn parse.
    let configs = v05_keyword_configs();
    if let Err(e) = preprocess_strict_reject_invalid(&source, &configs) {
        eprintln!("kobo: preprocess error: {e}");
        return Err(());
    }
    let (rewritten, markers) = preprocess_kobo_keywords(&source, &configs);

    // v0.8: rewrite spawn { ... } → __kobo_spawn_block!({ ... }) before syn parse.
    let (rewritten, _spawn_infos) = preprocess_spawn_blocks(&rewritten, file_id);

    let mut kobo_file = match parse_file(&rewritten, file_id, &mut session.id_gen) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("kobo: parse error: {error}");
            return Err(());
        }
    };

    // Validate #[kobo::handler] usage (must be async fn).
    if let Err(e) = kobo_parser::validate_handler_attributes(kobo_file.syn_file()) {
        eprintln!("kobo: {e}");
        return Err(());
    }

    // Collect @strict blocks/fns before stripping marker attributes.
    let (strict_blocks, strict_fns) =
        collect_strict_items_from_syn(kobo_file.syn_file(), &rewritten, file_id);
    if let Err(e) = postprocess_strict_markers(kobo_file.syn_file_mut(), &markers) {
        eprintln!("kobo: postprocess error: {e}");
        return Err(());
    }
    kobo_file.set_strict_items(strict_blocks, strict_fns);

    let kir = build_kir(
        &kobo_file,
        &mut session.id_gen,
        TransformOptions {
            small_struct_clone_threshold_bytes: session.config.small_struct_clone_threshold_bytes,
            copy_types: session.config.copy_types.clone(),
            mutating_methods: session.config.mutating_methods.clone(),
        },
    );

    // G5: copy relaxed fn ranges into session so the rendering path can filter warnings.
    session.relaxed_fn_ranges = kir.relaxed_fn_ranges().to_vec();

    Ok((kobo_file, kir))
}

pub fn run_check_pipeline(session: &mut CompileSession, input: &Path) -> Result<(), ()> {
    let (_, kir) = run_kir_phase(session, input)?;
    run_analysis_phase(session, &kir)
}

pub fn run_codegen_pipeline(
    session: &mut CompileSession,
    input: &Path,
) -> Result<CodegenArtifacts, ()> {
    let (kobo_file, kir) = run_kir_phase(session, input)?;
    run_analysis_phase(session, &kir)?;
    let (evidence, outcome) = resolve_solution(&kir);

    // Phase 00/05: Project non-Unique solver outcomes to K-code diagnostics.
    project_solver_diagnostics(session, &kir, &outcome);

    // Use solver solution for codegen when available (Phase 06), empty otherwise.
    let solution = match &outcome {
        SolveOutcome::Unique(map) => map.clone(),
        _ => SolutionMap::new(),
    };
    let rs_path = output_path_for(input, &session.config);
    let map_path = map_path_for(input, &session.config);
    let executor_choice = kobo_codegen::executor::select_executor(&session.config.dependencies);
    let CodegenOutput {
        rs_source,
        source_map,
    } = codegen_file(
        &kir,
        &kobo_file,
        &solution,
        input,
        &rs_path,
        &CodegenOptions {
            diag_mode: session.diag_enabled,
            executor_choice,
        },
    );

    // Inject solver evidence into source map before serialization.
    let injected_map = inject_solver_evidence(source_map, &evidence);

    let map_json = injected_map.to_json_string().map_err(|error| {
        eprintln!("kobo: failed to serialize source map: {error}");
    })?;

    if let Err(error) = write_rs_file(&rs_path, &rs_source) {
        eprintln!("kobo: write error: {error}");
        return Err(());
    }
    if let Err(error) = write_map_file(&map_path, &map_json) {
        eprintln!("kobo: write error: {error}");
        return Err(());
    }

    Ok(CodegenArtifacts {
        file_id: kobo_file.file_id,
        rs_source,
        rs_path,
        map_path,
        source_map: injected_map,
    })
}

pub fn run_and_compile(session: &mut CompileSession, input: &Path) -> Result<PathBuf, ()> {
    let artifacts = run_codegen_pipeline(session, input)?;
    let compile_output = compile_and_remap(
        session,
        &artifacts.rs_path,
        &artifacts.source_map,
        artifacts.file_id,
    )?;

    // v0.6 G4 §4.4: In checked mode, filter wrapper noise and remap surviving
    // warnings to .kobo spans, then push into session.diagnostics [R6-02].
    // ORDER IS CRITICAL: filter(§4.2) must use .rs spans → remap(§4.3) → merge.
    if session.mode().is_checked() && !compile_output.rustc_warnings.is_empty() {
        let kobo_regions = extract_kobo_regions(&artifacts.rs_source);
        let surviving = filter_wrapper_noise(&compile_output.rustc_warnings, &kobo_regions);
        remap_warnings_to_diagnostics(
            surviving,
            &artifacts.source_map,
            artifacts.file_id,
            session,
        );
    }

    Ok(compile_output.output_path)
}

pub fn run_pipeline(session: &mut CompileSession, input: &Path) -> Result<String, ()> {
    run_codegen_pipeline(session, input).map(|artifacts| artifacts.rs_source)
}

fn run_analysis_phase(session: &mut CompileSession, kir: &Kir) -> Result<(), ()> {
    let facts = run_analysis(kir, session.file_set());
    session.diagnostics.extend(facts_to_diagnostics(
        &facts,
        kir.transform_facts(),
        session.file_set(),
        kir,
        session.mode(),
    ));

    // Emit errors for malformed #[kobo::known_debt] attributes (C07).
    // K0025 malformed attribute is always Error — structural constraint [R6-03].
    for def in kir.struct_defs() {
        if let Some(error_msg) = &def.known_debt_parse_error {
            let span = def.known_debt_span.unwrap_or(def.span);
            let severity = resolve_severity(KErrorCode::K0025, session.mode())
                .unwrap_or(Severity::Error);
            session.diagnostics.push(KDiagnostic::new(
                KErrorCode::K0025,
                severity,
                DiagLabel::primary(span, error_msg.clone()),
                error_msg.clone(),
                DiagDecision(String::new()),
            ));
        }
    }

    // Emit diagnostics for #[kobo::relax] attribute validation errors/warnings [G5].
    // K0026 error path: structural validation errors use hardcoded Error (AC-19 exception).
    // K0026 warning path: use resolve_severity for consistency [BUG-06].
    for RelaxAttrError { span, message, is_error } in kir.relax_attr_errors() {
        let severity = if *is_error {
            Severity::Error
        } else {
            resolve_severity(KErrorCode::K0026, session.mode()).unwrap_or(Severity::Warning)
        };
        session.diagnostics.push(KDiagnostic::new(
            KErrorCode::K0026,
            severity,
            DiagLabel::primary(*span, message.clone()),
            message.clone(),
            DiagDecision(String::new()),
        ));
    }
    // Warn when #[kobo::relax] is used in script mode (has no effect).
    if session.mode().is_script() && !session.relaxed_fn_ranges.is_empty() {
        // Emit per-relaxed-fn advisory using the fn span itself.
        let severity = resolve_severity(KErrorCode::K0026, session.mode())
            .unwrap_or(Severity::Warning);
        for &fn_span in &session.relaxed_fn_ranges.clone() {
            session.diagnostics.push(KDiagnostic::new(
                KErrorCode::K0026,
                severity,
                DiagLabel::primary(fn_span, "`#[kobo::relax]` has no effect in script mode"),
                "`#[kobo::relax]` has no effect in script mode".to_owned(),
                DiagDecision(String::new()),
            ));
        }
    }

    // Emit K0080-P advisory notes for non-suppressed structural patterns.
    // Contract C08: these are always `note` severity, never `warning` or `error`.
    for fact in kir.warn_early_facts() {
        if fact.suppressed {
            continue;
        }
        let (code, label_text, explanation) = match &fact.pattern {
            WarnEarlyPattern::BidirectionalRcLinks { struct_name, field_pairs } => {
                let pairs_str = field_pairs
                    .iter()
                    .map(|(a, b)| format!("{a}↔{b}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                (
                    KErrorCode::K0080P1,
                    format!("bidirectional Rc links in `{struct_name}`"),
                    format!(
                        "struct `{struct_name}` has Rc<RefCell<T>> links that form a cycle ({pairs_str})\n   \
                         = this will leak memory unless Weak references are used"
                    ),
                )
            }
            WarnEarlyPattern::ParentChildBackPointer { struct_name, children_field, parent_field } => {
                (
                    KErrorCode::K0080P2,
                    format!("parent↔child back-pointer in `{struct_name}`"),
                    format!(
                        "struct `{struct_name}` has `{children_field}` (children) and `{parent_field}` (parent) — Rc cycle\n   \
                         = this will leak unless parent uses Weak references"
                    ),
                )
            }
            WarnEarlyPattern::SharedMutableAt3PlusSites { site_count, .. } => (
                KErrorCode::K0080P3,
                format!("shared mutable state at {site_count} call sites"),
                format!(
                    "the binding is mutated from {site_count} distinct call sites\n   \
                     = migration will require an architectural decision on ownership"
                ),
            ),
            WarnEarlyPattern::SelfReferentialStruct { struct_name } => (
                KErrorCode::K0080P4,
                format!("self-referential struct `{struct_name}` without indirection"),
                format!(
                    "struct `{struct_name}` contains a direct (non-indirected) field of the same type\n   \
                     = this would have infinite size; use Box<{struct_name}> or Rc<RefCell<{struct_name}>>"
                ),
            ),
        };

        // K0080P1-P4 structural advisories are always Note — not mode-dependent [R6-12].
        let advisory_severity = resolve_severity(code, session.mode()).unwrap_or(Severity::Note);
        session.diagnostics.push(KDiagnostic::new(
            code,
            advisory_severity,
            DiagLabel::primary(fact.span, label_text),
            explanation,
            DiagDecision("advisory only — no automatic fix; see `kobo debt` for migration guidance".to_owned()),
        ));
    }

    // Phase 11: Emit K006x diagnostics for async ownership violations.
    // BUG-5 fix: detect executor from config dependencies instead of hardcoding true.
    let has_executor = kobo_codegen::executor::select_executor(&session.config.dependencies)
        != kobo_codegen::executor::ExecutorChoice::None;
    let async_violations = check_strict_async(kir, session.mode(), has_executor);
    for violation in &async_violations {
        let (code, label_text, explanation) = match &violation.kind {
            AsyncViolationKind::NonSendCapture { binding_name, .. } => (
                KErrorCode::K0060,
                format!("binding `{binding_name}` is not Send"),
                format!(
                    "binding `{binding_name}` would be wrapped in Rc (not Send) but the async context requires Send\n   \
                     = kobo decision: refused to generate non-Send wrapper in async context"
                ),
            ),
            AsyncViolationKind::NonSyncShared { binding_name, .. } => (
                KErrorCode::K0061,
                format!("binding `{binding_name}` is not Sync for shared access"),
                format!(
                    "binding `{binding_name}` requires Sync for cross-task sharing but the current wrapper is not Sync\n   \
                     = kobo decision: consider restructuring with channels or an actor pattern"
                ),
            ),
            AsyncViolationKind::MissingExecutor => (
                KErrorCode::K0062,
                "no async executor configured".to_owned(),
                "async code detected but no executor (tokio/async-std) found in dependencies\n   \
                 = add tokio or async-std to [dependencies] in Cargo.toml".to_owned(),
            ),
            AsyncViolationKind::StrictAsyncViolation { binding_name, .. } => (
                KErrorCode::K0063,
                format!("strict mode: async wrapping not permitted for `{binding_name}`"),
                format!(
                    "in @strict mode, binding `{binding_name}` cannot use ownership wrappers in async context\n   \
                     = kobo decision: @strict requires zero-cost ownership — no Rc, Arc, or RefCell"
                ),
            ),
        };
        let severity = resolve_severity(code, session.mode()).unwrap_or(Severity::Error);
        session.diagnostics.push(KDiagnostic::new(
            code,
            severity,
            DiagLabel::primary(violation.span, label_text),
            explanation,
            DiagDecision(String::new()),
        ));
    }

    if session.has_errors() {
        Err(())
    } else {
        Ok(())
    }
}

fn resolve_solution(kir: &Kir) -> (SolverEvidence, SolveOutcome) {
    let graph = build_kir_constraint_graph(kir);
    let budget = SolverBudget::default();
    let evidence = solve_with_evidence(&graph, budget.clone());
    let outcome = kobo_migrate::solve(&graph, budget);
    (evidence, outcome)
}

/// Project non-Unique solver outcomes to K-code diagnostics (Phase 00/05).
fn project_solver_diagnostics(
    session: &mut CompileSession,
    kir: &Kir,
    outcome: &SolveOutcome,
) {
    match outcome {
        SolveOutcome::Unique(_) | SolveOutcome::MultiSolution(_) => {
            // Unique and multi-solution are not errors.
        }
        SolveOutcome::NoSolution(report) => {
            let span = report
                .conflicting_nodes
                .first()
                .and_then(|id| kir.get_node(*id))
                .map(|n| n.span)
                .unwrap_or(kobo_ir::KoboSpan::new(0, 0, kobo_ir::FileId(0)));
            let severity =
                resolve_severity(KErrorCode::K0080, session.mode()).unwrap_or(Severity::Warning);
            session.diagnostics.push(KDiagnostic::new(
                KErrorCode::K0080,
                severity,
                DiagLabel::primary(span, format!("ownership conflict: {}", report.conflict_reason)),
                format!(
                    "solver found no valid ownership assignment for {} node(s)",
                    report.conflicting_nodes.len()
                ),
                DiagDecision("review sharing patterns or add ownership hints".to_owned()),
            ));
        }
        SolveOutcome::ClusterTooLarge(report) => {
            let span = report
                .member_nodes
                .first()
                .and_then(|id| kir.get_node(*id))
                .map(|n| n.span)
                .unwrap_or(kobo_ir::KoboSpan::new(0, 0, kobo_ir::FileId(0)));
            let severity =
                resolve_severity(KErrorCode::K0081, session.mode()).unwrap_or(Severity::Warning);
            session.diagnostics.push(KDiagnostic::new(
                KErrorCode::K0081,
                severity,
                DiagLabel::primary(
                    span,
                    format!(
                        "constraint cluster too large ({} nodes, limit {})",
                        report.cluster_size, report.limit
                    ),
                ),
                format!(
                    "cluster of {} interdependent bindings exceeds solver limit of {}",
                    report.cluster_size, report.limit
                ),
                DiagDecision(
                    "split large functions or reduce sharing to lower cluster size".to_owned(),
                ),
            ));
        }
        SolveOutcome::BudgetExceeded(report) => {
            let span = kobo_ir::KoboSpan::new(0, 0, kobo_ir::FileId(0));
            let severity =
                resolve_severity(KErrorCode::K0082, session.mode()).unwrap_or(Severity::Warning);
            session.diagnostics.push(KDiagnostic::new(
                KErrorCode::K0082,
                severity,
                DiagLabel::primary(
                    span,
                    format!(
                        "solver budget exceeded ({:.1}s of {:.1}s, {}/{} resolved)",
                        report.elapsed_seconds,
                        report.budget_seconds,
                        report.resolved_count,
                        report.total_count,
                    ),
                ),
                format!(
                    "solver resolved {}/{} nodes before {:.1}s budget expired",
                    report.resolved_count, report.total_count, report.budget_seconds
                ),
                DiagDecision(
                    "increase solver_budget_seconds or reduce constraint complexity".to_owned(),
                ),
            ));
        }
        SolveOutcome::BoundaryStop(report) => {
            let span = kir
                .get_node(report.crossing_node)
                .map(|n| n.span)
                .unwrap_or(kobo_ir::KoboSpan::new(0, 0, kobo_ir::FileId(0)));
            let severity =
                resolve_severity(KErrorCode::K0090, session.mode()).unwrap_or(Severity::Warning);
            session.diagnostics.push(KDiagnostic::new(
                KErrorCode::K0090,
                severity,
                DiagLabel::primary(
                    span,
                    format!(
                        "ownership depends on external crate `{}`",
                        report.external_crate
                    ),
                ),
                format!(
                    "binding crosses into `{}::{}` — solver cannot infer ownership across crate boundaries",
                    report.external_crate, report.external_function
                ),
                DiagDecision(
                    "add an explicit ownership hint or use #[kobo::migrate]".to_owned(),
                ),
            ));
        }
    }
}

fn inject_solver_evidence(mut source_map: KoboSourceMap, evidence: &SolverEvidence) -> KoboSourceMap {
    // Set top-level solver evidence.
    source_map.solver_evidence = Some(SolverEvidenceJson {
        outcome: evidence.outcome_name.clone(),
        graph_fingerprint: evidence.graph_fingerprint.clone(),
        node_count: evidence.node_count as u64,
        edge_count: evidence.edge_count as u64,
        budget: SolverBudgetJson {
            max_cluster_size: evidence.budget.max_cluster_size as u64,
            budget_seconds: evidence.budget.budget_seconds,
        },
    });

    // Annotate each mapping with solver provenance.
    for entry in &mut source_map.x_kobo_mappings {
        entry.solver_outcome = Some(evidence.outcome_name.clone());
        entry.decision_source = Some("solver".to_owned());
    }

    source_map
}

/// Resolves the effective mode for a file.
///
/// Priority: CLI flag > file attribute (`//! kobo:mode = …`) > Kobo.toml [S-26].
pub fn effective_mode(
    cli_mode: Option<KoboMode>,
    file_mode: Option<KoboMode>,
    config_mode: KoboMode,
) -> KoboMode {
    cli_mode.or(file_mode).unwrap_or(config_mode)
}

#[cfg(test)]
mod tests {
    use kobo_ir::KoboMode;
    use super::effective_mode;

    /// S-26: CLI flag takes highest priority.
    #[test]
    fn cli_overrides_file_and_config() {
        let result = effective_mode(
            Some(KoboMode::Strict),
            Some(KoboMode::Checked),
            KoboMode::Script,
        );
        assert_eq!(result, KoboMode::Strict);
    }

    /// S-26: File attribute overrides Kobo.toml when no CLI flag.
    #[test]
    fn file_overrides_config() {
        let result = effective_mode(
            None,
            Some(KoboMode::Checked),
            KoboMode::Script,
        );
        assert_eq!(result, KoboMode::Checked);
    }

    /// S-26: Config (Kobo.toml) is the fallback.
    #[test]
    fn config_is_fallback() {
        let result = effective_mode(None, None, KoboMode::Checked);
        assert_eq!(result, KoboMode::Checked);
    }

    /// S-26: CLI overrides file attribute even when both are set.
    #[test]
    fn cli_overrides_file_when_both_set() {
        let result = effective_mode(
            Some(KoboMode::Script),
            Some(KoboMode::Strict),
            KoboMode::Checked,
        );
        assert_eq!(result, KoboMode::Script);
    }

    /// S-26: No file attribute + no CLI → uses config.
    #[test]
    fn no_file_no_cli_uses_config() {
        let result = effective_mode(None, None, KoboMode::Script);
        assert_eq!(result, KoboMode::Script);
    }

    /// S-8: spawn {} generates tokio::spawn(async move { ... }).
    #[test]
    fn spawn_generates_tokio_spawn() {
        let input = r#"
async fn main() {
    let x = 42;
    spawn {
        println!("{}", x);
    };
}
"#;
        let output = crate::test_utils::compile_and_inspect(input);
        assert!(
            output.contains("tokio::spawn(async move"),
            "expected tokio::spawn(async move in output, got:\n{output}"
        );
    }

    /// S-8 Step 2.5: async main with tokio dependency gets #[tokio::main].
    #[test]
    fn async_main_gets_executor_attribute() {
        let input = r#"
async fn main() {
    println!("hello");
}
"#;
        let mut config = crate::config::KoboConfig::default();
        config.dependencies.insert(
            "tokio".to_string(),
            toml::Value::String("1".into()),
        );
        let output = crate::test_utils::compile_and_inspect_with_config(input, config);
        assert!(
            output.contains("#[tokio::main]"),
            "expected #[tokio::main] in output, got:\n{output}"
        );
    }
}
