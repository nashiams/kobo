mod bench;
mod build;
mod check;
mod debt;
mod doctor;
mod explain;
mod fix;
mod fmt;
mod init;
mod lsp_diagnostics;
mod migrate;
mod ownership_analysis;
mod perf;
mod policy;
mod replay;
mod run;
mod session;
mod sim;
mod sim_model;
mod test_cmd;
mod watch;

use std::fmt as std_fmt;

use crate::{compiler_compatibility_mode, resolve_guarantee_profile, KoboCommand, SimCommand};

#[derive(Debug)]
pub(crate) struct DiagnosticExit;

impl std_fmt::Display for DiagnosticExit {
    fn fmt(&self, formatter: &mut std_fmt::Formatter<'_>) -> std_fmt::Result {
        formatter.write_str("diagnostics emitted")
    }
}

impl std::error::Error for DiagnosticExit {}

pub(crate) fn diagnostics_emitted() -> anyhow::Error {
    DiagnosticExit.into()
}

pub(crate) fn is_diagnostic_exit(error: &anyhow::Error) -> bool {
    error.downcast_ref::<DiagnosticExit>().is_some()
}

pub(crate) fn dispatch(command: KoboCommand) -> anyhow::Result<()> {
    match command {
        KoboCommand::Check {
            file,
            checked,
            strict,
            profile,
            print_policy,
            pipeline,
            error_format,
            color,
            recover_parse,
            replay_critical,
            max_diagnostics,
            visible_region,
            include_budgeted,
        } => {
            let guarantee_profile = resolve_guarantee_profile(checked, strict, profile);
            check::cmd_check(
                &file,
                compiler_compatibility_mode(guarantee_profile),
                guarantee_profile,
                print_policy,
                pipeline,
                error_format,
                color.color_mode(),
                recover_parse,
                replay_critical,
                max_diagnostics,
                visible_region.as_deref(),
                include_budgeted,
            )
        }
        KoboCommand::LspDiagnostics {
            file,
            format,
            no_project_ok,
            include_actions,
        } => lsp_diagnostics::cmd_lsp_diagnostics(&file, format, no_project_ok, include_actions),
        KoboCommand::Fmt { file } => fmt::cmd_fmt(&file),
        KoboCommand::Run {
            file,
            checked,
            strict,
            profile,
            erase_lifetimes,
        } => {
            let guarantee_profile = resolve_guarantee_profile(checked, strict, profile);
            run::cmd_run(
                &file,
                compiler_compatibility_mode(guarantee_profile),
                guarantee_profile,
                erase_lifetimes,
            )
        }
        KoboCommand::Inspect {
            file,
            checked,
            strict,
            clean,
            erase_lifetimes,
            scenario_metadata,
            sim,
            harness,
            cargo,
            profile,
            trait_default,
            audit,
        } => run::cmd_inspect(
            &file,
            compiler_compatibility_mode(resolve_guarantee_profile(checked, strict, None)),
            clean,
            erase_lifetimes,
            scenario_metadata,
            sim,
            harness,
            cargo.as_deref(),
            profile.as_deref(),
            trait_default.as_deref(),
            audit.as_deref(),
        ),
        KoboCommand::Doctor {
            deps,
            self_host,
            json,
        } => {
            let output_format = if json {
                doctor::DoctorOutputFormat::Json
            } else {
                doctor::DoctorOutputFormat::Human
            };
            let dependency_inspection = if deps {
                doctor::DependencyInspection::Requested
            } else {
                doctor::DependencyInspection::Default
            };
            let mode = if self_host {
                doctor::DoctorMode::SelfHost
            } else {
                doctor::DoctorMode::Dependencies(dependency_inspection)
            };
            doctor::cmd_doctor(doctor::DoctorOptions {
                mode,
                output_format,
            })
        }
        KoboCommand::Dump { file } => run::cmd_dump(&file),
        KoboCommand::Perf {
            file,
            from,
            threshold,
            format,
            evidence,
            session,
        } => perf::cmd_perf(
            &file,
            from.as_deref(),
            threshold,
            format.as_deref(),
            &evidence,
            session.as_deref(),
        ),
        KoboCommand::Sim { command } => match command {
            SimCommand::Init {
                target,
                minimal,
                profile,
            } => sim::cmd_sim_init(&target, minimal, profile.as_deref()),
            SimCommand::Scout {
                file,
                json,
                why,
                backend_recommendations,
            } => sim::cmd_sim_scout(&file, json, why, backend_recommendations),
            SimCommand::Backends { json } => sim::cmd_sim_backends(json),
        },
        KoboCommand::Test {
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
            file,
        } => test_cmd::cmd_test(
            &file,
            sim.as_deref(),
            profile.as_deref(),
            seed,
            events.as_deref(),
            inject.as_deref(),
            fuzz,
            event_budget,
            witness_dir.as_deref(),
            error_format,
            target.as_deref(),
            engine.as_deref(),
        ),
        KoboCommand::Replay {
            file,
            error_format,
            roundtrip_metadata,
        } => replay::cmd_replay(&file, error_format, roundtrip_metadata),
        KoboCommand::Fix {
            file,
            dry_run,
            apply,
            json,
        } => fix::cmd_fix(&file, dry_run, apply, json),
        KoboCommand::Debt {
            file,
            json,
            summary,
            borrows,
            patterns,
            errors,
            liveness,
            watch,
        } => {
            if watch {
                eprintln!("kobo debt --watch is planned for v0.5");
                return Ok(());
            }
            if borrows {
                return debt::cmd_debt_borrows(&file, json);
            }
            if patterns {
                return debt::cmd_debt_patterns(&file, json);
            }
            if errors {
                return debt::cmd_debt_errors(&file, json);
            }
            if liveness {
                return debt::cmd_debt_liveness(&file, json);
            }
            debt::cmd_debt(&file, json, summary)
        }
        KoboCommand::Init { name } => init::cmd_init(&name),
        KoboCommand::Build {
            checked,
            strict,
            profile,
            print_policy,
            error_format,
            color,
            emit_rust,
            file,
        } => {
            let guarantee_profile = resolve_guarantee_profile(checked, strict, profile);
            build::cmd_build(
                compiler_compatibility_mode(guarantee_profile),
                guarantee_profile,
                print_policy,
                error_format,
                color.color_mode(),
                emit_rust,
                file.as_deref(),
            )
        }
        KoboCommand::Migrate {
            file,
            dry_run,
            apply,
            graph,
            review,
            class_view,
            explain,
            budget,
            root,
            actor,
        } => migrate::cmd_migrate(
            &file,
            dry_run,
            apply,
            graph,
            review,
            class_view,
            explain,
            budget,
            root.as_deref(),
            actor.as_deref(),
        ),
        KoboCommand::Bench { file, tick_budget } => bench::cmd_bench_tick(&file, tick_budget),
        KoboCommand::Watch {
            file,
            simple,
            build,
        } => watch::cmd_watch(&file, simple, build),
        KoboCommand::Explain { code, verbose } => explain::cmd_explain(&code, verbose),
    }
}
