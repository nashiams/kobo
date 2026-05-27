use super::*;

pub(super) fn run_seed_portfolio(
    scenario_program: &kobo_ir::ScenarioProgram,
    generated_rust: &str,
    options: &kobo_sim_core::ScenarioOptions,
    engine: EngineMode,
    seed_count: u64,
) -> anyhow::Result<(FullDepthRun, u64)> {
    if seed_count <= 1 {
        let run = kobo_sim_core::run_full_depth_from_program(
            scenario_program,
            generated_rust,
            options,
            engine,
        )?;
        return Ok((run, options.seed));
    }
    if engine != EngineMode::SemanticOnly {
        return run_selected_seed_from_portfolio(
            scenario_program,
            generated_rust,
            options,
            engine,
            seed_count,
        );
    }

    let mut combined_events = Vec::new();
    let mut last_seed = options.seed;
    for index in 0..seed_count {
        let seed = options.seed.wrapping_add(index);
        combined_events.push(seed_case_event(index, seed, seed_count));
        let exploration = run_seed_case(
            scenario_program,
            generated_rust,
            options,
            seed,
            EngineMode::SemanticOnly,
        )?;
        if exploration.failure.is_some() || index + 1 == seed_count {
            let mut selected_run = run_selected_seed_case(
                scenario_program,
                generated_rust,
                options,
                seed,
                &engine,
                exploration,
            )?;
            combined_events.extend(selected_run.events.iter().cloned());
            selected_run.events = combined_events;
            refresh_seed_portfolio_digest(&mut selected_run);
            return Ok((selected_run, seed));
        }
        combined_events.extend(exploration.events.iter().cloned());
        last_seed = seed;
    }

    anyhow::bail!("seed_count {seed_count} did not execute any seed after {last_seed}")
}

fn run_selected_seed_from_portfolio(
    scenario_program: &kobo_ir::ScenarioProgram,
    generated_rust: &str,
    options: &kobo_sim_core::ScenarioOptions,
    engine: EngineMode,
    seed_count: u64,
) -> anyhow::Result<(FullDepthRun, u64)> {
    let mut selected_seed = options.seed;
    for index in 0..seed_count {
        let seed = options.seed.wrapping_add(index);
        let exploration = run_seed_case(
            scenario_program,
            generated_rust,
            options,
            seed,
            EngineMode::SemanticOnly,
        )?;
        selected_seed = seed;
        if exploration.failure.is_some() {
            break;
        }
    }
    let selected_run = run_seed_case(
        scenario_program,
        generated_rust,
        options,
        selected_seed,
        engine,
    )?;
    Ok((selected_run, selected_seed))
}

fn run_seed_case(
    scenario_program: &kobo_ir::ScenarioProgram,
    generated_rust: &str,
    options: &kobo_sim_core::ScenarioOptions,
    seed: u64,
    engine: EngineMode,
) -> anyhow::Result<FullDepthRun> {
    let mut case_options = options.clone();
    case_options.seed = seed;
    Ok(kobo_sim_core::run_full_depth_from_program(
        scenario_program,
        generated_rust,
        &case_options,
        engine,
    )?)
}

fn run_selected_seed_case(
    scenario_program: &kobo_ir::ScenarioProgram,
    generated_rust: &str,
    options: &kobo_sim_core::ScenarioOptions,
    seed: u64,
    engine: &EngineMode,
    exploration: FullDepthRun,
) -> anyhow::Result<FullDepthRun> {
    if *engine == EngineMode::SemanticOnly {
        return Ok(exploration);
    }
    run_seed_case(
        scenario_program,
        generated_rust,
        options,
        seed,
        engine.clone(),
    )
}

fn seed_case_event(index: u64, seed: u64, seed_count: u64) -> ScenarioEvent {
    ScenarioEvent {
        kind: "scheduler-seed-case".to_owned(),
        label: Some(format!("index={index};count={seed_count}")),
        value: Some(seed),
        io: None,
    }
}

fn refresh_seed_portfolio_digest(run: &mut FullDepthRun) {
    run.digest.semantic_trace_hash = kobo_sim_core::digest::events_hash(&run.events);
    if !run.digest.harness_trace_hash.is_empty() {
        run.digest.harness_trace_hash = kobo_sim_core::digest::stable_hash(&format!(
            "seed-portfolio:{}:{}",
            run.digest.harness_trace_hash, run.digest.semantic_trace_hash
        ));
    }
    if run.digest.agreement == "matched" {
        run.digest.agreement = "matched+seed-portfolio".to_owned();
    }
}
