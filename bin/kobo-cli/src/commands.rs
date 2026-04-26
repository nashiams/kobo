mod bench;
mod build;
mod check;
mod debt;
mod fmt;
mod init;
mod migrate;
mod perf;
mod run;
mod session;
mod watch;

use crate::{resolve_cli_mode, KoboCommand};

pub(crate) fn dispatch(command: KoboCommand) -> anyhow::Result<()> {
    match command {
        KoboCommand::Check {
            file,
            checked,
            strict,
            pipeline,
        } => check::cmd_check(&file, resolve_cli_mode(checked, strict), pipeline),
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
            cargo,
        } => run::cmd_inspect(
            &file,
            resolve_cli_mode(checked, strict),
            clean,
            erase_lifetimes,
            cargo.as_deref(),
        ),
        KoboCommand::Dump { file } => run::cmd_dump(&file),
        KoboCommand::Perf {
            file,
            from,
            threshold,
        } => perf::cmd_perf(&file, from.as_deref(), threshold),
        KoboCommand::Debt {
            file,
            json,
            summary,
            borrows,
            patterns,
            errors,
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
    }
}
