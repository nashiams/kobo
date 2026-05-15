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
    if semantic.failure.as_ref().is_some_and(|failure| {
        matches!(
            failure.code,
            KErrorCode::K0102 | KErrorCode::K0103 | KErrorCode::K0105 | KErrorCode::K0107
        )
    }) {
        return Ok(semantic);
    }

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
    remove_existing_output(&harness_bin_path)?;

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

fn remove_existing_output(path: &PathBuf) -> anyhow::Result<()> {
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    let pdb_path = path.with_extension("pdb");
    if pdb_path.exists() {
        std::fs::remove_file(pdb_path)?;
    }
    Ok(())
}

fn harness_source(
    program: &ScenarioProgram,
    generated_rust: &str,
    options: &ScenarioOptions,
) -> anyhow::Result<String> {
    if has_event_marker(generated_rust) {
        return Ok(generated_rust.to_owned());
    }

    instrument_generated_rust(program, generated_rust, options)
}

fn has_event_marker(source: &str) -> bool {
    source.contains(&event_marker())
}

fn event_marker() -> String {
    "KOBO_EVENT:".to_owned()
}

fn instrument_generated_rust(
    program: &ScenarioProgram,
    generated_rust: &str,
    options: &ScenarioOptions,
) -> anyhow::Result<String> {
    let mut source = String::new();
    source.push_str(&harness_support_source(program));
    source.push_str(generated_rust);
    if !source.ends_with('\n') {
        source.push('\n');
    }

    for operation in &program.operations {
        if let ScenarioOpKind::ModeledEffect { boundary } = &operation.kind {
            let event = modeled_boundary_event(boundary, options.seed);
            source = inject_modeled_boundary_event(source, boundary, &event)?;
        }
    }

    let final_events = terminal_failure_events(program);
    source.push_str(&main_wrapper_source(&program.target, &final_events)?);
    Ok(source)
}

fn harness_support_source(program: &ScenarioProgram) -> String {
    let mut source = String::from(
        r#"
#[allow(non_camel_case_types)]
struct __KoboWardTime;
#[allow(non_camel_case_types)]
struct __KoboWardRandom;
#[allow(non_camel_case_types)]
struct __KoboWard {
    time: __KoboWardTime,
    random: __KoboWardRandom,
}
#[allow(non_upper_case_globals)]
static ward: __KoboWard = __KoboWard {
    time: __KoboWardTime,
    random: __KoboWardRandom,
};
impl __KoboWard {
    fn task(&self) {}
}
impl __KoboWardTime {
    fn now(&self) -> u64 { 0 }
}
impl __KoboWardRandom {
    fn u64(&self) -> u64 { 0 }
    fn next_u64(&self) -> u64 { 0 }
}
"#,
    );

    let mut impls = Vec::new();
    for operation in &program.operations {
        if let ScenarioOpKind::CreateObligation {
            type_name, actions, ..
        } = &operation.kind
        {
            if impls.iter().any(|existing| existing == type_name) {
                continue;
            }
            impls.push(type_name.clone());
            source.push_str("impl ");
            source.push_str(type_name);
            source.push_str(" {\n");
            for action in actions {
                source.push_str("    fn ");
                source.push_str(action);
                source.push_str("(self) {}\n");
            }
            source.push_str("}\n");
        }
    }
    source
}

fn inject_modeled_boundary_event(
    source: String,
    boundary: &ScenarioModeledBoundary,
    event: &ScenarioEvent,
) -> anyhow::Result<String> {
    let print = event_print_statement(event)?;
    let replacements: &[(&str, &str)] = match boundary {
        ScenarioModeledBoundary::WardTask => &[("ward.task();", "ward.task();")],
        ScenarioModeledBoundary::WardTime => &[("ward.time.now()", "ward.time.now()")],
        ScenarioModeledBoundary::WardRandom => &[
            ("ward.random.u64()", "ward.random.u64()"),
            ("ward.random.next_u64()", "ward.random.next_u64()"),
        ],
    };
    for (needle, replacement) in replacements {
        if source.contains(needle) {
            return Ok(source.replacen(needle, &instrumented_expression(replacement, &print), 1));
        }
    }
    anyhow::bail!(
        "generated Rust did not contain modeled boundary {}",
        boundary_label(boundary)
    )
}

fn instrumented_expression(expression: &str, print: &str) -> String {
    if expression.ends_with(';') {
        format!("{expression}\n    {print}")
    } else {
        format!("{{ let __kobo_value = {expression}; {print} __kobo_value }}")
    }
}

fn main_wrapper_source(target: &str, final_events: &[ScenarioEvent]) -> anyhow::Result<String> {
    let mut source = String::from("\nfn main() {\n");
    source.push_str("    ");
    source.push_str(target);
    source.push_str("();\n");
    for event in final_events {
        source.push_str("    ");
        source.push_str(&event_print_statement(event)?);
        source.push('\n');
    }
    source.push_str("}\n");
    Ok(source)
}

fn event_print_statement(event: &ScenarioEvent) -> anyhow::Result<String> {
    let json = serde_json::to_string(event)?;
    Ok(format!("println!(\"KOBO_EVENT:{{}}\", r#\"{json}\"#);"))
}

fn terminal_failure_events(program: &ScenarioProgram) -> Vec<ScenarioEvent> {
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
                let _ = boundary;
            }
            ScenarioOpKind::RawNondeterminism { .. }
            | ScenarioOpKind::UncontrolledEffect { .. }
            | ScenarioOpKind::ExternalBoundary { .. }
            | ScenarioOpKind::Loop => {}
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

fn boundary_label(boundary: &ScenarioModeledBoundary) -> &'static str {
    match boundary {
        ScenarioModeledBoundary::WardTime => "ward.time",
        ScenarioModeledBoundary::WardRandom => "ward.random",
        ScenarioModeledBoundary::WardTask => "ward.task",
    }
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
