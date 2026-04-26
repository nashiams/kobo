use std::path::Path;

use kobo_driver::{run_check_pipeline, run_pipeline_ordering_check};
use kobo_ir::KoboMode;

use super::session::{build_session, render_diagnostics};

pub(super) fn cmd_check(file: &Path, cli_mode: Option<KoboMode>, pipeline: bool) -> anyhow::Result<()> {
    let mut session = build_session(file, cli_mode)?;
    if session.mode() == KoboMode::Strict {
        eprintln!("error: --strict mode is not yet implemented (target: v0.9)");
        eprintln!("hint: use --checked for advisory ownership warnings");
        std::process::exit(1);
    }

    if pipeline {
        eprintln!("[kobo] --pipeline: running full solver pipeline diagnostics");
    }

    match run_check_pipeline(&mut session, file) {
        Ok(()) => {
            render_diagnostics(&session);

            if pipeline {
                // S-14: Run middleware ordering heuristic.
                let warnings = run_pipeline_ordering_check(&mut session, file);
                if warnings.is_empty() {
                    eprintln!("[kobo] pipeline: no ordering issues detected");
                } else {
                    for w in &warnings {
                        eprintln!(
                            "[kobo] pipeline {}: {}",
                            w.kind.code(),
                            w.suggestion,
                        );
                    }
                }
                eprintln!("[kobo] pipeline: {} diagnostic(s) emitted", session.diagnostics.len());
            }

            Ok(())
        }
        Err(()) => {
            render_diagnostics(&session);
            anyhow::bail!("analysis failed");
        }
    }
}
