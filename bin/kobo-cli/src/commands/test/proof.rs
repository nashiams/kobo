use super::{
    runtime_boundary_evidence, serde_json, witness_evidence, Context, FullDepthRun, Path, PathBuf,
    ReplayGuarantee, ScenarioDocument, ScenarioProgram,
};
pub(super) fn write_proof_artifact(
    file: &Path,
    document: &ScenarioDocument,
    scenario_program: &ScenarioProgram,
    run: &FullDepthRun,
    witness_path: &Path,
    config: &kobo_driver::KoboConfig,
    source_map: &kobo_codegen::KoboSourceMap,
) -> anyhow::Result<()> {
    if run.failure.is_some() {
        return Ok(());
    }
    let runtime_boundaries = runtime_boundary_evidence(run);
    let certificate =
        kobo_driver::proof::emit_proof_certificate(kobo_driver::proof::ProofEmissionInput {
            source_path: file,
            source: &document.source,
            program: scenario_program,
            adapter_policies: &config.ecosystem_policy.adapters,
            runtime_boundaries: &runtime_boundaries,
            replay_grade: proof_replay_grade(&run.replay_guarantee),
            artifact_kind: kobo_driver::proof::ArtifactKind::KwitProofJson,
            source_map: Some(source_map),
        })?;
    let source_map_json = source_map.to_json_string()?;
    kobo_proof::verify_certificate(
        &certificate,
        &kobo_proof::VerificationContext {
            source: document.source.clone(),
            source_map: Some(source_map_json),
        },
    )?;
    let proof_path = kwit_proof_path(witness_path);
    std::fs::write(&proof_path, serde_json::to_string_pretty(&certificate)?)
        .with_context(|| format!("failed to write {}", proof_path.display()))?;
    Ok(())
}

pub(super) fn proof_replay_grade(guarantee: &ReplayGuarantee) -> kobo_driver::proof::ReplayGrade {
    match guarantee {
        ReplayGuarantee::Exact => kobo_driver::proof::ReplayGrade::Exact,
        ReplayGuarantee::Partial => kobo_driver::proof::ReplayGrade::Partial,
        ReplayGuarantee::NotReplayable => kobo_driver::proof::ReplayGrade::NotReplayable,
    }
}

pub(super) fn witness_replay_grade(
    run: &FullDepthRun,
    fuzz_enabled: bool,
    adjusted_proof_grade: &kobo_driver::proof::ReplayGrade,
) -> &'static str {
    let base = witness_evidence::replay_grade_json(run, fuzz_enabled);
    if !matches!(base, "exact" | "partial" | "not_replayable") {
        return base;
    }
    match adjusted_proof_grade {
        kobo_driver::proof::ReplayGrade::Exact => "exact",
        kobo_driver::proof::ReplayGrade::Partial => "partial",
        kobo_driver::proof::ReplayGrade::NotReplayable => "not_replayable",
        kobo_driver::proof::ReplayGrade::Debt => "debt",
    }
}

fn kwit_proof_path(witness_path: &Path) -> PathBuf {
    let proof_file_name = witness_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| format!("{name}.proof.json"))
        .unwrap_or_else(|| "witness.kwit.proof.json".to_owned());
    witness_path.with_file_name(proof_file_name)
}
