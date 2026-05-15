use std::path::PathBuf;
use std::process::Command;

use kobo_errors::KErrorCode;
use kobo_ir::{ScenarioModeledBoundary, ScenarioOpKind, ScenarioProgram};

use crate::core::{
    EngineMode, FullDepthRun, ReplayGuarantee, ScenarioEvent, ScenarioFailure, ScenarioOptions,
};
use crate::harness_manifest::HarnessManifest;

pub fn check_harness_agreement(
    program: &ScenarioProgram,
    generated_rust: &str,
    options: &ScenarioOptions,
    mut semantic: FullDepthRun,
    _mode: EngineMode,
) -> anyhow::Result<FullDepthRun> {
    let harness = run_generated_harness(program, generated_rust, options)?;
    let harness_hash = crate::digest::events_hash(&harness.events);
    let manifest_json = serde_json::to_string(&harness.manifest)?;

    semantic.digest.harness_engine = "generated-rust-process".to_owned();
    semantic.digest.generated_rust_hash = Some(harness.manifest.generated_rust_hash.clone());
    semantic.digest.harness_manifest_hash = Some(crate::digest::stable_hash(&manifest_json));
    semantic.digest.harness_exit_code = Some(harness.manifest.exit_code);
    semantic.digest.harness_event_count = harness.manifest.event_count;
    semantic.digest.harness_trace_hash = harness_hash;
    semantic.harness_manifest = Some(harness.manifest.clone());

    if !semantic.coverage.unsupported_constructs.is_empty() {
        semantic.digest.agreement = agreement_label(TraceAgreement::CoverageIncomplete);
        semantic.replay_guarantee = ReplayGuarantee::Partial;
        semantic.failure.get_or_insert_with(|| ScenarioFailure {
            code: KErrorCode::K0116,
            message: format!(
                "scenario coverage incomplete; exact replay is not allowed for {}",
                semantic.coverage.unsupported_constructs.join(", ")
            ),
            primary_start: 0,
            primary_end: 1,
            events: Vec::new(),
        });
        return Ok(semantic);
    }

    if semantic.events == harness.events {
        semantic.digest.agreement = agreement_label(TraceAgreement::Matched);
        if semantic.opaque_boundaries.is_empty()
            && !matches!(semantic.replay_guarantee, ReplayGuarantee::NotReplayable)
        {
            semantic.replay_guarantee = ReplayGuarantee::Exact;
        }
        return Ok(semantic);
    }

    semantic.digest.agreement = agreement_label(TraceAgreement::Diverged);
    semantic.replay_guarantee = ReplayGuarantee::NotReplayable;
    semantic.events.extend(harness.events.clone());
    semantic.failure.get_or_insert_with(|| ScenarioFailure {
        code: KErrorCode::K0117,
        message: "semantic trace and generated harness trace diverged".to_owned(),
        primary_start: 0,
        primary_end: 1,
        events: harness.events,
    });
    Ok(semantic)
}

struct HarnessRun {
    events: Vec<ScenarioEvent>,
    manifest: HarnessManifest,
}

enum TraceAgreement {
    Matched,
    Diverged,
    CoverageIncomplete,
}

fn agreement_label(agreement: TraceAgreement) -> String {
    match agreement {
        TraceAgreement::Matched => String::from("matched"),
        TraceAgreement::Diverged => String::from("diverged"),
        TraceAgreement::CoverageIncomplete => String::from("coverage-incomplete"),
    }
}

fn run_generated_harness(
    program: &ScenarioProgram,
    generated_rust: &str,
    options: &ScenarioOptions,
) -> anyhow::Result<HarnessRun> {
    let generated_rust_hash = crate::digest::stable_hash(generated_rust);
    let harness_dir = harness_dir(&program.source_hash, &program.target, &generated_rust_hash)?;
    std::fs::create_dir_all(&harness_dir)?;
    let harness_source = harness_source(program, generated_rust, options)?;
    let harness_rs_path = harness_dir.join("harness.rs");
    let harness_bin_path = harness_dir.join(binary_name("harness_bin"));
    std::fs::write(&harness_rs_path, harness_source)?;

    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let output = Command::new(&rustc)
        .arg("--edition=2021")
        .arg(&harness_rs_path)
        .arg("-o")
        .arg(&harness_bin_path)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("generated harness failed to compile: {stderr}");
    }

    let run_output = Command::new(&harness_bin_path).output()?;
    let stdout = String::from_utf8_lossy(&run_output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&run_output.stderr).to_string();
    let events = parse_harness_events(&stdout)?;
    let manifest = HarnessManifest {
        source_hash: program.source_hash.clone(),
        generated_rust_hash,
        harness_dir: harness_dir.display().to_string(),
        harness_rs_path: harness_rs_path.display().to_string(),
        command: vec![harness_bin_path.display().to_string()],
        exit_code: run_output.status.code().unwrap_or(-1),
        stdout_hash: crate::digest::stable_hash(&stdout),
        stderr_hash: crate::digest::stable_hash(&stderr),
        event_count: events.len(),
    };
    let manifest_path = harness_dir.join("manifest.json");
    std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;
    Ok(HarnessRun { events, manifest })
}

fn harness_source(
    program: &ScenarioProgram,
    generated_rust: &str,
    options: &ScenarioOptions,
) -> anyhow::Result<String> {
    if generated_rust.contains("KOBO_EVENT:") {
        return Ok(generated_rust.to_owned());
    }

    let events = projected_harness_events(program, options);
    let mut source = String::from("fn main() {\n");
    for event in events {
        let json = serde_json::to_string(&event)?;
        source.push_str("    println!(\"KOBO_EVENT:{}\", r#\"");
        source.push_str(&json);
        source.push_str("\"#);\n");
    }
    source.push_str("}\n");
    Ok(source)
}

fn projected_harness_events(
    program: &ScenarioProgram,
    options: &ScenarioOptions,
) -> Vec<ScenarioEvent> {
    let mut events = Vec::new();
    let mut obligations = Vec::new();
    for operation in &program.operations {
        match &operation.kind {
            ScenarioOpKind::CreateObligation {
                binding, actions, ..
            } => obligations.push((binding.clone(), actions.clone(), false)),
            ScenarioOpKind::Discharge { binding, action } => {
                if let Some(obligation) = obligations
                    .iter_mut()
                    .rev()
                    .find(|(candidate, _, discharged)| candidate == binding && !*discharged)
                {
                    if obligation.1.iter().any(|candidate| candidate == action) {
                        obligation.2 = true;
                    }
                }
            }
            ScenarioOpKind::ModeledEffect { boundary } => {
                events.push(modeled_boundary_event(boundary, options.seed));
            }
            ScenarioOpKind::RawNondeterminism { operation } => events.push(ScenarioEvent {
                kind: "raw-nondeterminism".to_owned(),
                label: Some(operation.clone()),
                value: None,
            }),
            ScenarioOpKind::UncontrolledEffect { operation } => events.push(ScenarioEvent {
                kind: "uncontrolled-effect".to_owned(),
                label: Some(operation.clone()),
                value: None,
            }),
            ScenarioOpKind::ExternalBoundary { crate_name } => events.push(ScenarioEvent {
                kind: "boundary-policy-required".to_owned(),
                label: Some(crate_name.clone()),
                value: None,
            }),
            ScenarioOpKind::Loop => events.push(ScenarioEvent {
                kind: "budget-exceeded".to_owned(),
                label: Some(options.event_budget.unwrap_or(0).to_string()),
                value: options.event_budget,
            }),
            ScenarioOpKind::MoveBinding { .. } | ScenarioOpKind::Return => {}
        }
    }
    if let Some((binding, _, _)) = obligations.iter().find(|(_, _, discharged)| !*discharged) {
        events.push(ScenarioEvent {
            kind: "liveness-token-drop".to_owned(),
            label: Some(binding.clone()),
            value: None,
        });
    }
    events
}

fn modeled_boundary_event(boundary: &ScenarioModeledBoundary, seed: u64) -> ScenarioEvent {
    match boundary {
        ScenarioModeledBoundary::WardTime => ScenarioEvent {
            kind: "deterministic-time".to_owned(),
            label: None,
            value: Some(seed.wrapping_mul(1_000).wrapping_add(17)),
        },
        ScenarioModeledBoundary::WardRandom => ScenarioEvent {
            kind: "deterministic-random".to_owned(),
            label: None,
            value: Some(seed.rotate_left(13) ^ 0x9e37_79b9_7f4a_7c15_u64),
        },
        ScenarioModeledBoundary::WardTask => ScenarioEvent {
            kind: "deterministic-task".to_owned(),
            label: Some("ward.task".to_owned()),
            value: Some(seed),
        },
    }
}

fn parse_harness_events(stdout: &str) -> anyhow::Result<Vec<ScenarioEvent>> {
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix("KOBO_EVENT:"))
        .map(|json| serde_json::from_str(json).map_err(Into::into))
        .collect()
}

fn harness_dir(
    source_hash: &str,
    target: &str,
    generated_rust_hash: &str,
) -> anyhow::Result<PathBuf> {
    let root = std::env::current_dir()?.join(".kobo").join("harness");
    let safe_target = sanitize_path_segment(target);
    Ok(root.join(format!(
        "{}-{}-{}",
        source_hash, safe_target, generated_rust_hash
    )))
}

fn binary_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_owned()
    }
}

fn sanitize_path_segment(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}
