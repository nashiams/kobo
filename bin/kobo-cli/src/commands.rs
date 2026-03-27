use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;

use kobo_driver::{
    load_config_for, run_and_compile, run_check_pipeline, run_codegen_pipeline, run_kir_phase,
    CodegenArtifacts, CompileSession,
};
use kobo_errors::format_diagnostic;

use crate::KoboCommand;

pub(crate) fn dispatch(command: KoboCommand) -> anyhow::Result<()> {
    match command {
        KoboCommand::Check { file } => cmd_check(&file),
        KoboCommand::Fmt { file } => cmd_fmt(&file),
        KoboCommand::Run { file } => cmd_run(&file),
        KoboCommand::Inspect { file } => cmd_inspect(&file),
        KoboCommand::Dump { file } => cmd_dump(&file),
    }
}

fn cmd_check(file: &Path) -> anyhow::Result<()> {
    let mut session = build_session(file)?;

    match run_check_pipeline(&mut session, file) {
        Ok(()) => Ok(()),
        Err(()) => {
            render_diagnostics(&session);
            anyhow::bail!("analysis failed");
        }
    }
}

fn cmd_run(file: &Path) -> anyhow::Result<()> {
    let mut session = build_session(file)?;

    let binary_path = run_and_compile(&mut session, file).map_err(|()| {
        render_diagnostics(&session);
        anyhow::anyhow!("compilation failed")
    })?;
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

    let CodegenArtifacts { rs_source, .. } = run_codegen_pipeline(&mut session, file).map_err(|()| {
        render_diagnostics(&session);
        anyhow::anyhow!("compilation failed")
    })?;

    print!("{rs_source}");
    Ok(())
}

fn cmd_fmt(file: &Path) -> anyhow::Result<()> {
    let mut session = build_session(file)?;
    let artifacts = run_codegen_pipeline(&mut session, file).map_err(|()| {
        render_diagnostics(&session);
        anyhow::anyhow!("compilation failed")
    })?;
    run_rustfmt_file(&artifacts.rs_path)?;

    let formatted_source = rewrite_lossless_kobo_source(file, &artifacts)?;
    fs::write(file, formatted_source)
        .with_context(|| format!("failed to write {}", file.display()))?;

    Ok(())
}

fn cmd_dump(file: &Path) -> anyhow::Result<()> {
    let mut session = build_session(file)?;
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

fn build_session(file: &Path) -> anyhow::Result<CompileSession> {
    let workspace_root = find_workspace_root(file)?;
    let crate_dir = find_crate_dir(file, &workspace_root);
    let config = load_config_for(&crate_dir, &workspace_root)
        .with_context(|| format!("failed to load config for {}", file.display()))?;

    Ok(CompileSession::new(config))
}

fn line_number_for_offset(source: &str, offset: u32) -> usize {
    let clamped = (offset as usize).min(source.len());
    source[..clamped]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn render_diagnostics(session: &CompileSession) {
    for diagnostic in &session.diagnostics {
        eprintln!("{}", format_diagnostic(session.file_set(), diagnostic));
    }
}

fn is_command_not_found(error: &anyhow::Error) -> bool {
    error
        .root_cause()
        .downcast_ref::<std::io::Error>()
        .is_some_and(|io_error| io_error.kind() == std::io::ErrorKind::NotFound)
}

fn run_rustfmt_file(path: &Path) -> anyhow::Result<()> {
    let rustfmt_output = Command::new("rustfmt")
        .arg(path)
        .output()
        .with_context(|| format!("failed to invoke rustfmt for {}", path.display()));

    let rustfmt_output = match rustfmt_output {
        Ok(output) => output,
        Err(error) if is_command_not_found(&error) => {
            anyhow::bail!("rustfmt is unavailable; install the Rust toolchain component first");
        }
        Err(error) => return Err(error),
    };

    if !rustfmt_output.status.success() {
        anyhow::bail!("rustfmt exited with a non-zero status");
    }

    Ok(())
}

fn rustfmt_original_kobo_source(file: &Path) -> anyhow::Result<String> {
    let source = fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;
    let temp_path = rustfmt_temp_path(file);
    fs::write(&temp_path, &source)
        .with_context(|| format!("failed to stage {}", temp_path.display()))?;
    run_rustfmt_file(&temp_path)?;

    let formatted = fs::read_to_string(&temp_path)
        .with_context(|| format!("failed to read {}", temp_path.display()));
    let _ = fs::remove_file(&temp_path);

    formatted
}

fn rewrite_lossless_kobo_source(file: &Path, artifacts: &CodegenArtifacts) -> anyhow::Result<String> {
    debug_assert!(
        artifacts.source_map.kobo_path().ends_with(
            &file
                .to_string_lossy()
                .replace('/', "\\")
        ),
        "source map and fmt target should refer to the same .kobo file"
    );

    // v0.2 only back-propagates formatting from the original `.kobo` text.
    // Compiler-owned wrapper lines exist only in generated Rust, so they never
    // flow back into the user file through `kobo fmt`.
    rustfmt_original_kobo_source(file)
}

fn rustfmt_temp_path(file: &Path) -> PathBuf {
    let stem = file
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "kobo".to_owned());
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);

    std::env::temp_dir().join(format!("kobo-fmt-{stem}-{}-{nonce}.rs", std::process::id()))
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
