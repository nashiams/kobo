use std::path::Path;

use kobo_driver::run_check_pipeline;
use kobo_ir::KoboMode;

use super::session::{build_session, render_diagnostics};

pub(super) fn cmd_check(file: &Path, cli_mode: Option<KoboMode>) -> anyhow::Result<()> {
    let mut session = build_session(file, cli_mode)?;
    if session.mode() == KoboMode::Strict {
        eprintln!("error: --strict mode is not yet implemented (target: v0.9)");
        eprintln!("hint: use --checked for advisory ownership warnings");
        std::process::exit(1);
    }

    match run_check_pipeline(&mut session, file) {
        Ok(()) => {
            render_diagnostics(&session);
            Ok(())
        }
        Err(()) => {
            render_diagnostics(&session);
            anyhow::bail!("analysis failed");
        }
    }
}
