use std::path::Path;

use kobo_driver::run_check_pipeline;
use kobo_errors::DiagnosticLspPayload;
use kobo_ir::KoboMode;

use crate::ErrorFormat;

use super::session::build_session;

pub(super) fn cmd_lsp_diagnostics(
    file: &Path,
    format: ErrorFormat,
    _no_project_ok: bool,
) -> anyhow::Result<()> {
    let mut session = build_session(file, Some(KoboMode::Checked))?;
    let _ = run_check_pipeline(&mut session, file);

    match format {
        ErrorFormat::Json => {
            for diagnostic in session.visible_diagnostics() {
                let payload = DiagnosticLspPayload::from_diagnostic(session.file_set(), diagnostic);
                println!(
                    "{}",
                    serde_json::to_string(&payload)
                        .expect("LSP diagnostic payload should serialize")
                );
            }
            Ok(())
        }
        ErrorFormat::Human => {
            anyhow::bail!("lsp-diagnostics currently supports --format=json")
        }
    }
}
