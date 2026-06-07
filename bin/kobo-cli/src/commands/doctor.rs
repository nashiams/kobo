use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;
use toml::Value as TomlValue;

#[path = "doctor/project_support.rs"]
mod project_support;
#[path = "doctor/supervisor_slice.rs"]
mod supervisor_slice;

pub(super) struct DoctorOptions {
    pub mode: DoctorMode,
    pub output_format: DoctorOutputFormat,
}

pub(super) enum DoctorMode {
    Dependencies(DependencyInspection),
    SelfHost,
    ProjectSupport {
        require_ready: bool,
    },
    SupervisorSlice {
        require_ready: bool,
        state_path: Option<PathBuf>,
    },
}

pub(super) enum DependencyInspection {
    Default,
    Requested,
}

#[derive(Clone, Copy)]
pub(super) enum DoctorOutputFormat {
    Human,
    Json,
}

struct DoctorReport {
    profile: &'static str,
    dependency_inspection: DependencyInspection,
    build_rs: bool,
    rust_version: Option<String>,
    dependencies: Vec<DependencyEvidence>,
    resolved_packages: Vec<ResolvedPackageEvidence>,
    metadata_status: String,
    proc_macro_candidates: Vec<String>,
    hotspots: Vec<String>,
    hints: Vec<String>,
}

struct DependencyEvidence {
    name: String,
    section: String,
    version: Option<String>,
    package: Option<String>,
    source: Option<String>,
    default_features: Option<bool>,
    features: Vec<String>,
}

struct ResolvedPackageEvidence {
    id: String,
    name: String,
    version: String,
    source: Option<String>,
    manifest_path: Option<String>,
    features: Vec<String>,
}

struct SelfHostReport {
    status: SelfHostStatus,
    signals: Vec<SelfHostSignal>,
    blockers: Vec<SelfHostBlocker>,
}

enum SelfHostStatus {
    Compatible,
    Blocked,
    OutsideScope,
}

enum SelfHostSignal {
    SourceDirectory,
    MultiFileKoboProject,
    LibraryRoot,
    KoboManifest,
    GeneratedCargoBuild,
}

enum SelfHostBlocker {
    MissingSourceDirectory,
    NotEnoughKoboModules,
    MissingLibraryRoot,
    MissingKoboManifest,
    UnsupportedDependencyForm,
}

pub(super) fn cmd_doctor(options: DoctorOptions) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;

    match options.mode {
        DoctorMode::Dependencies(dependency_inspection) => {
            let report = DoctorReport::read_project(&cwd, dependency_inspection);
            print_dependency_report(&report, options.output_format)
        }
        DoctorMode::SelfHost => {
            let report = SelfHostReport::from_project(&cwd);
            print_self_host_report(&report, options.output_format)
        }
        DoctorMode::ProjectSupport { require_ready } => {
            project_support::cmd_project_support(&cwd, options.output_format, require_ready)
        }
        DoctorMode::SupervisorSlice {
            require_ready,
            state_path,
        } => supervisor_slice::cmd_supervisor_slice(
            &cwd,
            options.output_format,
            require_ready,
            state_path.as_deref(),
        ),
    }
}

impl DependencyInspection {
    fn is_requested(&self) -> bool {
        matches!(self, Self::Requested)
    }
}

impl DoctorReport {
    fn read_project(root: &Path, dependency_inspection: DependencyInspection) -> Self {
        let cargo_toml = root.join("Cargo.toml");
        let cargo = std::fs::read_to_string(&cargo_toml).unwrap_or_default();
        let build_rs = root.join("build.rs").is_file();
        Self::from_project(root, &cargo, dependency_inspection, build_rs)
    }

    fn from_project(
        root: &Path,
        cargo: &str,
        dependency_inspection: DependencyInspection,
        build_rs: bool,
    ) -> Self {
        let lower = cargo.to_ascii_lowercase();
        let parsed = cargo.parse::<TomlValue>().ok();
        let dependencies = parsed
            .as_ref()
            .map(collect_dependencies)
            .unwrap_or_default();
        let (metadata_status, resolved_packages) = cargo_metadata_packages(root);
        let rust_version = parsed.as_ref().and_then(project_rust_version);
        let build_rs = build_rs || parsed.as_ref().is_some_and(package_declares_build_script);
        let proc_macro_candidates = proc_macro_candidates(&dependencies, &lower);
        let profile = infer_stack_profile(&lower);
        let mut hotspots = Vec::new();
        let mut hints = Vec::new();

        if dependency_inspection.is_requested() {
            hints.push("doctor --deps inspected Cargo.toml dependency shape".to_owned());
        }
        hints.push(format!("stack profile: {profile}"));
        hints.push("feature audit: check default-features and feature fanout".to_owned());
        if let Some(msrv) = &rust_version {
            hints.push(format!("msrv audit: project rust-version is {msrv}"));
        } else {
            hints.push("msrv audit: confirm dependency MSRV against project policy".to_owned());
        }

        add_profile_guidance(profile, &mut hotspots, &mut hints);

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
            dependency_inspection,
            build_rs,
            rust_version,
            dependencies,
            resolved_packages,
            metadata_status,
            proc_macro_candidates,
            hotspots,
            hints,
        }
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1,
            "command": "doctor --deps",
            "deps": self.dependency_inspection.is_requested(),
            "profile": self.profile,
            "stack_profile": self.profile,
            "build_rs": self.build_rs,
            "rust_version": self.rust_version.as_deref(),
            "dependencies": self.dependencies.iter().map(DependencyEvidence::to_json).collect::<Vec<_>>(),
            "cargo_metadata_status": self.metadata_status,
            "resolved_packages": self.resolved_packages.iter().map(ResolvedPackageEvidence::to_json).collect::<Vec<_>>(),
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
            "section": self.section.clone(),
            "version": self.version,
            "package": self.package,
            "source": self.source,
            "default_features": self.default_features,
            "features": self.features,
            "identity": self.identity(),
        })
    }

    fn identity(&self) -> String {
        let package = self.package.as_deref().unwrap_or(&self.name);
        let source = self.source.as_deref().unwrap_or_else(|| {
            self.version
                .as_deref()
                .map(|_| "registry")
                .unwrap_or("unspecified")
        });
        let default_features = self
            .default_features
            .map(|value| format!("default-features={value}"))
            .unwrap_or_else(|| "default-features=unspecified".to_owned());
        let features = if self.features.is_empty() {
            "features=<none>".to_owned()
        } else {
            format!("features={}", self.features.join(","))
        };
        format!(
            "alias={};package={package};section={};source={source};{};{}",
            self.name, self.section, default_features, features
        )
    }
}

impl ResolvedPackageEvidence {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "name": self.name,
            "version": self.version,
            "source": self.source,
            "manifest_path": self.manifest_path,
            "features": self.features,
            "identity": format!(
                "id={};name={};version={};source={};features={}",
                self.id,
                self.name,
                self.version,
                self.source.as_deref().unwrap_or("workspace-or-path"),
                if self.features.is_empty() { "<none>".to_owned() } else { self.features.join(",") }
            ),
        })
    }
}

fn cargo_metadata_packages(root: &Path) -> (String, Vec<ResolvedPackageEvidence>) {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--offline"])
        .current_dir(root)
        .output();
    let Ok(output) = output else {
        return ("unavailable".to_owned(), Vec::new());
    };
    if !output.status.success() {
        return ("unavailable-offline".to_owned(), Vec::new());
    }
    let Ok(metadata) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return ("invalid-json".to_owned(), Vec::new());
    };
    let mut feature_map = std::collections::BTreeMap::<String, Vec<String>>::new();
    if let Some(nodes) = metadata
        .get("resolve")
        .and_then(|resolve| resolve.get("nodes"))
        .and_then(serde_json::Value::as_array)
    {
        for node in nodes {
            let Some(id) = node.get("id").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let features = node
                .get("features")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            feature_map.insert(id.to_owned(), features);
        }
    }
    let packages = metadata
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|package| {
            let id = package.get("id")?.as_str()?.to_owned();
            Some(ResolvedPackageEvidence {
                features: feature_map.remove(&id).unwrap_or_default(),
                id,
                name: package.get("name")?.as_str()?.to_owned(),
                version: package.get("version")?.as_str()?.to_owned(),
                source: package
                    .get("source")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                manifest_path: package
                    .get("manifest_path")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect::<Vec<_>>();
    ("resolved-offline".to_owned(), packages)
}

impl SelfHostReport {
    fn from_project(root: &Path) -> Self {
        let source_dir = root.join("src");
        let manifest_path = root.join("Kobo.toml");
        let mut signals = Vec::new();
        let mut blockers = Vec::new();

        if source_dir.is_dir() {
            signals.push(SelfHostSignal::SourceDirectory);
        } else {
            blockers.push(SelfHostBlocker::MissingSourceDirectory);
        }

        let kobo_files = collect_kobo_files(&source_dir);
        if kobo_files.len() >= 3 {
            signals.push(SelfHostSignal::MultiFileKoboProject);
        } else {
            blockers.push(SelfHostBlocker::NotEnoughKoboModules);
        }

        if source_dir.join("lib.kobo").is_file() {
            signals.push(SelfHostSignal::LibraryRoot);
        } else {
            blockers.push(SelfHostBlocker::MissingLibraryRoot);
        }

        if manifest_path.is_file() {
            signals.push(SelfHostSignal::KoboManifest);
            if has_unsupported_dependency_form(&manifest_path) {
                blockers.push(SelfHostBlocker::UnsupportedDependencyForm);
            }
        } else {
            blockers.push(SelfHostBlocker::MissingKoboManifest);
        }

        if root.join("target/kobo-gen/Cargo.toml").is_file() {
            signals.push(SelfHostSignal::GeneratedCargoBuild);
        }

        let in_scope = manifest_path.is_file() || !kobo_files.is_empty();
        let status = if !in_scope {
            SelfHostStatus::OutsideScope
        } else if blockers.is_empty() {
            SelfHostStatus::Compatible
        } else {
            SelfHostStatus::Blocked
        };

        Self {
            status,
            signals,
            blockers,
        }
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "status": self.status.as_str(),
            "signals": self.signals.iter().map(SelfHostSignal::as_str).collect::<Vec<_>>(),
            "blockers": self.blockers.iter().map(SelfHostBlocker::as_str).collect::<Vec<_>>(),
            "meaning": "self-host-compatible, not self-hosted",
        })
    }
}

impl SelfHostStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Compatible => "compatible",
            Self::Blocked => "blocked",
            Self::OutsideScope => "outside-scope",
        }
    }
}

impl SelfHostSignal {
    fn as_str(&self) -> &'static str {
        match self {
            Self::SourceDirectory => "source-directory",
            Self::MultiFileKoboProject => "multi-file-kobo-project",
            Self::LibraryRoot => "library-root",
            Self::KoboManifest => "kobo-manifest",
            Self::GeneratedCargoBuild => "generated-cargo-build",
        }
    }
}

impl SelfHostBlocker {
    fn as_str(&self) -> &'static str {
        match self {
            Self::MissingSourceDirectory => "missing-source-directory",
            Self::NotEnoughKoboModules => "not-enough-kobo-modules",
            Self::MissingLibraryRoot => "missing-library-root",
            Self::MissingKoboManifest => "missing-kobo-manifest",
            Self::UnsupportedDependencyForm => "unsupported-dependency-form",
        }
    }
}

fn has_unsupported_dependency_form(manifest_path: &Path) -> bool {
    let Ok(manifest) = std::fs::read_to_string(manifest_path) else {
        return false;
    };
    let Ok(parsed) = manifest.parse::<TomlValue>() else {
        return false;
    };
    let Some(dependencies) = parsed.get("dependencies").and_then(TomlValue::as_table) else {
        return false;
    };

    dependencies
        .values()
        .any(|value| !matches!(value, TomlValue::String(_) | TomlValue::Table(_)))
}

fn print_dependency_report(
    report: &DoctorReport,
    output_format: DoctorOutputFormat,
) -> anyhow::Result<()> {
    match output_format {
        DoctorOutputFormat::Json => print_json(report.to_json()),
        DoctorOutputFormat::Human => {
            println!("doctor --deps profile: {}", report.profile);
            for hotspot in &report.hotspots {
                println!("- {hotspot}");
            }
            for hint in &report.hints {
                println!("- {hint}");
            }
            Ok(())
        }
    }
}

fn print_self_host_report(
    report: &SelfHostReport,
    output_format: DoctorOutputFormat,
) -> anyhow::Result<()> {
    match output_format {
        DoctorOutputFormat::Json => print_json(serde_json::json!({
            "schema_version": 1,
            "command": "doctor --self-host",
            "self_host": report.to_json(),
        })),
        DoctorOutputFormat::Human => {
            println!("doctor --self-host: {}", report.status.as_str());
            println!("meaning: self-host-compatible, not self-hosted");
            println!("signals:");
            for signal in &report.signals {
                println!("  - {}", signal.as_str());
            }
            if report.blockers.is_empty() {
                println!("blockers: none");
            } else {
                println!("blockers:");
                for blocker in &report.blockers {
                    println!("  - {}", blocker.as_str());
                }
            }
            Ok(())
        }
    }
}

fn print_json(value: serde_json::Value) -> anyhow::Result<()> {
    println!(
        "{}",
        serde_json::to_string(&value).context("failed to serialize doctor JSON")?
    );
    Ok(())
}

fn add_profile_guidance(profile: &str, hotspots: &mut Vec<String>, hints: &mut Vec<String>) {
    match profile {
        "api-service" => {
            hotspots
                .push("proc_macro fanout risk: web stacks often include derive macros".to_owned());
            hotspots.push("feature hotspot: tokio/reqwest TLS and runtime features".to_owned());
            hints.push("api-service hint: keep external HTTP boundaries policy-visible".to_owned());
        }
        "storage-engine" => {
            hotspots
                .push("build.rs hotspot: native storage crates may compile C/C++ code".to_owned());
            hotspots.push("feature hotspot: compression/backtrace/native feature sets".to_owned());
            hints.push(
                "storage-engine hint: separate deterministic core from disk boundaries".to_owned(),
            );
        }
        "game-server-core" => {
            hotspots.push("feature hotspot: engine plugins can inflate compile cost".to_owned());
            hotspots.push("msrv hotspot: game stacks often pin fast-moving crates".to_owned());
            hints.push("game-server-core hint: prefer generic facades for hot systems".to_owned());
        }
        _ => {
            hotspots.push("feature hotspot: review dependency feature fanout".to_owned());
        }
    }
}

fn collect_dependencies(cargo: &TomlValue) -> Vec<DependencyEvidence> {
    let mut dependencies = Vec::new();
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        dependencies.extend(dependencies_in_section(cargo, section.to_owned()));
    }
    if let Some(targets) = cargo.get("target").and_then(TomlValue::as_table) {
        for (target, target_config) in targets {
            for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                let label = format!("target.'{target}'.{section}");
                dependencies.extend(dependencies_in_table(target_config, label, section));
            }
        }
    }
    dependencies
}

fn dependencies_in_section(cargo: &TomlValue, section: String) -> Vec<DependencyEvidence> {
    let Some(table) = table_at_path(cargo, &section) else {
        return Vec::new();
    };
    dependencies_in_table_entries(table, section)
}

fn dependencies_in_table(
    cargo: &TomlValue,
    label: String,
    section: &str,
) -> Vec<DependencyEvidence> {
    let Some(table) = cargo.get(section).and_then(TomlValue::as_table) else {
        return Vec::new();
    };
    dependencies_in_table_entries(table, label)
}

fn dependencies_in_table_entries(
    table: &toml::map::Map<String, TomlValue>,
    section: String,
) -> Vec<DependencyEvidence> {
    table
        .iter()
        .filter_map(|(name, value)| dependency_evidence(name, &section, value))
        .collect()
}

fn dependency_evidence(name: &str, section: &str, value: &TomlValue) -> Option<DependencyEvidence> {
    if let Some(version) = value.as_str() {
        return Some(DependencyEvidence {
            name: name.to_owned(),
            section: section.to_owned(),
            version: Some(version.to_owned()),
            package: None,
            source: Some("registry".to_owned()),
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
    let source = table
        .get("path")
        .and_then(TomlValue::as_str)
        .map(|path| format!("path={path}"))
        .or_else(|| {
            table
                .get("git")
                .and_then(TomlValue::as_str)
                .map(|git| format!("git={git}"))
        })
        .or_else(|| version.as_ref().map(|_| "registry".to_owned()));
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
        section: section.to_owned(),
        version,
        package,
        source,
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

fn collect_kobo_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_kobo_files_into(root, &mut files);
    files
}

fn collect_kobo_files_into(root: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_kobo_files_into(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("kobo") {
            files.push(path);
        }
    }
}
