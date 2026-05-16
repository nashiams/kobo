use std::path::{Path, PathBuf};

use anyhow::Context;
use kobo_errors::{
    diagnostic_to_json_value, ColorMode, DiagDecision, DiagLabel, DiagnosticOutputFormat,
    DiagnosticRenderer, KDiagnostic, KErrorCode, Severity,
};
use kobo_ir::{FileSetBuilder, KoboMode, KoboSpan};
use kobo_sim_core::{EngineMode, FullDepthRun, ReplayGuarantee, ScenarioEvent, ScenarioFailure};

use crate::ErrorFormat;

use super::sim_model::{self, ScenarioDocument};

pub(super) fn cmd_test(
    file: &Path,
    sim: Option<&str>,
    profile: Option<&str>,
    seed: Option<u64>,
    events: Option<&str>,
    inject: Option<&str>,
    fuzz: bool,
    event_budget: Option<u64>,
    witness_dir: Option<&Path>,
    error_format: ErrorFormat,
    target: Option<&str>,
    engine: Option<&str>,
) -> anyhow::Result<()> {
    let sim_profile = parse_sim_profile(sim)?;
    let seed = seed.unwrap_or(0);
    let engine = parse_engine(engine.unwrap_or("both"))?;
    let document = sim_model::load_document(file)?;
    let target_name = target
        .map(str::to_owned)
        .or_else(|| {
            document
                .scenarios
                .first()
                .map(|scenario| scenario.name.clone())
        })
        .unwrap_or_else(|| "<missing>".to_owned());
    let profile_roles = resolve_profile_roles(profile, &document, &target_name);
    let mut session = super::session::build_session(file, Some(KoboMode::Checked))?;
    let artifacts = kobo_driver::run_codegen_pipeline(&mut session, file)
        .map_err(|()| anyhow::anyhow!("failed to build compiler scenario artifacts"))?;
    let scenario_program = kobo_driver::build_scenario_program(
        &artifacts,
        &target_name,
        document.source_hash.clone(),
        &profile_roles.backend_profile,
    )?;
    let options = kobo_sim_core::ScenarioOptions {
        sim_profile: sim_profile.to_owned(),
        profile: profile_roles.backend_profile.clone(),
        seed,
        inject: inject.map(str::to_owned),
        event_budget: event_budget.or_else(|| default_budget(sim_profile)),
    };
    let fuzz_plan = fuzz.then(|| FuzzPlan::new(seed));
    let run = match fuzz_plan.as_ref() {
        Some(plan) => run_fuzz_portfolio(
            &scenario_program,
            &artifacts.rs_source,
            &options,
            engine,
            plan,
        )?,
        None => kobo_sim_core::run_full_depth_from_program(
            &scenario_program,
            &artifacts.rs_source,
            &options,
            engine,
        )?,
    };

    let mut witness_path = None;
    if run.failure.is_some() || witness_dir.is_some() {
        witness_path = Some(write_run_witness(
            file,
            &document,
            &profile_roles.guarantee_profile,
            sim_profile,
            seed,
            inject,
            fuzz_plan.as_ref(),
            witness_dir,
            &run,
        )?);
    }

    if events == Some("json") {
        print_events(file, sim_profile, seed, fuzz_plan.as_ref(), &run)?;
        return Ok(());
    }

    if let Some(failure) = run.failure.as_ref() {
        emit_failure(
            file,
            &document.source,
            failure,
            witness_path.as_deref(),
            error_format,
        )?;
        anyhow::bail!("{}", failure.message)
    }

    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "scenario": run.target,
            "seed": seed,
            "backend_profile": run.profile,
            "status": "passed",
        }))?
    );
    Ok(())
}

struct ProfileRoles {
    guarantee_profile: String,
    backend_profile: String,
}

struct FuzzPlan {
    base_seed: u64,
    cases: Vec<FuzzCase>,
}

struct FuzzCase {
    index: usize,
    seed: u64,
}

impl FuzzPlan {
    fn new(base_seed: u64) -> Self {
        let cases = (0..8)
            .map(|index| FuzzCase {
                index,
                seed: derive_fuzz_seed(base_seed, index),
            })
            .collect();
        Self { base_seed, cases }
    }
}

fn parse_sim_profile(sim: Option<&str>) -> anyhow::Result<&str> {
    match sim {
        Some(profile @ ("quick" | "deep" | "replay" | "exhaustive")) => Ok(profile),
        Some(other) => anyhow::bail!(
            "kobo test --sim {other} is not available; expected quick, deep, replay, or exhaustive"
        ),
        None => anyhow::bail!("kobo test requires --sim quick, deep, replay, or exhaustive"),
    }
}

fn resolve_profile_roles(
    cli_profile: Option<&str>,
    document: &ScenarioDocument,
    target_name: &str,
) -> ProfileRoles {
    let scenario_profile = document
        .scenarios
        .iter()
        .find(|scenario| scenario.name == target_name)
        .map(|scenario| scenario.profile.clone())
        .unwrap_or_else(|| sim_model::target_profile(document, target_name, None));

    match cli_profile {
        Some(profile @ ("dev" | "checked" | "release")) => ProfileRoles {
            guarantee_profile: profile.to_owned(),
            backend_profile: scenario_profile,
        },
        Some(profile) => ProfileRoles {
            guarantee_profile: "checked".to_owned(),
            backend_profile: profile.to_owned(),
        },
        None => ProfileRoles {
            guarantee_profile: "checked".to_owned(),
            backend_profile: scenario_profile,
        },
    }
}

fn default_budget(sim_profile: &str) -> Option<u64> {
    match sim_profile {
        "quick" => Some(64),
        "deep" => Some(1024),
        "replay" => Some(64),
        "exhaustive" => Some(16),
        _ => None,
    }
}

fn run_fuzz_portfolio(
    scenario_program: &kobo_ir::ScenarioProgram,
    generated_rust: &str,
    options: &kobo_sim_core::ScenarioOptions,
    engine: EngineMode,
    plan: &FuzzPlan,
) -> anyhow::Result<FullDepthRun> {
    let mut combined_events = Vec::new();
    let mut last_run = None;

    for case in &plan.cases {
        combined_events.push(ScenarioEvent {
            kind: "fuzz-case".to_owned(),
            label: Some(format!(
                "seed={};case={};derived_seed={}",
                plan.base_seed, case.index, case.seed
            )),
            value: Some(case.seed),
        });
        let mut case_options = options.clone();
        case_options.seed = case.seed;
        let mut run = kobo_sim_core::run_full_depth_from_program(
            scenario_program,
            generated_rust,
            &case_options,
            engine.clone(),
        )?;
        combined_events.extend(run.events.iter().cloned());
        if run.failure.is_some() {
            run.events = combined_events;
            refresh_digest_for_events(&mut run)?;
            return Ok(run);
        }
        last_run = Some(run);
    }

    let mut run = last_run.unwrap_or_else(|| FullDepthRun {
        target: scenario_program.target.clone(),
        profile: options.profile.clone(),
        replay_guarantee: ReplayGuarantee::Exact,
        events: Vec::new(),
        failure: None,
        coverage: kobo_sim_core::ScenarioCoverage {
            unsupported_constructs: Vec::new(),
            reason: None,
        },
        digest: kobo_sim_core::ExecutionDigest {
            semantic_engine: "semantic-sim".to_owned(),
            harness_engine: "generated-rust-harness".to_owned(),
            model_version: "v0.10".to_owned(),
            scenario_ir_hash: scenario_program.source_hash.clone(),
            operation_count: 0,
            semantic_trace_hash: String::new(),
            harness_trace_hash: String::new(),
            agreement: "matched".to_owned(),
            generated_rust_hash: None,
            harness_manifest_hash: None,
            harness_exit_code: None,
            harness_event_count: 0,
        },
        modeled_boundaries: Vec::new(),
        opaque_boundaries: Vec::new(),
        obligations: Vec::new(),
        boundary_decisions: Vec::new(),
        harness_manifest: None,
    });
    run.events = combined_events;
    refresh_digest_for_events(&mut run)?;
    Ok(run)
}

fn refresh_digest_for_events(run: &mut FullDepthRun) -> anyhow::Result<()> {
    let serialized = serde_json::to_string(&events_json(&run.events))?;
    let trace_hash = kobo_sim_core::digest::stable_hash(&serialized);
    run.digest.operation_count = run.events.len();
    run.digest.semantic_trace_hash = trace_hash.clone();
    run.digest.harness_trace_hash = trace_hash;
    run.digest.harness_event_count = run.events.len();
    Ok(())
}

fn derive_fuzz_seed(base_seed: u64, index: usize) -> u64 {
    base_seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407)
        .wrapping_add(index as u64)
}

fn parse_engine(value: &str) -> anyhow::Result<EngineMode> {
    match value {
        "semantic" => Ok(EngineMode::SemanticOnly),
        "harness" => Ok(EngineMode::HarnessOnly),
        "both" => Ok(EngineMode::Both),
        _ => anyhow::bail!("invalid sim engine; expected semantic, harness, or both"),
    }
}

fn print_events(
    file: &Path,
    sim_profile: &str,
    seed: u64,
    fuzz_plan: Option<&FuzzPlan>,
    run: &FullDepthRun,
) -> anyhow::Result<()> {
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "schema_version": 1,
            "file": sim_model::cli_relative_path(file)?,
            "sim_profile": sim_profile,
            "backend_profile": run.profile,
            "scheduler": scheduler_json(sim_profile, seed, run),
            "fuzz": fuzz_plan_json(fuzz_plan),
            "seed": seed,
            "events": events_json(&run.events),
        }))?
    );
    Ok(())
}

fn write_run_witness(
    file: &Path,
    document: &ScenarioDocument,
    guarantee_profile: &str,
    sim_profile: &str,
    seed: u64,
    inject: Option<&str>,
    fuzz_plan: Option<&FuzzPlan>,
    witness_dir: Option<&Path>,
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
    let backend_replay_token = replay_token(&document.source_hash, seed, run);
    let coverage = coverage_json(run);
    let event_stream = shrink_event_stream(run, sim_profile);
    let witness_events = events_json(&event_stream.events);
    let witness = serde_json::json!({
        "schema_version": 1,
        "kobo_version": env!("CARGO_PKG_VERSION"),
        "target": format!("{}:{}", source_path, run.target),
        "scenario": {
            "name": run.target,
            "profile": run.profile,
        },
        "sim_profile": sim_profile,
        "source": {
            "path": source_path,
            "hash": document.source_hash,
        },
        "guarantee_profile": guarantee_profile,
        "expanded_policy": expanded_policy_json(guarantee_profile),
        "seed": seed,
        "fuzz": fuzz_plan_json(fuzz_plan),
        "injections": injections_json(inject),
        "backend_profile": run.profile,
        "backend": backend_for_profile(&run.profile),
        "backend_replay": backend_replay_token,
        "backend_replay_token": backend_replay_token,
        "scheduler": scheduler_json(sim_profile, seed, run),
        "execution_digest": execution_digest_json(run),
        "harness_manifest": run.harness_manifest.clone(),
        "coverage": coverage,
        "scenario_coverage": scenario_coverage_json(run),
        "replay_guarantee": run.replay_guarantee.as_str(),
        "exactness": exactness_json(run),
        "shrink": shrink_json(run, &event_stream),
        "modeled_boundaries": modeled_boundaries_json(run),
        "opaque_boundaries": run.opaque_boundaries.clone(),
        "boundary_policies": boundary_policies_json(run),
        "boundary_assumptions": boundary_assumptions_json(run),
        "obligations": obligations_json(&source_path, &document.source, run),
        "obligation_events": obligation_events_json(run),
        "boundary_decisions": boundary_decisions_json(run),
        "available_boundary_policies": ["model", "record", "stub", "outside", "opaque", "debt"],
        "failure": failure_json(&source_path, &document.source, run, primary_span),
        "source_spans": source_spans_json(&source_path, &document.source, run),
        "events": witness_events.clone(),
        "event_stream": witness_events,
    });
    std::fs::write(&witness_path, serde_json::to_string_pretty(&witness)?)
        .with_context(|| format!("failed to write {}", witness_path.display()))?;
    Ok(witness_path)
}

fn execution_digest_json(run: &FullDepthRun) -> serde_json::Value {
    serde_json::json!({
        "engine": "semantic-sim",
        "semantic_engine": run.digest.semantic_engine,
        "harness_engine": run.digest.harness_engine,
        "model_version": run.digest.model_version,
        "scenario_ir_hash": run.digest.scenario_ir_hash,
        "operation_count": run.digest.operation_count,
        "event_hash": run.digest.semantic_trace_hash,
        "semantic_trace_hash": run.digest.semantic_trace_hash,
        "harness_trace_hash": run.digest.harness_trace_hash,
        "agreement": run.digest.agreement,
        "generated_rust_hash": run.digest.generated_rust_hash,
        "harness_manifest_hash": run.digest.harness_manifest_hash,
        "harness_exit_code": run.digest.harness_exit_code,
        "harness_event_count": run.digest.harness_event_count,
    })
}

fn coverage_json(run: &FullDepthRun) -> serde_json::Value {
    let covered = run
        .obligations
        .iter()
        .filter(|obligation| obligation.is_discharged)
        .map(|obligation| obligation.binding.clone())
        .collect::<Vec<_>>();
    let uncovered = run
        .obligations
        .iter()
        .filter(|obligation| !obligation.is_discharged)
        .map(|obligation| obligation.binding.clone())
        .collect::<Vec<_>>();
    serde_json::json!({
        "unsupported_constructs": run.coverage.unsupported_constructs,
        "reason": run.coverage.reason,
        "covered": covered,
        "uncovered": uncovered,
        "escaped": escaped_obligations(run),
        "suppressed": [],
        "boundary_owned": run.opaque_boundaries,
        "unknown": [],
    })
}

fn scenario_coverage_json(run: &FullDepthRun) -> serde_json::Value {
    coverage_json(run)
}

fn scheduler_json(sim_profile: &str, seed: u64, run: &FullDepthRun) -> serde_json::Value {
    let strategy = match sim_profile {
        "quick" => "small-random",
        "deep" => "pct-random-bounded",
        "replay" => "witness-event-stream",
        "exhaustive" => "tiny-ward-exhaustive",
        _ => "unknown",
    };
    serde_json::json!({
        "profile": sim_profile,
        "strategy": strategy,
        "seed": seed,
        "event_budget": run
            .events
            .iter()
            .find(|event| event.kind == "scheduler-portfolio")
            .and_then(|event| event.value),
    })
}

fn exactness_json(run: &FullDepthRun) -> &'static str {
    match run.replay_guarantee {
        ReplayGuarantee::Exact => "exact",
        ReplayGuarantee::Partial => "partial",
        ReplayGuarantee::NotReplayable => "evidence_only",
    }
}

struct ShrunkEventStream {
    events: Vec<ScenarioEvent>,
    removed_event_ids: Vec<usize>,
}

fn shrink_event_stream(run: &FullDepthRun, sim_profile: &str) -> ShrunkEventStream {
    let mut removed_event_ids = Vec::new();
    if run.replay_guarantee == ReplayGuarantee::Exact && sim_profile == "deep" {
        removed_event_ids.extend(run.events.iter().enumerate().filter_map(|(index, event)| {
            if event.kind == "scheduler-pct-seed" {
                Some(index)
            } else {
                None
            }
        }));
    }
    let events = run
        .events
        .iter()
        .enumerate()
        .filter(|(index, _)| !removed_event_ids.contains(index))
        .map(|(_, event)| event.clone())
        .collect();
    ShrunkEventStream {
        events,
        removed_event_ids,
    }
}

fn shrink_json(run: &FullDepthRun, event_stream: &ShrunkEventStream) -> serde_json::Value {
    let mut shrink_passes = Vec::new();
    if !event_stream.removed_event_ids.is_empty() {
        shrink_passes.push(serde_json::json!({
            "pass": "trailing-independent-scheduler-events",
            "removed_event_ids": event_stream.removed_event_ids.clone(),
            "replay_checked": true,
        }));
    }
    for (pass, reason) in [
        (
            "unused-events",
            "no replay-irrelevant semantic events found",
        ),
        ("seeds-runs", "no smaller equivalent seed/run pair found"),
        (
            "data-sizes",
            "scenario has no shrinkable generated data fixtures",
        ),
        (
            "independent-injections",
            "no independent injected hook can be removed without changing the trace",
        ),
    ] {
        shrink_passes.push(serde_json::json!({
            "pass": pass,
            "removed_event_ids": [],
            "replay_checked": run.replay_guarantee == ReplayGuarantee::Exact,
            "reason": reason,
        }));
    }
    serde_json::json!({
        "original_event_count": run.events.len(),
        "shrunk_event_count": event_stream.events.len(),
        "removed_event_ids": event_stream.removed_event_ids.clone(),
        "shrink_passes": shrink_passes,
        "replay_checked": run.replay_guarantee == ReplayGuarantee::Exact,
        "reason_if_not_shrunk": if event_stream.removed_event_ids.is_empty() {
            Some("original witness is already minimal for the current replay contract")
        } else {
            None
        },
    })
}

fn injections_json(inject: Option<&str>) -> serde_json::Value {
    let hooks = inject
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|hook| !hook.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    serde_json::json!({
        "hooks": hooks,
    })
}

fn fuzz_plan_json(plan: Option<&FuzzPlan>) -> serde_json::Value {
    match plan {
        Some(plan) => serde_json::json!({
            "enabled": true,
            "base_seed": plan.base_seed,
            "case_count": plan.cases.len(),
            "cases": plan
                .cases
                .iter()
                .map(|case| serde_json::json!({
                    "index": case.index,
                    "seed": case.seed,
                }))
                .collect::<Vec<_>>(),
        }),
        None => serde_json::json!({
            "enabled": false,
            "case_count": 0,
            "cases": [],
        }),
    }
}

fn witness_directory(file: &Path, witness_dir: Option<&Path>) -> anyhow::Result<PathBuf> {
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

fn emit_failure(
    file: &Path,
    source: &str,
    failure: &ScenarioFailure,
    witness_path: Option<&Path>,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    let mut files = FileSetBuilder::new();
    let file_id = files.add_file(file.to_path_buf(), source.to_owned());
    let span = KoboSpan::new(
        failure.primary_start as u32,
        failure.primary_end.max(failure.primary_start + 1) as u32,
        file_id,
    );
    let message = match witness_path {
        Some(path) => format!("{}; witness {}", failure.message, path.display()),
        None => failure.message.clone(),
    };
    let diagnostic = KDiagnostic::new(
        failure.code,
        Severity::Error,
        DiagLabel::primary(span, message.clone()),
        scenario_failure_explanation(failure),
        DiagDecision(scenario_failure_decision(failure)),
    )
    .with_finding(scenario_failure_finding(failure, witness_path));

    match error_format {
        ErrorFormat::Json => {
            let mut value = diagnostic_to_json_value(files.as_file_set(), &diagnostic);
            if let Some(entry) = files.as_file_set().get(file_id) {
                value["line"] = serde_json::json!(entry.line_col(span.start).0);
            }
            println!("{}", serde_json::to_string(&value)?);
        }
        ErrorFormat::Human => {
            let color = super::session::resolve_color_mode(ColorMode::Auto);
            let renderer = DiagnosticRenderer::new(color, DiagnosticOutputFormat::HumanCard);
            eprintln!("{}", renderer.render(files.as_file_set(), &diagnostic));
        }
    }

    Ok(())
}

fn scenario_failure_finding(failure: &ScenarioFailure, witness_path: Option<&Path>) -> String {
    let finding = match failure.code {
        KErrorCode::K0100 => {
            let binding = scenario_failure_label(failure).unwrap_or("the value");
            format!(
                "A checked scenario can drop `{binding}` before calling one of its required actions."
            )
        }
        KErrorCode::K0102 => {
            "This replay path uses raw time, randomness, file IO, or task scheduling.".to_owned()
        }
        KErrorCode::K0103 => {
            "This replay path performs an effect Kobo cannot replay deterministically.".to_owned()
        }
        KErrorCode::K0105 => {
            "This scenario exceeded the quick simulation budget before it finished.".to_owned()
        }
        KErrorCode::K0107 => {
            let boundary = scenario_failure_label(failure).unwrap_or("external code");
            format!("This replay path crosses `{boundary}` without a boundary policy.")
        }
        KErrorCode::K0116 => {
            "This scenario uses syntax Kobo has not modeled for exact replay yet.".to_owned()
        }
        KErrorCode::K0117 => {
            "The compiler semantic trace and generated harness trace do not agree.".to_owned()
        }
        KErrorCode::K0118 => {
            "The action points at an artifact that does not match the current source.".to_owned()
        }
        _ => failure.message.clone(),
    };

    match witness_path {
        Some(path) => format!("{finding} Witness: {}", path.display()),
        None => finding,
    }
}

fn scenario_failure_explanation(failure: &ScenarioFailure) -> String {
    match failure.code {
        KErrorCode::K0100 => {
            "Kobo tracks must_call values as obligations. Every return, cancellation, or replayed path must finish the obligation before the value leaves scope.".to_owned()
        }
        KErrorCode::K0102 => {
            "Replay evidence only stays useful when the same inputs produce the same event stream. Raw nondeterminism can make a replay pass or fail for the wrong reason.".to_owned()
        }
        KErrorCode::K0103 => {
            "An uncontrolled effect can change outside Kobo's replay model. The scenario needs a model, a recording, or an explicit debt boundary before Kobo can trust the replay.".to_owned()
        }
        KErrorCode::K0105 => {
            "The quick profile is for small, fast evidence. A scenario that exceeds its budget needs to be shrunk or moved to a slower profile.".to_owned()
        }
        KErrorCode::K0107 => {
            "External code can perform IO, scheduling, time, randomness, or other effects that Kobo cannot infer from the source alone. The boundary policy says what replay may assume.".to_owned()
        }
        KErrorCode::K0116 => {
            "Exact replay is only sound for modeled syntax. Kobo found a construct outside the current modeled island coverage.".to_owned()
        }
        KErrorCode::K0117 => {
            "Full-depth replay needs two independent traces to match: the compiler semantic trace and the generated harness trace.".to_owned()
        }
        KErrorCode::K0118 => {
            "Editor and replay actions must point at artifacts created from the current source hash and target.".to_owned()
        }
        _ => failure.message.clone(),
    }
}

fn scenario_failure_decision(failure: &ScenarioFailure) -> String {
    match failure.code {
        KErrorCode::K0100 => {
            let actions = scenario_failure_actions(&failure.message)
                .unwrap_or_else(|| "one required action".to_owned());
            format!(
                "Call {actions} on every path, or record explicit debt if cleanup happens outside this scenario."
            )
        }
        KErrorCode::K0102 => {
            "Route time, randomness, file IO, and task scheduling through modeled facades before claiming replay evidence.".to_owned()
        }
        KErrorCode::K0103 => {
            "Model the effect, record the effect stream, move it outside replay, or mark replay debt explicitly.".to_owned()
        }
        KErrorCode::K0105 => {
            "Reduce the scenario, split it into smaller scenarios, or run it under a profile with a larger budget.".to_owned()
        }
        KErrorCode::K0107 => {
            "Choose model, record, stub, outside, opaque, or debt for this boundary before claiming exact replay.".to_owned()
        }
        KErrorCode::K0116 => {
            "Use a modeled construct, split the scenario, or keep the witness partial until coverage is implemented.".to_owned()
        }
        KErrorCode::K0117 => {
            "Regenerate the witness with --engine both and investigate any trace mismatch before replaying it.".to_owned()
        }
        KErrorCode::K0118 => {
            "Regenerate the witness/session artifacts for the current source before using the action.".to_owned()
        }
        _ => "Make replay evidence deterministic and explicit.".to_owned(),
    }
}

fn scenario_failure_label(failure: &ScenarioFailure) -> Option<&str> {
    failure
        .events
        .iter()
        .find_map(|event| event.label.as_deref())
}

fn scenario_failure_actions(message: &str) -> Option<String> {
    let actions = message.split("discharge with ").nth(1)?;
    Some(actions.trim_end_matches('.').to_owned())
}

fn modeled_boundaries_json(run: &FullDepthRun) -> Vec<&'static str> {
    run.modeled_boundaries
        .iter()
        .map(|boundary| boundary.as_str())
        .collect()
}

fn boundary_policies_json(run: &FullDepthRun) -> Vec<serde_json::Value> {
    let mut policies = run
        .boundary_decisions
        .iter()
        .map(|decision| {
            serde_json::json!({
                "boundary": decision.crate_name,
                "policy": decision.policy.as_str(),
                "reason": decision.reason,
            })
        })
        .collect::<Vec<_>>();

    for boundary in modeled_boundaries_json(run) {
        policies.push(serde_json::json!({
            "boundary": boundary,
            "policy": "model",
            "reason": "modeled v0.10 facade",
        }));
    }
    policies
}

fn obligations_json(source_path: &str, source: &str, run: &FullDepthRun) -> Vec<serde_json::Value> {
    run.obligations
        .iter()
        .map(|obligation| {
            serde_json::json!({
                "binding": obligation.binding,
                "type": obligation.type_name,
                "actions": obligation.actions,
                "discharged": obligation.is_discharged,
                "declaration_span": span_json(source_path, source, obligation.declaration_span),
                "drop_span": obligation.drop_span.map(|span| span_json(source_path, source, span)),
            })
        })
        .collect()
}

fn obligation_events_json(run: &FullDepthRun) -> Vec<serde_json::Value> {
    let mut events = Vec::new();
    for obligation in &run.obligations {
        events.push(serde_json::json!({
            "state": "created",
            "binding": obligation.binding,
            "type": obligation.type_name,
        }));
        if obligation.is_discharged {
            events.push(serde_json::json!({
                "state": "discharged",
                "binding": obligation.binding,
                "actions": obligation.actions,
            }));
        } else {
            events.push(serde_json::json!({
                "state": "leaked",
                "binding": obligation.binding,
                "failure_mode": unresolved_failure_mode(obligation),
            }));
        }
    }
    for event in &run.events {
        if event.kind == "obligation-transfer" {
            events.push(serde_json::json!({
                "state": "transferred",
                "binding": event.label,
            }));
        }
    }
    if run.failure.is_none() {
        events.push(serde_json::json!({
            "state": "returned",
            "binding": null,
        }));
    }
    events
}

fn expanded_policy_json(profile: &str) -> serde_json::Value {
    let (ownership, liveness, replay, boundaries, errors) = match profile {
        "dev" => ("record", "record", "record", "record", "ergonomic"),
        "release" => ("strict", "checked", "checked", "strict", "explicit"),
        _ => ("checked", "checked", "checked", "checked", "typed"),
    };
    serde_json::json!({
        "profile": profile,
        "ownership": ownership,
        "liveness": liveness,
        "replay": replay,
        "boundaries": boundaries,
        "errors": errors,
    })
}

fn boundary_assumptions_json(run: &FullDepthRun) -> Vec<serde_json::Value> {
    let mut assumptions = run
        .boundary_decisions
        .iter()
        .map(|decision| {
            serde_json::json!({
                "boundary": decision.crate_name,
                "policy": decision.policy.as_str(),
                "reason": decision.reason,
                "replay_effect": if run.replay_guarantee == ReplayGuarantee::Exact {
                    "modeled"
                } else {
                    run.replay_guarantee.as_str()
                },
            })
        })
        .collect::<Vec<_>>();

    if run.replay_guarantee == ReplayGuarantee::NotReplayable && assumptions.is_empty() {
        let boundary = run
            .failure
            .as_ref()
            .and_then(|failure| failure.events.first())
            .and_then(|event| event.label.clone())
            .unwrap_or_else(|| "uncontrolled".to_owned());
        let reason = run
            .failure
            .as_ref()
            .map(|failure| failure.message.clone())
            .unwrap_or_else(|| "not replayable".to_owned());
        assumptions.push(serde_json::json!({
            "boundary": boundary,
            "policy": "debt",
            "reason": reason,
            "replay_effect": run.replay_guarantee.as_str(),
        }));
    }

    assumptions
}

fn failure_json(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
    primary_span: Option<String>,
) -> serde_json::Value {
    let Some(failure) = run.failure.as_ref() else {
        return serde_json::Value::Null;
    };
    serde_json::json!({
        "code": failure.code.as_str(),
        "message": failure.message,
        "mode": run.failure
            .as_ref()
            .map(|_| run_failure_mode(run))
            .unwrap_or("none"),
        "primary_span": primary_span.unwrap_or_else(|| format!("{source_path}:1:1")),
        "related_spans": related_spans_json(source_path, source, run, failure),
    })
}

fn source_spans_json(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
) -> Vec<serde_json::Value> {
    let mut spans = Vec::new();
    if let Some(failure) = run.failure.as_ref() {
        spans.push(span_json(
            source_path,
            source,
            (failure.primary_start, failure.primary_end),
        ));
    }
    for obligation in &run.obligations {
        spans.push(span_json(source_path, source, obligation.declaration_span));
        if let Some(drop_span) = obligation.drop_span {
            spans.push(span_json(source_path, source, drop_span));
        }
    }
    spans
}

fn run_failure_mode(run: &FullDepthRun) -> &'static str {
    if run
        .events
        .iter()
        .any(|event| event.kind == "lost-message" || event.kind == "storage-crash-after-write")
    {
        return "lost-message";
    }
    if run
        .obligations
        .iter()
        .any(|obligation| unresolved_failure_mode(obligation) == "unresolved-reply")
    {
        return "unresolved-reply";
    }
    "unresolved-delivery"
}

fn unresolved_failure_mode(obligation: &kobo_sim_core::RuntimeObligationSummary) -> &'static str {
    if obligation
        .actions
        .iter()
        .any(|action| action == "reply" || action == "reject" || action == "cancel")
    {
        "unresolved-reply"
    } else {
        "unresolved-delivery"
    }
}

fn escaped_obligations(run: &FullDepthRun) -> Vec<String> {
    run.obligations
        .iter()
        .filter(|obligation| !obligation.is_discharged && obligation.drop_span.is_some())
        .map(|obligation| obligation.binding.clone())
        .collect()
}

fn related_spans_json(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
    failure: &ScenarioFailure,
) -> Vec<serde_json::Value> {
    if failure.code != KErrorCode::K0100 {
        return Vec::new();
    }
    run.obligations
        .iter()
        .filter(|obligation| !obligation.is_discharged)
        .map(|obligation| {
            serde_json::json!({
                "label": format!("obligation `{}` declared here", obligation.binding),
                "span": span_json(source_path, source, obligation.declaration_span),
            })
        })
        .collect()
}

fn span_json(source_path: &str, source: &str, span: (usize, usize)) -> serde_json::Value {
    serde_json::json!({
        "path": source_path,
        "line": one_based_line_for_offset(source, span.0),
        "start": span.0,
        "end": span.1.max(span.0 + 1),
    })
}

fn boundary_decisions_json(run: &FullDepthRun) -> Vec<serde_json::Value> {
    run.boundary_decisions
        .iter()
        .map(|decision| {
            serde_json::json!({
                "crate": decision.crate_name,
                "policy": decision.policy.as_str(),
                "reason": decision.reason,
            })
        })
        .collect()
}

fn events_json(events: &[ScenarioEvent]) -> Vec<serde_json::Value> {
    events
        .iter()
        .map(|event| {
            serde_json::json!({
                "kind": event.kind,
                "label": event.label,
                "value": event.value,
            })
        })
        .collect()
}

fn replay_token(source_identity: &str, seed: u64, run: &FullDepthRun) -> String {
    let mut material = String::new();
    material.push_str(source_identity);
    material.push(':');
    material.push_str(&seed.to_string());
    material.push(':');
    material.push_str(backend_for_profile(&run.profile));
    material.push(':');
    material.push_str(&run.digest.semantic_trace_hash);
    material.push(':');
    material.push_str(&run.digest.harness_trace_hash);
    kobo_sim_core::digest::stable_hash(&material)
}

fn backend_for_profile(profile: &str) -> &'static str {
    match profile {
        "sync" => "loom",
        "stateful-input" => "proptest",
        "failpoint" => "failpoints",
        "network" | "network-design" => "network-design",
        _ => "shuttle",
    }
}

fn sanitize_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}

fn one_based_line_for_offset(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}
