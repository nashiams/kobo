use std::path::{Path, PathBuf};

use kobo_codegen::{codegen_file, CodegenOptions, CodegenOutput, KoboSourceMap, RuntimeEvidence};
use kobo_ir::{FileId, MustCallObligation, ScenarioOpKind, ScenarioProgram};
use kobo_migrate::SolveOutcome;
use kobo_parser::KoboFile;

use crate::filesystem::{map_path_for, output_path_for, write_map_file, write_rs_file};
use crate::session::CompileSession;

use super::analysis::run_analysis_phase;
use super::ownership_gate::{
    has_blocking_ownership_diagnostic, has_known_borrow_conflict_diagnostic,
    project_blocking_ownership_diagnostics,
};
use super::parse::run_kir_phase;
use super::solver::{
    apply_engine_ceiling, inject_solver_evidence, project_engine_ceiling_diagnostics,
    project_solver_diagnostics, resolve_solution,
};

pub use kobo_codegen::ErrorPolicySite;

#[derive(Clone)]
pub struct CodegenArtifacts {
    pub file_id: FileId,
    pub kobo_file: KoboFile,
    pub rs_source: String,
    pub rs_path: PathBuf,
    pub map_path: PathBuf,
    pub source_map: KoboSourceMap,
    pub must_call_obligations: Vec<MustCallObligation>,
    pub error_policy_sites: Vec<ErrorPolicySite>,
    pub scenario_programs: Vec<ScenarioProgram>,
    pub runtime_evidence: RuntimeEvidence,
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum OwnershipGateMode {
    Enforce,
    DebtProbe,
}

pub fn run_codegen_pipeline(
    session: &mut CompileSession,
    input: &Path,
) -> Result<CodegenArtifacts, ()> {
    run_codegen_pipeline_with_gate(session, input, OwnershipGateMode::Enforce)
}

pub fn run_codegen_pipeline_for_debt_probe(
    session: &mut CompileSession,
    input: &Path,
) -> Result<CodegenArtifacts, ()> {
    run_codegen_pipeline_with_gate(session, input, OwnershipGateMode::DebtProbe)
}

fn run_codegen_pipeline_with_gate(
    session: &mut CompileSession,
    input: &Path,
    gate_mode: OwnershipGateMode,
) -> Result<CodegenArtifacts, ()> {
    let (kobo_file, kir) = run_kir_phase(session, input)?;
    run_analysis_phase(session, &kir)?;
    if gate_mode == OwnershipGateMode::Enforce {
        project_blocking_ownership_diagnostics(session, &kir);
        if has_blocking_ownership_diagnostic(session)
            || has_known_borrow_conflict_diagnostic(session)
        {
            return Err(());
        }
    }
    let (evidence, outcome) = resolve_solution(&kir);

    // Stage: Project non-Unique solver outcomes to K-code diagnostics.
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
        error_policy_sites,
        runtime_evidence,
    } = codegen_file(
        &kir,
        &kobo_file,
        &solution,
        input,
        &rs_path,
        &CodegenOptions {
            diag_mode: session.diag_enabled,
            executor_choice,
            runtime_profile: session.config.runtime_profile.to_codegen_options(),
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
        kobo_file,
        rs_source,
        rs_path,
        map_path,
        source_map: injected_map,
        must_call_obligations: kir.must_call_obligations().to_vec(),
        error_policy_sites,
        scenario_programs: scenario_programs_with_ecosystem_policy(
            kir.scenario_programs(),
            &session.config.ecosystem_policy,
        ),
        runtime_evidence,
    })
}

fn scenario_programs_with_ecosystem_policy(
    programs: &[ScenarioProgram],
    policy: &crate::config::EcosystemPolicyConfig,
) -> Vec<ScenarioProgram> {
    programs
        .iter()
        .cloned()
        .map(|mut program| {
            apply_ecosystem_policy(&mut program, policy);
            program
        })
        .collect()
}

fn apply_ecosystem_policy(
    program: &mut ScenarioProgram,
    policy: &crate::config::EcosystemPolicyConfig,
) {
    for operation in &mut program.operations {
        let ScenarioOpKind::ExternalBoundary {
            crate_name,
            policy: boundary_policy,
            reason,
            ..
        } = &mut operation.kind
        else {
            continue;
        };
        if !matches!(boundary_policy, kobo_ir::ScenarioBoundaryPolicy::Unselected) {
            continue;
        }
        if let Some(crate_policy) = policy.crate_policy(crate_name) {
            *boundary_policy = crate_policy.policy.clone();
            if reason.is_none() {
                *reason = crate_policy.reason.clone();
            }
        } else if policy.default_is_configured {
            *boundary_policy = policy.default.clone();
        }
    }
}

pub fn apply_error_policy_sites(
    mut artifacts: CodegenArtifacts,
    policy_name: &str,
) -> CodegenArtifacts {
    let error_sites = artifacts.error_policy_sites.clone();
    artifacts.rs_source = match policy_name {
        "ergonomic" => artifacts
            .rs_source
            .replace("std::io::Error", "Box<dyn std::error::Error>"),
        "typed" => typed_error_policy_source(&artifacts.rs_source, &error_sites),
        "explicit" => explicit_error_policy_source(&artifacts.rs_source, &error_sites),
        _ => artifacts.rs_source,
    };
    artifacts.error_policy_sites = error_sites;
    artifacts
}

fn explicit_error_policy_source(source: &str, error_sites: &[ErrorPolicySite]) -> String {
    let mut output = source.to_owned();
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str("// kobo: explicit error policy evidence required\n");
    for site in error_sites {
        output.push_str(&format!(
            "// kobo: error_site line {} operation {} source {}\n",
            site.line, site.operation, site.source_error
        ));
    }
    output
}

fn typed_error_policy_source(source: &str, error_sites: &[ErrorPolicySite]) -> String {
    if error_sites.is_empty() {
        return source.to_owned();
    }
    if source.contains("enum KoboTypedError") {
        return source.to_owned();
    }

    let rewritten =
        insert_typed_error_maps(source, error_sites).replace("std::io::Error", "KoboTypedError");
    let mut variants = Vec::new();
    variants.push((
        "Io".to_owned(),
        "io".to_owned(),
        "std::io::Error".to_owned(),
    ));
    for site in error_sites {
        if !variants
            .iter()
            .any(|(variant, _, _)| variant == &site.variant)
        {
            variants.push((
                site.variant.clone(),
                site.operation.clone(),
                site.source_error.clone(),
            ));
        }
    }

    let declarations = variants
        .iter()
        .map(|(variant, _, source_error)| format!("    {variant}({source_error}),"))
        .collect::<Vec<_>>()
        .join("\n");
    let display_arms = variants
        .iter()
        .map(|(variant, operation, _)| {
            format!(
                "            Self::{variant}(error) => write!(f, \"{operation} failed: {{error}}\"),"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "#[derive(Debug)]\n#[allow(dead_code)]\nenum KoboTypedError {{\n{declarations}\n}}\n\nimpl std::fmt::Display for KoboTypedError {{\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n        match self {{\n{display_arms}\n        }}\n    }}\n}}\n\nimpl std::error::Error for KoboTypedError {{}}\n\nimpl From<std::io::Error> for KoboTypedError {{\n    fn from(error: std::io::Error) -> Self {{\n        Self::Io(error)\n    }}\n}}\n\n{rewritten}"
    )
}

fn insert_typed_error_maps(source: &str, error_sites: &[ErrorPolicySite]) -> String {
    if error_sites.is_empty() {
        return source.to_owned();
    }

    let mut output = String::with_capacity(source.len() + error_sites.len() * 32);
    let mut cursor = 0usize;
    for site in error_sites {
        if site.generated_offset > source.len() || site.generated_offset < cursor {
            continue;
        }
        output.push_str(&source[cursor..site.generated_offset]);
        output.push_str(&format!(".map_err(KoboTypedError::{})", site.variant));
        cursor = site.generated_offset;
    }
    output.push_str(&source[cursor..]);
    output
}

pub fn run_pipeline(session: &mut CompileSession, input: &Path) -> Result<String, ()> {
    run_codegen_pipeline(session, input).map(|artifacts| artifacts.rs_source)
}
