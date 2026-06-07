use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Context;
use kobo_proof::stable_hash;
use serde_json::Value;
use toml::Value as TomlValue;

use super::DoctorOutputFormat;

struct ProjectSupportReport {
    status: ProjectSupportStatus,
    claim: String,
    replacement_claim: ReplacementClaimPosture,
    inventory: ProjectInventory,
    evidence: ProjectSupportEvidence,
    blockers: Vec<ProjectSupportBlocker>,
}

struct ReplacementClaimPosture {
    full_rewrite: ReplacementClaimState,
    clean_replacement: ReplacementClaimState,
    support_scope: &'static str,
    statement: &'static str,
    required_before_clean_replacement: &'static [&'static str],
}

struct ProjectInventory {
    cargo_manifest: bool,
    kobo_manifest: bool,
    feature_flags: Vec<String>,
    workspace_members: Vec<String>,
    kobo_modules: Vec<String>,
    rust_modules: Vec<String>,
    examples: Vec<String>,
    tests: Vec<String>,
    build_scripts: Vec<String>,
    release_artifacts: Vec<String>,
}

struct ProjectSupportEvidence {
    manifest_path: Option<PathBuf>,
    manifest: Option<Value>,
}

struct UpstreamProjectEvidence {
    root: PathBuf,
    cargo_metadata: Value,
}

struct ProjectSupportBlocker {
    message: String,
}

#[derive(Default)]
struct AdapterDependencyIndex {
    features_by_name: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Clone)]
struct ObservedCommand {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

struct CommandTranscriptEvidence {
    transcript: Value,
    observed: Option<ObservedCommand>,
}

type CommandStreamHandle = thread::JoinHandle<std::io::Result<Vec<u8>>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProjectSupportStatus {
    Ready,
    Blocked,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReplacementClaimState {
    Blocked,
}

const REQUIRED_LANGUAGE_CONSTRUCTS: &[&str] = &[
    "modules",
    "workspace",
    "feature_flags",
    "platform_cfg",
    "generics",
    "traits",
    "async_code",
    "error_types",
    "iterators",
    "tests",
    "examples",
    "public_api",
    "build_scripts",
    "release_artifacts",
];

const REQUIRED_LANGUAGE_FLAGS: &[&str] =
    &["parse", "check", "lower", "source_map", "rare_diagnostics"];

const REQUIRED_PLATFORM_MODELS: &[&str] = &[
    "filesystem_events",
    "watcher_backend",
    "paths",
    "process_execution",
    "signals",
    "process_groups",
    "environment_variables",
    "terminal_io",
    "stdio",
    "timers",
];

const REQUIRED_PLATFORMS: &[&str] = &["windows", "macos", "linux"];

const REQUIRED_ADAPTERS: &[&str] = &[
    "watcher_backend",
    "async_runtime",
    "process_handling",
    "signal_handling",
    "ignore_path",
    "config",
    "cli",
    "shell_parsing",
    "serialization",
    "logging_tracing",
    "terminal_helpers",
    "errors",
];

const REQUIRED_ASYNC_SEMANTICS: &[&str] = &[
    "spawn",
    "join",
    "cancel",
    "select",
    "timer",
    "channel",
    "backpressure",
    "shutdown",
    "blocking",
];

const REQUIRED_ASYNC_MUTATIONS: &[&str] = &[
    "task-order",
    "timer-order",
    "cancel-order",
    "channel-delivery",
];

const REQUIRED_BACKEND_FLAGS: &[&str] = &[
    "reviewable",
    "deterministic",
    "source_mapped",
    "diagnostics_on_kobo_source",
    "replay_on_kobo_source",
    "debt_on_kobo_source",
    "proof_on_kobo_source",
    "lsp_on_kobo_source",
];

const REQUIRED_PARITY_PATHS: &[&str] = &[
    "upstream_tests",
    "kobo_replay_tests",
    "kobo_liveness_tests",
    "cli_behavior",
    "config_behavior",
    "exit_behavior",
    "logging_behavior",
    "package_behavior",
    "platform_behavior",
    "install_behavior",
];

const REQUIRED_PERFORMANCE_FIELDS: &[&str] = &[
    "startup",
    "steady_state",
    "restart",
    "memory",
    "binary",
    "watch_tree_scaling",
    "event_burst_scaling",
];

const REQUIRED_MUTATION_TESTS: &[&str] = &[
    "watcher",
    "child",
    "cancellation",
    "config",
    "ignore_rules",
    "async_scheduling",
    "platform",
    "generated_backend_output",
];

const REQUIRED_REVIEW_SECTIONS: &[&str] = &[
    "behavior",
    "generated_rust",
    "diagnostics",
    "proof_debt",
    "real_command_output",
    "release_artifacts",
];

const REQUIRED_PLATFORM_BEHAVIOR_TESTS: &[&str] =
    &["watcher", "restart", "signal", "stdin", "path_filter"];

const REQUIRED_BACKEND_WORKFLOWS: &[&str] = &[
    "inspect_clean_cargo",
    "cargo_check",
    "source_map_diagnostics",
    "replay_debug",
];

const REQUIRED_UPSTREAM_INVENTORY_FIELDS: &[&str] = &[
    "crate_tree",
    "modules",
    "public_types",
    "cli_surfaces",
    "test_fixtures",
    "platform_paths",
    "feature_combinations",
    "examples",
    "build_scripts",
    "release_artifacts",
];

const REQUIRED_UPSTREAM_PACKAGE_NAMES: &[&str] = &[
    "watchexec",
    "watchexec-cli",
    "watchexec-events",
    "watchexec-signals",
    "watchexec-supervisor",
];

const MIN_UPSTREAM_CRATE_PATHS: usize = 8;
const MIN_UPSTREAM_MODULES: usize = 10;
const MIN_UPSTREAM_PUBLIC_TYPES: usize = 8;
const MIN_UPSTREAM_CLI_SURFACES: usize = 5;
const MIN_UPSTREAM_TEST_FIXTURES: usize = 6;
const MIN_UPSTREAM_PLATFORM_PATHS: usize = 4;
const MIN_UPSTREAM_FEATURE_COMBINATIONS: usize = 3;
const MIN_UPSTREAM_EXAMPLES: usize = 2;

const SUPPORTED_PROJECT_DISPOSITIONS: &[&str] = &[
    "kobo-owned",
    "formal_adapter",
    "generated_backend",
    "foreign_boundary",
];

const REQUIRED_PROOF_DEBT_REPORTS: &[&str] = &[
    "debt_summary",
    "proof_report",
    "replay_report",
    "inspect_output",
];

const DEFAULT_EVIDENCE_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_EVIDENCE_COMMAND_TIMEOUT_SECONDS: u64 = 1_800;

const REQUIRED_CLEAN_REPLACEMENT_GATES: &[&str] = &[
    "whole_application_source_semantics",
    "external_crates_replaced_or_formally_modeled",
    "platform_differences_replayable_or_declared",
    "upstream_behavior_and_release_workflow_equivalent_or_stronger",
    "two_independent_reviewers",
];

static EVIDENCE_COMMAND_CACHE: OnceLock<Mutex<BTreeMap<String, ObservedCommand>>> = OnceLock::new();

pub(super) fn cmd_project_support(
    root: &Path,
    output_format: DoctorOutputFormat,
    require_ready: bool,
) -> anyhow::Result<()> {
    let report = ProjectSupportReport::from_project(root);
    emit_project_support_report(&report, output_format)?;
    if require_ready && report.status != ProjectSupportStatus::Ready {
        anyhow::bail!("doctor project support gate blocked");
    }
    Ok(())
}

impl ProjectSupportReport {
    fn from_project(root: &Path) -> Self {
        let inventory = ProjectInventory::from_project(root);
        let evidence = ProjectSupportEvidence::read(root);
        let mut blockers = Vec::new();
        validate_manifest_exists(&evidence, &mut blockers);
        if let Some(manifest) = evidence.manifest.as_ref() {
            validate_manifest(root, manifest, &mut blockers);
        }
        let status = if blockers.is_empty() {
            ProjectSupportStatus::Ready
        } else {
            ProjectSupportStatus::Blocked
        };
        let claim = evidence
            .manifest
            .as_ref()
            .and_then(|manifest| manifest["claim"].as_str())
            .unwrap_or("project_support")
            .to_owned();
        Self {
            status,
            claim,
            replacement_claim: ReplacementClaimPosture::scoped_project_support(),
            inventory,
            evidence,
            blockers,
        }
    }

    fn to_json(&self) -> Value {
        serde_json::json!({
            "schema_version": 1,
            "command": "doctor --project-support",
            "project_support": {
                "status": self.status.as_str(),
                "claim": self.claim.as_str(),
                "replacement_claim": self.replacement_claim.to_json(),
                "manifest": self.evidence.manifest_path.as_ref().map(|path| path.display().to_string()),
                "inventory": self.inventory.to_json(),
                "blockers": self.blockers.iter().map(ProjectSupportBlocker::message).collect::<Vec<_>>(),
                "evidence": self.evidence.manifest.clone().unwrap_or(Value::Null),
            }
        })
    }
}

impl ReplacementClaimPosture {
    fn scoped_project_support() -> Self {
        Self {
            full_rewrite: ReplacementClaimState::Blocked,
            clean_replacement: ReplacementClaimState::Blocked,
            support_scope: "scoped_project_support",
            statement: "Kobo supports the project where the evidence covers the project; scoped dogfood is evidence of language capability, not a special-purpose language design.",
            required_before_clean_replacement: REQUIRED_CLEAN_REPLACEMENT_GATES,
        }
    }

    fn to_json(&self) -> Value {
        serde_json::json!({
            "full_rewrite": self.full_rewrite.as_str(),
            "clean_replacement": self.clean_replacement.as_str(),
            "support_scope": self.support_scope,
            "statement": self.statement,
            "required_before_clean_replacement": self.required_before_clean_replacement,
        })
    }
}

impl ProjectInventory {
    fn from_project(root: &Path) -> Self {
        let cargo_path = root.join("Cargo.toml");
        let cargo = std::fs::read_to_string(&cargo_path).unwrap_or_default();
        let parsed = cargo.parse::<TomlValue>().ok();
        Self {
            cargo_manifest: cargo_path.is_file(),
            kobo_manifest: root.join("Kobo.toml").is_file(),
            feature_flags: parsed.as_ref().map(feature_flags).unwrap_or_default(),
            workspace_members: parsed.as_ref().map(workspace_members).unwrap_or_default(),
            kobo_modules: collect_relative_files(root, root, "kobo"),
            rust_modules: collect_relative_files(root, root, "rs"),
            examples: collect_relative_files(root, &root.join("examples"), "kobo"),
            tests: collect_relative_files(root, &root.join("tests"), "kobo"),
            build_scripts: build_scripts(root, parsed.as_ref()),
            release_artifacts: collect_release_artifacts(root),
        }
    }

    fn to_json(&self) -> Value {
        serde_json::json!({
            "cargo_manifest": self.cargo_manifest,
            "kobo_manifest": self.kobo_manifest,
            "feature_flags": &self.feature_flags,
            "workspace_members": &self.workspace_members,
            "kobo_modules": &self.kobo_modules,
            "rust_modules": &self.rust_modules,
            "examples": &self.examples,
            "tests": &self.tests,
            "build_scripts": &self.build_scripts,
            "release_artifacts": &self.release_artifacts,
        })
    }
}

impl ProjectSupportEvidence {
    fn read(root: &Path) -> Self {
        let manifest_path = root.join(".kobo").join("project-support.json");
        if !manifest_path.is_file() {
            return Self {
                manifest_path: None,
                manifest: None,
            };
        }
        let manifest = read_json(&manifest_path).ok();
        Self {
            manifest_path: Some(manifest_path),
            manifest,
        }
    }
}

impl ProjectSupportBlocker {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn message(&self) -> &str {
        &self.message
    }
}

impl ProjectSupportStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Blocked => "blocked",
        }
    }
}

impl ReplacementClaimState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
        }
    }
}

impl AdapterDependencyIndex {
    fn from_cargo_metadata(metadata: Option<&Value>) -> Self {
        let mut index = Self::default();
        let Some(metadata) = metadata else {
            return index;
        };
        for package in metadata["packages"].as_array().into_iter().flatten() {
            if let Some(name) = package["name"].as_str() {
                index.insert_features(name, package_feature_names(package));
            }
            for dependency in package["dependencies"].as_array().into_iter().flatten() {
                let features = metadata_dependency_features(dependency);
                if let Some(name) = dependency["name"].as_str() {
                    index.insert_features(name, features.clone());
                }
                if let Some(rename) = dependency["rename"].as_str() {
                    index.insert_features(rename, features);
                }
            }
        }
        index
    }

    fn features_for(&self, name: &str) -> Option<&BTreeSet<String>> {
        self.features_by_name.get(name)
    }

    fn insert_features(&mut self, name: &str, features: BTreeSet<String>) {
        self.features_by_name
            .entry(name.to_owned())
            .or_default()
            .extend(features);
    }
}

fn emit_project_support_report(
    report: &ProjectSupportReport,
    output_format: DoctorOutputFormat,
) -> anyhow::Result<()> {
    match output_format {
        DoctorOutputFormat::Json => {
            let mut value = report.to_json();
            if report.status != ProjectSupportStatus::Ready {
                value["message"] = Value::from("doctor project support gate blocked");
            }
            println!("{}", serde_json::to_string(&value)?);
        }
        DoctorOutputFormat::Human => {
            println!("doctor --project-support: {}", report.status.as_str());
            println!("claim: {}", report.claim);
            println!(
                "full rewrite: {}",
                report.replacement_claim.full_rewrite.as_str()
            );
            println!(
                "clean replacement: {}",
                report.replacement_claim.clean_replacement.as_str()
            );
            println!("{}", report.replacement_claim.statement);
            if report.blockers.is_empty() {
                println!("blockers: none");
            } else {
                println!("blockers:");
                for blocker in &report.blockers {
                    println!("  - {}", blocker.message());
                }
            }
        }
    }
    Ok(())
}

fn validate_manifest_exists(
    evidence: &ProjectSupportEvidence,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    if evidence.manifest_path.is_none() {
        blockers.push(ProjectSupportBlocker::new(
            "missing .kobo/project-support.json",
        ));
    } else if evidence.manifest.is_none() {
        blockers.push(ProjectSupportBlocker::new(
            "invalid .kobo/project-support.json",
        ));
    }
}

fn validate_manifest(root: &Path, manifest: &Value, blockers: &mut Vec<ProjectSupportBlocker>) {
    validate_schema(manifest, blockers);
    let upstream = validate_upstream_inventory(root, manifest, blockers);
    validate_language_surface(root, manifest, blockers);
    validate_platform_models(root, manifest, blockers);
    validate_adapters(
        root,
        manifest,
        upstream.as_ref().map(|evidence| &evidence.cargo_metadata),
        blockers,
    );
    validate_async_runtime(root, manifest, blockers);
    validate_generated_backend(root, manifest, blockers);
    validate_test_release_parity(
        root,
        manifest,
        upstream.as_ref().map(|evidence| evidence.root.as_path()),
        blockers,
    );
    validate_proof_debt_map(root, manifest, blockers);
    validate_independent_equivalence(root, manifest, blockers);
}

fn validate_schema(manifest: &Value, blockers: &mut Vec<ProjectSupportBlocker>) {
    if manifest["schema_version"].as_u64() != Some(1) {
        blockers.push(ProjectSupportBlocker::new(
            "project support schema_version must be 1",
        ));
    }
    if manifest["claim"].as_str() != Some("project_support") {
        blockers.push(ProjectSupportBlocker::new(
            "project support claim must be project_support",
        ));
    }
    validate_replacement_claim_posture(manifest, blockers);
}

fn validate_replacement_claim_posture(manifest: &Value, blockers: &mut Vec<ProjectSupportBlocker>) {
    let replacement_claim = &manifest["replacement_claim"];
    if replacement_claim.is_null() {
        return;
    }
    if replacement_claim["full_rewrite"].as_str() != Some("blocked") {
        blockers.push(ProjectSupportBlocker::new(
            "project support full rewrite claim must remain blocked",
        ));
    }
    if replacement_claim["clean_replacement"].as_str() != Some("blocked") {
        blockers.push(ProjectSupportBlocker::new(
            "project support clean replacement claim must remain blocked",
        ));
    }
}

fn validate_language_surface(
    root: &Path,
    manifest: &Value,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let surface = &manifest["language_surface"];
    require_set(
        surface,
        "constructs",
        REQUIRED_LANGUAGE_CONSTRUCTS,
        |missing| format!("missing language construct {missing}"),
        blockers,
    );
    for field in REQUIRED_LANGUAGE_FLAGS {
        if surface[*field].as_bool() != Some(true) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "language surface {field} is not proven"
            )));
        }
    }
    require_non_empty_array(
        surface,
        "feature_matrix",
        "language feature_matrix",
        blockers,
    );
    require_feature_matrix_coverage(
        &manifest["upstream_inventory"]["feature_combinations"],
        &surface["feature_matrix"],
        "language feature_matrix",
        blockers,
    );
    if let Some(evidence) = require_evidence_document(
        root,
        surface,
        "evidence_path",
        "language_surface",
        None,
        blockers,
    ) {
        require_evidence_checks(
            &evidence,
            REQUIRED_LANGUAGE_FLAGS,
            "language_surface",
            blockers,
        );
        require_evidence_covers_inventory_paths(
            &evidence,
            &manifest["upstream_inventory"],
            "modules",
            "language_surface",
            blockers,
        );
    }
}

fn validate_upstream_inventory(
    root: &Path,
    manifest: &Value,
    blockers: &mut Vec<ProjectSupportBlocker>,
) -> Option<UpstreamProjectEvidence> {
    let inventory = &manifest["upstream_inventory"];
    if !inventory.is_object() {
        blockers.push(ProjectSupportBlocker::new(
            "missing upstream inventory evidence",
        ));
        return None;
    }

    let Some(upstream_root) = inventory["root"].as_str() else {
        blockers.push(ProjectSupportBlocker::new(
            "missing upstream inventory root",
        ));
        return None;
    };
    let upstream_root_path = project_path(root, upstream_root);
    if !upstream_root_path.is_dir() {
        blockers.push(ProjectSupportBlocker::new(format!(
            "upstream inventory root does not exist: {upstream_root}"
        )));
        return None;
    }
    validate_upstream_identity(root, &upstream_root_path, inventory, blockers);

    require_evidence_document(
        root,
        inventory,
        "evidence_path",
        "upstream_inventory",
        None,
        blockers,
    );
    require_inventory_path(
        &upstream_root_path,
        inventory,
        "workspace_manifest",
        "upstream workspace manifest",
        blockers,
    );
    let cargo_metadata = validate_upstream_cargo_metadata(&upstream_root_path, inventory, blockers);
    for field in REQUIRED_UPSTREAM_INVENTORY_FIELDS {
        require_non_empty_array(
            inventory,
            field,
            &format!("upstream inventory {field}"),
            blockers,
        );
    }
    require_min_array(
        inventory,
        "crate_tree",
        MIN_UPSTREAM_CRATE_PATHS,
        "upstream inventory crate_tree",
        blockers,
    );
    require_min_array(
        inventory,
        "modules",
        MIN_UPSTREAM_MODULES,
        "upstream inventory modules",
        blockers,
    );
    require_min_array(
        inventory,
        "public_types",
        MIN_UPSTREAM_PUBLIC_TYPES,
        "upstream inventory public_types",
        blockers,
    );
    require_min_array(
        inventory,
        "cli_surfaces",
        MIN_UPSTREAM_CLI_SURFACES,
        "upstream inventory cli_surfaces",
        blockers,
    );
    require_min_array(
        inventory,
        "test_fixtures",
        MIN_UPSTREAM_TEST_FIXTURES,
        "upstream inventory test_fixtures",
        blockers,
    );
    require_min_array(
        inventory,
        "platform_paths",
        MIN_UPSTREAM_PLATFORM_PATHS,
        "upstream inventory platform_paths",
        blockers,
    );
    require_min_array(
        inventory,
        "feature_combinations",
        MIN_UPSTREAM_FEATURE_COMBINATIONS,
        "upstream inventory feature_combinations",
        blockers,
    );
    require_min_array(
        inventory,
        "examples",
        MIN_UPSTREAM_EXAMPLES,
        "upstream inventory examples",
        blockers,
    );
    for field in [
        "crate_tree",
        "modules",
        "test_fixtures",
        "examples",
        "build_scripts",
        "release_artifacts",
    ] {
        for path in inventory[field].as_array().into_iter().flatten() {
            require_inventory_path_value(
                &upstream_root_path,
                path,
                &format!("upstream inventory {field}"),
                blockers,
            );
        }
    }
    for platform_path in inventory["platform_paths"].as_array().into_iter().flatten() {
        require_inventory_path_field(
            &upstream_root_path,
            platform_path,
            "path",
            "upstream platform path",
            blockers,
        );
        let disposition = platform_path["disposition"].as_str().unwrap_or("missing");
        if !SUPPORTED_PROJECT_DISPOSITIONS.contains(&disposition) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "unsupported platform path disposition: {disposition}"
            )));
        }
        if platform_path["correctness_relevant"].as_bool() == Some(true)
            && platform_path["source_preserved"].as_bool() != Some(true)
        {
            let path = platform_path["path"].as_str().unwrap_or("<unknown>");
            blockers.push(ProjectSupportBlocker::new(format!(
                "correctness-relevant platform path is not preserved: {path}"
            )));
        }
    }
    cargo_metadata.map(|cargo_metadata| UpstreamProjectEvidence {
        root: upstream_root_path,
        cargo_metadata,
    })
}

fn validate_upstream_identity(
    root: &Path,
    upstream_root: &Path,
    inventory: &Value,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    if inventory["project_name"].as_str() != Some("watchexec") {
        blockers.push(ProjectSupportBlocker::new(
            "upstream inventory project_name must be watchexec",
        ));
    }
    if inventory["source_revision"]
        .as_str()
        .map_or(true, |revision| revision.trim().is_empty())
    {
        blockers.push(ProjectSupportBlocker::new(
            "upstream inventory source_revision is missing",
        ));
    }
    let Ok(project_root) = root.canonicalize() else {
        return;
    };
    let Ok(upstream_root) = upstream_root.canonicalize() else {
        return;
    };
    if upstream_root == project_root {
        blockers.push(ProjectSupportBlocker::new(
            "upstream inventory root must not be the Kobo project root",
        ));
    }
}

fn validate_upstream_cargo_metadata(
    upstream_root: &Path,
    inventory: &Value,
    blockers: &mut Vec<ProjectSupportBlocker>,
) -> Option<Value> {
    let workspace_manifest = inventory["workspace_manifest"]
        .as_str()
        .unwrap_or("Cargo.toml");
    let manifest_path = upstream_root.join(workspace_manifest);
    let Some(metadata) = read_cargo_metadata(&manifest_path, blockers) else {
        return None;
    };

    validate_upstream_package_names(&metadata, blockers);
    let target_paths = metadata_workspace_target_paths(upstream_root, &metadata, blockers);
    if target_paths.is_empty() {
        blockers.push(ProjectSupportBlocker::new(
            "upstream cargo metadata did not expose workspace targets",
        ));
    }
    for path in target_paths {
        if !inventory_declares_path(inventory, &path) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "upstream inventory missing Cargo metadata target {path}"
            )));
        }
    }

    let observed_feature_matrix = feature_matrix_signatures(&inventory["feature_combinations"]);
    for feature in metadata_workspace_features(&metadata) {
        if !feature_matrix_contains_feature(&observed_feature_matrix, &feature) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "upstream feature combination missing Cargo feature {feature}"
            )));
        }
    }
    Some(metadata)
}

fn validate_upstream_package_names(metadata: &Value, blockers: &mut Vec<ProjectSupportBlocker>) {
    let package_names = metadata_package_names(metadata);
    for required in REQUIRED_UPSTREAM_PACKAGE_NAMES {
        if !package_names.contains(*required) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "upstream Cargo metadata missing package {required}"
            )));
        }
    }
}

fn read_cargo_metadata(
    manifest_path: &Path,
    blockers: &mut Vec<ProjectSupportBlocker>,
) -> Option<Value> {
    let output = match Command::new("cargo")
        .arg("metadata")
        .arg("--no-deps")
        .arg("--format-version")
        .arg("1")
        .arg("--manifest-path")
        .arg(manifest_path)
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            blockers.push(ProjectSupportBlocker::new(format!(
                "failed to run cargo metadata for upstream inventory: {error}"
            )));
            return None;
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        blockers.push(ProjectSupportBlocker::new(format!(
            "upstream cargo metadata failed: {}",
            stderr.trim()
        )));
        return None;
    }
    match serde_json::from_slice(&output.stdout) {
        Ok(metadata) => Some(metadata),
        Err(error) => {
            blockers.push(ProjectSupportBlocker::new(format!(
                "upstream cargo metadata was not valid JSON: {error}"
            )));
            None
        }
    }
}

fn metadata_package_names(metadata: &Value) -> BTreeSet<&str> {
    metadata["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|package| package["name"].as_str())
        .collect()
}

fn package_feature_names(package: &Value) -> BTreeSet<String> {
    package["features"]
        .as_object()
        .map(|features| features.keys().cloned().collect())
        .unwrap_or_default()
}

fn metadata_dependency_features(dependency: &Value) -> BTreeSet<String> {
    let mut features = dependency["features"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if dependency["uses_default_features"].as_bool() == Some(true) {
        features.insert("default".to_owned());
    }
    features
}

fn metadata_workspace_target_paths(
    upstream_root: &Path,
    metadata: &Value,
    blockers: &mut Vec<ProjectSupportBlocker>,
) -> BTreeSet<String> {
    let workspace_members = string_set(&metadata["workspace_members"]);
    let mut paths = BTreeSet::new();
    let Some(packages) = metadata["packages"].as_array() else {
        blockers.push(ProjectSupportBlocker::new(
            "upstream cargo metadata missing packages",
        ));
        return paths;
    };
    for package in packages {
        let package_id = package["id"].as_str().unwrap_or("");
        if !workspace_members.is_empty() && !workspace_members.contains(package_id) {
            continue;
        }
        for target in package["targets"].as_array().into_iter().flatten() {
            let Some(src_path) = target["src_path"].as_str() else {
                continue;
            };
            if let Some(relative) = metadata_relative_path(upstream_root, src_path) {
                paths.insert(relative);
            }
        }
    }
    paths
}

fn metadata_workspace_features(metadata: &Value) -> BTreeSet<String> {
    let workspace_members = string_set(&metadata["workspace_members"]);
    let mut features = BTreeSet::new();
    for package in metadata["packages"].as_array().into_iter().flatten() {
        let package_id = package["id"].as_str().unwrap_or("");
        if !workspace_members.is_empty() && !workspace_members.contains(package_id) {
            continue;
        }
        if let Some(table) = package["features"].as_object() {
            features.extend(table.keys().cloned());
        }
    }
    features
}

fn metadata_relative_path(root: &Path, source_path: &str) -> Option<String> {
    let root = root.canonicalize().ok()?;
    let source_path = Path::new(source_path).canonicalize().ok()?;
    Some(relative_path(&root, &source_path))
}

fn inventory_declares_path(inventory: &Value, path: &str) -> bool {
    [
        "crate_tree",
        "modules",
        "test_fixtures",
        "examples",
        "build_scripts",
    ]
    .iter()
    .any(|field| {
        inventory[*field]
            .as_array()
            .into_iter()
            .flatten()
            .any(|value| value.as_str() == Some(path))
    })
}

fn validate_platform_models(
    root: &Path,
    manifest: &Value,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let models = manifest["platform_models"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for required_kind in REQUIRED_PLATFORM_MODELS {
        let Some(model) = find_object_by_kind(&models, required_kind) else {
            blockers.push(ProjectSupportBlocker::new(format!(
                "missing platform model {required_kind}"
            )));
            continue;
        };
        require_set(
            model,
            "platforms",
            REQUIRED_PLATFORMS,
            |platform| format!("platform model {required_kind} missing {platform}"),
            blockers,
        );
        require_non_empty_array(
            model,
            "behaviors",
            &format!("platform model {required_kind} behaviors"),
            blockers,
        );
        if let Some(evidence) = require_evidence_document(
            root,
            model,
            "evidence_path",
            "platform_model",
            Some(required_kind),
            blockers,
        ) {
            require_non_empty_array(
                &evidence,
                "behavior_tests",
                &format!("platform model {required_kind} behavior_tests"),
                blockers,
            );
            require_set(
                &evidence,
                "tested_platforms",
                REQUIRED_PLATFORMS,
                |platform| {
                    format!("platform model {required_kind} evidence missing {platform} test")
                },
                blockers,
            );
        }
        let replay_grade = model["replay_grade"].as_str().unwrap_or("missing");
        if matches!(
            replay_grade,
            "opaque" | "debt" | "missing" | "metadata-only"
        ) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "critical platform model {required_kind} is {replay_grade}"
            )));
        }
    }
}

fn validate_adapters(
    root: &Path,
    manifest: &Value,
    upstream_metadata: Option<&Value>,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let adapters = manifest["adapters"].as_array().cloned().unwrap_or_default();
    let cargo = read_project_cargo(root);
    let upstream_dependencies = AdapterDependencyIndex::from_cargo_metadata(upstream_metadata);
    for required_kind in REQUIRED_ADAPTERS {
        let Some(adapter) = find_object_by_kind(&adapters, required_kind) else {
            blockers.push(ProjectSupportBlocker::new(format!(
                "missing critical adapter {required_kind}"
            )));
            continue;
        };
        require_non_empty_str(
            adapter,
            "name",
            &format!("adapter {required_kind}"),
            blockers,
        );
        require_non_empty_str(
            adapter,
            "version_range",
            &format!("adapter {required_kind}"),
            blockers,
        );
        require_non_empty_array(
            adapter,
            "cargo_features",
            &format!("adapter {required_kind} cargo_features"),
            blockers,
        );
        require_non_empty_array(
            adapter,
            "modeled_facts",
            &format!("adapter {required_kind} modeled_facts"),
            blockers,
        );
        require_non_empty_array(
            adapter,
            "unsupported_guarantees",
            &format!("adapter {required_kind} unsupported_guarantees"),
            blockers,
        );
        require_non_empty_array(
            adapter,
            "conformance_tests",
            &format!("adapter {required_kind} conformance_tests"),
            blockers,
        );
        if adapter["summary_version"].as_u64() != Some(1) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "adapter {required_kind} summary_version is stale"
            )));
        }
        if adapter["stale_summary_detection"].as_bool() != Some(true) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "adapter {required_kind} lacks stale summary detection"
            )));
        }
        validate_adapter_source(
            root,
            cargo.as_ref(),
            &upstream_dependencies,
            adapter,
            required_kind,
            blockers,
        );
        if let Some(evidence) = require_evidence_document(
            root,
            adapter,
            "replay_evidence",
            "adapter_summary",
            Some(required_kind),
            blockers,
        ) {
            require_non_empty_array(
                &evidence,
                "conformance_results",
                &format!("adapter {required_kind} conformance_results"),
                blockers,
            );
            if evidence["stale_check"]["status"].as_str() != Some("passed") {
                blockers.push(ProjectSupportBlocker::new(format!(
                    "adapter {required_kind} stale check did not pass"
                )));
            }
            require_evidence_feature_coverage(
                &evidence,
                &adapter["cargo_features"],
                &format!("adapter {required_kind}"),
                blockers,
            );
        }
    }
}

fn validate_adapter_source(
    root: &Path,
    cargo: Option<&TomlValue>,
    upstream_dependencies: &AdapterDependencyIndex,
    adapter: &Value,
    required_kind: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let source = &adapter["crate_source"];
    let source_kind = source["kind"].as_str().unwrap_or("missing");
    match source_kind {
        "cargo_dependency" => validate_cargo_adapter_source(
            cargo,
            upstream_dependencies,
            adapter,
            source,
            required_kind,
            blockers,
        ),
        "std" => {
            if source["name"].as_str().unwrap_or("").trim().is_empty() {
                blockers.push(ProjectSupportBlocker::new(format!(
                    "adapter {required_kind} std source missing name"
                )));
            }
        }
        "project_module" => {
            let Some(path) = source["path"].as_str() else {
                blockers.push(ProjectSupportBlocker::new(format!(
                    "adapter {required_kind} project_module source missing path"
                )));
                return;
            };
            if !project_path(root, path).is_file() {
                blockers.push(ProjectSupportBlocker::new(format!(
                    "adapter {required_kind} project_module source does not exist: {path}"
                )));
            }
        }
        _ => blockers.push(ProjectSupportBlocker::new(format!(
            "adapter {required_kind} source kind is unsupported: {source_kind}"
        ))),
    }
}

fn validate_cargo_adapter_source(
    cargo: Option<&TomlValue>,
    upstream_dependencies: &AdapterDependencyIndex,
    adapter: &Value,
    source: &Value,
    required_kind: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let dependency_name = source["name"]
        .as_str()
        .or_else(|| adapter["name"].as_str())
        .unwrap_or("");
    let Some(declared_features) =
        adapter_dependency_features(cargo, upstream_dependencies, dependency_name)
    else {
        blockers.push(ProjectSupportBlocker::new(format!(
            "adapter {required_kind} cargo dependency is not declared: {dependency_name}"
        )));
        return;
    };
    for feature in adapter["cargo_features"].as_array().into_iter().flatten() {
        let Some(feature) = feature.as_str() else {
            continue;
        };
        if feature == "default" {
            continue;
        }
        if !declared_features.contains(feature) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "adapter {required_kind} feature {feature} is not enabled on {dependency_name}"
            )));
        }
    }
}

fn validate_async_runtime(
    root: &Path,
    manifest: &Value,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let runtime = &manifest["async_runtime"];
    let model = runtime["model"].as_str().unwrap_or("");
    if !matches!(model, "native_kobo_runtime" | "versioned_tokio_adapter") {
        blockers.push(ProjectSupportBlocker::new(
            "async runtime model must be native_kobo_runtime or versioned_tokio_adapter",
        ));
    }
    require_set(
        runtime,
        "semantics",
        REQUIRED_ASYNC_SEMANTICS,
        |semantic| format!("missing async semantic {semantic}"),
        blockers,
    );
    require_non_empty_array(
        runtime,
        "scheduler_facts",
        "async scheduler_facts",
        blockers,
    );
    require_set(
        runtime,
        "mutation_checks",
        REQUIRED_ASYNC_MUTATIONS,
        |mutation| format!("missing async mutation check {mutation}"),
        blockers,
    );
    if let Some(evidence) = require_evidence_document(
        root,
        runtime,
        "conformance_evidence",
        "async_runtime",
        Some("async_runtime"),
        blockers,
    ) {
        require_set(
            &evidence,
            "mutation_results",
            REQUIRED_ASYNC_MUTATIONS,
            |mutation| format!("missing async conformance mutation result {mutation}"),
            blockers,
        );
        require_non_empty_array(
            &evidence,
            "scheduler_facts",
            "async evidence scheduler_facts",
            blockers,
        );
    }
}

fn validate_generated_backend(
    root: &Path,
    manifest: &Value,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let backend = &manifest["generated_backend"];
    for field in REQUIRED_BACKEND_FLAGS {
        if backend[*field].as_bool() != Some(true) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "generated backend {field} is not proven"
            )));
        }
    }
    require_set(
        backend,
        "targets",
        REQUIRED_PLATFORMS,
        |target| format!("generated backend missing target {target}"),
        blockers,
    );
    require_evidence_document(
        root,
        backend,
        "evidence_path",
        "generated_backend",
        None,
        blockers,
    );
}

fn validate_test_release_parity(
    root: &Path,
    manifest: &Value,
    upstream_root: Option<&Path>,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let parity = &manifest["test_release_parity"];
    for field in REQUIRED_PARITY_PATHS {
        require_evidence_document(
            root,
            parity,
            field,
            "test_release_parity",
            Some(field),
            blockers,
        );
    }
    let performance = &parity["performance"];
    for field in REQUIRED_PERFORMANCE_FIELDS {
        require_evidence_document(
            root,
            performance,
            field,
            "performance",
            Some(field),
            blockers,
        );
    }
    let artifacts = parity["release_artifacts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if artifacts.is_empty() {
        blockers.push(ProjectSupportBlocker::new(
            "missing release artifact evidence",
        ));
    }
    for artifact in artifacts {
        require_release_artifact_path(root, upstream_root, &artifact, blockers);
    }
}

fn validate_proof_debt_map(
    root: &Path,
    manifest: &Value,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let entries = manifest["proof_debt_map"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if entries.is_empty() {
        blockers.push(ProjectSupportBlocker::new("missing proof debt map"));
        return;
    }
    for entry in &entries {
        let module = entry["module"].as_str().unwrap_or("<unknown>");
        let classification = entry["classification"].as_str().unwrap_or("missing");
        let criticality = entry["criticality"].as_str().unwrap_or("missing");
        if criticality == "correctness-critical"
            && matches!(
                classification,
                "debt" | "opaque" | "metadata-only" | "missing"
            )
        {
            blockers.push(ProjectSupportBlocker::new(format!(
                "critical module {module} is classified as {classification}"
            )));
        }
        if criticality == "non-critical"
            && classification == "debt"
            && entry["justification"]
                .as_str()
                .unwrap_or("")
                .trim()
                .is_empty()
        {
            blockers.push(ProjectSupportBlocker::new(format!(
                "non-critical debt {module} lacks justification"
            )));
        }
    }
    require_complete_proof_debt_map(manifest, &entries, blockers);
    let reports = &manifest["proof_debt_map_reports"];
    if !reports.is_object() {
        blockers.push(ProjectSupportBlocker::new(
            "missing proof debt report agreement evidence",
        ));
        return;
    }
    for report in REQUIRED_PROOF_DEBT_REPORTS {
        if let Some(evidence) = require_evidence_document(
            root,
            reports,
            report,
            "proof_debt_report",
            Some(report),
            blockers,
        ) {
            require_evidence_covers_debt_modules(&evidence, &entries, report, blockers);
            if evidence["decision"].as_str() != Some("same_project_map") {
                blockers.push(ProjectSupportBlocker::new(format!(
                    "{report} does not agree with the proof debt map"
                )));
            }
        }
    }
}

fn require_complete_proof_debt_map(
    manifest: &Value,
    entries: &[Value],
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let classified_modules = entries
        .iter()
        .filter_map(|entry| entry["module"].as_str())
        .collect::<BTreeSet<_>>();
    for module in manifest["upstream_inventory"]["modules"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let Some(module) = module.as_str() else {
            continue;
        };
        if !classified_modules.contains(module) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "proof debt map missing upstream module {module}"
            )));
        }
    }
}

fn require_evidence_covers_debt_modules(
    evidence: &Value,
    entries: &[Value],
    report: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let covered_modules = string_set(&evidence["covered_modules"]);
    for entry in entries {
        let Some(module) = entry["module"].as_str() else {
            continue;
        };
        if !covered_modules.contains(module) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{report} evidence does not cover proof debt module {module}"
            )));
        }
    }
}

fn validate_independent_equivalence(
    root: &Path,
    manifest: &Value,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let equivalence = &manifest["independent_equivalence"];
    require_evidence_document(
        root,
        equivalence,
        "original_upstream_tests",
        "upstream_tests",
        None,
        blockers,
    );
    require_set(
        equivalence,
        "mutation_tests",
        REQUIRED_MUTATION_TESTS,
        |mutation| format!("missing equivalence mutation test {mutation}"),
        blockers,
    );
    let reviewer_reports = equivalence["reviewer_reports"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if reviewer_reports.len() < 2 {
        blockers.push(ProjectSupportBlocker::new(
            "independent equivalence review requires two reviewer reports",
        ));
    }
    let mut reviewers = BTreeSet::new();
    let mut report_paths = BTreeSet::new();
    for report in reviewer_reports {
        let reviewer = report["reviewer"].as_str().unwrap_or("").trim();
        if reviewer.is_empty() {
            blockers.push(ProjectSupportBlocker::new(
                "reviewer report missing reviewer name",
            ));
        } else if !reviewers.insert(reviewer.to_owned()) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "duplicate reviewer report: {reviewer}"
            )));
        }
        if let Some(path) = report["evidence_path"].as_str() {
            if !report_paths.insert(path.to_owned()) {
                blockers.push(ProjectSupportBlocker::new(format!(
                    "reviewer reports must use distinct evidence paths: {path}"
                )));
            }
        }
        let expected_subject = if reviewer.is_empty() {
            None
        } else {
            Some(reviewer)
        };
        if let Some(evidence) = require_evidence_document(
            root,
            &report,
            "evidence_path",
            "reviewer_report",
            expected_subject,
            blockers,
        ) {
            if evidence["decision"].as_str() != Some("equivalent_or_stronger") {
                blockers.push(ProjectSupportBlocker::new(format!(
                    "reviewer {reviewer} did not approve equivalence"
                )));
            }
        }
    }
}

fn require_set(
    value: &Value,
    field: &str,
    required: &[&str],
    message: impl Fn(&str) -> String,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let observed = string_set(&value[field]);
    for item in required {
        if !observed.contains(*item) {
            blockers.push(ProjectSupportBlocker::new(message(item)));
        }
    }
}

fn require_non_empty_str(
    value: &Value,
    field: &str,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    if value[field].as_str().unwrap_or("").trim().is_empty() {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} has empty {field}"
        )));
    }
}

fn require_non_empty_array(
    value: &Value,
    field: &str,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    if value[field].as_array().map_or(true, Vec::is_empty) {
        blockers.push(ProjectSupportBlocker::new(format!("{label} is empty")));
    }
}

fn require_min_array(
    value: &Value,
    field: &str,
    minimum: usize,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let count = value[field].as_array().map_or(0, Vec::len);
    if count < minimum {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} needs at least {minimum} entries, found {count}"
        )));
    }
}

fn require_evidence_document(
    root: &Path,
    value: &Value,
    field: &str,
    expected_kind: &str,
    expected_subject: Option<&str>,
    blockers: &mut Vec<ProjectSupportBlocker>,
) -> Option<Value> {
    let Some(path) = value[field].as_str() else {
        blockers.push(ProjectSupportBlocker::new(format!(
            "missing evidence path {field}"
        )));
        return None;
    };
    let path = require_non_empty_file(root, path, field, blockers)?;
    let document = match read_json(&path) {
        Ok(document) => document,
        Err(error) => {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{field} evidence is not valid JSON: {error}"
            )));
            return None;
        }
    };
    validate_evidence_document(
        root,
        &document,
        expected_kind,
        expected_subject,
        field,
        blockers,
    );
    Some(document)
}

fn validate_evidence_document(
    root: &Path,
    document: &Value,
    expected_kind: &str,
    expected_subject: Option<&str>,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    if !document.is_object() || document.as_object().is_some_and(serde_json::Map::is_empty) {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence must be a non-empty JSON object"
        )));
    }
    if document["schema_version"].as_u64() != Some(1) {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence schema_version must be 1"
        )));
    }
    if document["evidence_kind"].as_str() != Some(expected_kind) {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence_kind must be {expected_kind}"
        )));
    }
    if let Some(expected_subject) = expected_subject {
        let subject = document["subject"]
            .as_str()
            .or_else(|| document["kind"].as_str())
            .unwrap_or("");
        if subject != expected_subject {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{label} evidence subject must be {expected_subject}"
            )));
        }
    }
    require_subject_coverage(document, expected_kind, expected_subject, label, blockers);
    require_covered_paths_exist(root, document, label, blockers);
    let commands = document["commands"].as_array();
    if commands.map_or(true, Vec::is_empty) {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence must include command results"
        )));
    }
    let target_name = evidence_target_name(expected_kind, expected_subject);
    let mut has_kind_command = false;
    let mut has_subject_command = false;
    for command in commands.into_iter().flatten() {
        let command_text = command["command"].as_str().unwrap_or("").trim();
        let output_hash = command["output_hash"].as_str().unwrap_or("").trim();
        let transcript_evidence =
            validate_command_transcript(root, command, command_text, output_hash, label, blockers);
        if let Some(argv) = command_argv(command, label, blockers) {
            if evidence_command_proves_kind(expected_kind, &argv) {
                has_kind_command = true;
                if evidence_command_declares_target(command, &target_name) {
                    has_subject_command = true;
                    if let Some(transcript_evidence) = transcript_evidence.as_ref() {
                        validate_command_proof_markers(
                            command,
                            transcript_evidence,
                            expected_kind,
                            &target_name,
                            label,
                            blockers,
                        );
                        validate_subject_command_evidence(
                            root,
                            document,
                            command,
                            &argv,
                            transcript_evidence,
                            expected_kind,
                            &target_name,
                            label,
                            blockers,
                        );
                    }
                }
            }
        }
        if command_text.is_empty() {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{label} evidence command is empty"
            )));
        }
        if command["status"].as_str() != Some("passed") {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{label} evidence command did not pass"
            )));
        }
        if output_hash.is_empty() {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{label} evidence command missing output_hash"
            )));
        }
    }
    if !has_kind_command {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence command does not prove {}",
            target_name
        )));
    } else if !has_subject_command {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence command missing proves entry for {target_name}"
        )));
    }
    if expected_kind == "performance" {
        require_non_empty_array(document, "measurements", label, blockers);
        validate_performance_measurements(document, &target_name, label, blockers);
    }
    validate_structured_future_evidence(document, expected_kind, &target_name, label, blockers);
    if expected_kind == "reviewer_report" {
        validate_reviewer_report_document(document, &target_name, label, blockers);
    }
}

fn require_subject_coverage(
    document: &Value,
    expected_kind: &str,
    expected_subject: Option<&str>,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let Some(expected_subject) = expected_subject else {
        return;
    };
    let Some(required_paths) = required_subject_paths(expected_kind, expected_subject) else {
        return;
    };
    let covered_paths = string_set(&document["covered_paths"]);
    for required_path in required_paths {
        let has_subject_path = covered_paths
            .iter()
            .any(|path| path.contains(required_path));
        if !has_subject_path {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{label} evidence does not cover {expected_subject} subject path {required_path}"
            )));
        }
    }
}

fn require_covered_paths_exist(
    root: &Path,
    document: &Value,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let base = evidence_source_base(root, document);
    for covered_path in document["covered_paths"].as_array().into_iter().flatten() {
        let Some(covered_path) = covered_path.as_str() else {
            continue;
        };
        if covered_path.trim().is_empty() {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{label} evidence covered path is empty"
            )));
            continue;
        }
        if !project_path(&base, covered_path).exists() {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{label} evidence covered path does not exist: {covered_path}"
            )));
        }
    }
}

fn evidence_source_base(root: &Path, document: &Value) -> PathBuf {
    if let Some(upstream_root) = document["upstream_root"].as_str() {
        return project_path(root, upstream_root);
    }
    document["commands"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|command| command["cwd"].as_str())
        .map(|cwd| project_path(root, cwd))
        .unwrap_or_else(|| root.to_path_buf())
}

fn required_subject_paths(
    expected_kind: &str,
    expected_subject: &str,
) -> Option<&'static [&'static str]> {
    match (expected_kind, expected_subject) {
        ("platform_model", "filesystem_events") => Some(&[
            "crates/events/src/fs.rs",
            "crates/lib/src/sources/fs.rs",
            "crates/lib/src/watchexec.rs",
        ]),
        ("platform_model", "watcher_backend") => Some(&[
            "crates/lib/src/watchexec.rs",
            "crates/lib/src/sources/fs.rs",
        ]),
        ("platform_model", "paths") => Some(&["crates/lib/src/paths.rs"]),
        ("platform_model", "process_execution") => {
            Some(&["crates/supervisor/src", "crates/lib/src/action"])
        }
        ("platform_model", "signals") => {
            Some(&["crates/signals/src", "crates/lib/src/sources/signal.rs"])
        }
        ("platform_model", "process_groups") => Some(&["crates/supervisor/src/command.rs"]),
        ("platform_model", "environment_variables") => Some(&["crates/cli/src/config.rs"]),
        ("platform_model", "terminal_io") => Some(&["crates/cli/src/lib.rs"]),
        ("platform_model", "stdio") => Some(&["crates/cli/src/lib.rs"]),
        ("platform_model", "timers") => Some(&[
            "crates/lib/src/action/worker.rs",
            "crates/cli/src/config.rs",
        ]),
        ("adapter_summary", "watcher_backend") => Some(&["crates/lib/src/sources/fs.rs"]),
        ("adapter_summary", "async_runtime") => Some(&["crates/lib/src/late_join_set.rs"]),
        ("adapter_summary", "process_handling") => Some(&["crates/supervisor/src/command.rs"]),
        ("adapter_summary", "signal_handling") => Some(&["crates/signals/src"]),
        ("adapter_summary", "ignore_path") => Some(&["crates/ignore-files/src"]),
        ("adapter_summary", "config") => Some(&["crates/cli/src/config.rs"]),
        ("adapter_summary", "cli") => Some(&["crates/cli/src/args"]),
        ("adapter_summary", "shell_parsing") => Some(&["crates/supervisor/src/command"]),
        ("adapter_summary", "serialization") => Some(&["crates/events/src/serde_formats.rs"]),
        ("adapter_summary", "logging_tracing") => Some(&["crates/cli/src/args/logging.rs"]),
        ("adapter_summary", "terminal_helpers") => Some(&["crates/cli/src"]),
        ("adapter_summary", "errors") => Some(&["crates/supervisor/src/errors.rs"]),
        ("async_runtime", "async_runtime") | ("async_runtime", "async") => Some(&[
            "crates/lib/src/late_join_set.rs",
            "crates/lib/src/action/worker.rs",
        ]),
        ("test_release_parity", "upstream_tests")
        | ("upstream_tests", "upstream_tests")
        | ("upstream_tests", "upstream") => Some(&["crates/cli/tests", "crates/supervisor/tests"]),
        ("test_release_parity", "kobo_replay_tests") => Some(&["fixtures/save.json"]),
        ("test_release_parity", "kobo_liveness_tests") => {
            Some(&["crates/supervisor/src/job/state.rs"])
        }
        ("test_release_parity", "cli_behavior") => Some(&["crates/cli/src"]),
        ("test_release_parity", "config_behavior") => Some(&["crates/cli/src/config.rs"]),
        ("test_release_parity", "exit_behavior") => Some(&["crates/lib/src/action/return.rs"]),
        ("test_release_parity", "logging_behavior") => Some(&["crates/cli/src/args/logging.rs"]),
        ("test_release_parity", "package_behavior") => Some(&["Cargo.toml", "cliff.toml"]),
        ("test_release_parity", "platform_behavior") => {
            Some(&["crates/lib/src/paths.rs", "crates/lib/src/sources/fs.rs"])
        }
        ("test_release_parity", "install_behavior") => {
            Some(&["crates/cli/Cargo.toml", "completions"])
        }
        ("performance", "startup")
        | ("performance", "steady_state")
        | ("performance", "restart") => Some(&["crates/lib/src/watchexec.rs"]),
        ("performance", "memory") | ("performance", "binary") => {
            Some(&["Cargo.toml", "crates/cli/Cargo.toml"])
        }
        ("performance", "watch_tree_scaling") | ("performance", "event_burst_scaling") => {
            Some(&["crates/lib/src/sources/fs.rs"])
        }
        _ => None,
    }
}

fn evidence_target_name(expected_kind: &str, expected_subject: Option<&str>) -> String {
    expected_subject.unwrap_or(expected_kind).to_owned()
}

fn validate_performance_measurements(
    document: &Value,
    expected_subject: &str,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let Some(required_units) = required_performance_units(expected_subject) else {
        return;
    };
    let measurements = document["measurements"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut has_subject_measurement = false;
    for measurement in measurements {
        if measurement["name"].as_str() != Some(expected_subject) {
            continue;
        }
        has_subject_measurement = true;
        validate_positive_number(&measurement, "value", label, blockers);
        validate_positive_number(&measurement, "baseline_value", label, blockers);
        validate_positive_u64(&measurement, "sample_count", label, blockers);
        let unit = measurement["unit"].as_str().unwrap_or("");
        if !required_units.contains(&unit) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{label} performance measurement unit {unit} does not prove {expected_subject}"
            )));
        }
    }
    if !has_subject_measurement {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} performance measurements missing {expected_subject}"
        )));
    }
}

fn validate_structured_future_evidence(
    document: &Value,
    expected_kind: &str,
    target_name: &str,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    match expected_kind {
        "language_surface" => validate_language_evidence_document(document, label, blockers),
        "platform_model" => validate_platform_evidence_document(document, target_name, blockers),
        "adapter_summary" => {
            validate_adapter_evidence_document(document, target_name, label, blockers)
        }
        "async_runtime" => validate_async_evidence_document(document, label, blockers),
        "generated_backend" => validate_generated_backend_evidence(document, label, blockers),
        "test_release_parity" => {
            validate_parity_evidence_document(document, target_name, label, blockers)
        }
        "performance" => {
            validate_performance_evidence_document(document, target_name, label, blockers)
        }
        "upstream_tests" => validate_replacement_suite_document(document, label, blockers),
        _ => {}
    }
}

fn validate_language_evidence_document(
    document: &Value,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    if document["source_kind"].as_str() != Some("kobo_whole_project") {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence source_kind must be kobo_whole_project"
        )));
    }
    require_non_empty_array(document, "kobo_owned_modules", label, blockers);
    require_set(
        document,
        "checks",
        REQUIRED_LANGUAGE_FLAGS,
        |check| format!("{label} evidence missing language check {check}"),
        blockers,
    );
}

fn validate_platform_evidence_document(
    document: &Value,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    validate_platform_observations(document, label, blockers);
    require_set(
        document,
        "behavior_tests",
        REQUIRED_PLATFORM_BEHAVIOR_TESTS,
        |behavior| format!("{label} evidence missing platform behavior test {behavior}"),
        blockers,
    );
}

fn validate_platform_observations(
    document: &Value,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let observations = document["platform_observations"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if observations.is_empty() {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence missing platform_observations"
        )));
        return;
    }
    for platform in REQUIRED_PLATFORMS {
        let observed_behaviors = observations
            .iter()
            .filter(|observation| observation["platform"].as_str() == Some(platform))
            .flat_map(|observation| string_set(&observation["behaviors"]))
            .collect::<BTreeSet<_>>();
        for behavior in REQUIRED_PLATFORM_BEHAVIOR_TESTS {
            if !observed_behaviors.contains(behavior) {
                blockers.push(ProjectSupportBlocker::new(format!(
                    "{label} evidence missing {platform} {behavior} platform observation"
                )));
            }
        }
    }
}

fn validate_adapter_evidence_document(
    document: &Value,
    target_name: &str,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    require_non_empty_str(document, "adapter_name", label, blockers);
    require_non_empty_str(document, "adapter_version", label, blockers);
    require_non_empty_array(document, "conformance_results", label, blockers);
    let has_subject_result = document["conformance_results"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|result| {
            result["adapter_kind"].as_str() == Some(target_name)
                && result["status"].as_str() == Some("passed")
        });
    if !has_subject_result {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence missing passed conformance result for {target_name}"
        )));
    }
}

fn validate_async_evidence_document(
    document: &Value,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    require_set(
        document,
        "runtime_semantics",
        REQUIRED_ASYNC_SEMANTICS,
        |semantic| format!("{label} evidence missing runtime semantic {semantic}"),
        blockers,
    );
    require_set(
        document,
        "mutation_results",
        REQUIRED_ASYNC_MUTATIONS,
        |mutation| format!("{label} evidence missing async mutation {mutation}"),
        blockers,
    );
}

fn validate_generated_backend_evidence(
    document: &Value,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    require_non_empty_array(document, "generated_artifacts", label, blockers);
    require_set(
        document,
        "debug_workflows",
        REQUIRED_BACKEND_WORKFLOWS,
        |workflow| format!("{label} evidence missing generated backend workflow {workflow}"),
        blockers,
    );
    for artifact in document["generated_artifacts"]
        .as_array()
        .into_iter()
        .flatten()
    {
        require_non_empty_str(
            artifact,
            "path",
            &format!("{label} generated artifact"),
            blockers,
        );
        require_non_empty_str(
            artifact,
            "source_map",
            &format!("{label} generated artifact"),
            blockers,
        );
    }
}

fn validate_parity_evidence_document(
    document: &Value,
    target_name: &str,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    require_non_empty_array(document, "parity_results", label, blockers);
    let has_subject_result = document["parity_results"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|result| {
            result["name"].as_str() == Some(target_name)
                && result["status"].as_str() == Some("passed")
        });
    if !has_subject_result {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence missing passed parity result for {target_name}"
        )));
    }
    if target_name == "install_behavior" {
        require_non_empty_array(document, "install_artifacts", label, blockers);
    }
}

fn validate_performance_evidence_document(
    document: &Value,
    target_name: &str,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let source = document["measurement_source"].as_str().unwrap_or("");
    if !matches!(source, "bench_run" | "release_profile_run" | "stress_run") {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} performance evidence measurement_source does not prove {target_name}"
        )));
    }
}

fn validate_replacement_suite_document(
    document: &Value,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    require_non_empty_str(document, "replacement_root", label, blockers);
    if document["replacement_source"].as_str() != Some("kobo_generated") {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence replacement_source must be kobo_generated"
        )));
    }
}

fn required_performance_units(subject: &str) -> Option<&'static [&'static str]> {
    match subject {
        "startup" | "steady_state" | "restart" => Some(&["ms"]),
        "memory" => Some(&["bytes", "MiB"]),
        "binary" => Some(&["bytes"]),
        "watch_tree_scaling" => Some(&["paths_per_second"]),
        "event_burst_scaling" => Some(&["events_per_second"]),
        _ => None,
    }
}

fn validate_positive_number(
    value: &Value,
    field: &str,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    if !matches!(value[field].as_f64(), Some(number) if number > 0.0) {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} performance measurement {field} must be positive"
        )));
    }
}

fn validate_positive_u64(
    value: &Value,
    field: &str,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    if !matches!(value[field].as_u64(), Some(number) if number > 0) {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} performance measurement {field} must be positive"
        )));
    }
}

fn evidence_command_proves_kind(expected_kind: &str, argv: &[String]) -> bool {
    match expected_kind {
        "upstream_inventory" => is_cargo_subcommand(argv, "metadata"),
        "language_surface" => {
            is_cargo_subcommand(argv, "check")
                || is_kobo_subcommand(argv, "check")
                || is_kobo_subcommand(argv, "inspect")
        }
        "generated_backend" => {
            is_cargo_subcommand(argv, "check")
                || is_cargo_subcommand(argv, "test")
                || is_kobo_subcommand(argv, "inspect")
        }
        "platform_model"
        | "adapter_summary"
        | "async_runtime"
        | "test_release_parity"
        | "performance"
        | "upstream_tests"
        | "proof_debt_report"
        | "reviewer_report" => {
            is_cargo_subcommand(argv, "test")
                || is_kobo_subcommand(argv, "watch")
                || is_kobo_subcommand(argv, "doctor")
        }
        _ => false,
    }
}

fn evidence_command_declares_target(command: &Value, target_name: &str) -> bool {
    string_set(&command["proves"]).contains(target_name)
}

fn validate_command_proof_markers(
    command: &Value,
    transcript_evidence: &CommandTranscriptEvidence,
    expected_kind: &str,
    target_name: &str,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let Some(required_markers) = required_subject_proof_markers(expected_kind, target_name) else {
        return;
    };
    let declared_markers = string_set(&command["proof_markers"]);
    for marker in required_markers {
        if !declared_markers.contains(marker) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{label} evidence command missing proof marker declaration {marker}"
            )));
            continue;
        }
        if !transcript_contains_marker(&transcript_evidence.transcript, marker) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{label} evidence transcript missing proof marker {marker}"
            )));
        }
        if !observed_contains_marker(transcript_evidence.observed.as_ref(), marker) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{label} evidence command output missing proof marker {marker}"
            )));
        }
    }
}

fn validate_subject_command_evidence(
    root: &Path,
    document: &Value,
    command: &Value,
    argv: &[String],
    transcript_evidence: &CommandTranscriptEvidence,
    expected_kind: &str,
    target_name: &str,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    if matches!(
        (expected_kind, target_name),
        ("upstream_tests", "upstream")
            | ("upstream_tests", "upstream_tests")
            | ("test_release_parity", "upstream_tests")
    ) {
        validate_original_upstream_test_suite(
            root,
            document,
            command,
            argv,
            transcript_evidence,
            label,
            blockers,
        );
    }
}

fn validate_original_upstream_test_suite(
    root: &Path,
    document: &Value,
    command: &Value,
    argv: &[String],
    transcript_evidence: &CommandTranscriptEvidence,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    if !is_original_upstream_test_command(argv) {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence must run the original upstream workspace test suite"
        )));
    }
    if !command_cwd_matches_replacement_root(root, document, command) {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence upstream test command must run from the Kobo-generated replacement root"
        )));
    }
    let transcript_output = command_output_from_json(&transcript_evidence.transcript);
    validate_upstream_test_output(&transcript_output, "transcript", label, blockers);
    if let Some(observed) = transcript_evidence.observed.as_ref() {
        let observed_output = command_output_from_observed(observed);
        validate_upstream_test_output(&observed_output, "rerun command output", label, blockers);
    }
}

fn command_cwd_matches_replacement_root(root: &Path, document: &Value, command: &Value) -> bool {
    let Some(replacement_root) = document["replacement_root"].as_str() else {
        return false;
    };
    let Some(cwd) = command["cwd"].as_str() else {
        return false;
    };
    let replacement_root = project_path(root, replacement_root);
    let cwd = project_path(root, cwd);
    paths_match(&replacement_root, &cwd)
}

fn paths_match(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn is_original_upstream_test_command(argv: &[String]) -> bool {
    if !is_cargo_subcommand(argv, "test") {
        return false;
    }
    let cargo_args = argv
        .iter()
        .skip(2)
        .take_while(|argument| argument.as_str() != "--")
        .map(String::as_str)
        .collect::<Vec<_>>();
    cargo_args.contains(&"--workspace")
        && cargo_args.contains(&"--all-targets")
        && !cargo_args
            .iter()
            .any(|argument| is_upstream_test_filter_argument(argument))
}

fn is_upstream_test_filter_argument(argument: &str) -> bool {
    matches!(
        argument,
        "-p" | "--package"
            | "--lib"
            | "--bin"
            | "--bins"
            | "--example"
            | "--examples"
            | "--test"
            | "--tests"
            | "--bench"
            | "--benches"
            | "--doc"
    ) || argument.starts_with("-p")
        || argument.starts_with("--package=")
        || argument.starts_with("--bin=")
        || argument.starts_with("--example=")
        || argument.starts_with("--test=")
        || argument.starts_with("--bench=")
}

fn command_output_from_json(transcript: &Value) -> String {
    format!(
        "{}\n{}",
        transcript["stdout"].as_str().unwrap_or(""),
        transcript["stderr"].as_str().unwrap_or("")
    )
}

fn command_output_from_observed(observed: &ObservedCommand) -> String {
    format!("{}\n{}", observed.stdout, observed.stderr)
}

fn validate_upstream_test_output(
    output: &str,
    source_name: &str,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    if output.contains("test result: FAILED") {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence {source_name} includes failed upstream tests"
        )));
    }
    let passed_tests = upstream_test_pass_count(output);
    if passed_tests == 0 {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence {source_name} has no passing upstream tests"
        )));
    }
}

fn upstream_test_pass_count(output: &str) -> usize {
    output
        .lines()
        .filter_map(parse_test_result_pass_count)
        .sum()
}

fn parse_test_result_pass_count(line: &str) -> Option<usize> {
    let passed_marker = " passed;";
    let passed_end = line.find(passed_marker)?;
    let before_passed = &line[..passed_end];
    let number = before_passed
        .rsplit(|character: char| !character.is_ascii_digit())
        .find(|part| !part.is_empty())?;
    number.parse().ok()
}

fn validate_reviewer_report_document(
    document: &Value,
    reviewer: &str,
    _label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    if document["reviewer"].as_str() != Some(reviewer) {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{reviewer} reviewer report must name reviewer {reviewer}"
        )));
    }
    if document["boundary_decision"].as_str() != Some("honest_boundary") {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{reviewer} reviewer report must agree the remaining boundary is honest"
        )));
    }
    require_non_empty_str(
        document,
        "comparison_summary",
        &format!("{reviewer} reviewer report"),
        blockers,
    );
    let command_hashes = document_command_hashes(document);
    let review_sections = document["review_sections"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if review_sections.is_empty() {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{reviewer} reviewer report missing review_sections"
        )));
    }
    for required_section in REQUIRED_REVIEW_SECTIONS {
        validate_reviewer_section(
            &review_sections,
            required_section,
            &command_hashes,
            reviewer,
            blockers,
        );
    }
}

fn document_command_hashes(document: &Value) -> BTreeSet<String> {
    document["commands"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|command| command["output_hash"].as_str())
        .map(str::to_owned)
        .collect()
}

fn validate_reviewer_section(
    review_sections: &[Value],
    required_section: &str,
    command_hashes: &BTreeSet<String>,
    reviewer: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let Some(section) = review_sections
        .iter()
        .find(|section| section["name"].as_str() == Some(required_section))
    else {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{reviewer} reviewer report missing {required_section} comparison"
        )));
        return;
    };
    if section["decision"].as_str() != Some("equivalent_or_stronger") {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{reviewer} reviewer {required_section} comparison did not approve equivalence"
        )));
    }
    if section["boundary"].as_str() != Some("honest") {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{reviewer} reviewer {required_section} comparison did not mark the boundary honest"
        )));
    }
    let command_output_hash = section["command_output_hash"].as_str().unwrap_or("");
    if !command_hashes.contains(command_output_hash) {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{reviewer} reviewer {required_section} comparison is not tied to a command transcript"
        )));
    }
    require_non_empty_array(
        section,
        "evidence_refs",
        &format!("{reviewer} reviewer {required_section} comparison"),
        blockers,
    );
}

fn required_subject_proof_markers(
    expected_kind: &str,
    target_name: &str,
) -> Option<&'static [&'static str]> {
    match (expected_kind, target_name) {
        ("async_runtime", "async_runtime") => Some(&[
            "kobo-proof:async_runtime:spawn",
            "kobo-proof:async_runtime:join",
            "kobo-proof:async_runtime:cancel",
            "kobo-proof:async_runtime:select",
            "kobo-proof:async_runtime:timer",
            "kobo-proof:async_runtime:channel",
            "kobo-proof:async_runtime:backpressure",
            "kobo-proof:async_runtime:shutdown",
            "kobo-proof:async_runtime:blocking",
        ]),
        ("test_release_parity", "kobo_replay_tests") => {
            Some(&["kobo-proof:kobo_replay_tests:replay"])
        }
        ("test_release_parity", "kobo_liveness_tests") => {
            Some(&["kobo-proof:kobo_liveness_tests:liveness"])
        }
        ("test_release_parity", "cli_behavior") => Some(&["kobo-proof:cli_behavior:cli"]),
        ("test_release_parity", "config_behavior") => Some(&["kobo-proof:config_behavior:config"]),
        ("test_release_parity", "exit_behavior") => Some(&["kobo-proof:exit_behavior:exit"]),
        ("test_release_parity", "logging_behavior") => {
            Some(&["kobo-proof:logging_behavior:logging"])
        }
        ("test_release_parity", "package_behavior") => {
            Some(&["kobo-proof:package_behavior:package"])
        }
        ("test_release_parity", "platform_behavior") => {
            Some(&["kobo-proof:platform_behavior:platform"])
        }
        ("test_release_parity", "install_behavior") => {
            Some(&["kobo-proof:install_behavior:install"])
        }
        ("performance", "startup") => Some(&["kobo-proof:startup:measurement"]),
        ("performance", "steady_state") => Some(&["kobo-proof:steady_state:measurement"]),
        ("performance", "restart") => Some(&["kobo-proof:restart:measurement"]),
        ("performance", "memory") => Some(&["kobo-proof:memory:measurement"]),
        ("performance", "binary") => Some(&["kobo-proof:binary:measurement"]),
        ("performance", "watch_tree_scaling") => {
            Some(&["kobo-proof:watch_tree_scaling:measurement"])
        }
        ("performance", "event_burst_scaling") => {
            Some(&["kobo-proof:event_burst_scaling:measurement"])
        }
        ("reviewer_report", "reviewer-a") => Some(&["kobo-proof:reviewer-a:independent-review"]),
        ("reviewer_report", "reviewer-b") => Some(&["kobo-proof:reviewer-b:independent-review"]),
        ("adapter_summary", "async_runtime") => Some(&[
            "kobo-proof:adapter_summary:async_runtime",
            "kobo-proof:async_runtime:cancel",
            "kobo-proof:async_runtime:timer",
        ]),
        ("adapter_summary", target) => adapter_proof_marker(target),
        ("platform_model", target) => platform_proof_marker(target),
        _ => None,
    }
}

fn adapter_proof_marker(target_name: &str) -> Option<&'static [&'static str]> {
    match target_name {
        "watcher_backend" => Some(&["kobo-proof:adapter_summary:watcher_backend"]),
        "process_handling" => Some(&["kobo-proof:adapter_summary:process_handling"]),
        "signal_handling" => Some(&["kobo-proof:adapter_summary:signal_handling"]),
        "ignore_path" => Some(&["kobo-proof:adapter_summary:ignore_path"]),
        "config" => Some(&["kobo-proof:adapter_summary:config"]),
        "cli" => Some(&["kobo-proof:adapter_summary:cli"]),
        "shell_parsing" => Some(&["kobo-proof:adapter_summary:shell_parsing"]),
        "serialization" => Some(&["kobo-proof:adapter_summary:serialization"]),
        "logging_tracing" => Some(&["kobo-proof:adapter_summary:logging_tracing"]),
        "terminal_helpers" => Some(&["kobo-proof:adapter_summary:terminal_helpers"]),
        "errors" => Some(&["kobo-proof:adapter_summary:errors"]),
        _ => None,
    }
}

fn platform_proof_marker(target_name: &str) -> Option<&'static [&'static str]> {
    match target_name {
        "filesystem_events" => Some(&["kobo-proof:platform_model:filesystem_events"]),
        "watcher_backend" => Some(&["kobo-proof:platform_model:watcher_backend"]),
        "paths" => Some(&["kobo-proof:platform_model:paths"]),
        "process_execution" => Some(&["kobo-proof:platform_model:process_execution"]),
        "signals" => Some(&["kobo-proof:platform_model:signals"]),
        "process_groups" => Some(&["kobo-proof:platform_model:process_groups"]),
        "environment_variables" => Some(&["kobo-proof:platform_model:environment_variables"]),
        "terminal_io" => Some(&["kobo-proof:platform_model:terminal_io"]),
        "stdio" => Some(&["kobo-proof:platform_model:stdio"]),
        "timers" => Some(&["kobo-proof:platform_model:timers"]),
        _ => None,
    }
}

fn transcript_contains_marker(transcript: &Value, marker: &str) -> bool {
    transcript["stdout"]
        .as_str()
        .is_some_and(|stdout| stdout.contains(marker))
        || transcript["stderr"]
            .as_str()
            .is_some_and(|stderr| stderr.contains(marker))
}

fn observed_contains_marker(observed: Option<&ObservedCommand>, marker: &str) -> bool {
    observed.is_some_and(|observed| {
        observed.stdout.contains(marker) || observed.stderr.contains(marker)
    })
}

fn is_cargo_subcommand(argv: &[String], subcommand: &str) -> bool {
    argv.first()
        .is_some_and(|program| is_program_named(program, "cargo"))
        && argv.get(1).map(String::as_str) == Some(subcommand)
}

fn is_kobo_subcommand(argv: &[String], subcommand: &str) -> bool {
    if argv
        .first()
        .is_some_and(|program| is_program_named(program, "kobo"))
    {
        return argv.get(1).map(String::as_str) == Some(subcommand);
    }
    if !argv
        .first()
        .is_some_and(|program| is_program_named(program, "cargo"))
    {
        return false;
    }
    let Some(separator) = argv.iter().position(|argument| argument == "--") else {
        return false;
    };
    argv.get(separator + 1).map(String::as_str) == Some(subcommand)
}

fn is_program_named(program: &str, expected: &str) -> bool {
    let normalized = program.replace('\\', "/").to_ascii_lowercase();
    normalized == expected
        || normalized.ends_with(&format!("/{expected}"))
        || normalized.ends_with(&format!("/{expected}.exe"))
}

fn validate_command_transcript(
    root: &Path,
    command: &Value,
    expected_command: &str,
    expected_hash: &str,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) -> Option<CommandTranscriptEvidence> {
    let Some(transcript_path) = command["transcript_path"].as_str() else {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence command missing transcript_path"
        )));
        return None;
    };
    let transcript_path = project_path(root, transcript_path);
    if !transcript_path.is_file() {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence transcript does not exist: {}",
            transcript_path.display()
        )));
        return None;
    }
    let Ok(transcript_source) = std::fs::read_to_string(&transcript_path) else {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence transcript is not readable: {}",
            transcript_path.display()
        )));
        return None;
    };
    if transcript_source.trim().is_empty() {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence transcript is empty: {}",
            transcript_path.display()
        )));
        return None;
    }
    if stable_hash(&transcript_source) != expected_hash {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence transcript hash mismatch"
        )));
    }
    match serde_json::from_str::<Value>(&transcript_source) {
        Ok(transcript) => {
            let observed = rerun_evidence_command(root, command, label, blockers);
            let output_match = command["output_match"].as_str().unwrap_or("exact");
            validate_command_transcript_json(
                &transcript,
                expected_command,
                observed.as_ref(),
                output_match,
                label,
                blockers,
            );
            Some(CommandTranscriptEvidence {
                transcript,
                observed,
            })
        }
        Err(error) => {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{label} evidence transcript is not valid JSON: {error}"
            )));
            None
        }
    }
}

fn validate_command_transcript_json(
    transcript: &Value,
    expected_command: &str,
    observed: Option<&ObservedCommand>,
    output_match: &str,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    if !matches!(output_match, "exact" | "exit_code") {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence output_match is unsupported: {output_match}"
        )));
    }
    if transcript["schema_version"].as_u64() != Some(1) {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence transcript schema_version must be 1"
        )));
    }
    if transcript["command"].as_str() != Some(expected_command) {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence transcript command mismatch"
        )));
    }
    if transcript["exit_code"].as_i64() != Some(0) {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence transcript exit_code must be 0"
        )));
    }
    if transcript["status"].as_str() != Some("passed") {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence transcript status must be passed"
        )));
    }
    if let Some(observed) = observed {
        if transcript["exit_code"].as_i64() != observed.exit_code.map(i64::from) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{label} evidence transcript exit_code does not match rerun command"
            )));
        }
        if output_match == "exact" {
            if transcript["stdout"].as_str().unwrap_or("") != observed.stdout.as_str() {
                blockers.push(ProjectSupportBlocker::new(format!(
                    "{label} evidence transcript stdout does not match rerun command"
                )));
            }
            if transcript["stderr"].as_str().unwrap_or("") != observed.stderr.as_str() {
                blockers.push(ProjectSupportBlocker::new(format!(
                    "{label} evidence transcript stderr does not match rerun command"
                )));
            }
        }
    }
}

fn rerun_evidence_command(
    root: &Path,
    command: &Value,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) -> Option<ObservedCommand> {
    let argv = command_argv(command, label, blockers)?;
    if !is_allowed_evidence_program(&argv[0]) {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence command program is not allowed: {}",
            argv[0]
        )));
        return None;
    }
    let cwd = command["cwd"]
        .as_str()
        .map(|path| project_path(root, path))
        .unwrap_or_else(|| root.to_path_buf());
    if !cwd.is_dir() {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence command cwd does not exist: {}",
            cwd.display()
        )));
        return None;
    }
    let path_prepend = match command["env_path_prepend"].as_str() {
        Some(path) => {
            let path = project_path(root, path);
            if !path.is_dir() {
                blockers.push(ProjectSupportBlocker::new(format!(
                    "{label} evidence command env_path_prepend does not exist: {}",
                    path.display()
                )));
                return None;
            }
            Some(path)
        }
        None => None,
    };
    let timeout = evidence_command_timeout(command, label, blockers)?;
    let key = command_cache_key(&cwd, &argv, path_prepend.as_deref());
    if let Some(observed) = cached_evidence_command(&key) {
        return Some(observed);
    }
    let observed = run_evidence_command(
        &cwd,
        &argv,
        path_prepend.as_deref(),
        timeout,
        label,
        blockers,
    )?;
    cache_evidence_command(key, observed.clone());
    Some(observed)
}

fn evidence_command_timeout(
    command: &Value,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) -> Option<Duration> {
    let Some(timeout_seconds) = command["timeout_seconds"].as_u64() else {
        return Some(DEFAULT_EVIDENCE_COMMAND_TIMEOUT);
    };
    if timeout_seconds == 0 || timeout_seconds > MAX_EVIDENCE_COMMAND_TIMEOUT_SECONDS {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence command timeout_seconds must be between 1 and {MAX_EVIDENCE_COMMAND_TIMEOUT_SECONDS}"
        )));
        return None;
    }
    Some(Duration::from_secs(timeout_seconds))
}

fn command_argv(
    command: &Value,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) -> Option<Vec<String>> {
    let Some(values) = command["argv"].as_array() else {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence command missing argv"
        )));
        return None;
    };
    let argv = values
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if argv.is_empty() || argv.iter().any(|value| value.trim().is_empty()) {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence command argv is empty"
        )));
        return None;
    }
    if command["command"].as_str() != Some(argv.join(" ").as_str()) {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence command text does not match argv"
        )));
        return None;
    }
    Some(argv)
}

fn is_allowed_evidence_program(program: &str) -> bool {
    let normalized = program.replace('\\', "/").to_ascii_lowercase();
    normalized == "cargo"
        || normalized.ends_with("/cargo")
        || normalized.ends_with("/cargo.exe")
        || normalized == "kobo"
        || normalized.ends_with("/kobo")
        || normalized.ends_with("/kobo.exe")
}

fn run_evidence_command(
    cwd: &Path,
    argv: &[String],
    path_prepend: Option<&Path>,
    timeout: Duration,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) -> Option<ObservedCommand> {
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path_prepend) = path_prepend {
        let Some(path) = path_with_prepend(path_prepend, blockers, label) else {
            return None;
        };
        command.env("PATH", path);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{label} evidence command failed to start: {error}"
            )));
            return None;
        }
    };
    let mut stdout_reader = child.stdout.take().map(spawn_command_stream_reader);
    let mut stderr_reader = child.stderr.take().map(spawn_command_stream_reader);
    let started_at = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                return Some(finish_observed_command(
                    _status,
                    stdout_reader.take(),
                    stderr_reader.take(),
                    label,
                    blockers,
                ));
            }
            Ok(None) if started_at.elapsed() < timeout => {
                thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                drop(join_command_stream(
                    stdout_reader.take(),
                    label,
                    "stdout",
                    blockers,
                ));
                drop(join_command_stream(
                    stderr_reader.take(),
                    label,
                    "stderr",
                    blockers,
                ));
                blockers.push(ProjectSupportBlocker::new(format!(
                    "{label} evidence command timed out"
                )));
                return None;
            }
            Err(error) => {
                let _ = child.kill();
                drop(join_command_stream(
                    stdout_reader.take(),
                    label,
                    "stdout",
                    blockers,
                ));
                drop(join_command_stream(
                    stderr_reader.take(),
                    label,
                    "stderr",
                    blockers,
                ));
                blockers.push(ProjectSupportBlocker::new(format!(
                    "{label} evidence command status failed: {error}"
                )));
                return None;
            }
        }
    }
}

fn path_with_prepend(
    path_prepend: &Path,
    blockers: &mut Vec<ProjectSupportBlocker>,
    label: &str,
) -> Option<std::ffi::OsString> {
    let existing_path = std::env::var_os("PATH").unwrap_or_default();
    let paths =
        std::iter::once(path_prepend.to_path_buf()).chain(std::env::split_paths(&existing_path));
    match std::env::join_paths(paths) {
        Ok(path) => Some(path),
        Err(error) => {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{label} evidence command env_path_prepend is invalid: {error}"
            )));
            None
        }
    }
}

fn spawn_command_stream_reader<TStream>(mut stream: TStream) -> CommandStreamHandle
where
    TStream: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).map(|_| bytes)
    })
}

fn finish_observed_command(
    status: ExitStatus,
    stdout_reader: Option<CommandStreamHandle>,
    stderr_reader: Option<CommandStreamHandle>,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) -> ObservedCommand {
    ObservedCommand {
        exit_code: status.code(),
        stdout: join_command_stream(stdout_reader, label, "stdout", blockers),
        stderr: join_command_stream(stderr_reader, label, "stderr", blockers),
    }
}

fn join_command_stream(
    reader: Option<CommandStreamHandle>,
    label: &str,
    stream_name: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) -> String {
    let Some(reader) = reader else {
        return String::new();
    };
    match reader.join() {
        Ok(Ok(bytes)) => String::from_utf8_lossy(&bytes).to_string(),
        Ok(Err(error)) => {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{label} evidence command {stream_name} read failed: {error}"
            )));
            String::new()
        }
        Err(_) => {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{label} evidence command {stream_name} reader failed"
            )));
            String::new()
        }
    }
}

fn command_cache_key(cwd: &Path, argv: &[String], path_prepend: Option<&Path>) -> String {
    let path_prepend = path_prepend
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    format!("{}|{}|{}", cwd.display(), path_prepend, argv.join("\u{1f}"))
}

fn cached_evidence_command(key: &str) -> Option<ObservedCommand> {
    EVIDENCE_COMMAND_CACHE
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .ok()
        .and_then(|cache| cache.get(key).cloned())
}

fn cache_evidence_command(key: String, observed: ObservedCommand) {
    if let Ok(mut cache) = EVIDENCE_COMMAND_CACHE
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
    {
        cache.insert(key, observed);
    }
}

fn require_evidence_checks(
    document: &Value,
    required: &[&str],
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    require_set(
        document,
        "checks",
        required,
        |check| format!("{label} evidence missing check {check}"),
        blockers,
    );
}

fn require_evidence_covers_inventory_paths(
    evidence: &Value,
    inventory: &Value,
    inventory_field: &str,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let covered_paths = string_set(&evidence["covered_paths"]);
    for path in inventory[inventory_field].as_array().into_iter().flatten() {
        let Some(path) = path.as_str() else {
            continue;
        };
        if !covered_paths.contains(path) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{label} evidence does not cover upstream path {path}"
            )));
        }
    }
}

fn require_evidence_feature_coverage(
    evidence: &Value,
    required_features: &Value,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let observed = string_set(&evidence["stale_check"]["features"]);
    for feature in required_features.as_array().into_iter().flatten() {
        let Some(feature) = feature.as_str() else {
            continue;
        };
        if !observed.contains(feature) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{label} evidence stale check missing feature {feature}"
            )));
        }
    }
}

fn require_feature_matrix_coverage(
    required_matrix: &Value,
    observed_matrix: &Value,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let required = feature_matrix_signatures(required_matrix);
    let observed = feature_matrix_signatures(observed_matrix);
    for feature_combination in required {
        if !observed.contains(&feature_combination) {
            blockers.push(ProjectSupportBlocker::new(format!(
                "{label} missing feature combination {feature_combination}"
            )));
        }
    }
}

fn feature_matrix_contains_feature(signatures: &BTreeSet<String>, feature: &str) -> bool {
    signatures
        .iter()
        .any(|signature| signature.split('+').any(|part| part == feature))
}

fn feature_matrix_signatures(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(feature_combination_signature)
        .collect()
}

fn feature_combination_signature(value: &Value) -> Option<String> {
    if let Some(feature) = value.as_str() {
        return Some(feature.to_owned());
    }
    let mut features = value
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    features.sort();
    if features.is_empty() {
        None
    } else {
        Some(features.join("+"))
    }
}

fn require_release_artifact_path(
    root: &Path,
    upstream_root: Option<&Path>,
    value: &Value,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    let Some(path) = value.as_str() else {
        blockers.push(ProjectSupportBlocker::new(
            "release artifact must be a path string",
        ));
        return;
    };
    if path.trim().is_empty() {
        blockers.push(ProjectSupportBlocker::new("release artifact path is empty"));
        return;
    }
    if is_project_evidence_path(path) && path_is_non_empty_file(&project_path(root, path)) {
        return;
    }
    if let Some(upstream_root) = upstream_root {
        if path_is_non_empty_file(&project_path(upstream_root, path)) {
            return;
        }
    }
    if path_is_non_empty_file(&project_path(root, path)) {
        return;
    }
    blockers.push(ProjectSupportBlocker::new(format!(
        "release artifact evidence path does not exist: {path}"
    )));
}

fn is_project_evidence_path(path: &str) -> bool {
    path.replace('\\', "/").starts_with(".kobo/")
}

fn path_is_non_empty_file(path: &Path) -> bool {
    path.is_file()
        && path
            .metadata()
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)
}

fn require_non_empty_file(
    root: &Path,
    path: &str,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) -> Option<PathBuf> {
    if path.trim().is_empty() {
        blockers.push(ProjectSupportBlocker::new(format!("{label} path is empty")));
        return None;
    }
    let absolute = project_path(root, path);
    if !absolute.is_file() {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence path does not exist: {path}"
        )));
        return None;
    }
    if absolute
        .metadata()
        .map(|metadata| metadata.len())
        .unwrap_or(0)
        == 0
    {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} evidence path is empty: {path}"
        )));
        return None;
    }
    Some(absolute)
}

fn require_inventory_path(
    upstream_root: &Path,
    value: &Value,
    field: &str,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    if let Some(path) = value[field].as_str() {
        require_inventory_path_str(upstream_root, path, label, blockers);
    } else {
        blockers.push(ProjectSupportBlocker::new(format!("missing {label}")));
    }
}

fn require_inventory_path_field(
    upstream_root: &Path,
    value: &Value,
    field: &str,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    if let Some(path) = value[field].as_str() {
        require_inventory_path_str(upstream_root, path, label, blockers);
    } else {
        blockers.push(ProjectSupportBlocker::new(format!("missing {label} path")));
    }
}

fn require_inventory_path_value(
    upstream_root: &Path,
    value: &Value,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    if let Some(path) = value.as_str() {
        require_inventory_path_str(upstream_root, path, label, blockers);
    } else {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} must be a path string"
        )));
    }
}

fn require_inventory_path_str(
    upstream_root: &Path,
    path: &str,
    label: &str,
    blockers: &mut Vec<ProjectSupportBlocker>,
) {
    if path.trim().is_empty() {
        blockers.push(ProjectSupportBlocker::new(format!("{label} path is empty")));
        return;
    }
    if !upstream_root.join(path).exists() {
        blockers.push(ProjectSupportBlocker::new(format!(
            "{label} does not exist in upstream inventory: {path}"
        )));
    }
}

fn project_path(root: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn find_object_by_kind<'a>(values: &'a [Value], kind: &str) -> Option<&'a Value> {
    values
        .iter()
        .find(|value| value["kind"].as_str() == Some(kind))
}

fn string_set<'a>(value: &'a Value) -> BTreeSet<&'a str> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

fn read_json(path: &Path) -> anyhow::Result<Value> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&source).with_context(|| format!("failed to parse {}", path.display()))
}

fn collect_relative_files(root: &Path, directory: &Path, extension: &str) -> Vec<String> {
    let mut files = Vec::new();
    collect_relative_files_into(root, directory, extension, &mut files);
    files.sort();
    files
}

fn collect_relative_files_into(
    root: &Path,
    directory: &Path,
    extension: &str,
    files: &mut Vec<String>,
) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_relative_files_into(root, &path, extension, files);
        } else if path.extension().and_then(|value| value.to_str()) == Some(extension) {
            files.push(relative_path(root, &path));
        }
    }
}

fn build_scripts(root: &Path, cargo: Option<&TomlValue>) -> Vec<String> {
    let mut scripts = Vec::new();
    if root.join("build.rs").is_file() {
        scripts.push("build.rs".to_owned());
    }
    if let Some(script) = cargo.and_then(package_build_script) {
        scripts.push(script);
    }
    scripts.sort();
    scripts.dedup();
    scripts
}

fn collect_release_artifacts(root: &Path) -> Vec<String> {
    let mut artifacts = Vec::new();
    collect_release_artifacts_into(root, &root.join("target").join("release"), &mut artifacts);
    artifacts.sort();
    artifacts
}

fn collect_release_artifacts_into(root: &Path, directory: &Path, artifacts: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_release_artifacts_into(root, &path, artifacts);
        } else if path.is_file() {
            artifacts.push(relative_path(root, &path));
        }
    }
}

fn feature_flags(cargo: &TomlValue) -> Vec<String> {
    table_keys(cargo.get("features"))
}

fn read_project_cargo(root: &Path) -> Option<TomlValue> {
    std::fs::read_to_string(root.join("Cargo.toml"))
        .ok()?
        .parse::<TomlValue>()
        .ok()
}

fn find_dependency<'a>(cargo: &'a TomlValue, name: &str) -> Option<&'a TomlValue> {
    ["dependencies", "dev-dependencies", "build-dependencies"]
        .iter()
        .find_map(|section| cargo.get(*section).and_then(|table| table.get(name)))
        .or_else(|| {
            cargo
                .get("workspace")
                .and_then(|workspace| workspace.get("dependencies"))
                .and_then(|table| table.get(name))
        })
}

fn adapter_dependency_features(
    cargo: Option<&TomlValue>,
    upstream_dependencies: &AdapterDependencyIndex,
    dependency_name: &str,
) -> Option<BTreeSet<String>> {
    let mut features = BTreeSet::new();
    let mut is_declared = false;
    if let Some(dependency) = cargo.and_then(|cargo| find_dependency(cargo, dependency_name)) {
        is_declared = true;
        features.extend(dependency_feature_names(dependency));
    }
    if let Some(upstream_features) = upstream_dependencies.features_for(dependency_name) {
        is_declared = true;
        features.extend(upstream_features.iter().cloned());
    }
    is_declared.then_some(features)
}

fn dependency_feature_names(dependency: &TomlValue) -> BTreeSet<String> {
    dependency_features(dependency)
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn dependency_features(dependency: &TomlValue) -> BTreeSet<&str> {
    dependency
        .get("features")
        .and_then(TomlValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(TomlValue::as_str)
        .collect()
}

fn workspace_members(cargo: &TomlValue) -> Vec<String> {
    cargo
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(TomlValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(TomlValue::as_str)
        .map(str::to_owned)
        .collect()
}

fn package_build_script(cargo: &TomlValue) -> Option<String> {
    cargo
        .get("package")
        .and_then(|package| package.get("build"))
        .and_then(TomlValue::as_str)
        .map(str::to_owned)
}

fn table_keys(value: Option<&TomlValue>) -> Vec<String> {
    let mut keys = value
        .and_then(TomlValue::as_table)
        .map(|table| table.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    keys.sort();
    keys
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependency_lookup_reads_workspace_dependencies() {
        let cargo = r#"
[workspace]
members = ["."]

[workspace.dependencies]
clap = { version = "4", features = ["derive"] }
"#
        .parse::<TomlValue>()
        .expect("workspace manifest should parse");

        let dependency = find_dependency(&cargo, "clap").expect("workspace dependency is visible");
        assert!(dependency_features(dependency).contains("derive"));
        assert!(find_dependency(&cargo, "missing").is_none());
    }
}
