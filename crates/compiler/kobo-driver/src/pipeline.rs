use std::path::{Path, PathBuf};

use kobo_analysis::{facts_to_diagnostics, run_analysis};
use kobo_codegen::{codegen_file, CodegenOptions, CodegenOutput, KoboSourceMap};
use kobo_errors::{DiagDecision, DiagLabel, KDiagnostic, KErrorCode, Severity};
use kobo_ir::{FileId, Kir, SolutionMap, WarnEarlyPattern};
use kobo_migrate::{solve, ConstraintGraph, SolveResult, SolverBudget};
use kobo_parser::{
    collect_strict_items_from_syn, parse_file, postprocess_strict_markers,
    preprocess_kobo_keywords, preprocess_strict_reject_invalid, v05_keyword_configs, KoboFile,
};
use kobo_transform::{build_kir, TransformOptions};

use crate::filesystem::{
    map_path_for, output_path_for, read_kobo_file, write_map_file, write_rs_file,
};
use crate::rustc::compile_and_remap;
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
    compile_and_remap(
        session,
        &artifacts.rs_path,
        &artifacts.source_map,
        artifacts.file_id,
    )
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
    ));

    // Emit errors for malformed #[kobo::known_debt] attributes (C07).
    for def in kir.struct_defs() {
        if let Some(error_msg) = &def.known_debt_parse_error {
            let span = def.known_debt_span.unwrap_or(def.span);
            session.diagnostics.push(KDiagnostic::new(
                KErrorCode::K0025,
                Severity::Error,
                DiagLabel::primary(span, error_msg.clone()),
                error_msg.clone(),
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

        session.diagnostics.push(KDiagnostic::new(
            code,
            Severity::Note,
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
