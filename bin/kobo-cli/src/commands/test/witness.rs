mod sections;

use super::fuzz::{fuzz_plan_json, FuzzPlan};
use super::json_schema::{
    events_json, one_based_line_for_offset, sanitize_name, scheduler_json, sim_config_json,
};
use super::proof::write_proof_artifact;
use super::replay_scope::{backend_replay_evidence_json, backend_replay_id};
use super::{
    serde_json, sim_model, BackendExpertOptions, Context, FullDepthRun, Path, PathBuf,
    ScenarioDocument, ScenarioProgram,
};
use sections::{
    base_witness_object, insert_coverage_and_boundary_sections,
    insert_policy_and_scheduler_sections, insert_proof_and_model_sections, insert_runtime_sections,
};

pub(super) fn print_events(
    file: &Path,
    sim_profile: &str,
    seed: u64,
    fuzz_plan: Option<&FuzzPlan>,
    expert_options: &BackendExpertOptions,
    effective_max_branches: Option<u64>,
    effective_shrink: &str,
    effective_replay_token: &str,
    effective_checkpoint_replay: bool,
    config: &kobo_driver::KoboConfig,
    effective_scheduler: Option<&str>,
    run: &FullDepthRun,
) -> anyhow::Result<()> {
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "schema_version": 1,
            "file": sim_model::cli_relative_path(file)?,
            "sim_profile": sim_profile,
            "backend_profile": run.profile,
            "backend": expert_options.backend_name(&run.profile),
            "scheduler": scheduler_json(sim_profile, seed, run, effective_scheduler),
            "backend_native": expert_options.backend_native,
            "max_branches": effective_max_branches,
            "shrink": effective_shrink,
            "replay_token": effective_replay_token,
            "checkpoint_replay": effective_checkpoint_replay,
            "sim_config": sim_config_json(config),
            "fuzz": fuzz_plan_json(fuzz_plan),
            "seed": seed,
            "events": events_json(&run.events),
        }))?
    );
    Ok(())
}

pub(super) fn write_run_witness(
    file: &Path,
    document: &ScenarioDocument,
    scenario_program: &ScenarioProgram,
    guarantee_profile: &str,
    sim_profile: &str,
    seed: u64,
    inject: Option<&str>,
    fuzz_plan: Option<&FuzzPlan>,
    witness_dir: Option<&Path>,
    config: &kobo_driver::KoboConfig,
    runtime_evidence: &kobo_codegen::RuntimeEvidence,
    source_map: &kobo_codegen::KoboSourceMap,
    expert_options: &BackendExpertOptions,
    effective_scheduler: Option<&str>,
    effective_max_branches: Option<u64>,
    effective_shrink: &str,
    effective_replay_token: &str,
    effective_checkpoint_replay: bool,
    run: &FullDepthRun,
) -> anyhow::Result<PathBuf> {
    let directory = witness_directory(file, witness_dir)?;
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create witness directory {}", directory.display()))?;
    let witness_path = directory.join(format!("{}-{seed}.kwit", sanitize_name(&run.target)));
    let source_path = sim_model::cli_relative_path(file)?;
    let primary_span = run.failure.as_ref().map(|failure| {
        format!(
            "{}:{}:1",
            source_path,
            one_based_line_for_offset(&document.source, failure.primary_start)
        )
    });
    let witness = build_witness_json(
        file,
        document,
        scenario_program,
        guarantee_profile,
        sim_profile,
        seed,
        inject,
        fuzz_plan,
        config,
        runtime_evidence,
        expert_options,
        effective_scheduler,
        effective_max_branches,
        effective_shrink,
        effective_replay_token,
        effective_checkpoint_replay,
        run,
        &source_path,
        primary_span,
    )?;
    std::fs::write(&witness_path, serde_json::to_string_pretty(&witness)?)
        .with_context(|| format!("failed to write {}", witness_path.display()))?;
    write_proof_artifact(
        file,
        document,
        scenario_program,
        run,
        &witness_path,
        config,
        source_map,
    )?;
    Ok(witness_path)
}

fn build_witness_json(
    file: &Path,
    document: &ScenarioDocument,
    scenario_program: &ScenarioProgram,
    guarantee_profile: &str,
    sim_profile: &str,
    seed: u64,
    inject: Option<&str>,
    fuzz_plan: Option<&FuzzPlan>,
    config: &kobo_driver::KoboConfig,
    runtime_evidence: &kobo_codegen::RuntimeEvidence,
    expert_options: &BackendExpertOptions,
    effective_scheduler: Option<&str>,
    effective_max_branches: Option<u64>,
    effective_shrink: &str,
    effective_replay_token: &str,
    effective_checkpoint_replay: bool,
    run: &FullDepthRun,
    source_path: &str,
    primary_span: Option<String>,
) -> anyhow::Result<serde_json::Value> {
    let backend_name = expert_options.backend_name(&run.profile);
    let backend_replay =
        backend_replay_id(effective_replay_token, &document.source_hash, seed, run);
    let backend_replay_evidence =
        backend_replay_evidence_json(effective_replay_token, &document.source_hash, seed, run);
    let mut object = base_witness_object(
        source_path,
        guarantee_profile,
        sim_profile,
        seed,
        backend_name,
        backend_replay,
        backend_replay_evidence,
        run,
    );
    insert_policy_and_scheduler_sections(
        &mut object,
        source_path,
        document,
        guarantee_profile,
        sim_profile,
        seed,
        inject,
        fuzz_plan,
        config,
        expert_options,
        effective_scheduler,
        effective_max_branches,
        effective_replay_token,
        effective_checkpoint_replay,
        run,
        backend_name,
    )?;
    insert_coverage_and_boundary_sections(
        &mut object,
        file,
        document,
        scenario_program,
        config,
        run,
        source_path,
        primary_span,
        sim_profile,
        effective_shrink,
    )?;
    insert_proof_and_model_sections(
        &mut object,
        file,
        document,
        scenario_program,
        config,
        run,
        seed,
        fuzz_plan,
        source_path,
    )?;
    insert_runtime_sections(
        &mut object,
        config,
        runtime_evidence,
        run,
        sim_profile,
        seed,
    )?;
    Ok(serde_json::Value::Object(object))
}

pub(super) fn witness_directory(
    file: &Path,
    witness_dir: Option<&Path>,
) -> anyhow::Result<PathBuf> {
    match witness_dir {
        Some(directory) if directory.is_absolute() => Ok(directory.to_path_buf()),
        Some(directory) => Ok(std::env::current_dir()
            .context("failed to determine current directory")?
            .join(directory)),
        None => Ok(file
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))),
    }
}
