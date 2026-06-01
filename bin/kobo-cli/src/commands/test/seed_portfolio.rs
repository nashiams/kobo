use super::{EngineMode, FullDepthRun, ScenarioEvent};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub(super) struct SeedPortfolioPlan {
    pub target_seed_count: u64,
    pub wall_clock_limit: Option<Duration>,
}

impl SeedPortfolioPlan {
    pub(super) fn with_target_seed_count(target_seed_count: u64) -> Self {
        Self {
            target_seed_count,
            wall_clock_limit: None,
        }
    }

    pub(super) fn with_wall_clock_limit(mut self, wall_clock_limit: Duration) -> Self {
        self.wall_clock_limit = Some(wall_clock_limit);
        self
    }

    fn has_expired(self, started_at: Instant) -> bool {
        self.wall_clock_limit
            .is_some_and(|wall_clock_limit| started_at.elapsed() >= wall_clock_limit)
    }
}

pub(super) fn run_seed_portfolio(
    scenario_program: &kobo_ir::ScenarioProgram,
    generated_rust: &str,
    options: &kobo_sim_core::ScenarioOptions,
    engine: EngineMode,
    plan: SeedPortfolioPlan,
) -> anyhow::Result<(FullDepthRun, u64)> {
    if plan.target_seed_count <= 1 {
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
            plan,
        );
    }

    let mut combined_events = Vec::new();
    let mut last_seed = options.seed;
    let mut last_run = None;
    let started_at = Instant::now();
    for index in 0..plan.target_seed_count {
        if index > 0 && plan.has_expired(started_at) {
            combined_events.push(seed_portfolio_cap_event(
                index,
                plan.target_seed_count,
                started_at.elapsed(),
            ));
            break;
        }
        let seed = options.seed.wrapping_add(index);
        combined_events.push(seed_case_event(index, seed, plan.target_seed_count));
        let exploration = run_seed_case(
            scenario_program,
            generated_rust,
            options,
            seed,
            EngineMode::SemanticOnly,
        )?;
        if exploration.failure.is_some() || index + 1 == plan.target_seed_count {
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
        last_run = Some(exploration);
        last_seed = seed;
    }

    if let Some(mut selected_run) = last_run {
        selected_run.events = combined_events;
        refresh_seed_portfolio_digest(&mut selected_run);
        return Ok((selected_run, last_seed));
    }

    anyhow::bail!(
        "seed_count {} did not execute any seed after {last_seed}",
        plan.target_seed_count
    )
}

fn run_selected_seed_from_portfolio(
    scenario_program: &kobo_ir::ScenarioProgram,
    generated_rust: &str,
    options: &kobo_sim_core::ScenarioOptions,
    engine: EngineMode,
    plan: SeedPortfolioPlan,
) -> anyhow::Result<(FullDepthRun, u64)> {
    let mut combined_events = Vec::new();
    let mut selected_seed = options.seed;
    let started_at = Instant::now();
    for index in 0..plan.target_seed_count {
        if index > 0 && plan.has_expired(started_at) {
            combined_events.push(seed_portfolio_cap_event(
                index,
                plan.target_seed_count,
                started_at.elapsed(),
            ));
            break;
        }
        let seed = options.seed.wrapping_add(index);
        combined_events.push(seed_case_event(index, seed, plan.target_seed_count));
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
    let mut selected_run = run_seed_case(
        scenario_program,
        generated_rust,
        options,
        selected_seed,
        engine,
    )?;
    combined_events.extend(selected_run.events.iter().cloned());
    selected_run.events = combined_events;
    refresh_seed_portfolio_digest(&mut selected_run);
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

fn seed_portfolio_cap_event(
    executed_count: u64,
    seed_count: u64,
    elapsed: Duration,
) -> ScenarioEvent {
    ScenarioEvent {
        kind: "scheduler-seed-portfolio-cap".to_owned(),
        label: Some(format!(
            "reason=wall-clock;executed={executed_count};count={seed_count}"
        )),
        value: Some(elapsed.as_millis().try_into().unwrap_or(u64::MAX)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_cmd::json_schema;
    use kobo_ir::{FileId, KoboSpan, ScenarioCoverageFacts, ScenarioOp, ScenarioOpKind};

    #[test]
    fn semantic_only_returns_capped_portfolio_evidence() {
        let program = kobo_ir::ScenarioProgram {
            file_id: FileId(0),
            target: "capped".to_owned(),
            source_hash: "hash".to_owned(),
            operations: vec![ScenarioOp {
                span: KoboSpan::generated(FileId(0)),
                kind: ScenarioOpKind::Return,
            }],
            boundaries: Vec::new(),
            coverage: ScenarioCoverageFacts::default(),
        };
        let options = kobo_sim_core::ScenarioOptions {
            seed: 41,
            ..kobo_sim_core::ScenarioOptions::default()
        };

        let (run, executed_seed) = run_seed_portfolio(
            &program,
            "",
            &options,
            EngineMode::SemanticOnly,
            SeedPortfolioPlan::with_target_seed_count(3)
                .with_wall_clock_limit(Duration::from_millis(0)),
        )
        .expect("semantic-only capped portfolio should return evidence");

        assert_eq!(executed_seed, 41);
        assert!(
            run.events
                .iter()
                .any(|event| event.kind == "scheduler-seed-case" && event.value == Some(41)),
            "first seed should still be represented"
        );
        assert!(
            run.events
                .iter()
                .any(|event| event.kind == "scheduler-seed-portfolio-cap"),
            "semantic-only cap should be observable in events"
        );
        let scheduler = json_schema::scheduler_json("deep", executed_seed, &run, None);
        assert_eq!(scheduler["seed_count"], 1);
        assert_eq!(scheduler["configured_seed_count"], 3);
        assert_eq!(scheduler["portfolio_complete"], false);
        assert_eq!(scheduler["portfolio_cap"]["reason"], "wall-clock");
        assert_eq!(scheduler["portfolio_cap"]["executed_seed_count"], 1);
        assert_eq!(scheduler["portfolio_cap"]["configured_seed_count"], 3);
        assert_eq!(run.digest.agreement, "semantic-only");
    }
}
