use std::path::{Path, PathBuf};

use anyhow::Context;
use kobo_driver::{load_config_for, CompileSession};
use kobo_errors::format_diagnostic;

pub(super) fn build_session(file: &Path) -> anyhow::Result<CompileSession> {
    let workspace_root = find_workspace_root(file)?;
    let crate_dir = find_crate_dir(file, &workspace_root);
    let config = load_config_for(&crate_dir, &workspace_root)
        .with_context(|| format!("failed to load config for {}", file.display()))?;

    Ok(CompileSession::new(config))
}

pub(super) fn render_diagnostics(session: &CompileSession) {
    for diagnostic in &session.diagnostics {
        eprintln!("{}", format_diagnostic(session.file_set(), diagnostic));
    }
}

pub(super) fn line_number_for_offset(source: &str, offset: u32) -> usize {
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
