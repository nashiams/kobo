use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use kobo_errors::KErrorCode;
use kobo_ir::{
    ScenarioBoundaryCallArgument, ScenarioBoundaryPolicy, ScenarioExternalCallShape,
    ScenarioModeledBoundary, ScenarioOpKind, ScenarioProgram,
};

use crate::core::{
    EngineMode, FullDepthRun, ReplayGuarantee, ScenarioEvent, ScenarioFailure, ScenarioOptions,
};
use crate::error::{Result, SimCoreError};
use crate::harness_manifest::{HarnessManifest, ServiceHookEvent};
use crate::network::NetworkModel;
use crate::storage::StorageModel;

pub fn check_harness_agreement(
    program: &ScenarioProgram,
    generated_rust: &str,
    options: &ScenarioOptions,
    mut semantic: FullDepthRun,
    _mode: EngineMode,
) -> Result<FullDepthRun> {
    if semantic.failure.as_ref().is_some_and(|failure| {
        matches!(
            failure.code,
            KErrorCode::K0102 | KErrorCode::K0103 | KErrorCode::K0105 | KErrorCode::K0107
        )
    }) {
        return Ok(semantic);
    }

    if !semantic.coverage.unsupported_constructs.is_empty() {
        semantic.digest.agreement = agreement_label(TraceAgreement::CoverageIncomplete);
        semantic.replay_guarantee = ReplayGuarantee::Partial;
        semantic.failure = Some(ScenarioFailure {
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

    if !semantic.opaque_boundaries.is_empty() {
        semantic.digest.agreement = agreement_label(TraceAgreement::BoundaryPartial);
        semantic.replay_guarantee = ReplayGuarantee::Partial;
        return Ok(semantic);
    }

    let harness = run_generated_harness(program, generated_rust, options)?;
    let comparable_harness_events = comparable_harness_events(&harness.events);
    if events_match_with_harness_recording(&semantic.events, &comparable_harness_events) {
        merge_harness_recordings(&mut semantic, &comparable_harness_events);
        if comparable_harness_events.len() != harness.events.len() {
            semantic.events = harness.events.clone();
        }
        semantic.digest.semantic_trace_hash = crate::digest::events_hash(&semantic.events);
    }
    let mut harness_hash = crate::digest::events_hash(&harness.events);
    let manifest_json = serde_json::to_string(&harness.manifest)
        .map_err(|source| SimCoreError::json("serialize harness manifest", source))?;
    let backend_execution = crate::backend::execute_profile(
        &semantic.profile,
        options.seed,
        &semantic.digest.semantic_trace_hash,
        &harness_hash,
        &harness.manifest.generated_rust_hash,
        &harness.events,
    );

    semantic.digest.harness_engine = harness.engine.clone();
    if let Some(execution) = backend_execution {
        harness_hash =
            crate::digest::stable_hash(&format!("{}:{}", harness_hash, execution.token_material));
    }
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

fn events_match_with_harness_recording(
    semantic_events: &[ScenarioEvent],
    harness_events: &[ScenarioEvent],
) -> bool {
    semantic_events.len() == harness_events.len()
        && semantic_events
            .iter()
            .zip(harness_events)
            .all(|(semantic, harness)| {
                semantic.kind == harness.kind
                    && semantic.label == harness.label
                    && semantic.value == harness.value
                    && (semantic.io == harness.io
                        || (semantic.kind == "boundary-record"
                            && semantic.io.is_none()
                            && harness.io.is_some()))
            })
}

fn comparable_harness_events(events: &[ScenarioEvent]) -> Vec<ScenarioEvent> {
    events
        .iter()
        .filter(|event| event.kind != "service-scheduler-hook")
        .cloned()
        .collect()
}

fn merge_harness_recordings(run: &mut FullDepthRun, harness_events: &[ScenarioEvent]) {
    for (semantic, harness) in run.events.iter_mut().zip(harness_events) {
        if semantic.kind == "boundary-record" && semantic.io.is_none() {
            semantic.io = harness.io.clone();
        }
    }
    for decision in &mut run.boundary_decisions {
        if decision.policy.as_str() != "record" {
            continue;
        }
        let expected_label = format!(
            "{}@{}..{}",
            decision
                .call_path
                .as_deref()
                .unwrap_or(decision.crate_name.as_str()),
            decision.span_start,
            decision.span_end
        );
        decision.recorded_io = run
            .events
            .iter()
            .find(|event| {
                event.kind == "boundary-record"
                    && event.label.as_deref() == Some(expected_label.as_str())
            })
            .and_then(|event| event.io.clone());
    }
}

struct HarnessRun {
    events: Vec<ScenarioEvent>,
    manifest: HarnessManifest,
    engine: String,
}

struct HarnessObligation {
    binding: String,
    actions: Vec<String>,
    is_discharged: bool,
    declaration_span: (usize, usize),
}

enum TraceAgreement {
    Matched,
    Diverged,
    CoverageIncomplete,
    BoundaryPartial,
}

fn agreement_label(agreement: TraceAgreement) -> String {
    match agreement {
        TraceAgreement::Matched => String::from("matched"),
        TraceAgreement::Diverged => String::from("diverged"),
        TraceAgreement::CoverageIncomplete => String::from("coverage-incomplete"),
        TraceAgreement::BoundaryPartial => String::from("partial-boundary"),
    }
}

fn run_generated_harness(
    program: &ScenarioProgram,
    generated_rust: &str,
    options: &ScenarioOptions,
) -> Result<HarnessRun> {
    let generated_rust_hash = crate::digest::stable_hash(generated_rust);
    let harness_dir = harness_dir(&program.source_hash, &program.target, &generated_rust_hash)?;
    std::fs::create_dir_all(&harness_dir)
        .map_err(|source| SimCoreError::io("create harness directory", source))?;
    let checkpoint_path = loom_checkpoint_path(options, &harness_dir);
    let harness_source =
        harness_source(program, generated_rust, options, checkpoint_path.as_deref())?;
    let uses_loom = options.profile == "sync";
    let harness_rs_path = if uses_loom {
        harness_dir.join("src").join("main.rs")
    } else {
        harness_dir.join("harness.rs")
    };
    if let Some(parent) = harness_rs_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|source| SimCoreError::io("create harness source directory", source))?;
    }
    std::fs::write(&harness_rs_path, harness_source)
        .map_err(|source| SimCoreError::io("write generated harness source", source))?;
    let process = if uses_loom {
        run_loom_cargo_harness(&harness_dir)?
    } else {
        run_rustc_harness(&harness_dir, &harness_rs_path)?
    };
    let checkpoint_artifact_hash = checkpoint_artifact_hash(checkpoint_path.as_deref())?;
    let events = parse_harness_events(&process.stdout)?;
    let service_hook_events = parse_harness_service_hook_events(&process.stdout)?;
    let engine = if uses_loom {
        "generated-rust-loom-process"
    } else {
        "generated-rust-process"
    };
    let manifest = HarnessManifest {
        source_hash: program.source_hash.clone(),
        generated_rust_hash,
        execution_scope: harness_execution_scope(options).to_owned(),
        full_ecosystem_exploration: false,
        facades: harness_facades(program),
        harness_dir: harness_dir.display().to_string(),
        harness_rs_path: harness_rs_path.display().to_string(),
        command: process.command,
        exit_code: process.exit_code,
        stdout_hash: crate::digest::stable_hash(&process.stdout),
        stderr_hash: crate::digest::stable_hash(&process.stderr),
        event_count: events.len(),
        service_hook_events,
        checkpoint_path: checkpoint_path.map(|path| path.display().to_string()),
        checkpoint_artifact_hash,
    };
    let manifest_path = harness_dir.join("manifest.json");
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|source| SimCoreError::json("serialize harness manifest", source))?;
    std::fs::write(&manifest_path, manifest_json)
        .map_err(|source| SimCoreError::io("write harness manifest", source))?;
    Ok(HarnessRun {
        events,
        manifest,
        engine: engine.to_owned(),
    })
}

fn harness_execution_scope(options: &ScenarioOptions) -> &'static str {
    match options.profile.as_str() {
        "sync" => "generated-user-rust-loom",
        "network" | "network-design" => "generated-user-rust-os-facade",
        "async" | "distributed" | "madsim" => "generated-user-rust-adapter",
        _ => "generated-user-rust",
    }
}

fn harness_facades(program: &ScenarioProgram) -> Vec<String> {
    let mut facades = Vec::new();
    for operation in &program.operations {
        match &operation.kind {
            ScenarioOpKind::ModeledEffect { boundary } => match boundary {
                ScenarioModeledBoundary::WardTime => facades.push("time-facade".to_owned()),
                ScenarioModeledBoundary::WardRandom => facades.push("random-facade".to_owned()),
                ScenarioModeledBoundary::WardTask => {
                    facades.push("scheduler-task-facade".to_owned());
                    facades.push("tokio-spawn-facade".to_owned());
                }
                ScenarioModeledBoundary::WardTaskLocal => {
                    facades.push("scheduler-task-local-facade".to_owned());
                    facades.push("tokio-spawn-local-facade".to_owned());
                }
            },
            ScenarioOpKind::StorageEvent { .. } => {
                facades.push("storage-filesystem-facade".to_owned());
            }
            ScenarioOpKind::NetworkEvent { .. } => {
                facades.push("network-loopback-facade".to_owned());
            }
            ScenarioOpKind::ExternalBoundary {
                crate_name, policy, ..
            } if is_replay_owned_boundary(policy) => {
                facades.push(format!(
                    "external-boundary-{}-facade:{crate_name}",
                    policy.as_str()
                ));
            }
            _ => {}
        }
    }
    facades.sort();
    facades.dedup();
    facades
}

struct HarnessProcess {
    command: Vec<String>,
    exit_code: i32,
    stdout: String,
    stderr: String,
}

fn run_rustc_harness(harness_dir: &PathBuf, harness_rs_path: &PathBuf) -> Result<HarnessProcess> {
    let harness_bin_path = harness_dir.join(binary_name("harness_bin"));
    remove_existing_output(&harness_bin_path)?;

    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let output = Command::new(&rustc)
        .arg("--edition=2021")
        .arg(harness_rs_path)
        .arg("-o")
        .arg(&harness_bin_path)
        .output()
        .map_err(|source| SimCoreError::io("compile generated harness", source))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(SimCoreError::HarnessCompileFailed { stderr });
    }

    let run_output = Command::new(&harness_bin_path)
        .output()
        .map_err(|source| SimCoreError::io("run generated harness", source))?;
    if harness_bin_path.exists() {
        std::fs::remove_file(&harness_bin_path)
            .map_err(|source| SimCoreError::io("remove generated harness binary", source))?;
    }
    Ok(HarnessProcess {
        command: vec![harness_bin_path.display().to_string()],
        exit_code: run_output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&run_output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&run_output.stderr).to_string(),
    })
}

fn run_loom_cargo_harness(harness_dir: &PathBuf) -> Result<HarnessProcess> {
    let manifest_path = harness_dir.join("Cargo.toml");
    std::fs::write(&manifest_path, loom_cargo_manifest())
        .map_err(|source| SimCoreError::io("write loom harness manifest", source))?;
    let target_dir = harness_dir.join("target");
    if target_dir.exists() {
        std::fs::remove_dir_all(&target_dir)
            .map_err(|source| SimCoreError::io("remove stale loom harness target", source))?;
    }

    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(&cargo)
        .arg("run")
        .arg("--quiet")
        .arg("--offline")
        .arg("--manifest-path")
        .arg(&manifest_path)
        .env("CARGO_TARGET_DIR", &target_dir)
        .output()
        .map_err(|source| SimCoreError::io("run loom harness cargo", source))?;
    if target_dir.exists() {
        std::fs::remove_dir_all(&target_dir)
            .map_err(|source| SimCoreError::io("remove loom harness target", source))?;
    }
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(SimCoreError::HarnessCompileFailed { stderr });
    }
    Ok(HarnessProcess {
        command: vec![
            cargo,
            "run".to_owned(),
            "--quiet".to_owned(),
            "--offline".to_owned(),
            "--manifest-path".to_owned(),
            manifest_path.display().to_string(),
        ],
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn checkpoint_artifact_hash(checkpoint_path: Option<&Path>) -> Result<Option<String>> {
    let Some(path) = checkpoint_path else {
        return Ok(None);
    };
    let contents = std::fs::read_to_string(path)
        .map_err(|source| SimCoreError::io("read loom checkpoint artifact", source))?;
    Ok(Some(crate::digest::stable_hash(&contents)))
}

fn loom_cargo_manifest() -> &'static str {
    r#"[workspace]

[package]
name = "kobo_generated_loom_harness"
version = "0.0.0"
edition = "2021"

[dependencies]
loom = { version = "0.7", features = ["checkpoint"] }
"#
}

fn remove_existing_output(path: &PathBuf) -> Result<()> {
    if path.exists() {
        std::fs::remove_file(path)
            .map_err(|source| SimCoreError::io("remove stale harness binary", source))?;
    }
    let pdb_path = path.with_extension("pdb");
    if pdb_path.exists() {
        std::fs::remove_file(pdb_path)
            .map_err(|source| SimCoreError::io("remove stale harness debug file", source))?;
    }
    Ok(())
}

fn harness_source(
    program: &ScenarioProgram,
    generated_rust: &str,
    options: &ScenarioOptions,
    loom_checkpoint_path: Option<&Path>,
) -> Result<String> {
    if has_event_marker(generated_rust) {
        return Ok(generated_rust.to_owned());
    }

    instrument_generated_rust(program, generated_rust, options, loom_checkpoint_path)
}

fn has_event_marker(source: &str) -> bool {
    source.contains(&event_marker()) && source.contains("struct __KoboWard")
}

fn event_marker() -> String {
    "KOBO_EVENT:".to_owned()
}

fn instrument_generated_rust(
    program: &ScenarioProgram,
    generated_rust: &str,
    options: &ScenarioOptions,
    loom_checkpoint_path: Option<&Path>,
) -> Result<String> {
    let mut source = String::new();
    source.push_str(&harness_support_source(program, generated_rust, options)?);
    source.push_str(&strip_harness_only_attrs(generated_rust));
    if !source.ends_with('\n') {
        source.push('\n');
    }

    for operation in &program.operations {
        if let ScenarioOpKind::ModeledEffect { boundary } = &operation.kind {
            let events = modeled_boundary_events(boundary, options);
            source = inject_modeled_boundary_event(source, boundary, &events)?;
        }
    }

    let final_events = terminal_failure_events(program, options);
    source.push_str(&main_wrapper_source(
        &program.target,
        &final_events,
        options,
        loom_checkpoint_path,
        target_is_async(generated_rust, &program.target),
    )?);
    Ok(source)
}

fn strip_harness_only_attrs(source: &str) -> String {
    let mut output = String::new();
    let mut skipping_kobo_attr = false;
    for line in source.lines() {
        let trimmed = line.trim_start();
        if skipping_kobo_attr {
            if trimmed.ends_with(']') {
                skipping_kobo_attr = false;
            }
            continue;
        }
        if is_harness_only_kobo_attr(trimmed) {
            skipping_kobo_attr = !trimmed.ends_with(']');
            continue;
        }
        output.push_str(line);
        output.push('\n');
    }
    output
}

fn is_harness_only_kobo_attr(trimmed_line: &str) -> bool {
    trimmed_line.starts_with("#[kobo::boundary")
        || trimmed_line.starts_with("#[kobo::record")
        || trimmed_line.starts_with("#[kobo::activity")
}

fn harness_support_source(
    program: &ScenarioProgram,
    generated_rust: &str,
    options: &ScenarioOptions,
) -> Result<String> {
    let mut source = String::from(
        r#"
#[allow(non_camel_case_types)]
struct __KoboWardTime;
#[allow(non_camel_case_types)]
struct __KoboWardRandom;
#[allow(non_camel_case_types)]
struct __KoboWardStorage;
#[allow(non_camel_case_types)]
struct __KoboWardNetwork;
#[allow(non_camel_case_types)]
struct __KoboWard {
    time: __KoboWardTime,
    random: __KoboWardRandom,
    storage: __KoboWardStorage,
    network: __KoboWardNetwork,
}
#[allow(non_upper_case_globals)]
static ward: __KoboWard = __KoboWard {
    time: __KoboWardTime,
    random: __KoboWardRandom,
    storage: __KoboWardStorage,
    network: __KoboWardNetwork,
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

fn __kobo_block_on<F: std::future::Future>(future: F) -> F::Output {
    fn clone(_: *const ()) -> std::task::RawWaker {
        raw_waker()
    }
    fn wake(_: *const ()) {}
    fn wake_by_ref(_: *const ()) {}
    fn drop(_: *const ()) {}
    fn raw_waker() -> std::task::RawWaker {
        std::task::RawWaker::new(
            std::ptr::null(),
            &std::task::RawWakerVTable::new(clone, wake, wake_by_ref, drop),
        )
    }

    let waker = unsafe { std::task::Waker::from_raw(raw_waker()) };
    let mut context = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(output) => return output,
            std::task::Poll::Pending => std::thread::yield_now(),
        }
    }
}
"#,
    );

    source.push_str(&storage_support_source(program, options)?);
    source.push_str(&network_support_source(program, options)?);
    source.push_str(&external_boundary_support_source(
        program,
        generated_rust,
        options,
    )?);
    source.push_str(&tokio_support_source(program, options)?);

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
            if !generated_rust_defines_type(generated_rust, type_name) {
                source.push_str("struct ");
                source.push_str(type_name);
                source.push_str(";\n");
            }
            source.push_str("impl ");
            source.push_str(type_name);
            source.push_str(" {\n");
            for action in actions {
                source.push_str("    fn ");
                source.push_str(&rust_method_name(action));
                source.push_str("(self) {}\n");
            }
            source.push_str("}\n");
        }
    }
    Ok(source)
}

fn generated_rust_defines_type(source: &str, type_name: &str) -> bool {
    let struct_pattern = format!("struct {type_name}");
    let enum_pattern = format!("enum {type_name}");
    let type_pattern = format!("type {type_name}");
    source.contains(&struct_pattern)
        || source.contains(&enum_pattern)
        || source.contains(&type_pattern)
}

fn rust_method_name(action: &str) -> String {
    match action {
        "await" => "r#await".to_owned(),
        _ => action.replace('-', "_"),
    }
}

fn external_boundary_support_source(
    program: &ScenarioProgram,
    generated_rust: &str,
    options: &ScenarioOptions,
) -> Result<String> {
    let mut crate_facades = BTreeMap::<String, BoundaryFacade>::new();
    for operation in &program.operations {
        let ScenarioOpKind::ExternalBoundary {
            crate_name,
            call_path,
            call_arguments,
            return_type,
            call_shape,
            policy,
            reason,
            ..
        } = &operation.kind
        else {
            continue;
        };
        if !is_replay_owned_boundary(policy) || !is_rust_identifier(crate_name) {
            continue;
        }
        let target = boundary_facade_target(call_path.as_deref(), call_shape);
        let event = ScenarioEvent {
            kind: format!("boundary-{}", policy.as_str()),
            label: Some(boundary_event_label(
                crate_name,
                call_path.as_deref(),
                (operation.span.start as usize, operation.span.end as usize),
            )),
            value: Some(options.seed),
            io: None,
        };
        let record_capture = (*policy == ScenarioBoundaryPolicy::Record).then(|| {
            let span = (operation.span.start as usize, operation.span.end as usize);
            BoundaryFacadeRecordCapture {
                crate_name: crate_name.clone(),
                call_path: call_path.clone(),
                call_shape: call_shape.as_str().to_owned(),
                policy: policy.as_str().to_owned(),
                reason: reason.clone(),
                return_type: return_type.clone(),
                span,
                replay_key: boundary_event_label(crate_name, call_path.as_deref(), span),
                call_arguments: call_arguments.clone(),
            }
        });
        let facade_event = BoundaryFacadeEvent {
            event,
            record_capture,
        };
        match target {
            BoundaryFacadeTarget::Function {
                module_path,
                function_name,
            } if is_rust_identifier(&function_name)
                && module_path.iter().all(|module| is_rust_identifier(module)) =>
            {
                insert_function_event(
                    &mut crate_facades
                        .entry(crate_name.clone())
                        .or_default()
                        .functions,
                    &module_path,
                    function_name,
                    facade_event,
                );
            }
            BoundaryFacadeTarget::Method {
                type_name,
                method_name,
            } if is_rust_identifier(&type_name) && is_rust_identifier(&method_name) => {
                crate_facades
                    .entry(crate_name.clone())
                    .or_default()
                    .methods
                    .entry(type_name)
                    .or_default()
                    .entry(method_name)
                    .or_default()
                    .push(facade_event);
            }
            _ => {}
        }
    }

    let mut source = String::new();
    if crate_facades
        .values()
        .any(BoundaryFacade::has_record_capture)
    {
        source.push_str(record_boundary_runtime_support_source());
    }
    for (crate_name, facade) in crate_facades {
        source.push_str("mod ");
        source.push_str(&crate_name);
        source.push_str(" {\n");
        source.push_str("    pub struct __KoboBoundaryValue;\n");
        source.push_str(&function_tree_source(
            &facade.functions,
            1,
            vec![crate_name.clone()],
        )?);
        for (type_name, methods) in &facade.methods {
            for method_name in methods.keys() {
                source.push_str("    static ");
                source.push_str(&boundary_counter_name(type_name, method_name));
                source.push_str(
                    ": std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);\n",
                );
            }
        }
        for struct_name in facade_struct_names(&facade) {
            source.push_str("    pub struct ");
            source.push_str(&struct_name);
            source.push_str(";\n");
        }
        for (type_name, methods) in &facade.methods {
            source.push_str("    impl ");
            source.push_str(type_name);
            source.push_str(" {\n");
            for (method_name, events) in methods {
                let associated = generated_rust.contains(&format!("{type_name}::{method_name}("));
                if associated {
                    source.push_str("        pub fn ");
                    source.push_str(method_name);
                    source.push_str(&generic_argument_signature(events, true));
                    source.push_str(" -> ");
                    source.push_str(&facade_return_type_name(events, type_name));
                    source.push_str(" {\n");
                } else {
                    source.push_str("        pub fn ");
                    source.push_str(method_name);
                    source.push_str(&generic_argument_signature(events, false));
                    source.push_str(" -> ");
                    source.push_str(&facade_return_type_name(events, type_name));
                    source.push_str(" {\n");
                }
                source.push_str(&boundary_event_sequence_source(
                    &boundary_counter_name(type_name, method_name),
                    &format!("{crate_name}::{type_name}::{method_name}"),
                    events,
                )?);
                source.push_str("            ");
                source.push_str(&facade_return_type_name(events, type_name));
                source.push('\n');
                source.push_str("        }\n");
            }
            source.push_str("    }\n");
        }
        source.push_str("}\n");
    }
    Ok(source)
}

struct BoundaryFacade {
    functions: FunctionTree,
    methods: BTreeMap<String, BTreeMap<String, Vec<BoundaryFacadeEvent>>>,
}

impl Default for BoundaryFacade {
    fn default() -> Self {
        Self {
            functions: FunctionTree::Module(BTreeMap::new()),
            methods: BTreeMap::new(),
        }
    }
}

impl BoundaryFacade {
    fn has_record_capture(&self) -> bool {
        self.functions.has_record_capture()
            || self.methods.values().any(|methods| {
                methods
                    .values()
                    .any(|events| events.iter().any(|event| event.record_capture.is_some()))
            })
    }
}

fn facade_struct_names(facade: &BoundaryFacade) -> Vec<String> {
    let mut names = facade.methods.keys().cloned().collect::<Vec<_>>();
    for methods in facade.methods.values() {
        for events in methods.values() {
            if let Some(return_type) = facade_return_type_path(events) {
                if let Some(name) = boundary_type_leaf(&return_type) {
                    names.push(name);
                }
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

enum FunctionTree {
    Module(BTreeMap<String, FunctionTree>),
    Function(Vec<BoundaryFacadeEvent>),
}

impl FunctionTree {
    fn has_record_capture(&self) -> bool {
        match self {
            Self::Module(children) => children.values().any(Self::has_record_capture),
            Self::Function(events) => events.iter().any(|event| event.record_capture.is_some()),
        }
    }
}

#[derive(Clone)]
struct BoundaryFacadeEvent {
    event: ScenarioEvent,
    record_capture: Option<BoundaryFacadeRecordCapture>,
}

#[derive(Clone)]
struct BoundaryFacadeRecordCapture {
    crate_name: String,
    call_path: Option<String>,
    call_shape: String,
    policy: String,
    reason: Option<String>,
    return_type: Option<String>,
    span: (usize, usize),
    replay_key: String,
    call_arguments: Vec<ScenarioBoundaryCallArgument>,
}

enum BoundaryFacadeTarget {
    Function {
        module_path: Vec<String>,
        function_name: String,
    },
    Method {
        type_name: String,
        method_name: String,
    },
}

fn boundary_facade_target(
    call_path: Option<&str>,
    call_shape: &ScenarioExternalCallShape,
) -> BoundaryFacadeTarget {
    let Some(call_path) = call_path else {
        return BoundaryFacadeTarget::Method {
            type_name: "Client".to_owned(),
            method_name: "new".to_owned(),
        };
    };
    let segments = call_path
        .split("::")
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if matches!(call_shape, ScenarioExternalCallShape::FreeFunction) {
        return BoundaryFacadeTarget::Function {
            module_path: segments
                .iter()
                .skip(1)
                .take(segments.len().saturating_sub(2))
                .map(|segment| (*segment).to_owned())
                .collect(),
            function_name: segments.last().unwrap_or(&"boundary").to_string(),
        };
    }
    if segments.len() < 2 {
        return BoundaryFacadeTarget::Function {
            module_path: Vec::new(),
            function_name: segments.last().unwrap_or(&"boundary").to_string(),
        };
    }
    BoundaryFacadeTarget::Method {
        type_name: segments[segments.len() - 2].to_owned(),
        method_name: segments[segments.len() - 1].to_owned(),
    }
}

fn insert_function_event(
    tree: &mut FunctionTree,
    module_path: &[String],
    function_name: String,
    event: BoundaryFacadeEvent,
) {
    let FunctionTree::Module(children) = tree else {
        return;
    };
    let Some((module_name, remaining_modules)) = module_path.split_first() else {
        let function = children
            .entry(function_name)
            .or_insert_with(|| FunctionTree::Function(Vec::new()));
        if let FunctionTree::Function(events) = function {
            events.push(event);
        }
        return;
    };
    let module = children
        .entry(module_name.clone())
        .or_insert_with(|| FunctionTree::Module(BTreeMap::new()));
    insert_function_event(module, remaining_modules, function_name, event);
}

fn function_tree_source(
    tree: &FunctionTree,
    indent_level: usize,
    module_path: Vec<String>,
) -> Result<String> {
    let mut source = String::new();
    let FunctionTree::Module(children) = tree else {
        return Ok(source);
    };
    for (name, child) in children {
        match child {
            FunctionTree::Module(_) => {
                let indent = "    ".repeat(indent_level);
                source.push_str(&indent);
                source.push_str("pub mod ");
                source.push_str(name);
                source.push_str(" {\n");
                let mut child_path = module_path.clone();
                child_path.push(name.clone());
                source.push_str(&function_tree_source(child, indent_level + 1, child_path)?);
                source.push_str(&indent);
                source.push_str("}\n");
            }
            FunctionTree::Function(events) => {
                let indent = "    ".repeat(indent_level);
                let counter_name = boundary_counter_name("fn", name);
                source.push_str(&indent);
                source.push_str("static ");
                source.push_str(&counter_name);
                source.push_str(
                    ": std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);\n",
                );
                source.push_str(&indent);
                source.push_str("pub fn ");
                source.push_str(name);
                source.push_str(&generic_argument_signature(events, true));
                source.push_str(" -> crate::");
                source.push_str(module_path.first().map(String::as_str).unwrap_or("self"));
                source.push_str("::__KoboBoundaryValue {\n");
                let mut function_path = module_path.clone();
                function_path.push(name.clone());
                source.push_str(&boundary_event_sequence_source(
                    &counter_name,
                    &function_path.join("::"),
                    events,
                )?);
                source.push_str(&indent);
                source.push_str("    crate::");
                source.push_str(module_path.first().map(String::as_str).unwrap_or("self"));
                source.push_str("::__KoboBoundaryValue\n");
                source.push_str(&indent);
                source.push_str("}\n");
            }
        }
    }
    Ok(source)
}

fn generic_argument_signature(events: &[BoundaryFacadeEvent], no_self: bool) -> String {
    let arguments = facade_call_arguments(events);
    let generics = if arguments.is_empty() {
        String::new()
    } else {
        format!(
            "<{}>",
            arguments
                .iter()
                .map(|argument| format!("__KoboArg{}", argument.index))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let mut parameters = Vec::new();
    if !no_self {
        parameters.push("self".to_owned());
    }
    parameters.extend(
        arguments
            .iter()
            .map(|argument| format!("__kobo_arg{}: __KoboArg{}", argument.index, argument.index)),
    );
    format!("{generics}({})", parameters.join(", "))
}

fn facade_call_arguments(events: &[BoundaryFacadeEvent]) -> Vec<ScenarioBoundaryCallArgument> {
    events
        .iter()
        .find_map(|event| event.record_capture.as_ref())
        .map(|capture| capture.call_arguments.clone())
        .unwrap_or_default()
}

fn facade_return_type_name(events: &[BoundaryFacadeEvent], fallback: &str) -> String {
    facade_return_type_path(events)
        .and_then(|path| boundary_type_leaf(&path))
        .unwrap_or_else(|| fallback.to_owned())
}

fn facade_return_type_path(events: &[BoundaryFacadeEvent]) -> Option<String> {
    events
        .iter()
        .find_map(|event| event.record_capture.as_ref())
        .and_then(|capture| capture.return_type.clone())
}

fn boundary_type_leaf(path: &str) -> Option<String> {
    path.split("::")
        .filter(|segment| !segment.is_empty())
        .last()
        .map(str::to_owned)
}

fn boundary_counter_name(type_name: &str, method_name: &str) -> String {
    format!(
        "__KOBO_{}_{}_BOUNDARY_INDEX",
        type_name.to_ascii_uppercase(),
        method_name.to_ascii_uppercase()
    )
}

fn boundary_call_arguments_source(events: &[BoundaryFacadeEvent]) -> String {
    let arguments = facade_call_arguments(events);
    if arguments.is_empty() {
        return "            let __kobo_call_arguments: Vec<(usize, &'static str, String, String)> = Vec::new();\n".to_owned();
    }
    let mut source =
        "            let __kobo_call_arguments: Vec<(usize, &'static str, String, String)> = vec![\n"
            .to_owned();
    for argument in arguments {
        source.push_str("                (");
        source.push_str(&argument.index.to_string());
        source.push_str(", ");
        source.push_str(&format!("{:?}", argument.source));
        source.push_str(", std::any::type_name::<__KoboArg");
        source.push_str(&argument.index.to_string());
        source.push_str(">().to_owned(), std::mem::size_of_val(&__kobo_arg");
        source.push_str(&argument.index.to_string());
        source.push_str(").to_string()),\n");
    }
    source.push_str("            ];\n");
    source
}

fn boundary_facade_event_statement(event: &BoundaryFacadeEvent) -> Result<String> {
    if let Some(capture) = event.record_capture.as_ref() {
        return Ok(record_boundary_event_statement(&event.event, capture));
    }
    event_print_statement(&event.event)
}

fn record_boundary_event_statement(
    event: &ScenarioEvent,
    capture: &BoundaryFacadeRecordCapture,
) -> String {
    let label = event.label.as_deref().unwrap_or("");
    let value = event.value.unwrap_or_default();
    let call_path = capture.call_path.as_deref().unwrap_or(&capture.crate_name);
    let reason = capture.reason.as_deref().unwrap_or("");
    let return_payload = facade_return_payload(capture);
    format!(
        "crate::__kobo_emit_record_boundary_event({:?}, {:?}, {}, {:?}, {:?}, {:?}, {:?}, {:?}, {}, {}, {:?}, {:?}, &__kobo_call_arguments);",
        event.kind,
        label,
        value,
        capture.crate_name,
        call_path,
        capture.call_shape,
        capture.policy,
        reason,
        capture.span.0,
        capture.span.1,
        capture.replay_key,
        return_payload
    )
}

fn facade_return_payload(capture: &BoundaryFacadeRecordCapture) -> String {
    let return_path =
        capture
            .return_type
            .clone()
            .unwrap_or_else(|| match capture.call_shape.as_str() {
                "associated_function" | "method" => capture
                    .call_path
                    .as_deref()
                    .and_then(boundary_receiver_type_path)
                    .unwrap_or_else(|| capture.crate_name.clone()),
                _ => format!("{}::__KoboBoundaryValue", capture.crate_name),
            });
    format!("facade_return:{return_path}")
}

fn boundary_receiver_type_path(call_path: &str) -> Option<String> {
    let mut segments = call_path
        .split("::")
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    (segments.len() >= 2).then(|| {
        let _method = segments.pop();
        segments.join("::")
    })
}

fn boundary_event_sequence_source(
    counter_name: &str,
    label: &str,
    events: &[BoundaryFacadeEvent],
) -> Result<String> {
    let mut source = String::new();
    source.push_str(&boundary_call_arguments_source(events));
    source.push_str("            let __kobo_index = ");
    source.push_str(counter_name);
    source.push_str(".fetch_add(1, std::sync::atomic::Ordering::SeqCst);\n");
    source.push_str("            match __kobo_index {\n");
    for (index, event) in events.iter().enumerate() {
        source.push_str("                ");
        source.push_str(&index.to_string());
        source.push_str(" => { ");
        source.push_str(&boundary_facade_event_statement(event)?);
        source.push_str(" }\n");
    }
    let overflow = BoundaryFacadeEvent {
        event: ScenarioEvent {
            kind: "boundary-overflow".to_owned(),
            label: Some(label.to_owned()),
            value: None,
            io: None,
        },
        record_capture: None,
    };
    source.push_str("                _ => { ");
    source.push_str(&boundary_facade_event_statement(&overflow)?);
    source.push_str(" }\n");
    source.push_str("            }\n");
    Ok(source)
}

fn boundary_event_label(crate_name: &str, call_path: Option<&str>, span: (usize, usize)) -> String {
    format!("{}@{}..{}", call_path.unwrap_or(crate_name), span.0, span.1)
}

fn record_boundary_runtime_support_source() -> &'static str {
    r#"
fn __kobo_json_string(value: &str) -> String {
    let mut escaped = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => escaped.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

fn __kobo_stable_hash(source: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in source.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn __kobo_field_json(key: &str, value: &str) -> String {
    format!(
        "{{\"key\":{},\"value\":{}}}",
        __kobo_json_string(key),
        __kobo_json_string(value)
    )
}

fn __kobo_payload_json(kind: &str, fields: &[(String, String)]) -> String {
    let fields = fields
        .iter()
        .map(|(key, value)| __kobo_field_json(key, value))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"kind\":{},\"fields\":[{}]}}",
        __kobo_json_string(kind),
        fields
    )
}

fn __kobo_payload_material(kind: &str, fields: &[(String, String)]) -> String {
    let mut material = String::from(kind);
    for (key, value) in fields {
        material.push('|');
        material.push_str(key);
        material.push('=');
        material.push_str(value);
    }
    material
}

fn __kobo_boundary_protocol(crate_name: &str, call_path: &str) -> &'static str {
    if crate_name.contains("sql")
        || call_path.contains("query")
        || call_path.contains("transaction")
        || call_path.contains("pool")
    {
        "database-like"
    } else {
        "http-like"
    }
}

fn __kobo_emit_record_boundary_event(
    kind: &str,
    label: &str,
    value: u64,
    crate_name: &str,
    call_path: &str,
    call_shape: &str,
    policy: &str,
    reason: &str,
    span_start: usize,
    span_end: usize,
    replay_key: &str,
    return_payload: &str,
    call_arguments: &[(usize, &'static str, String, String)],
) {
    let mut argument_fragments = Vec::new();
    let mut request_fields = vec![
        ("capture_source".to_owned(), "generated-boundary-facade-runtime".to_owned()),
        ("protocol".to_owned(), __kobo_boundary_protocol(crate_name, call_path).to_owned()),
        ("crate".to_owned(), crate_name.to_owned()),
        ("call_path".to_owned(), call_path.to_owned()),
        ("call_shape".to_owned(), call_shape.to_owned()),
        ("policy".to_owned(), policy.to_owned()),
        ("reason".to_owned(), reason.to_owned()),
        ("argument_count".to_owned(), call_arguments.len().to_string()),
        ("source_span_start".to_owned(), span_start.to_string()),
        ("source_span_end".to_owned(), span_end.to_string()),
    ];
    for (index, source, type_name, size) in call_arguments {
        request_fields.push((format!("argument_{index}_source"), (*source).to_owned()));
        request_fields.push((format!("argument_{index}_type"), type_name.clone()));
        request_fields.push((format!("argument_{index}_size"), size.clone()));
        argument_fragments.push(format!(
            "{{\"index\":{},\"source\":{},\"type\":{},\"size\":{}}}",
            index,
            __kobo_json_string(source),
            __kobo_json_string(type_name),
            size
        ));
    }
    let request_body = format!(
        "{{\"boundary\":{},\"operation\":{},\"arguments\":[{}]}}",
        __kobo_json_string(crate_name),
        __kobo_json_string(call_path),
        argument_fragments.join(",")
    );
    request_fields.push(("request_body".to_owned(), request_body));
    let request_hash = __kobo_stable_hash(&__kobo_payload_material(
        "kobo-boundary-request",
        &request_fields,
    ));

    let replay_result =
        __kobo_stable_hash(&format!("recorded-response:{replay_key}:{request_hash}:{return_payload}"));
    let response_body = format!(
        "{{\"recorded\":true,\"return\":{},\"replay_result\":{}}}",
        __kobo_json_string(return_payload),
        __kobo_json_string(&replay_result)
    );
    let response_fields = vec![
        ("capture_source".to_owned(), "generated-boundary-facade-runtime".to_owned()),
        ("status".to_owned(), "recorded".to_owned()),
        ("status_code".to_owned(), "200".to_owned()),
        ("replay_key".to_owned(), replay_key.to_owned()),
        ("replay_result".to_owned(), replay_result),
        ("return_payload".to_owned(), return_payload.to_owned()),
        ("response_body".to_owned(), response_body),
        ("external_internals_replayed".to_owned(), "false".to_owned()),
    ];
    let response_hash = __kobo_stable_hash(&__kobo_payload_material(
        "kobo-boundary-response",
        &response_fields,
    ));

    let request_json = __kobo_payload_json("kobo-boundary-request", &request_fields);
    let response_json = __kobo_payload_json("kobo-boundary-response", &response_fields);
    let io_json = format!(
        "{{\"mode\":\"recorded-boundary-io\",\"replay_key\":{},\"request\":{},\"response\":{},\"request_hash\":{},\"response_hash\":{}}}",
        __kobo_json_string(replay_key),
        request_json,
        response_json,
        __kobo_json_string(&request_hash),
        __kobo_json_string(&response_hash)
    );
    println!(
        "KOBO_EVENT:{{\"kind\":{},\"label\":{},\"value\":{},\"io\":{}}}",
        __kobo_json_string(kind),
        __kobo_json_string(label),
        value,
        io_json
    );
}

"#
}

fn tokio_support_source(_program: &ScenarioProgram, _options: &ScenarioOptions) -> Result<String> {
    let mut source = String::from(
        r#"
mod tokio {
    pub mod sync {
        pub mod mpsc {
            use std::sync::{mpsc as std_mpsc, Arc, Mutex};

            pub struct Sender<T> {
                inner: std_mpsc::SyncSender<T>,
            }

            pub struct Receiver<T> {
                inner: Arc<Mutex<std_mpsc::Receiver<T>>>,
            }

            pub mod error {
                pub struct SendError<T>(pub T);
                pub enum TrySendError<T> { Full(T), Closed(T) }
            }

            pub fn channel<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
                let (tx, rx) = std_mpsc::sync_channel(capacity);
                (
                    Sender { inner: tx },
                    Receiver { inner: Arc::new(Mutex::new(rx)) },
                )
            }

            impl<T> Clone for Sender<T> {
                fn clone(&self) -> Self {
                    Self { inner: self.inner.clone() }
                }
            }

            impl<T> Sender<T> {
                pub async fn send(&self, value: T) -> Result<(), error::SendError<T>> {
                    self.inner.send(value).map_err(|error| error::SendError(error.0))
                }

                pub fn try_send(&self, value: T) -> Result<(), error::TrySendError<T>> {
                    self.inner.try_send(value).map_err(|error| match error {
                        std_mpsc::TrySendError::Full(value) => error::TrySendError::Full(value),
                        std_mpsc::TrySendError::Disconnected(value) => error::TrySendError::Closed(value),
                    })
                }
            }

            impl<T> Receiver<T> {
                pub async fn recv(&mut self) -> Option<T> {
                    self.inner.lock().ok()?.recv().ok()
                }
            }
        }

        pub mod oneshot {
            use std::sync::mpsc as std_mpsc;

            pub struct Sender<T> {
                inner: Option<std_mpsc::Sender<T>>,
            }

            pub struct Receiver<T> {
                inner: std_mpsc::Receiver<T>,
            }

            pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
                let (tx, rx) = std_mpsc::channel();
                (Sender { inner: Some(tx) }, Receiver { inner: rx })
            }

            impl<T> Sender<T> {
                pub fn send(mut self, value: T) -> Result<(), T> {
                    match self.inner.take() {
                        Some(sender) => sender.send(value).map_err(|error| error.0),
                        None => Err(value),
                    }
                }
            }

            impl<T> Unpin for Receiver<T> {}

            impl<T> std::future::Future for Receiver<T> {
                type Output = Result<T, ()>;

                fn poll(
                    self: std::pin::Pin<&mut Self>,
                    cx: &mut std::task::Context<'_>,
                ) -> std::task::Poll<Self::Output> {
                    match self.get_mut().inner.try_recv() {
                        Ok(value) => std::task::Poll::Ready(Ok(value)),
                        Err(std_mpsc::TryRecvError::Disconnected) => std::task::Poll::Ready(Err(())),
                        Err(std_mpsc::TryRecvError::Empty) => {
                            cx.waker().wake_by_ref();
                            std::task::Poll::Pending
                        }
                    }
                }
            }
        }
    }

    #[derive(Debug)]
    pub struct JoinError;

    enum JoinState<T> {
        Thread(std::thread::JoinHandle<T>),
        Value(T),
    }

    pub struct JoinHandle<T = ()> {
        state: Option<JoinState<T>>,
    }

    impl<T> JoinHandle<T> {
        fn from_thread(handle: std::thread::JoinHandle<T>) -> Self {
            Self { state: Some(JoinState::Thread(handle)) }
        }

        fn from_value(value: T) -> Self {
            Self { state: Some(JoinState::Value(value)) }
        }

        pub fn abort(self) {}
        pub fn detach_with_policy(self) {}
    }

    impl<T> Unpin for JoinHandle<T> {}

    impl<T> std::future::Future for JoinHandle<T> {
        type Output = Result<T, JoinError>;

        fn poll(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            let this = self.get_mut();
            if matches!(this.state.as_ref(), Some(JoinState::Thread(handle)) if !handle.is_finished()) {
                cx.waker().wake_by_ref();
                return std::task::Poll::Pending;
            }
            match this.state.take().expect("join handle polled after completion") {
                JoinState::Thread(handle) => std::task::Poll::Ready(handle.join().map_err(|_| JoinError)),
                JoinState::Value(value) => std::task::Poll::Ready(Ok(value)),
            }
        }
    }

    pub mod runtime {
        pub struct Handle;
        pub struct Runtime;
        #[derive(Debug)]
        pub struct BuildError;
        pub struct Builder;

        impl Builder {
            pub fn new_current_thread() -> Self { Self }
            pub fn enable_all(self) -> Self { self }
            pub fn build(self) -> Result<Runtime, BuildError> { Ok(Runtime) }
        }

        impl Runtime {
            pub fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
                crate::__kobo_block_on(future)
            }
        }

        impl Handle {
            pub fn current() -> Self { Self }
            pub fn try_current() -> Result<Self, ()> { Ok(Self) }

            pub fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
                crate::__kobo_block_on(future)
            }

            pub fn spawn<F>(&self, future: F) -> super::JoinHandle<F::Output>
            where
                F: std::future::Future + Send + 'static,
                F::Output: Send + 'static,
            {
                super::spawn(future)
            }
        }
    }

    pub mod task {
        pub type JoinHandle<T = ()> = super::JoinHandle<T>;

        pub struct LocalSet;

        impl LocalSet {
            pub fn new() -> Self { Self }

            pub async fn run_until<F: std::future::Future>(&self, future: F) -> F::Output {
                future.await
            }
        }

        pub fn spawn_local<F>(future: F) -> JoinHandle<F::Output>
        where
            F: std::future::Future + 'static,
            F::Output: 'static,
        {
"#,
    );
    source.push_str(
        r#"
            let output = crate::__kobo_block_on(future);
            super::JoinHandle::from_value(output)
        }
    }

    pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
"#,
    );
    source.push_str(
        r#"
        JoinHandle::from_thread(std::thread::spawn(move || crate::__kobo_block_on(future)))
    }
}
"#,
    );
    Ok(source)
}

fn storage_support_source(program: &ScenarioProgram, options: &ScenarioOptions) -> Result<String> {
    let mut methods = Vec::new();
    for operation in &program.operations {
        if let ScenarioOpKind::StorageEvent { action } = &operation.kind {
            if methods.iter().any(|existing| existing == action) {
                continue;
            }
            methods.push(action.clone());
        }
    }
    if methods.is_empty() {
        methods.push("write".to_owned());
        methods.push("crash_after_write".to_owned());
    }
    let mut source = String::from(
        r#"
fn __kobo_storage_root() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("kobo-generated-storage-{}", std::process::id()))
}

fn __kobo_storage_journal_path() -> std::path::PathBuf {
    let root = __kobo_storage_root();
    let _ = std::fs::create_dir_all(&root);
    root.join("journal.log")
}

fn __kobo_storage_write_record() {
    let path = __kobo_storage_journal_path();
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = std::io::Write::write_all(&mut file, b"kobo-storage-record\n");
    }
}

fn __kobo_storage_commit_record() {
    let path = __kobo_storage_journal_path();
    if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.sync_all();
    }
}

fn __kobo_storage_recover_record() {
    let path = __kobo_storage_journal_path();
    let _ = std::fs::read(path);
}

"#,
    );
    source.push_str("impl __KoboWardStorage {\n");
    for method in methods {
        source.push_str("    fn ");
        source.push_str(&method);
        if crate::storage::method_takes_value(&method) {
            source.push_str("<T>(&self, _value: T) {\n        ");
        } else {
            source.push_str("(&self) {\n        ");
        }
        source.push_str(&storage_runtime_statement(&method));
        source.push_str("\n        ");
        source.push_str(&event_print_statements(
            &crate::storage::events_for_action(&method, options.seed),
        )?);
        source.push_str("\n    }\n");
    }
    source.push_str("}\n");
    Ok(source)
}

fn storage_runtime_statement(method: &str) -> &'static str {
    match normalized_storage_method(method).as_str() {
        "write" | "append" | "journal" => "__kobo_storage_write_record();",
        "commit" | "flush" | "fsync" => "__kobo_storage_commit_record();",
        "recover" | "replay" => "__kobo_storage_recover_record();",
        _ => "",
    }
}

fn normalized_storage_method(method: &str) -> String {
    method
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect::<String>()
        .to_ascii_lowercase()
}

fn network_support_source(program: &ScenarioProgram, options: &ScenarioOptions) -> Result<String> {
    let mut methods = Vec::new();
    for operation in &program.operations {
        if let ScenarioOpKind::NetworkEvent { action } = &operation.kind {
            if methods.iter().any(|existing| existing == action) {
                continue;
            }
            methods.push(action.clone());
        }
    }
    for default_method in ["send", "delay", "reorder", "receive", "drop_message"] {
        if !methods.iter().any(|existing| existing == default_method) {
            methods.push(default_method.to_owned());
        }
    }
    let mut source = String::from(
        r#"
fn __kobo_network_loopback() {
    let Ok(socket) = std::net::UdpSocket::bind("127.0.0.1:0") else {
        return;
    };
    let Ok(address) = socket.local_addr() else {
        return;
    };
    let _ = socket.set_nonblocking(true);
    let _ = socket.send_to(b"kobo-network-frame", address);
    let mut buffer = [0_u8; 64];
    let _ = socket.recv_from(&mut buffer);
}

"#,
    );
    source.push_str("impl __KoboWardNetwork {\n");
    for method in methods {
        source.push_str("    fn ");
        source.push_str(&method);
        source.push_str("<T>(&self, _value: T) {\n        ");
        source.push_str(network_runtime_statement(&method));
        source.push_str("\n        ");
        source.push_str(&event_print_statements(
            &crate::network::harness_events_for_action(&method, options.seed),
        )?);
        source.push_str("\n    }\n");
    }
    source.push_str("}\n");
    Ok(source)
}

fn network_runtime_statement(method: &str) -> &'static str {
    match normalized_network_method(method).as_str() {
        "send" | "receive" => "__kobo_network_loopback();",
        _ => "",
    }
}

fn normalized_network_method(method: &str) -> String {
    method
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect::<String>()
        .to_ascii_lowercase()
}

fn inject_modeled_boundary_event(
    source: String,
    boundary: &ScenarioModeledBoundary,
    events: &[ScenarioEvent],
) -> Result<String> {
    let print = event_print_statements(events)?;
    let replacements: &[(&str, &str)] = match boundary {
        ScenarioModeledBoundary::WardTask | ScenarioModeledBoundary::WardTaskLocal => {
            &[("ward.task();", "ward.task();")]
        }
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
    if boundary == &ScenarioModeledBoundary::WardTask {
        for needle in ["tokio::spawn(async {})", "tokio :: spawn(async {})"] {
            if source.contains(needle) {
                return Ok(source.replacen(
                    needle,
                    &format!("{{\n        {print}\n        {needle}\n    }}"),
                    1,
                ));
            }
        }
        for needle in [
            "tokio::spawn(async move {",
            "tokio :: spawn(async move {",
            "tokio::spawn(async {",
            "tokio :: spawn(async {",
        ] {
            if source.contains(needle) {
                return Ok(source.replacen(needle, &format!("{print}\n    {needle}"), 1));
            }
        }
    }
    Err(SimCoreError::ModeledBoundaryMissing {
        boundary: boundary_label(boundary),
    })
}

fn instrumented_expression(expression: &str, print: &str) -> String {
    if expression.ends_with(';') {
        format!("{expression}\n    {print}")
    } else {
        format!("{{ let __kobo_value = {expression}; {print} __kobo_value }}")
    }
}

fn main_wrapper_source(
    target: &str,
    final_events: &[ScenarioEvent],
    options: &ScenarioOptions,
    loom_checkpoint_path: Option<&Path>,
    target_is_async: bool,
) -> Result<String> {
    let mut source = if options.profile == "sync" {
        let mut source = String::from("\nfn main() {\n");
        if options.scheduler == crate::SchedulerPolicy::Exhaustive
            || options.loom_max_branches.is_some()
            || loom_checkpoint_path.is_some()
        {
            source.push_str("    let mut __kobo_loom = loom::model::Builder::new();\n");
            if let Some(max_branches) = options.loom_max_branches {
                source.push_str(&format!(
                    "    __kobo_loom.max_branches = {max_branches}_usize;\n"
                ));
            }
            if let Some(checkpoint_path) = loom_checkpoint_path {
                source.push_str("    __kobo_loom.checkpoint_file(");
                source.push_str(&format!("{:?}", checkpoint_path.display().to_string()));
                source.push_str(");\n");
                source.push_str("    __kobo_loom.checkpoint_interval = 1;\n");
            }
            source.push_str("    __kobo_loom.check(|| {\n");
        } else {
            source.push_str("    loom::model(|| {\n");
        }
        source
    } else {
        String::from("\nfn main() {\n")
    };
    source.push_str("    ");
    if options.profile == "sync" {
        source.push_str("    ");
    }
    if target_is_async {
        source.push_str("__kobo_block_on(");
        source.push_str(target);
        source.push_str("());\n");
    } else {
        source.push_str(target);
        source.push_str("();\n");
    }
    for event in final_events {
        source.push_str("    ");
        if options.profile == "sync" {
            source.push_str("    ");
        }
        source.push_str(&event_print_statement(event)?);
        source.push('\n');
    }
    if options.profile == "sync" {
        source.push_str("    });\n");
    }
    source.push_str("}\n");
    Ok(source)
}

fn loom_checkpoint_path(options: &ScenarioOptions, harness_dir: &Path) -> Option<PathBuf> {
    (options.profile == "sync" && options.loom_checkpoint_replay)
        .then(|| harness_dir.join("loom-checkpoint.json"))
}

fn target_is_async(source: &str, target: &str) -> bool {
    source.contains(&format!("async fn {target}"))
}

fn event_print_statement(event: &ScenarioEvent) -> Result<String> {
    let json = serde_json::to_string(event)
        .map_err(|source| SimCoreError::json("serialize harness event", source))?;
    Ok(format!("println!(\"KOBO_EVENT:{{}}\", r#\"{json}\"#);"))
}

fn event_print_statements(events: &[ScenarioEvent]) -> Result<String> {
    let statements = events
        .iter()
        .map(event_print_statement)
        .collect::<Result<Vec<_>>>()?;
    Ok(statements.join("\n        "))
}

fn terminal_failure_events(
    program: &ScenarioProgram,
    options: &ScenarioOptions,
) -> Vec<ScenarioEvent> {
    let mut events = Vec::new();
    let mut obligations = Vec::new();
    let mut storage = StorageModel::default();
    let mut network = NetworkModel::default();
    let mut failure_events = None;
    for operation in &program.operations {
        let span = (
            operation.span.start as usize,
            operation.span.end.max(operation.span.start + 1) as usize,
        );
        match &operation.kind {
            ScenarioOpKind::CreateObligation {
                binding, actions, ..
            } => obligations.push(HarnessObligation {
                binding: binding.clone(),
                actions: actions.clone(),
                is_discharged: false,
                declaration_span: span,
            }),
            ScenarioOpKind::Discharge { binding, action } => {
                storage.note_obligation_action(action);
                if let Some(obligation) = obligations
                    .iter_mut()
                    .rev()
                    .find(|obligation| obligation.binding == *binding && !obligation.is_discharged)
                {
                    if obligation
                        .actions
                        .iter()
                        .any(|candidate| candidate == action)
                    {
                        obligation.is_discharged = true;
                    }
                }
            }
            ScenarioOpKind::Transfer {
                binding, callee, ..
            } => events.push(ScenarioEvent {
                kind: "obligation-transfer".to_owned(),
                label: Some(format!("{binding}->{callee}")),
                value: None,
                io: None,
            }),
            ScenarioOpKind::UnsupportedContainer {
                binding,
                type_name,
                container,
            } => {
                if failure_events.is_none() {
                    failure_events = Some(vec![ScenarioEvent {
                        kind: "unsupported-container".to_owned(),
                        label: Some(format!("{binding}:{container}:{type_name}")),
                        value: None,
                        io: None,
                    }]);
                }
            }
            ScenarioOpKind::BranchUnresolved { binding } => {
                if failure_events.is_none() {
                    failure_events = Some(vec![ScenarioEvent {
                        kind: "branch-unresolved".to_owned(),
                        label: Some(binding.clone()),
                        value: None,
                        io: None,
                    }]);
                }
            }
            ScenarioOpKind::ModeledEffect { boundary } => {
                if failure_events.is_none() {
                    let active_obligation = obligations
                        .iter()
                        .rev()
                        .find(|obligation| !obligation.is_discharged)
                        .map(|obligation| {
                            (obligation.binding.as_str(), obligation.declaration_span)
                        });
                    if let Some(failure) = crate::scheduler::schedule_failure(
                        &core_boundary(boundary),
                        options,
                        active_obligation,
                        span,
                    ) {
                        failure_events = Some(failure.events);
                    }
                }
            }
            ScenarioOpKind::StorageEvent { action } => {
                if failure_events.is_none() {
                    if let Some(failure) = storage.apply_action(action, options.seed, span).failure
                    {
                        failure_events = Some(failure.events);
                    }
                }
            }
            ScenarioOpKind::NetworkEvent { action } => {
                if failure_events.is_none() {
                    if let Some(failure) = network.apply_action(action, options.seed, span).failure
                    {
                        failure_events = Some(failure.events);
                    }
                }
            }
            ScenarioOpKind::Select { .. } => {}
            ScenarioOpKind::RawNondeterminism { .. }
            | ScenarioOpKind::UncontrolledEffect { .. }
            | ScenarioOpKind::ExternalBoundary { .. }
            | ScenarioOpKind::CoreTerminator { .. }
            | ScenarioOpKind::LoopStart
            | ScenarioOpKind::LoopBackEdge { .. }
            | ScenarioOpKind::LoopContinue
            | ScenarioOpKind::LoopBreak
            | ScenarioOpKind::Loop => {}
            ScenarioOpKind::MoveBinding { .. } | ScenarioOpKind::Return => {}
        }
    }
    if failure_events.is_none() && has_cancel_injection(options) {
        if let Some(obligation) = obligations
            .iter()
            .find(|obligation| !obligation.is_discharged)
        {
            failure_events = Some(vec![ScenarioEvent {
                kind: "failure-injection-cancel".to_owned(),
                label: Some(obligation.binding.clone()),
                value: None,
                io: None,
            }]);
        }
        if failure_events.is_none() {
            if let Some(boundary) = first_modeled_boundary(program) {
                failure_events = Some(vec![ScenarioEvent {
                    kind: "failure-injection-cancel".to_owned(),
                    label: Some(boundary.to_owned()),
                    value: None,
                    io: None,
                }]);
            }
        }
    }

    if failure_events.is_none() {
        failure_events = obligations
            .iter()
            .find(|obligation| !obligation.is_discharged)
            .map(|obligation| {
                vec![ScenarioEvent {
                    kind: "liveness-token-drop".to_owned(),
                    label: Some(obligation.binding.clone()),
                    value: None,
                    io: None,
                }]
            });
    }
    if let Some(failure) = failure_events {
        events.extend(failure);
    }
    events.extend(crate::core::scheduler_events(options));
    events
}

fn has_cancel_injection(options: &ScenarioOptions) -> bool {
    options
        .inject
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .any(|hook| hook == "cancel")
}

fn is_replay_owned_boundary(policy: &ScenarioBoundaryPolicy) -> bool {
    matches!(
        policy,
        ScenarioBoundaryPolicy::Model
            | ScenarioBoundaryPolicy::Record
            | ScenarioBoundaryPolicy::Stub
    )
}

fn is_rust_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn first_modeled_boundary(program: &ScenarioProgram) -> Option<&'static str> {
    program.operations.iter().find_map(|operation| {
        if let ScenarioOpKind::ModeledEffect { boundary } = &operation.kind {
            Some(boundary_label(boundary))
        } else {
            None
        }
    })
}

fn boundary_label(boundary: &ScenarioModeledBoundary) -> &'static str {
    match boundary {
        ScenarioModeledBoundary::WardTime => "ward.time",
        ScenarioModeledBoundary::WardRandom => "ward.random",
        ScenarioModeledBoundary::WardTask => "ward.task",
        ScenarioModeledBoundary::WardTaskLocal => "ward.task.local",
    }
}

fn modeled_boundary_events(
    boundary: &ScenarioModeledBoundary,
    options: &ScenarioOptions,
) -> Vec<ScenarioEvent> {
    crate::scheduler::modeled_boundary_events(&core_boundary(boundary), options)
}

fn core_boundary(boundary: &ScenarioModeledBoundary) -> crate::core::ModeledBoundary {
    match boundary {
        ScenarioModeledBoundary::WardTime => crate::core::ModeledBoundary::WardTime,
        ScenarioModeledBoundary::WardRandom => crate::core::ModeledBoundary::WardRandom,
        ScenarioModeledBoundary::WardTask => crate::core::ModeledBoundary::WardTask,
        ScenarioModeledBoundary::WardTaskLocal => crate::core::ModeledBoundary::WardTaskLocal,
    }
}

fn parse_harness_events(stdout: &str) -> Result<Vec<ScenarioEvent>> {
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix("KOBO_EVENT:"))
        .map(|json| {
            serde_json::from_str(json)
                .map_err(|source| SimCoreError::json("parse generated harness event", source))
        })
        .collect()
}

fn parse_harness_service_hook_events(stdout: &str) -> Result<Vec<ServiceHookEvent>> {
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix("KOBO_SERVICE_HOOK:"))
        .map(|json| {
            serde_json::from_str(json)
                .map_err(|source| SimCoreError::json("parse generated service hook event", source))
        })
        .collect()
}

fn harness_dir(source_hash: &str, target: &str, generated_rust_hash: &str) -> Result<PathBuf> {
    let root = std::env::current_dir()
        .map_err(|source| SimCoreError::io("resolve current directory", source))?
        .join(".kobo")
        .join("harness");
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
