use std::path::{Path, PathBuf};
use std::process::Command;

use kobo_ir::ScenarioProgram;

use crate::core::{ScenarioEvent, ScenarioOptions};
use crate::error::{Result, SimCoreError};
use crate::harness_manifest::HarnessManifest;

use super::events::{parse_harness_events, parse_harness_service_hook_events};
use super::facade_manifest::harness_facades;
use super::source::harness_source;

pub(super) struct HarnessRun {
    pub(super) events: Vec<ScenarioEvent>,
    pub(super) manifest: HarnessManifest,
    pub(super) engine: String,
}

struct HarnessProcess {
    command: Vec<String>,
    exit_code: i32,
    stdout: String,
    stderr: String,
}

pub(super) fn run_generated_harness(
    program: &ScenarioProgram,
    generated_rust: &str,
    options: &ScenarioOptions,
) -> Result<HarnessRun> {
    let generated_rust_hash = crate::digest::stable_hash(generated_rust);
    let harness_dir = harness_dir(&program.source_hash, &program.target, &generated_rust_hash)?;
    std::fs::create_dir_all(&harness_dir)
        .map_err(|source| SimCoreError::io("create harness directory", source))?;
    let checkpoint_path = loom_checkpoint_path(options, &harness_dir);
    let harness_source =
        harness_source(program, generated_rust, options, checkpoint_path.as_deref())?;
    let uses_loom = options.profile == "sync";
    let harness_rs_path = if uses_loom {
        harness_dir.join("src").join("main.rs")
    } else {
        harness_dir.join("harness.rs")
    };
    if let Some(parent) = harness_rs_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|source| SimCoreError::io("create harness source directory", source))?;
    }
    std::fs::write(&harness_rs_path, harness_source)
        .map_err(|source| SimCoreError::io("write generated harness source", source))?;
    let process = if uses_loom {
        run_loom_cargo_harness(&harness_dir)?
    } else {
        run_rustc_harness(&harness_dir, &harness_rs_path)?
    };
    let checkpoint_artifact_hash = checkpoint_artifact_hash(checkpoint_path.as_deref())?;
    let events = parse_harness_events(&process.stdout)?;
    let service_hook_events = parse_harness_service_hook_events(&process.stdout)?;
    let engine = if uses_loom {
        "generated-rust-loom-process"
    } else {
        "generated-rust-process"
    };
    let manifest = HarnessManifest {
        source_hash: program.source_hash.clone(),
        generated_rust_hash,
        execution_scope: harness_execution_scope(options).to_owned(),
        full_ecosystem_exploration: false,
        facades: harness_facades(program),
        harness_dir: harness_dir.display().to_string(),
        harness_rs_path: harness_rs_path.display().to_string(),
        command: process.command,
        exit_code: process.exit_code,
        stdout_hash: crate::digest::stable_hash(&process.stdout),
        stderr_hash: crate::digest::stable_hash(&process.stderr),
        event_count: events.len(),
        service_hook_events,
        checkpoint_path: checkpoint_path.map(|path| path.display().to_string()),
        checkpoint_artifact_hash,
    };
    let manifest_path = harness_dir.join("manifest.json");
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|source| SimCoreError::json("serialize harness manifest", source))?;
    std::fs::write(&manifest_path, manifest_json)
        .map_err(|source| SimCoreError::io("write harness manifest", source))?;
    Ok(HarnessRun {
        events,
        manifest,
        engine: engine.to_owned(),
    })
}

fn harness_execution_scope(options: &ScenarioOptions) -> &'static str {
    match options.profile.as_str() {
        "sync" => "generated-user-rust-loom",
        "network" | "network-design" => "generated-user-rust-os-facade",
        "async" | "distributed" | "madsim" => "generated-user-rust-adapter",
        _ => "generated-user-rust",
    }
}

fn run_rustc_harness(harness_dir: &PathBuf, harness_rs_path: &PathBuf) -> Result<HarnessProcess> {
    let harness_bin_path = harness_dir.join(binary_name("harness_bin"));
    remove_existing_output(&harness_bin_path)?;

    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let output = Command::new(&rustc)
        .arg("--edition=2021")
        .arg(harness_rs_path)
        .arg("-o")
        .arg(&harness_bin_path)
        .output()
        .map_err(|source| SimCoreError::io("compile generated harness", source))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(SimCoreError::HarnessCompileFailed { stderr });
    }

    let run_output = Command::new(&harness_bin_path)
        .output()
        .map_err(|source| SimCoreError::io("run generated harness", source))?;
    if harness_bin_path.exists() {
        std::fs::remove_file(&harness_bin_path)
            .map_err(|source| SimCoreError::io("remove generated harness binary", source))?;
    }
    Ok(HarnessProcess {
        command: vec![harness_bin_path.display().to_string()],
        exit_code: run_output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&run_output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&run_output.stderr).to_string(),
    })
}

fn run_loom_cargo_harness(harness_dir: &PathBuf) -> Result<HarnessProcess> {
    let manifest_path = harness_dir.join("Cargo.toml");
    std::fs::write(&manifest_path, loom_cargo_manifest())
        .map_err(|source| SimCoreError::io("write loom harness manifest", source))?;
    let target_dir = harness_dir.join("target");
    if target_dir.exists() {
        std::fs::remove_dir_all(&target_dir)
            .map_err(|source| SimCoreError::io("remove stale loom harness target", source))?;
    }

    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(&cargo)
        .arg("run")
        .arg("--quiet")
        .arg("--offline")
        .arg("--manifest-path")
        .arg(&manifest_path)
        .env("CARGO_TARGET_DIR", &target_dir)
        .output()
        .map_err(|source| SimCoreError::io("run loom harness cargo", source))?;
    if target_dir.exists() {
        std::fs::remove_dir_all(&target_dir)
            .map_err(|source| SimCoreError::io("remove loom harness target", source))?;
    }
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(SimCoreError::HarnessCompileFailed { stderr });
    }
    Ok(HarnessProcess {
        command: vec![
            cargo,
            "run".to_owned(),
            "--quiet".to_owned(),
            "--offline".to_owned(),
            "--manifest-path".to_owned(),
            manifest_path.display().to_string(),
        ],
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn checkpoint_artifact_hash(checkpoint_path: Option<&Path>) -> Result<Option<String>> {
    let Some(path) = checkpoint_path else {
        return Ok(None);
    };
    let contents = std::fs::read_to_string(path)
        .map_err(|source| SimCoreError::io("read loom checkpoint artifact", source))?;
    Ok(Some(crate::digest::stable_hash(&contents)))
}

fn loom_cargo_manifest() -> &'static str {
    r#"[workspace]

[package]
name = "kobo_generated_loom_harness"
version = "0.0.0"
edition = "2021"

[dependencies]
loom = { version = "0.7", features = ["checkpoint"] }
"#
}

fn remove_existing_output(path: &PathBuf) -> Result<()> {
    if path.exists() {
        std::fs::remove_file(path)
            .map_err(|source| SimCoreError::io("remove stale harness binary", source))?;
    }
    let pdb_path = path.with_extension("pdb");
    if pdb_path.exists() {
        std::fs::remove_file(pdb_path)
            .map_err(|source| SimCoreError::io("remove stale harness debug file", source))?;
    }
    Ok(())
}

fn loom_checkpoint_path(options: &ScenarioOptions, harness_dir: &Path) -> Option<PathBuf> {
    (options.profile == "sync" && options.loom_checkpoint_replay)
        .then(|| harness_dir.join("loom-checkpoint.json"))
}

fn harness_dir(source_hash: &str, target: &str, generated_rust_hash: &str) -> Result<PathBuf> {
    let root = std::env::current_dir()
        .map_err(|source| SimCoreError::io("resolve current directory", source))?
        .join(".kobo")
        .join("harness");
    let safe_target = sanitize_path_segment(target);
    Ok(root.join(format!(
        "{}-{}-{}",
        source_hash, safe_target, generated_rust_hash
    )))
}

fn binary_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_owned()
    }
}

fn sanitize_path_segment(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}
