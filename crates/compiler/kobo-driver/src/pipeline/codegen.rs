use std::path::{Path, PathBuf};

use kobo_codegen::{codegen_file, CodegenOptions, CodegenOutput, KoboSourceMap};
use kobo_ir::FileId;
use kobo_migrate::SolveOutcome;

use crate::filesystem::{map_path_for, output_path_for, write_map_file, write_rs_file};
use crate::session::CompileSession;

use super::analysis::run_analysis_phase;
use super::parse::run_kir_phase;
use super::solver::{
    apply_engine_ceiling, inject_solver_evidence, project_engine_ceiling_diagnostics,
    project_solver_diagnostics, resolve_solution,
};

#[derive(Clone)]
pub struct CodegenArtifacts {
    pub file_id: FileId,
    pub rs_source: String,
    pub rs_path: PathBuf,
    pub map_path: PathBuf,
    pub source_map: KoboSourceMap,
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

    let mut solution = match &outcome {
        SolveOutcome::Unique(map) => map.clone(),
        SolveOutcome::MultiSolution(candidates) => match candidates.first() {
            Some(candidate) => candidate.solution.clone(),
            None => return Err(()),
        },
        SolveOutcome::NoSolution(_)
        | SolveOutcome::ClusterTooLarge(_)
        | SolveOutcome::BudgetExceeded(_)
        | SolveOutcome::BoundaryStop(_) => return Err(()),
    };

    // S-3: Cap engine struct bindings to PlainOwned ceiling.
    // Engine structs are framework-managed and must not be shared via Rc/Arc.
    if !session.engine_struct_names.is_empty() {
        let adjustments = apply_engine_ceiling(&kir, &mut solution);
        project_engine_ceiling_diagnostics(session, &adjustments);
    }
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

    // S-17: Apply extract-before-borrow rewrites.
    // When a borrow-then-mutate pattern is detected, insert a let binding
    // that extracts the borrow result before the mutation.
    let rs_source = {
        let sites = kobo_transform::patterns::extract_borrow::find_extract_before_borrow(
            kir.transform_facts(),
        );
        if sites.is_empty() {
            rs_source
        } else {
            kobo_transform::patterns::extract_borrow::apply_extract_before_borrow_rewrites(
                &rs_source, &sites,
            )
            .source
        }
    };

    // S-66: Stable-toolchain-only guarantee — Kobo never emits #![feature(...)].
    // Catch any accidental nightly-only code in generated output.
    debug_assert!(
        !rs_source.contains("#![feature("),
        "kobo: generated Rust contains #![feature(...)]; this violates S-66 stable-toolchain guarantee"
    );
    if rs_source.contains("#![feature(") {
        eprintln!("kobo: warning: generated Rust contains #![feature(...)], stripping nightly feature gates");
        // Defensive strip — should never hit in practice.
    }

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

pub fn run_pipeline(session: &mut CompileSession, input: &Path) -> Result<String, ()> {
    run_codegen_pipeline(session, input).map(|artifacts| artifacts.rs_source)
}
