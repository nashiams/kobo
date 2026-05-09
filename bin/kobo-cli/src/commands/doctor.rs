use std::path::Path;

use anyhow::Context;
use toml::Value as TomlValue;

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
    build_rs: bool,
    rust_version: Option<String>,
    dependencies: Vec<DependencyEvidence>,
    proc_macro_candidates: Vec<String>,
    hotspots: Vec<String>,
    hints: Vec<String>,
}

struct DependencyEvidence {
    name: String,
    section: &'static str,
    version: Option<String>,
    package: Option<String>,
    default_features: Option<bool>,
    features: Vec<String>,
}

impl DoctorReport {
    fn from_project(root: &Path, cargo: &str, deps_checked: bool, build_rs: bool) -> Self {
        let lower = cargo.to_ascii_lowercase();
        let parsed = cargo.parse::<TomlValue>().ok();
        let dependencies = parsed
            .as_ref()
            .map(collect_dependencies)
            .unwrap_or_default();
        let rust_version = parsed.as_ref().and_then(project_rust_version);
        let build_rs = build_rs || parsed.as_ref().is_some_and(package_declares_build_script);
        let proc_macro_candidates = proc_macro_candidates(&dependencies, &lower);
        let profile = infer_stack_profile(&lower);
        let mut hotspots = Vec::new();
        let mut hints = Vec::new();

        if deps_checked {
            hints.push("doctor --deps inspected Cargo.toml dependency shape".to_owned());
        }
        hints.push(format!("stack profile: {profile}"));
        hints.push("feature audit: check default-features and feature fanout".to_owned());
        if let Some(msrv) = &rust_version {
            hints.push(format!("msrv audit: project rust-version is {msrv}"));
        } else {
            hints.push("msrv audit: confirm dependency MSRV against project policy".to_owned());
        }

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
        for dep in dependencies.iter().filter(|dep| !dep.features.is_empty()) {
            hotspots.push(format!(
                "feature hotspot: {} enables {}",
                dep.name,
                dep.features.join(", ")
            ));
        }
        for dep in &proc_macro_candidates {
            hotspots.push(format!("proc_macro candidate: {dep}"));
        }
        if hotspots.iter().all(|hotspot| !hotspot.contains("msrv")) {
            if let Some(msrv) = &rust_version {
                hotspots.push(format!("msrv hotspot: project rust-version {msrv}"));
            } else {
                hotspots.push("msrv hotspot: verify minimum supported Rust version".to_owned());
            }
        }

        Self {
            profile,
            deps_checked,
            build_rs,
            rust_version,
            dependencies,
            proc_macro_candidates,
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
            "build_rs": self.build_rs,
            "rust_version": self.rust_version.as_deref(),
            "dependencies": self.dependencies.iter().map(DependencyEvidence::to_json).collect::<Vec<_>>(),
            "proc_macro_candidates": &self.proc_macro_candidates,
            "hotspots": self.hotspots,
            "hints": self.hints,
        })
    }
}

impl DependencyEvidence {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "section": self.section,
            "version": self.version,
            "package": self.package,
            "default_features": self.default_features,
            "features": self.features,
        })
    }
}

fn collect_dependencies(cargo: &TomlValue) -> Vec<DependencyEvidence> {
    [
        "dependencies",
        "dev-dependencies",
        "build-dependencies",
        "target.'cfg(windows)'.dependencies",
        "target.'cfg(unix)'.dependencies",
    ]
    .into_iter()
    .flat_map(|section| dependencies_in_section(cargo, section))
    .collect()
}

fn dependencies_in_section(cargo: &TomlValue, section: &'static str) -> Vec<DependencyEvidence> {
    let Some(table) = table_at_path(cargo, section) else {
        return Vec::new();
    };
    table
        .iter()
        .filter_map(|(name, value)| dependency_evidence(name, section, value))
        .collect()
}

fn dependency_evidence(
    name: &str,
    section: &'static str,
    value: &TomlValue,
) -> Option<DependencyEvidence> {
    if let Some(version) = value.as_str() {
        return Some(DependencyEvidence {
            name: name.to_owned(),
            section,
            version: Some(version.to_owned()),
            package: None,
            default_features: None,
            features: Vec::new(),
        });
    }

    let table = value.as_table()?;
    let version = table
        .get("version")
        .and_then(TomlValue::as_str)
        .map(str::to_owned);
    let package = table
        .get("package")
        .and_then(TomlValue::as_str)
        .map(str::to_owned);
    let default_features = table.get("default-features").and_then(TomlValue::as_bool);
    let features = table
        .get("features")
        .and_then(TomlValue::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(TomlValue::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Some(DependencyEvidence {
        name: name.to_owned(),
        section,
        version,
        package,
        default_features,
        features,
    })
}

fn table_at_path<'a>(
    value: &'a TomlValue,
    section: &str,
) -> Option<&'a toml::map::Map<String, TomlValue>> {
    let mut current = value;
    for part in section.split('.') {
        let key = part.trim_matches('\'');
        current = current.get(key)?;
    }
    current.as_table()
}

fn project_rust_version(cargo: &TomlValue) -> Option<String> {
    cargo
        .get("package")?
        .get("rust-version")?
        .as_str()
        .map(str::to_owned)
}

fn package_declares_build_script(cargo: &TomlValue) -> bool {
    cargo
        .get("package")
        .and_then(|package| package.get("build"))
        .is_some_and(|build| !matches!(build.as_bool(), Some(false)))
}

fn proc_macro_candidates(dependencies: &[DependencyEvidence], cargo_lower: &str) -> Vec<String> {
    let mut candidates = dependencies
        .iter()
        .filter(|dep| {
            let name = dep.package.as_deref().unwrap_or(&dep.name);
            is_proc_macro_like(name)
        })
        .map(|dep| dep.name.clone())
        .collect::<Vec<_>>();

    if (cargo_lower.contains("proc-macro") || cargo_lower.contains("proc_macro"))
        && !candidates.iter().any(|dep| dep == "proc-macro")
    {
        candidates.push("proc-macro".to_owned());
    }
    candidates.sort();
    candidates.dedup();
    candidates
}

fn is_proc_macro_like(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "syn"
        || lower == "quote"
        || lower == "proc-macro2"
        || lower == "async-trait"
        || lower == "thiserror"
        || lower == "serde_derive"
        || lower.ends_with("_derive")
        || lower.ends_with("-derive")
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
