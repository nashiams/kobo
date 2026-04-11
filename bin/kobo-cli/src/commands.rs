mod check;
mod debt;
mod fmt;
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
        KoboCommand::Debt { file, json, summary, watch } => {
            if watch {
                eprintln!("kobo debt --watch is planned for v0.5");
                return Ok(());
            }
            debt::cmd_debt(&file, json, summary)
        }
    }
}