use std::path::PathBuf;
use std::process::Command;

use kobo_errors::KErrorCode;
use kobo_ir::{ScenarioBoundaryPolicy, ScenarioModeledBoundary, ScenarioOpKind, ScenarioProgram};

use crate::core::{
    EngineMode, FullDepthRun, ReplayGuarantee, ScenarioEvent, ScenarioFailure, ScenarioOptions,
};
use crate::error::{Result, SimCoreError};
use crate::harness_manifest::HarnessManifest;
use crate::network::NetworkModel;
use crate::storage::StorageModel;

pub fn check_harness_agreement(
    program: &ScenarioProgram,
    generated_rust: &str,
    options: &ScenarioOptions,
    mut semantic: FullDepthRun,
    _mode: EngineMode,
) -> Result<FullDepthRun> {
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
    if semantic.failure.as_ref().is_some_and(|failure| {
        matches!(
            failure.code,
            KErrorCode::K0102 | KErrorCode::K0103 | KErrorCode::K0105 | KErrorCode::K0107
        )
    }) {
        return Ok(semantic);
    }

    let harness = run_generated_harness(program, generated_rust, options)?;
    let mut harness_hash = crate::digest::events_hash(&harness.events);
    let manifest_json = serde_json::to_string(&harness.manifest)
        .map_err(|source| SimCoreError::json("serialize harness manifest", source))?;
    let lowered_for_backend = crate::lower::lower_from_program(program, &semantic.profile);
    let backend_execution = crate::backend::execute_profile(
        &semantic.profile,
        options.seed,
        &semantic.digest.semantic_trace_hash,
        &lowered_for_backend.operations,
    );

    semantic.digest.harness_engine = backend_execution
        .as_ref()
        .map(|execution| format!("generated-rust-process+{}", execution.engine))
        .unwrap_or_else(|| "generated-rust-process".to_owned());
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

struct HarnessRun {
    events: Vec<ScenarioEvent>,
    manifest: HarnessManifest,
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
) -> Result<HarnessRun> {
    let generated_rust_hash = crate::digest::stable_hash(generated_rust);
    let harness_dir = harness_dir(&program.source_hash, &program.target, &generated_rust_hash)?;
    std::fs::create_dir_all(&harness_dir)
        .map_err(|source| SimCoreError::io("create harness directory", source))?;
    let harness_source = harness_source(program, generated_rust, options)?;
    let harness_rs_path = harness_dir.join("harness.rs");
    let harness_bin_path = harness_dir.join(binary_name("harness_bin"));
    std::fs::write(&harness_rs_path, harness_source)
        .map_err(|source| SimCoreError::io("write generated harness source", source))?;
    remove_existing_output(&harness_bin_path)?;

    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let output = Command::new(&rustc)
        .arg("--edition=2021")
        .arg(&harness_rs_path)
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
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|source| SimCoreError::json("serialize harness manifest", source))?;
    std::fs::write(&manifest_path, manifest_json)
        .map_err(|source| SimCoreError::io("write harness manifest", source))?;
    Ok(HarnessRun { events, manifest })
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
) -> Result<String> {
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
) -> Result<String> {
    let mut source = String::new();
    source.push_str(&harness_support_source(program, options)?);
    source.push_str(&strip_harness_only_attrs(generated_rust));
    if !source.ends_with('\n') {
        source.push('\n');
    }

    for operation in &program.operations {
        if let ScenarioOpKind::ModeledEffect { boundary } = &operation.kind {
            if boundary == &ScenarioModeledBoundary::WardTask
                && source.contains("tokio::spawn")
                && !source.contains("ward.task();")
            {
                continue;
            }
            let events = modeled_boundary_events(boundary, options);
            source = inject_modeled_boundary_event(source, boundary, &events)?;
        }
    }

    let final_events = terminal_failure_events(program, options);
    source.push_str(&main_wrapper_source(&program.target, &final_events)?);
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
        if trimmed.starts_with("#[kobo::boundary") {
            skipping_kobo_attr = !trimmed.ends_with(']');
            continue;
        }
        output.push_str(line);
        output.push('\n');
    }
    output
}

fn harness_support_source(program: &ScenarioProgram, options: &ScenarioOptions) -> Result<String> {
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
"#,
    );

    source.push_str(&storage_support_source(program, options)?);
    source.push_str(&network_support_source(program, options)?);
    source.push_str(&external_boundary_support_source(program, options)?);
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
    Ok(source)
}

fn external_boundary_support_source(
    program: &ScenarioProgram,
    options: &ScenarioOptions,
) -> Result<String> {
    let mut source = String::new();
    let mut crate_names = Vec::new();
    for operation in &program.operations {
        let ScenarioOpKind::ExternalBoundary {
            crate_name, policy, ..
        } = &operation.kind
        else {
            continue;
        };
        if !is_replay_owned_boundary(policy)
            || crate_names.iter().any(|existing| existing == crate_name)
            || !is_rust_identifier(crate_name)
        {
            continue;
        }
        crate_names.push(crate_name.clone());
        let event = ScenarioEvent {
            kind: format!("boundary-{}", policy.as_str()),
            label: Some(crate_name.clone()),
            value: Some(options.seed),
        };
        source.push_str("mod ");
        source.push_str(crate_name);
        source.push_str(" {\n");
        source.push_str("    pub struct Client;\n");
        source.push_str("    impl Client {\n");
        source.push_str("        pub fn new() -> Self {\n            ");
        source.push_str(&event_print_statement(&event)?);
        source.push_str("\n            Client\n        }\n    }\n}\n");
    }
    Ok(source)
}

fn tokio_support_source(program: &ScenarioProgram, options: &ScenarioOptions) -> Result<String> {
    if !program.operations.iter().any(|operation| {
        matches!(
            operation.kind,
            ScenarioOpKind::ModeledEffect {
                boundary: ScenarioModeledBoundary::WardTask
            }
        )
    }) {
        return Ok(String::new());
    }
    let events = modeled_boundary_events(&ScenarioModeledBoundary::WardTask, options);
    let mut source = String::from("mod tokio {\n    pub fn spawn<F>(_future: F) {\n        ");
    source.push_str(&event_print_statements(&events)?);
    source.push_str("\n    }\n}\n");
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
    let mut source = String::from("impl __KoboWardStorage {\n");
    for method in methods {
        source.push_str("    fn ");
        source.push_str(&method);
        if crate::storage::method_takes_value(&method) {
            source.push_str("<T>(&self, _value: T) {\n        ");
        } else {
            source.push_str("(&self) {\n        ");
        }
        source.push_str(&event_print_statements(
            &crate::storage::events_for_action(&method, options.seed),
        )?);
        source.push_str("\n    }\n");
    }
    source.push_str("}\n");
    Ok(source)
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
    if methods.is_empty() {
        methods.extend(["send", "delay", "reorder", "drop"].map(str::to_owned));
    }
    let mut source = String::from("impl __KoboWardNetwork {\n");
    for method in methods {
        source.push_str("    fn ");
        source.push_str(&method);
        source.push_str("<T>(&self, _value: T) {\n        ");
        source.push_str(&event_print_statements(
            &crate::network::harness_events_for_action(&method, options.seed),
        )?);
        source.push_str("\n    }\n");
    }
    source.push_str("}\n");
    Ok(source)
}

fn inject_modeled_boundary_event(
    source: String,
    boundary: &ScenarioModeledBoundary,
    events: &[ScenarioEvent],
) -> Result<String> {
    let print = event_print_statements(events)?;
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

fn main_wrapper_source(target: &str, final_events: &[ScenarioEvent]) -> Result<String> {
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
            ScenarioOpKind::Transfer { binding, callee } => events.push(ScenarioEvent {
                kind: "obligation-transfer".to_owned(),
                label: Some(format!("{binding}->{callee}")),
                value: None,
            }),
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
            }]);
        }
        if failure_events.is_none() {
            if let Some(boundary) = first_modeled_boundary(program) {
                failure_events = Some(vec![ScenarioEvent {
                    kind: "failure-injection-cancel".to_owned(),
                    label: Some(boundary.to_owned()),
                    value: None,
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
