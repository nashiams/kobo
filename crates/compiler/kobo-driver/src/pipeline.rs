use std::path::{Path, PathBuf};

use kobo_analysis::{facts_to_diagnostics, run_analysis};
use kobo_codegen::{codegen_file, CodegenOutput, KoboSourceMap};
use kobo_ir::{FileId, Kir, SolutionMap};
use kobo_migrate::{solve, ConstraintGraph, SolveResult, SolverBudget};
use kobo_parser::{parse_file, KoboFile};
use kobo_transform::build_kir;

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

    let kobo_file = match parse_file(&source, file_id, &mut session.id_gen) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("kobo: parse error: {error}");
            return Err(());
        }
    };

    let kir = build_kir(&kobo_file, &mut session.id_gen);
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
    } = codegen_file(&kir, &kobo_file, &solution, input, &rs_path);
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
    session
        .diagnostics
        .extend(facts_to_diagnostics(&facts, session.file_set()));

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
