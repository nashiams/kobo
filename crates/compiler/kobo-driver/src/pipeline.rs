use std::path::Path;

use kobo_codegen::{annotate, emit_file, lower};
use kobo_ir::{Kir, SolutionMap};
use kobo_migrate::{solve, ConstraintGraph, SolverBudget};
use kobo_parser::{parse_file, KoboFile};
use kobo_transform::build_kir;

use crate::filesystem::{output_path_for, read_kobo_file, write_rs_file};
use crate::session::CompileSession;

/// Runs the pipeline through the KIR phase and returns the parsed AST plus frozen KIR.
pub fn run_kir_phase(session: &mut CompileSession, input: &Path) -> Result<(KoboFile, Kir), ()> {
    let (file_id, source) = match read_kobo_file(input, &mut session.file_set) {
        Ok(pair) => pair,
        Err(error) => {
            eprintln!("kobo: io error: {error}");
            return Err(());
        }
    };

    let kobo_file = match parse_file(&source, file_id, &mut session.id_gen) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("kobo: parse error: {error}");
            return Err(());
        }
    };

    if session.has_errors() {
        return Err(());
    }

    let kir = build_kir(&kobo_file, &mut session.id_gen);
    Ok((kobo_file, kir))
}

/// Runs the full `.kobo -> .rs` compilation pipeline for one input file.
///
/// Returns the formatted `.rs` string on success, or `Err(())` if any
/// hard error was accumulated. Callers read `session.diagnostics` for details.
pub fn run_pipeline(session: &mut CompileSession, input: &Path) -> Result<String, ()> {
    let (kobo_file, kir) = run_kir_phase(session, input)?;
    let solution = {
        let graph = ConstraintGraph;
        let budget = SolverBudget::default();
        match solve(&graph, budget) {
            kobo_migrate::SolveResult::Unique(map) => map,
            kobo_migrate::SolveResult::MultiSolution(map) => map,
            kobo_migrate::SolveResult::Timeout(map) => map,
            kobo_migrate::SolveResult::Conflict => SolutionMap::new(),
        }
    };

    let mut rs_file = lower(&kir, &kobo_file, &solution);
    annotate(&mut rs_file, &kir);
    let rs_string = emit_file(&rs_file);

    let output_path = output_path_for(input, &session.config);
    if let Err(error) = write_rs_file(&output_path, &rs_string) {
        eprintln!("kobo: write error: {error}");
        return Err(());
    }

    Ok(rs_string)
}
