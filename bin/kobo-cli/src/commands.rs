use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;

use kobo_driver::{load_config_for, output_path_for, run_kir_phase, run_pipeline, CompileSession};
use kobo_errors::format_diagnostic;

use crate::KoboCommand;

pub(crate) fn dispatch(command: KoboCommand) -> anyhow::Result<()> {
    match command {
        KoboCommand::Run { file } => cmd_run(&file),
        KoboCommand::Inspect { file } => cmd_inspect(&file),
        KoboCommand::Dump { file } => cmd_dump(&file),
    }
}

fn cmd_run(file: &Path) -> anyhow::Result<()> {
    let mut session = build_session(file)?;

    run_pipeline(&mut session, file).map_err(|()| {
        for diagnostic in &session.diagnostics {
            eprintln!("{}", format_diagnostic(diagnostic));
        }
        anyhow::anyhow!("compilation failed")
    })?;

    let rs_path = output_path_for(file, &session.config);
    let binary_path = executable_path(&rs_path)?;
    let rustc_status = Command::new("rustc")
        .arg("--edition=2021")
        .arg(&rs_path)
        .arg("-o")
        .arg(&binary_path)
        .status()
        .context("failed to invoke rustc")?;

    if !rustc_status.success() {
        anyhow::bail!("rustc compilation failed");
    }

    if !binary_path.is_file() {
        anyhow::bail!("compiled binary missing at {}", binary_path.display());
    }

    let run_status = Command::new(&binary_path)
        .status()
        .with_context(|| format!("failed to run {}", binary_path.display()))?;

    if !run_status.success() {
        anyhow::bail!("program exited with non-zero status");
    }

    Ok(())
}

fn cmd_inspect(file: &Path) -> anyhow::Result<()> {
    let mut session = build_session(file)?;

    let rs_string = run_pipeline(&mut session, file).map_err(|()| {
        for diagnostic in &session.diagnostics {
            eprintln!("{}", format_diagnostic(diagnostic));
        }
        anyhow::anyhow!("compilation failed")
    })?;

    print!("{rs_string}");
    Ok(())
}

fn cmd_dump(file: &Path) -> anyhow::Result<()> {
    let mut session = build_session(file)?;
    let (_, kir) = run_kir_phase(&mut session, file)
        .map_err(|()| anyhow::anyhow!("failed to build KIR for {}", file.display()))?;

    for node in kir.iter_nodes() {
        let entry = session
            .file_set
            .get(node.span.file_id)
            .context("missing file entry for KIR node")?;
        let line = line_number_for_offset(&entry.source, node.span.start);

        println!(
            "[{}] span={}:{} tier={:?} ast={}",
            node.id.0,
            entry.path.display(),
            line,
            node.ownership,
            node.ast_id.0,
        );
    }

    Ok(())
}

fn build_session(file: &Path) -> anyhow::Result<CompileSession> {
    let workspace_root = find_workspace_root(file)?;
    let crate_dir = find_crate_dir(file, &workspace_root);
    let config = load_config_for(&crate_dir, &workspace_root)
        .with_context(|| format!("failed to load config for {}", file.display()))?;

    Ok(CompileSession::new(config))
}

fn executable_path(rs_path: &Path) -> anyhow::Result<PathBuf> {
    let binary_path = rs_path.with_extension(std::env::consts::EXE_EXTENSION);
    if binary_path.is_absolute() {
        return Ok(binary_path);
    }

    Ok(std::env::current_dir()
        .context("failed to determine current directory")?
        .join(binary_path))
}

fn line_number_for_offset(source: &str, offset: u32) -> usize {
    let clamped = (offset as usize).min(source.len());
    source[..clamped]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn find_workspace_root(file: &Path) -> anyhow::Result<PathBuf> {
    let search_root = input_directory(file)?;

    let mut workspace_root = None;
    for ancestor in search_root.ancestors() {
        if has_workspace_marker(ancestor) {
            workspace_root = Some(ancestor.to_path_buf());
        }
    }

    workspace_root
        .ok_or_else(|| anyhow::anyhow!("could not find workspace root for {}", file.display()))
}

fn find_crate_dir(file: &Path, workspace_root: &Path) -> PathBuf {
    let Ok(search_root) = input_directory(file) else {
        return workspace_root.to_path_buf();
    };

    for ancestor in search_root.ancestors() {
        if ancestor == workspace_root {
            break;
        }

        if has_workspace_marker(ancestor) {
            return ancestor.to_path_buf();
        }
    }

    workspace_root.to_path_buf()
}

fn has_workspace_marker(path: &Path) -> bool {
    path.join("Cargo.toml").is_file() || path.join("Kobo.toml").is_file()
}

fn input_directory(file: &Path) -> anyhow::Result<PathBuf> {
    let absolute_file = if file.is_absolute() {
        file.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to determine current directory")?
            .join(file)
    };

    Ok(absolute_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or(absolute_file))
}
