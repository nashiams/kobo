use std::path::{Path, PathBuf};

use anyhow::Context;
use kobo_driver::{load_config_for, CompileSession};
use kobo_errors::{
    diagnostic_to_json_value, ColorMode, DiagnosticOutputFormat, DiagnosticRenderer, Severity,
};
use kobo_ir::KoboMode;

use crate::ErrorFormat;

/// Build a CompileSession for the given file, applying an optional CLI mode
/// override BEFORE constructing the session [Contract R06, Trap 5].
///
/// CLI mode overrides Kobo.toml. `config.mode` is finalized before
/// `CompileSession::new()` so `diag_enabled` is computed from the correct mode.
pub(super) fn build_session(
    file: &Path,
    cli_mode: Option<KoboMode>,
) -> anyhow::Result<CompileSession> {
    let workspace_root = find_workspace_root(file)?;
    let crate_dir = find_crate_dir(file, &workspace_root);
    let mut config = load_config_for(&crate_dir, &workspace_root)
        .with_context(|| format!("failed to load config for {}", file.display()))?;
    if let Some(mode) = cli_mode {
        config.mode = mode; // CLI overrides Kobo.toml [Trap 5]
    }
    let mut session = CompileSession::new(config);
    session.cli_mode_override = cli_mode.is_some();
    Ok(session)
}

pub(super) fn render_diagnostics(session: &CompileSession) {
    render_diagnostics_with_format(session, ErrorFormat::Human);
}

pub(super) fn render_diagnostics_with_format(session: &CompileSession, format: ErrorFormat) {
    let color = if std::env::var_os("NO_COLOR").is_some() {
        ColorMode::Never
    } else {
        ColorMode::Auto
    };
    let renderer = DiagnosticRenderer::new(color, DiagnosticOutputFormat::HumanCard);

    for diagnostic in session.visible_diagnostics() {
        // G5: suppress Severity::Warning diagnostics inside #[kobo::relax] ranges in checked mode.
        if diagnostic.severity == Severity::Warning
            && session.mode().is_checked()
            && kobo_driver::is_inside_relaxed_fn(
                diagnostic.primary.span,
                &session.relaxed_fn_ranges,
            )
        {
            continue;
        }

        match format {
            ErrorFormat::Human => eprintln!("{}", renderer.render(session.file_set(), diagnostic)),
            ErrorFormat::Json => {
                let value = diagnostic_to_json_value(session.file_set(), diagnostic);
                println!(
                    "{}",
                    serde_json::to_string(&value).expect("diagnostic JSON value should serialize")
                );
            }
        }
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

    if let Some(workspace_root) = workspace_root {
        return Ok(workspace_root);
    }

    let current_dir = std::env::current_dir().context("failed to determine current directory")?;
    for ancestor in current_dir.ancestors() {
        if has_workspace_marker(ancestor) {
            return Ok(ancestor.to_path_buf());
        }
    }

    Ok(input_directory(file)?)
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
