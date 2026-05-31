use super::json_schema::scheduler_json;
use super::model_compare::apply_model_vs_implementation;
use super::{
    apply_trace_checks, backend_debt, emit_failure, failure_exit_message, formal_core,
    parse_engine, print_events, reserved_backend_fit_json, run_fuzz_portfolio, run_seed_portfolio,
    serde_json, sim_model, validate_run_boundary_declarations, write_run_witness,
    BackendExpertOptions, DebtControlSource, EngineMode, ErrorFormat, FullDepthRun, FuzzPlan,
    GuaranteePolicy, GuaranteeProfile, Path, PathBuf, ProfileRoles, ScenarioDocument,
    ScenarioProgram, SeedPortfolioPlan,
};
use std::time::Duration;

const DEFAULT_DEEP_SEED_COUNT: u64 = 1024;
const DEFAULT_DEEP_WALL_CLOCK_LIMIT: Duration = Duration::from_secs(5);
use crate::commands::session;

struct TestCommandRequest<'a> {
    file: &'a Path,
    sim: Option<&'a str>,
    profile: Option<&'a str>,
    seed: Option<u64>,
    events: Option<&'a str>,
    inject: Option<&'a str>,
    fuzz: bool,
    event_budget: Option<u64>,
    witness_dir: Option<&'a Path>,
    error_format: ErrorFormat,
    target: Option<&'a str>,
    engine: Option<&'a str>,
}

struct TestExecutionContext {
    session: kobo_driver::CompileSession,
    document: ScenarioDocument,
    profile_roles: ProfileRoles,
    sim_profile: String,
    requested_seed: u64,
    engine: EngineMode,
    effective_scheduler: Option<String>,
    effective_max_branches: Option<u64>,
    effective_shrink: String,
    effective_replay_token: String,
    effective_checkpoint_replay: bool,
    artifacts: kobo_driver::CodegenArtifacts,
    scenario_program: ScenarioProgram,
    options: kobo_sim_core::ScenarioOptions,
    configured_seed_count: Option<u64>,
}

pub(crate) fn cmd_test(
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
    expert_options: BackendExpertOptions,
) -> anyhow::Result<()> {
    let request = TestCommandRequest {
        file,
        sim,
        profile,
        seed,
        events,
        inject,
        fuzz,
        event_budget,
        witness_dir,
        error_format,
        target,
        engine,
    };
    let context = prepare_test_context(&request, &expert_options)?;
    let (mut run, executed_seed, fuzz_plan) = execute_scenario(&request, &context)?;
    apply_replay_checks(&request, &context, executed_seed, &mut run)?;
    let witness_path = maybe_write_witness(
        &request,
        &context,
        &expert_options,
        executed_seed,
        fuzz_plan.as_ref(),
        &run,
    )?;
    finish_test_result(
        &request,
        &context,
        &expert_options,
        executed_seed,
        fuzz_plan.as_ref(),
        &run,
        witness_path.as_deref(),
    )
}

fn prepare_test_context(
    request: &TestCommandRequest<'_>,
    expert_options: &BackendExpertOptions,
) -> anyhow::Result<TestExecutionContext> {
    let mut session = session::build_session(
        request.file,
        Some(GuaranteePolicy::for_profile(GuaranteeProfile::Checked)),
    )?;
    let sim_profile = parse_sim_profile(request.sim, &session.config)?;
    let requested_seed = request.seed.unwrap_or(0);
    let engine = parse_engine(request.engine.unwrap_or("both"))?;
    let document = sim_model::load_document(request.file)?;
    let target_name = request
        .target
        .map(str::to_owned)
        .or_else(|| {
            document
                .scenarios
                .first()
                .map(|scenario| scenario.name.clone())
        })
        .unwrap_or_else(|| "<missing>".to_owned());
    let profile_roles = resolve_profile_roles(request.profile, &document, &target_name)?;
    let execution_profile = expert_options.execution_profile(&profile_roles.backend_profile)?;
    let backend_name = expert_options.backend_name(&execution_profile).to_owned();
    let effective_scheduler = expert_options
        .effective_scheduler(&session.config, &execution_profile)
        .map(str::to_owned);
    let effective_max_branches = expert_options
        .max_branches
        .or_else(|| session.config.sim.max_branches_for_backend(&backend_name));
    let effective_shrink = session
        .config
        .sim
        .shrink_for(&sim_profile)
        .unwrap_or_else(|| default_shrink_mode(&sim_profile))
        .to_owned();
    let effective_replay_token = session
        .config
        .sim
        .replay_token_for_backend(&backend_name)
        .unwrap_or("record")
        .to_owned();
    let effective_checkpoint_replay = session
        .config
        .sim
        .checkpoint_replay_for_backend(&backend_name)
        .unwrap_or(false);
    validate_backend_selection(
        request.file,
        &target_name,
        &session.config,
        &execution_profile,
        &backend_name,
        effective_scheduler.as_deref(),
        expert_options,
    )?;
    expert_options.validate(
        &execution_profile,
        &sim_profile,
        effective_scheduler.as_deref(),
        effective_max_branches,
    )?;
    let artifacts = kobo_driver::run_codegen_pipeline(&mut session, request.file)
        .map_err(|()| anyhow::anyhow!("failed to build compiler scenario artifacts"))?;
    let scenario_program = kobo_driver::build_scenario_program(
        &artifacts,
        &target_name,
        document.source_hash.clone(),
        &execution_profile,
    )?;
    let options = kobo_sim_core::ScenarioOptions {
        sim_profile: sim_profile.to_owned(),
        profile: execution_profile,
        seed: requested_seed,
        inject: request.inject.map(str::to_owned),
        event_budget: request
            .event_budget
            .or(effective_max_branches)
            .or_else(|| session.config.sim.schedule_budget_for(&sim_profile))
            .or_else(|| default_budget(&sim_profile)),
        scheduler: kobo_sim_core::SchedulerPolicy::from_name(effective_scheduler.as_deref()),
        loom_max_branches: effective_max_branches,
        loom_checkpoint_replay: effective_checkpoint_replay,
    };
    let configured_seed_count = session.config.sim.seed_count_for(&sim_profile);

    Ok(TestExecutionContext {
        session,
        document,
        profile_roles,
        sim_profile,
        requested_seed,
        engine,
        effective_scheduler,
        effective_max_branches,
        effective_shrink,
        effective_replay_token,
        effective_checkpoint_replay,
        artifacts,
        scenario_program,
        options,
        configured_seed_count,
    })
}

fn validate_backend_selection(
    file: &Path,
    target_name: &str,
    config: &kobo_driver::KoboConfig,
    execution_profile: &str,
    backend_name: &str,
    effective_scheduler: Option<&str>,
    expert_options: &BackendExpertOptions,
) -> anyhow::Result<()> {
    if let Some(capability) = backend_debt::unsupported_backend_capability(backend_name) {
        let debt_path = backend_debt::write_unsupported_backend_debt(
            file,
            target_name,
            capability,
            effective_scheduler,
            DebtControlSource::TestCommand,
        )?;
        anyhow::bail!(
            "{}",
            backend_debt::unsupported_backend_debt_message(capability, &debt_path)?
        );
    }
    if expert_options.backend.is_none() {
        if let Some(intent) =
            backend_debt::configured_reserved_backend_intent(config, execution_profile)
        {
            let debt_path = backend_debt::write_unsupported_backend_debt(
                file,
                target_name,
                intent.capability,
                intent.scheduler,
                DebtControlSource::Config,
            )?;
            anyhow::bail!(
                "{}",
                backend_debt::unsupported_backend_debt_message(intent.capability, &debt_path)?
            );
        }
    }
    Ok(())
}

fn execute_scenario(
    request: &TestCommandRequest<'_>,
    context: &TestExecutionContext,
) -> anyhow::Result<(FullDepthRun, u64, Option<FuzzPlan>)> {
    let fuzz_plan = if request.fuzz {
        Some(FuzzPlan::new(
            context.requested_seed,
            &context.scenario_program,
            context
                .configured_seed_count
                .unwrap_or(FuzzPlan::DEFAULT_CASES),
        )?)
    } else {
        None
    };
    let (run, executed_seed) = match fuzz_plan.as_ref() {
        Some(plan) => (
            run_fuzz_portfolio(
                &context.scenario_program,
                &context.artifacts.rs_source,
                &context.options,
                context.engine.clone(),
                plan,
            )?,
            context.requested_seed,
        ),
        None => run_seed_portfolio(
            &context.scenario_program,
            &context.artifacts.rs_source,
            &context.options,
            context.engine.clone(),
            seed_portfolio_plan(context),
        )?,
    };
    Ok((run, executed_seed, fuzz_plan))
}

fn seed_portfolio_plan(context: &TestExecutionContext) -> SeedPortfolioPlan {
    let target_seed_count = context
        .configured_seed_count
        .or_else(|| default_seed_count(&context.sim_profile))
        .unwrap_or(1);
    let plan = SeedPortfolioPlan::with_target_seed_count(target_seed_count);
    if context.sim_profile == "deep" && target_seed_count >= DEFAULT_DEEP_SEED_COUNT {
        return plan.with_wall_clock_limit(DEFAULT_DEEP_WALL_CLOCK_LIMIT);
    }
    plan
}

fn apply_replay_checks(
    request: &TestCommandRequest<'_>,
    context: &TestExecutionContext,
    executed_seed: u64,
    run: &mut FullDepthRun,
) -> anyhow::Result<()> {
    let strict_source_path = sim_model::cli_relative_path(request.file)?;
    formal_core::apply_strict_liveness(
        &strict_source_path,
        &context.document.source,
        &context.scenario_program,
        run,
    );
    apply_trace_checks(&strict_source_path, &context.document.source, run);
    apply_model_vs_implementation(
        &strict_source_path,
        &context.document.source,
        executed_seed,
        run,
    );
    validate_run_boundary_declarations(request.file, &context.session.config, run)
}

fn maybe_write_witness(
    request: &TestCommandRequest<'_>,
    context: &TestExecutionContext,
    expert_options: &BackendExpertOptions,
    executed_seed: u64,
    fuzz_plan: Option<&FuzzPlan>,
    run: &FullDepthRun,
) -> anyhow::Result<Option<PathBuf>> {
    if run.failure.is_some() || request.witness_dir.is_some() {
        return Ok(Some(write_run_witness(
            request.file,
            &context.document,
            &context.scenario_program,
            &context.profile_roles.guarantee_profile,
            &context.sim_profile,
            executed_seed,
            request.inject,
            fuzz_plan,
            request.witness_dir,
            &context.session.config,
            &context.artifacts.runtime_evidence,
            &context.artifacts.source_map,
            expert_options,
            context.effective_scheduler.as_deref(),
            context.effective_max_branches,
            context.effective_shrink.as_str(),
            context.effective_replay_token.as_str(),
            context.effective_checkpoint_replay,
            run,
        )?));
    }
    Ok(None)
}

fn finish_test_result(
    request: &TestCommandRequest<'_>,
    context: &TestExecutionContext,
    expert_options: &BackendExpertOptions,
    executed_seed: u64,
    fuzz_plan: Option<&FuzzPlan>,
    run: &FullDepthRun,
    witness_path: Option<&Path>,
) -> anyhow::Result<()> {
    if request.events == Some("json") {
        print_events(
            request.file,
            &context.sim_profile,
            executed_seed,
            fuzz_plan,
            expert_options,
            context.effective_max_branches,
            context.effective_shrink.as_str(),
            context.effective_replay_token.as_str(),
            context.effective_checkpoint_replay,
            &context.session.config,
            context.effective_scheduler.as_deref(),
            run,
        )?;
        return Ok(());
    }

    if let Some(failure) = run.failure.as_ref() {
        emit_failure(
            request.file,
            &context.document.source,
            failure,
            witness_path,
            request.error_format,
        )?;
        anyhow::bail!("{}", failure_exit_message(failure))
    }
    print_success_response(context, expert_options, executed_seed, run)
}

fn print_success_response(
    context: &TestExecutionContext,
    expert_options: &BackendExpertOptions,
    executed_seed: u64,
    run: &FullDepthRun,
) -> anyhow::Result<()> {
    let mut response = serde_json::Map::new();
    response.insert("scenario".to_owned(), run.target.clone().into());
    response.insert("seed".to_owned(), serde_json::json!(executed_seed));
    response.insert("sim_profile".to_owned(), context.sim_profile.clone().into());
    response.insert("status".to_owned(), "passed".into());
    if context.session.config.sim.show_backend_choices
        || expert_options.has_explicit_backend_controls()
    {
        response.insert("backend_profile".to_owned(), run.profile.clone().into());
        response.insert(
            "backend".to_owned(),
            expert_options.backend_name(&run.profile).into(),
        );
        response.insert(
            "reserved_backend_fit".to_owned(),
            reserved_backend_fit_json(&run.profile),
        );
        response.insert(
            "scheduler".to_owned(),
            scheduler_json(
                &context.sim_profile,
                executed_seed,
                run,
                context.effective_scheduler.as_deref(),
            ),
        );
    }
    println!(
        "{}",
        serde_json::to_string(&serde_json::Value::Object(response))?
    );
    Ok(())
}

fn parse_sim_profile(
    sim: Option<&str>,
    config: &kobo_driver::KoboConfig,
) -> anyhow::Result<String> {
    match sim {
        Some(profile @ ("quick" | "deep" | "replay" | "exhaustive")) => Ok(profile.to_owned()),
        Some(other) => anyhow::bail!(
            "kobo test --sim {other} is not available; expected quick, deep, replay, or exhaustive"
        ),
        None => Ok(config.sim.default_profile.clone()),
    }
}

fn resolve_profile_roles(
    cli_profile: Option<&str>,
    document: &ScenarioDocument,
    target_name: &str,
) -> anyhow::Result<ProfileRoles> {
    let scenario_profile = document
        .scenarios
        .iter()
        .find(|scenario| scenario.name == target_name)
        .map(|scenario| scenario.profile.clone())
        .unwrap_or_else(|| sim_model::target_profile(document, target_name, None));
    validate_simulation_profile(&scenario_profile)?;

    Ok(match cli_profile {
        Some(profile @ ("dev" | "checked" | "release")) => ProfileRoles {
            guarantee_profile: profile.to_owned(),
            backend_profile: scenario_profile,
        },
        Some(profile) if is_stable_simulation_profile(profile) => ProfileRoles {
            guarantee_profile: "checked".to_owned(),
            backend_profile: profile.to_owned(),
        },
        Some(profile) => {
            anyhow::bail!(
                "unsupported simulation profile `{profile}`; expected sync, async, stateful-input, failpoint, network, distributed, or guarantee profile dev, checked, release"
            );
        }
        None => ProfileRoles {
            guarantee_profile: "checked".to_owned(),
            backend_profile: scenario_profile,
        },
    })
}

fn validate_simulation_profile(profile: &str) -> anyhow::Result<()> {
    if is_stable_simulation_profile(profile) {
        return Ok(());
    }
    anyhow::bail!(
        "unsupported simulation profile `{profile}`; expected sync, async, stateful-input, failpoint, network, distributed"
    )
}

fn is_stable_simulation_profile(profile: &str) -> bool {
    matches!(
        profile,
        "sync" | "async" | "stateful-input" | "failpoint" | "network" | "distributed"
    )
}

fn default_budget(sim_profile: &str) -> Option<u64> {
    match sim_profile {
        "quick" => Some(64),
        "deep" => Some(1_000_000),
        "replay" => Some(64),
        "exhaustive" => Some(16),
        _ => None,
    }
}

fn default_seed_count(sim_profile: &str) -> Option<u64> {
    match sim_profile {
        "deep" => Some(DEFAULT_DEEP_SEED_COUNT),
        _ => None,
    }
}

fn default_shrink_mode(sim_profile: &str) -> &'static str {
    if sim_profile == "deep" {
        "best-effort"
    } else {
        "off"
    }
}
