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
mod perf;
mod replay;
mod run;
mod session;
mod sim;
mod watch;

use crate::{resolve_cli_mode, KoboCommand, SimCommand};

pub(crate) fn dispatch(command: KoboCommand) -> anyhow::Result<()> {
    match command {
        KoboCommand::Check {
            file,
            checked,
            strict,
            pipeline,
            error_format,
            recover_parse,
            replay_critical,
            max_diagnostics,
            visible_region,
            include_budgeted,
        } => check::cmd_check(
            &file,
            resolve_cli_mode(checked, strict),
            pipeline,
            error_format,
            recover_parse,
            replay_critical,
            max_diagnostics,
            visible_region.as_deref(),
            include_budgeted,
        ),
        KoboCommand::LspDiagnostics {
            file,
            format,
            no_project_ok,
        } => lsp_diagnostics::cmd_lsp_diagnostics(&file, format, no_project_ok),
        KoboCommand::Fmt { file } => fmt::cmd_fmt(&file),
        KoboCommand::Run {
            file,
            checked,
            strict,
            erase_lifetimes,
        } => run::cmd_run(&file, resolve_cli_mode(checked, strict), erase_lifetimes),
        KoboCommand::Inspect {
            file,
            checked,
            strict,
            clean,
            erase_lifetimes,
            scenario_metadata,
            cargo,
            profile,
            trait_default,
        } => run::cmd_inspect(
            &file,
            resolve_cli_mode(checked, strict),
            clean,
            erase_lifetimes,
            scenario_metadata,
            cargo.as_deref(),
            profile.as_deref(),
            trait_default.as_deref(),
        ),
        KoboCommand::Doctor { deps, json } => doctor::cmd_doctor(deps, json),
        KoboCommand::Dump { file } => run::cmd_dump(&file),
        KoboCommand::Perf {
            file,
            from,
            threshold,
        } => perf::cmd_perf(&file, from.as_deref(), threshold),
        KoboCommand::Sim { command } => match command {
            SimCommand::Scout {
                file,
                json,
                backend_recommendations,
            } => sim::cmd_sim_scout(&file, json, backend_recommendations),
            SimCommand::Backends { json } => sim::cmd_sim_backends(json),
        },
        KoboCommand::Replay {
            file,
            roundtrip_metadata,
        } => replay::cmd_replay(&file, roundtrip_metadata),
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
        KoboCommand::Build { checked, strict } => {
            build::cmd_build(resolve_cli_mode(checked, strict))
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
        KoboCommand::Explain { code } => explain::cmd_explain(&code),
    }
}
