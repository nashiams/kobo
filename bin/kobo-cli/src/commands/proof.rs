use std::path::{Path, PathBuf};

use anyhow::Context;
use kobo_driver::run_codegen_pipeline;
use kobo_proof::{
    parse_certificate_json, verify_certificate, ArtifactKind, ProofCertificate,
    VerificationContext, VerificationError, VerificationReport,
};

use crate::ProofReplayGradeArg;

use super::session::{build_session, render_diagnostics};

pub(super) fn cmd_emit(
    file: &Path,
    target: Option<&str>,
    output: Option<&Path>,
    replay_grade: ProofReplayGradeArg,
) -> anyhow::Result<()> {
    let artifact_path = output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| file.with_extension("kproof"));
    let artifact_kind = artifact_kind_for_path(&artifact_path);
    let (certificate, source) = emit_certificate(file, target, replay_grade, artifact_kind)?;
    verify_before_write(&certificate, &source)?;
    write_certificate(&artifact_path, &certificate)?;
    println!(
        "proof emitted: {} (claim: modeled Core obligation flow)",
        artifact_path.display()
    );
    Ok(())
}

pub(super) fn cmd_verify(artifact: &Path, json: bool) -> anyhow::Result<()> {
    let certificate = read_certificate(artifact)?;
    let source = read_certificate_source(artifact, &certificate)?;
    match verify_certificate(
        &certificate,
        &VerificationContext {
            source: source.clone(),
        },
    ) {
        Ok(report) => {
            emit_verified(artifact, &certificate, &report, json)?;
            Ok(())
        }
        Err(error) => {
            emit_rejected(artifact, &error, json)?;
            anyhow::bail!("{error}")
        }
    }
}

pub(super) fn emit_check_proof(
    file: &Path,
    replay_grade: ProofReplayGradeArg,
) -> anyhow::Result<()> {
    let artifact_path = file.with_extension("kproof");
    let (certificate, source) = emit_certificate(file, None, replay_grade, ArtifactKind::Kproof)?;
    verify_before_write(&certificate, &source)?;
    write_certificate(&artifact_path, &certificate)?;
    eprintln!("proof emitted: {}", artifact_path.display());
    Ok(())
}

fn emit_certificate(
    file: &Path,
    target: Option<&str>,
    replay_grade: ProofReplayGradeArg,
    artifact_kind: ArtifactKind,
) -> anyhow::Result<(ProofCertificate, String)> {
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read source {}", file.display()))?;
    let mut session = build_session(file, None)?;
    let artifacts = match run_codegen_pipeline(&mut session, file) {
        Ok(artifacts) => artifacts,
        Err(()) => {
            render_diagnostics(&session);
            anyhow::bail!("proof emission requires a checkable modeled scenario")
        }
    };
    let program = select_scenario_program(&artifacts.scenario_programs, target)?;
    let certificate =
        kobo_driver::proof::emit_proof_certificate(kobo_driver::proof::ProofEmissionInput {
            source_path: file,
            source: &source,
            program,
            adapter_policies: &session.config.ecosystem_policy.adapters,
            replay_grade: replay_grade.proof_grade(),
            artifact_kind,
        })?;
    Ok((certificate, source))
}

fn select_scenario_program<'a>(
    programs: &'a [kobo_ir::ScenarioProgram],
    target: Option<&str>,
) -> anyhow::Result<&'a kobo_ir::ScenarioProgram> {
    if let Some(target) = target {
        return programs
            .iter()
            .find(|program| program.target == target)
            .with_context(|| format!("no modeled scenario named `{target}`"));
    }
    match programs {
        [program] => Ok(program),
        [] => anyhow::bail!("proof emission requires a #[kobo::scenario] function"),
        _ => anyhow::bail!("multiple scenarios found; pass --target"),
    }
}

fn verify_before_write(certificate: &ProofCertificate, source: &str) -> anyhow::Result<()> {
    verify_certificate(
        certificate,
        &VerificationContext {
            source: source.to_owned(),
        },
    )
    .map(|_| ())
    .map_err(|error| anyhow::anyhow!(error))
}

fn write_certificate(path: &Path, certificate: &ProofCertificate) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create proof directory {}", parent.display()))?;
    }
    std::fs::write(path, serde_json::to_string_pretty(certificate)?)
        .with_context(|| format!("failed to write proof artifact {}", path.display()))
}

fn read_certificate(path: &Path) -> anyhow::Result<ProofCertificate> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read proof artifact {}", path.display()))?;
    parse_certificate_json(&source).map_err(|error| anyhow::anyhow!(error))
}

fn read_certificate_source(
    artifact: &Path,
    certificate: &ProofCertificate,
) -> anyhow::Result<String> {
    let source_path = PathBuf::from(&certificate.source.path);
    let resolved = if source_path.is_absolute() {
        source_path
    } else {
        artifact
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(source_path)
    };
    std::fs::read_to_string(&resolved)
        .with_context(|| format!("failed to read certificate source {}", resolved.display()))
}

fn emit_verified(
    artifact: &Path,
    certificate: &ProofCertificate,
    report: &VerificationReport,
    json: bool,
) -> anyhow::Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "status": "verified",
                "artifact": artifact.display().to_string(),
                "replay_grade": &certificate.replay_grade,
                "adapter_confidence": &certificate.adapter_confidence,
                "candidate_admission": &certificate.candidate_admission,
                "source_hash": report.source_hash,
                "core_hash": report.core_hash,
                "checked_obligation_events": report.checked_obligation_events,
                "certificate_material_hash": report.certificate_material_hash,
            }))?
        );
    } else {
        println!(
            "proof verified: {} ({} obligation event(s) replayed)",
            artifact.display(),
            report.checked_obligation_events
        );
    }
    Ok(())
}

fn emit_rejected(artifact: &Path, error: &VerificationError, json: bool) -> anyhow::Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "status": "rejected",
                "artifact": artifact.display().to_string(),
                "error": error.to_string(),
            }))?
        );
    } else {
        eprintln!("proof rejected: {}: {error}", artifact.display());
    }
    Ok(())
}

fn artifact_kind_for_path(path: &Path) -> ArtifactKind {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if file_name.ends_with(".kwit.proof.json") {
        ArtifactKind::KwitProofJson
    } else {
        ArtifactKind::Kproof
    }
}
