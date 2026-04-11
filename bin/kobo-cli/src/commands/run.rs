use std::path::Path;
use std::process::Command;

use anyhow::Context;
use kobo_driver::{run_and_compile, run_codegen_pipeline, run_kir_phase, CodegenArtifacts};
use kobo_ir::KoboMode;

use super::session::{build_session, line_number_for_offset, render_diagnostics};

pub(super) fn cmd_run(file: &Path, cli_mode: Option<KoboMode>) -> anyhow::Result<()> {
    let mut session = build_session(file, cli_mode)?;
    if session.mode() == KoboMode::Strict {
        eprintln!("error: --strict mode is not yet implemented (target: v0.9)");
        eprintln!("hint: use --checked for advisory ownership warnings");
        std::process::exit(1);
    }

    let binary_path = run_and_compile(&mut session, file).map_err(|()| {
        render_diagnostics(&session);
        anyhow::anyhow!("compilation failed")
    })?;
    if !binary_path.is_file() {
        anyhow::bail!("compiled binary missing at {}", binary_path.display());
    }

    let mut run_cmd = Command::new(&binary_path);
    // Runtime layer [G1 §1.4 / R6-04]: in checked mode DiagOwner output must
    // appear without requiring the user to set KOBO_DIAG=1 manually.
    // We propagate KOBO_CHECKED_MODE=1 to the child so kobo-diag::DiagOwner
    // knows to emit on Drop even without the manual opt-in env var.
    if session.mode().is_checked() {
        run_cmd.env("KOBO_CHECKED_MODE", "1");
    }
    let run_status = run_cmd
        .status()
        .with_context(|| format!("failed to run {}", binary_path.display()))?;

    if !run_status.success() {
        anyhow::bail!("program exited with non-zero status");
    }

    Ok(())
}

pub(super) fn cmd_inspect(file: &Path, cli_mode: Option<KoboMode>) -> anyhow::Result<()> {
    let mut session = build_session(file, cli_mode)?;
    if session.mode() == KoboMode::Strict {
        eprintln!("error: --strict mode is not yet implemented (target: v0.9)");
        eprintln!("hint: use --checked for advisory ownership warnings");
        std::process::exit(1);
    }

    let CodegenArtifacts { rs_source, .. } =
        run_codegen_pipeline(&mut session, file).map_err(|()| {
            render_diagnostics(&session);
            anyhow::anyhow!("compilation failed")
        })?;
    // v0.6 §3.3b: Render K-code warnings on success path too [R6-06].
    render_diagnostics(&session);

    print!("{rs_source}");
    Ok(())
}

pub(super) fn cmd_dump(file: &Path) -> anyhow::Result<()> {
    let mut session = build_session(file, None)?;
    let (_, kir) = run_kir_phase(&mut session, file)
        .map_err(|()| anyhow::anyhow!("failed to build KIR for {}", file.display()))?;

    for node in kir.iter_nodes() {
        let entry = session
            .file_set()
            .get(node.span.file_id)
            .context("missing file entry for KIR node")?;
        let line = line_number_for_offset(&entry.source, node.span.start);
        let ast_id = node
            .ast_id
            .map(|id| id.0.to_string())
            .unwrap_or_else(|| "-".to_owned());

        println!(
            "[{}] kind={:?} span={}:{} tier={:?} ast={}",
            node.id.0,
            node.kind,
            entry.path.display(),
            line,
            node.ownership,
            ast_id,
        );
    }

    Ok(())
}
