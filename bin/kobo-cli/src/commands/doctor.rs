use std::path::Path;

use anyhow::Context;

pub(super) fn cmd_doctor(deps: bool, json: bool) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let cargo_toml = cwd.join("Cargo.toml");
    let cargo = std::fs::read_to_string(&cargo_toml).unwrap_or_default();
    let build_rs = cwd.join("build.rs").is_file();
    let report = DoctorReport::from_project(&cwd, &cargo, deps, build_rs);

    if json {
        println!(
            "{}",
            serde_json::to_string(&report.to_json()).expect("doctor JSON should serialize")
        );
    } else {
        println!("doctor --deps profile: {}", report.profile);
        for hotspot in &report.hotspots {
            println!("- {hotspot}");
        }
        for hint in &report.hints {
            println!("- {hint}");
        }
    }

    Ok(())
}

struct DoctorReport {
    profile: &'static str,
    deps_checked: bool,
    hotspots: Vec<String>,
    hints: Vec<String>,
}

impl DoctorReport {
    fn from_project(root: &Path, cargo: &str, deps_checked: bool, build_rs: bool) -> Self {
        let lower = cargo.to_ascii_lowercase();
        let profile = infer_stack_profile(&lower);
        let mut hotspots = Vec::new();
        let mut hints = Vec::new();

        if deps_checked {
            hints.push("doctor --deps inspected Cargo.toml dependency shape".to_owned());
        }
        hints.push(format!("stack profile: {profile}"));
        hints.push("feature audit: check default-features and feature fanout".to_owned());
        hints.push("msrv audit: confirm dependency MSRV against project policy".to_owned());

        match profile {
            "api-service" => {
                hotspots.push(
                    "proc_macro fanout risk: web stacks often include derive macros".to_owned(),
                );
                hotspots.push("feature hotspot: tokio/reqwest TLS and runtime features".to_owned());
                hints.push(
                    "api-service hint: keep external HTTP boundaries policy-visible".to_owned(),
                );
            }
            "storage-engine" => {
                hotspots.push(
                    "build.rs hotspot: native storage crates may compile C/C++ code".to_owned(),
                );
                hotspots
                    .push("feature hotspot: compression/backtrace/native feature sets".to_owned());
                hints.push(
                    "storage-engine hint: separate deterministic core from disk boundaries"
                        .to_owned(),
                );
            }
            "game-server-core" => {
                hotspots
                    .push("feature hotspot: engine plugins can inflate compile cost".to_owned());
                hotspots.push("msrv hotspot: game stacks often pin fast-moving crates".to_owned());
                hints.push(
                    "game-server-core hint: prefer generic facades for hot systems".to_owned(),
                );
            }
            _ => {
                hotspots.push("feature hotspot: review dependency feature fanout".to_owned());
            }
        }

        if build_rs || root.join("build.rs").is_file() {
            hotspots.push("build.rs present: generated/native build steps need review".to_owned());
        }
        if lower.contains("proc-macro") || lower.contains("proc_macro") {
            hotspots.push("proc_macro crate declared directly".to_owned());
        }
        if hotspots.iter().all(|hotspot| !hotspot.contains("msrv")) {
            hotspots.push("msrv hotspot: verify minimum supported Rust version".to_owned());
        }

        Self {
            profile,
            deps_checked,
            hotspots,
            hints,
        }
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1,
            "command": "doctor --deps",
            "deps": self.deps_checked,
            "profile": self.profile,
            "stack_profile": self.profile,
            "hotspots": self.hotspots,
            "hints": self.hints,
        })
    }
}

fn infer_stack_profile(cargo: &str) -> &'static str {
    if cargo.contains("bevy") || cargo.contains("rapier") || cargo.contains("winit") {
        return "game-server-core";
    }
    if cargo.contains("rocksdb") || cargo.contains("sled") || cargo.contains("redb") {
        return "storage-engine";
    }
    if cargo.contains("reqwest")
        || cargo.contains("tokio")
        || cargo.contains("axum")
        || cargo.contains("hyper")
    {
        return "api-service";
    }
    "cargo-project"
}
