mod check;
mod debt;
mod fmt;
mod perf;
mod run;
mod session;

use crate::KoboCommand;

pub(crate) fn dispatch(command: KoboCommand) -> anyhow::Result<()> {
    match command {
        KoboCommand::Check { file } => check::cmd_check(&file),
        KoboCommand::Fmt { file } => fmt::cmd_fmt(&file),
        KoboCommand::Run { file } => run::cmd_run(&file),
        KoboCommand::Inspect { file } => run::cmd_inspect(&file),
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