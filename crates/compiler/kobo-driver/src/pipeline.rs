use std::path::{Path, PathBuf};

use kobo_analysis::{facts_to_diagnostics, run_analysis};
use kobo_codegen::{codegen_file, CodegenOptions, CodegenOutput, KoboSourceMap};
use kobo_errors::{resolve_severity, DiagDecision, DiagLabel, KDiagnostic, KErrorCode, Severity};
use kobo_ir::{FileId, Kir, RelaxAttrError, SolutionMap, WarnEarlyPattern};
use kobo_migrate::{solve, ConstraintGraph, SolveResult, SolverBudget};
use kobo_parser::{
    collect_strict_items_from_syn, parse_file, postprocess_strict_markers,
    preprocess_kobo_keywords, preprocess_strict_reject_invalid, v05_keyword_configs, KoboFile,
};
use kobo_transform::{build_kir, TransformOptions};

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

    // v0.5 preprocessing: rewrite @strict → marker attributes before syn parse.
    let configs = v05_keyword_configs();
    if let Err(e) = preprocess_strict_reject_invalid(&source, &configs) {
        eprintln!("kobo: preprocess error: {e}");
        return Err(());
    }
    let (rewritten, markers) = preprocess_kobo_keywords(&source, &configs);

    let mut kobo_file = match parse_file(&rewritten, file_id, &mut session.id_gen) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("kobo: parse error: {error}");
            return Err(());
        }
    };

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
    let solution = resolve_solution();
    let rs_path = output_path_for(input, &session.config);
    let map_path = map_path_for(input, &session.config);
    let CodegenOutput {
        rs_source,
        source_map,
    } = codegen_file(
        &kir,
        &kobo_file,
        &solution,
        input,
        &rs_path,
        &CodegenOptions { diag_mode: session.diag_enabled },
    );
    let map_json = source_map.to_json_string().map_err(|error| {
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
        source_map,
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
    // "in script mode" advisory: relax has no effect in script mode.
    for RelaxAttrError { span, message, is_error } in kir.relax_attr_errors() {
        let severity = if *is_error { Severity::Error } else { Severity::Warning };
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
        for &fn_span in &session.relaxed_fn_ranges.clone() {
            session.diagnostics.push(KDiagnostic::new(
                KErrorCode::K0026,
                Severity::Warning,
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

    if session.has_errors() {
        Err(())
    } else {
        Ok(())
    }
}

fn resolve_solution() -> SolutionMap {
    let graph = ConstraintGraph;
    let budget = SolverBudget::default();
    match solve(&graph, budget) {
        SolveResult::Unique(map) | SolveResult::MultiSolution(map) | SolveResult::Timeout(map) => {
            map
        }
        SolveResult::Conflict => SolutionMap::new(),
    }
}
