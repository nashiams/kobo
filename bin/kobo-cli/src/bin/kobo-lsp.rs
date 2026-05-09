use std::path::{Path, PathBuf};

use anyhow::Context;
use kobo_driver::{load_config_for, run_check_pipeline, CompileSession};
use kobo_errors::DiagnosticLspPayload;
use kobo_ir::KoboMode;

fn main() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("kobo-lsp 0.8.5");
        return Ok(());
    }

    if args.iter().any(|arg| arg == "--stdio") {
        println!(
            "{}",
            serde_json::json!({
                "server": "kobo-lsp",
                "schema_version": 1,
                "transport": "stdio",
                "capabilities": {
                    "textDocumentSync": 1,
                    "publishDiagnostics": true,
                    "codeDescription": true
                }
            })
        );
        return Ok(());
    }

    let Some(file) = diagnostics_file_arg(&args) else {
        eprintln!("usage: kobo-lsp --stdio | --version | --diagnostics FILE");
        std::process::exit(2);
    };

    publish_diagnostics(&file)
}

fn diagnostics_file_arg(args: &[String]) -> Option<PathBuf> {
    for (index, arg) in args.iter().enumerate() {
        if arg == "--diagnostics" {
            return args.get(index + 1).map(PathBuf::from);
        }
    }
    args.iter()
        .find(|arg| !arg.starts_with('-'))
        .map(PathBuf::from)
}

fn publish_diagnostics(file: &Path) -> anyhow::Result<()> {
    let mut session = build_lsp_session(file)?;
    let _ = run_check_pipeline(&mut session, file);

    for diagnostic in session.visible_diagnostics() {
        let payload = DiagnosticLspPayload::from_diagnostic(session.file_set(), diagnostic);
        println!(
            "{}",
            serde_json::to_string(&payload).expect("LSP diagnostic payload should serialize")
        );
    }
    Ok(())
}

fn build_lsp_session(file: &Path) -> anyhow::Result<CompileSession> {
    let workspace_root = find_workspace_root(file)?;
    let crate_dir = find_crate_dir(file, &workspace_root);
    let mut config = load_config_for(&crate_dir, &workspace_root)
        .with_context(|| format!("failed to load config for {}", file.display()))?;
    config.mode = KoboMode::Checked;
    let mut session = CompileSession::new(config);
    session.cli_mode_override = true;
    Ok(session)
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

    input_directory(file)
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
