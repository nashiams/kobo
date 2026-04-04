use std::path::Path;

use kobo_driver::run_check_pipeline;

use super::session::{build_session, render_diagnostics};

pub(super) fn cmd_check(file: &Path) -> anyhow::Result<()> {
    let mut session = build_session(file)?;

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
