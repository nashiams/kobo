mod check;
mod debt;
mod fmt;
mod init;
mod build;
mod perf;
mod run;
mod session;

use crate::{resolve_cli_mode, KoboCommand};

pub(crate) fn dispatch(command: KoboCommand) -> anyhow::Result<()> {
    match command {
        KoboCommand::Check { file, checked, strict } => {
            check::cmd_check(&file, resolve_cli_mode(checked, strict))
        }
        KoboCommand::Fmt { file } => fmt::cmd_fmt(&file),
        KoboCommand::Run { file, checked, strict } => {
            run::cmd_run(&file, resolve_cli_mode(checked, strict))
        }
        KoboCommand::Inspect { file, checked, strict } => {
            run::cmd_inspect(&file, resolve_cli_mode(checked, strict))
        }
        KoboCommand::Dump { file } => run::cmd_dump(&file),
        KoboCommand::Perf { file, from, threshold } => {
            perf::cmd_perf(&file, from.as_deref(), threshold)
        }
        KoboCommand::Debt { file, json, summary, borrows, watch } => {
            if watch {
                eprintln!("kobo debt --watch is planned for v0.5");
                return Ok(());
            }
            if borrows {
                return debt::cmd_debt_borrows(&file, json);
            }
            debt::cmd_debt(&file, json, summary)
        }
        KoboCommand::Init { name } => init::cmd_init(&name),
        KoboCommand::Build { checked, strict } => {
            build::cmd_build(resolve_cli_mode(checked, strict))
        }
    }
}