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
    // v0.6 §3.3b: Render K-code warnings on success path [BUG-01 / R02].
    render_diagnostics(&session);
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

pub(super) fn cmd_inspect(
    file: &Path,
    cli_mode: Option<KoboMode>,
    clean: bool,
    cargo_dir: Option<&Path>,
) -> anyhow::Result<()> {
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

    let output = if clean || cargo_dir.is_some() {
        kobo_codegen::clean::strip_kobo_wrappers(&rs_source)
    } else {
        // Annotate lock acquisition order for inspect output.
        kobo_codegen::annotate_lock_order(&rs_source)
    };

    if let Some(dir) = cargo_dir {
        let kobo_toml_path = file.parent().unwrap_or(Path::new(".")).join("Kobo.toml");
        let config = if kobo_toml_path.exists() {
            let toml_str = std::fs::read_to_string(&kobo_toml_path)
                .with_context(|| format!("failed to read {}", kobo_toml_path.display()))?;
            kobo_codegen::cargo_gen::KoboProjectConfig::from_toml(&toml_str)
                .map_err(|e| anyhow::anyhow!("{e}"))?
        } else {
            kobo_codegen::cargo_gen::KoboProjectConfig::default()
        };

        let source_files = vec![(file.to_path_buf(), output.clone())];
        kobo_codegen::cargo_gen::generate_cargo_project(&config, &source_files, dir)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        eprintln!("cargo project generated at {}", dir.display());
    }

    // S-26: Show effective mode so user can verify per-module mode resolution.
    eprintln!("// effective mode: {}", session.mode());
    print!("{output}");
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
