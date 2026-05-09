use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;
use kobo_driver::{
    load_config, run_and_compile, run_and_compile_with_lifetime_erasure, run_codegen_pipeline,
    run_kir_phase, CodegenArtifacts,
};
use kobo_ir::KoboMode;

use super::session::{build_session, line_number_for_offset, render_diagnostics};

pub(super) fn cmd_run(
    file: &Path,
    cli_mode: Option<KoboMode>,
    erase_lifetimes: bool,
) -> anyhow::Result<()> {
    let mut session = build_session(file, cli_mode)?;
    if session.mode() == KoboMode::Strict {
        eprintln!("error: --strict mode is not yet implemented (target: v0.9)");
        eprintln!("hint: use --checked for advisory ownership warnings");
        std::process::exit(1);
    }

    let compile_result = if erase_lifetimes {
        run_and_compile_with_lifetime_erasure(&mut session, file)
    } else {
        run_and_compile(&mut session, file)
    };
    let binary_path = compile_result.map_err(|()| {
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
    erase_lifetimes: bool,
    cargo_dir: Option<&Path>,
) -> anyhow::Result<()> {
    let mut session = build_session(file, cli_mode)?;
    if session.mode() == KoboMode::Strict {
        eprintln!("error: --strict mode is not yet implemented (target: v0.9)");
        eprintln!("hint: use --checked for advisory ownership warnings");
        std::process::exit(1);
    }

    if let Some(dir) = cargo_dir {
        let InspectCargoOutput {
            project_config,
            source_files,
            main_output,
        } = build_inspect_cargo_output(file, cli_mode, erase_lifetimes)?;

        kobo_codegen::cargo_gen::generate_cargo_project(&project_config, &source_files, dir)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        eprintln!("cargo project generated at {}", dir.display());
        eprintln!("// effective mode: {}", session.mode());
        print!("{main_output}");
        return Ok(());
    }

    let CodegenArtifacts { rs_source, .. } =
        run_codegen_pipeline(&mut session, file).map_err(|()| {
            render_diagnostics(&session);
            anyhow::anyhow!("compilation failed")
        })?;
    // v0.6 §3.3b: Render K-code warnings on success path too [R6-06].
    render_diagnostics(&session);

    // S-21: Apply lifetime erasure when requested (script mode only).
    let rs_source = if erase_lifetimes {
        kobo_driver::apply_lifetime_erasure(&rs_source, session.mode())
    } else {
        rs_source
    };

    let output = if clean {
        kobo_codegen::clean::strip_kobo_wrappers(&rs_source)
    } else {
        // Annotate lock acquisition order for inspect output.
        kobo_codegen::annotate_lock_order(&rs_source)
    };

    // S-26: Show effective mode so user can verify per-module mode resolution.
    eprintln!("// effective mode: {}", session.mode());
    print!("{output}");
    Ok(())
}

struct InspectCargoOutput {
    project_config: kobo_codegen::cargo_gen::KoboProjectConfig,
    source_files: Vec<(PathBuf, String)>,
    main_output: String,
}

fn build_inspect_cargo_output(
    file: &Path,
    cli_mode: Option<KoboMode>,
    erase_lifetimes: bool,
) -> anyhow::Result<InspectCargoOutput> {
    let absolute_file = absolute_input_path(file)?;
    let Some(project_root) = find_nearest_kobo_project_root(&absolute_file) else {
        return build_single_file_inspect_cargo_output(file, cli_mode, erase_lifetimes);
    };

    let driver_config = load_config(&project_root)
        .with_context(|| format!("failed to load config for {}", project_root.display()))?;
    let project_config = cargo_project_config_from_root(&project_root)?;
    let src_dir = project_root.join(&driver_config.src_dir);
    let mut kobo_files = Vec::new();
    collect_kobo_files(&src_dir, &mut kobo_files);
    if kobo_files.is_empty() {
        return build_single_file_inspect_cargo_output(file, cli_mode, erase_lifetimes);
    }

    let canonical_input = absolute_file
        .canonicalize()
        .unwrap_or(absolute_file.clone());
    let mut source_files = Vec::new();
    let mut main_output = None;

    for kobo_file in kobo_files {
        let mut session = build_session(&kobo_file, cli_mode)?;
        let CodegenArtifacts { rs_source, .. } = run_codegen_pipeline(&mut session, &kobo_file)
            .map_err(|()| {
                render_diagnostics(&session);
                anyhow::anyhow!("compilation failed")
            })?;
        render_diagnostics(&session);

        let rs_source = if erase_lifetimes {
            kobo_driver::apply_lifetime_erasure(&rs_source, session.mode())
        } else {
            rs_source
        };
        let clean_source = kobo_codegen::clean::strip_kobo_wrappers(&rs_source);
        let rel = kobo_file
            .strip_prefix(&src_dir)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| {
                kobo_file
                    .file_name()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("main.kobo"))
            });

        if kobo_file.canonicalize().unwrap_or(kobo_file.clone()) == canonical_input {
            main_output = Some(clean_source.clone());
        }
        source_files.push((rel, clean_source));
    }

    Ok(InspectCargoOutput {
        project_config,
        source_files,
        main_output: main_output.unwrap_or_default(),
    })
}

fn build_single_file_inspect_cargo_output(
    file: &Path,
    cli_mode: Option<KoboMode>,
    erase_lifetimes: bool,
) -> anyhow::Result<InspectCargoOutput> {
    let mut session = build_session(file, cli_mode)?;
    let CodegenArtifacts { rs_source, .. } =
        run_codegen_pipeline(&mut session, file).map_err(|()| {
            render_diagnostics(&session);
            anyhow::anyhow!("compilation failed")
        })?;
    render_diagnostics(&session);

    let rs_source = if erase_lifetimes {
        kobo_driver::apply_lifetime_erasure(&rs_source, session.mode())
    } else {
        rs_source
    };
    let output = kobo_codegen::clean::strip_kobo_wrappers(&rs_source);
    let project_config = cargo_project_config_from_optional_adjacent_toml(file)?;

    Ok(InspectCargoOutput {
        project_config,
        source_files: vec![(file.to_path_buf(), output.clone())],
        main_output: output,
    })
}

fn cargo_project_config_from_root(
    project_root: &Path,
) -> anyhow::Result<kobo_codegen::cargo_gen::KoboProjectConfig> {
    let kobo_toml_path = project_root.join("Kobo.toml");
    if !kobo_toml_path.exists() {
        return Ok(kobo_codegen::cargo_gen::KoboProjectConfig::default());
    }

    let toml_str = fs::read_to_string(&kobo_toml_path)
        .with_context(|| format!("failed to read {}", kobo_toml_path.display()))?;
    kobo_codegen::cargo_gen::KoboProjectConfig::from_toml(&toml_str)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

fn cargo_project_config_from_optional_adjacent_toml(
    file: &Path,
) -> anyhow::Result<kobo_codegen::cargo_gen::KoboProjectConfig> {
    let kobo_toml_path = file.parent().unwrap_or(Path::new(".")).join("Kobo.toml");
    if !kobo_toml_path.exists() {
        return Ok(kobo_codegen::cargo_gen::KoboProjectConfig::default());
    }

    let toml_str = fs::read_to_string(&kobo_toml_path)
        .with_context(|| format!("failed to read {}", kobo_toml_path.display()))?;
    kobo_codegen::cargo_gen::KoboProjectConfig::from_toml(&toml_str)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

fn collect_kobo_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_kobo_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "kobo") {
            out.push(path);
        }
    }
    out.sort();
}

fn absolute_input_path(file: &Path) -> anyhow::Result<PathBuf> {
    if file.is_absolute() {
        Ok(file.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .context("failed to determine current directory")?
            .join(file))
    }
}

fn find_nearest_kobo_project_root(file: &Path) -> Option<PathBuf> {
    let search_root = file.parent().unwrap_or(file);
    for ancestor in search_root.ancestors() {
        if ancestor.join("Kobo.toml").is_file() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
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
