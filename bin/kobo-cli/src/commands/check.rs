use std::path::Path;

use kobo_driver::{run_check_pipeline, run_pipeline_ordering_check};
use kobo_ir::KoboMode;

use crate::ErrorFormat;

use super::session::{build_session, render_diagnostics_with_format};

pub(super) fn cmd_check(
    file: &Path,
    cli_mode: Option<KoboMode>,
    pipeline: bool,
    error_format: ErrorFormat,
    recover_parse: bool,
) -> anyhow::Result<()> {
    let mut session = build_session(file, cli_mode)?;
    session.config.enable_parse_recovery = recover_parse;
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
            render_diagnostics_with_format(&session, error_format);

            if pipeline {
                // S-14: Run middleware ordering heuristic.
                let warnings = run_pipeline_ordering_check(&mut session, file);
                if warnings.is_empty() {
                    eprintln!("[kobo] pipeline: no ordering issues detected");
                } else {
                    for w in &warnings {
                        eprintln!("[kobo] pipeline {}: {}", w.kind.code(), w.suggestion,);
                    }
                }
                eprintln!(
                    "[kobo] pipeline: {} diagnostic(s) emitted",
                    session.diagnostics.len()
                );
            }

            Ok(())
        }
        Err(()) => {
            render_diagnostics_with_format(&session, error_format);
            anyhow::bail!("analysis failed");
        }
    }
}
