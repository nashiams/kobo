use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;

const PROJECT_MAP_SCHEMA_VERSION: u64 = 1;

pub(super) struct ProjectMapReport {
    source_root: String,
    digest: String,
    module_ownership: ModuleOwnershipSummary,
    module_classification: ModuleClassificationSummary,
    adapter_debt: AdapterDebtSummary,
    whole_project_coverage: WholeProjectCoverageSummary,
}

struct WholeProjectCoverageSummary {
    manifest_path: Option<String>,
    status: &'static str,
    inventory_counts: BTreeMap<&'static str, usize>,
    language_flags: BTreeMap<&'static str, bool>,
    feature_combinations: usize,
    platform_model_grades: BTreeMap<String, String>,
    adapter_decisions: BTreeMap<String, String>,
    generated_backend_flags: BTreeMap<&'static str, bool>,
    parity_counts: BTreeMap<&'static str, usize>,
    proof_debt_modules: usize,
    mutation_tests: usize,
    reviewer_reports: usize,
    release_gate: &'static str,
}

struct ModuleClassificationSummary {
    entries: Vec<ModuleClassificationEntry>,
}

struct ModuleClassificationEntry {
    module: String,
    classification: &'static str,
    criticality: &'static str,
    release_blocking: bool,
    justification: String,
}

struct AdapterDebtSummary {
    watcher: AdapterDebtStatus,
    process: AdapterDebtStatus,
    time: AdapterDebtStatus,
    path_filter: AdapterDebtStatus,
    async_runtime: AdapterDebtStatus,
}

struct AdapterDebtStatus {
    status: &'static str,
    reason: String,
}

struct ModuleOwnershipSummary {
    kobo_owned: Vec<String>,
    rust_owned: Vec<String>,
    boundary_debt: Vec<String>,
}

impl ProjectMapReport {
    pub(super) fn for_file(file: &Path) -> anyhow::Result<Self> {
        let source_root = source_tree_root(file);
        let project_root = project_root_for_source_root(&source_root);
        let adapter_debt = adapter_debt_summary_for_file(file);
        let module_ownership = module_ownership_summary_for_root(&source_root, &adapter_debt)?;
        let module_classification =
            ModuleClassificationSummary::from_ownership(&module_ownership, &adapter_debt);
        let whole_project_coverage = whole_project_coverage_for_root(&project_root);
        let source_root = normalized_display(&source_root);
        let digest = project_map_digest(
            &source_root,
            &module_ownership,
            &module_classification,
            &adapter_debt,
            &whole_project_coverage,
        )?;
        Ok(Self {
            source_root,
            digest,
            module_ownership,
            module_classification,
            adapter_debt,
            whole_project_coverage,
        })
    }

    pub(super) fn to_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "schema_version": PROJECT_MAP_SCHEMA_VERSION,
            "source_root": self.source_root,
            "digest": self.digest,
            "module_ownership": self.module_ownership.to_json_value(),
            "module_classification": self.module_classification.to_json_value(),
            "adapter_debt": self.adapter_debt.to_json_value(),
            "whole_project_coverage": self.whole_project_coverage.to_json_value(),
        })
    }

    pub(super) fn adapter_debt_json(&self) -> serde_json::Value {
        self.adapter_debt.to_json_value()
    }

    pub(super) fn module_ownership_json(&self) -> serde_json::Value {
        self.module_ownership.to_json_value()
    }

    pub(super) fn adapter_debt_label(&self) -> String {
        self.adapter_debt.human_label()
    }

    pub(super) fn module_ownership_label(&self) -> String {
        self.module_ownership.human_label()
    }

    pub(super) fn human_label(&self) -> String {
        format!(
            "project map: digest={} kobo-owned={} rust-owned={} boundary-debt={}",
            self.digest,
            self.module_ownership.kobo_owned.len(),
            self.module_ownership.rust_owned.len(),
            self.module_ownership.boundary_debt.len(),
        )
    }

    pub(super) fn inspect_comment(&self) -> String {
        format!("// kobo-project-map: {}", self.human_label_fields())
    }

    fn human_label_fields(&self) -> String {
        format!(
            "digest={} kobo-owned={} rust-owned={} boundary-debt={} classified={} coverage={}",
            self.digest,
            self.module_ownership.kobo_owned.len(),
            self.module_ownership.rust_owned.len(),
            self.module_ownership.boundary_debt.len(),
            self.module_classification.entries.len(),
            self.whole_project_coverage.status,
        )
    }
}

impl WholeProjectCoverageSummary {
    fn missing() -> Self {
        Self {
            manifest_path: None,
            status: "missing",
            inventory_counts: BTreeMap::new(),
            language_flags: BTreeMap::new(),
            feature_combinations: 0,
            platform_model_grades: BTreeMap::new(),
            adapter_decisions: BTreeMap::new(),
            generated_backend_flags: BTreeMap::new(),
            parity_counts: BTreeMap::new(),
            proof_debt_modules: 0,
            mutation_tests: 0,
            reviewer_reports: 0,
            release_gate: "blocked",
        }
    }

    fn from_manifest(path: &Path, manifest: &serde_json::Value) -> Self {
        let mut generated_backend_flags = BTreeMap::new();
        for flag in [
            "reviewable",
            "deterministic",
            "source_mapped",
            "diagnostics_on_kobo_source",
            "replay_on_kobo_source",
            "debt_on_kobo_source",
            "proof_on_kobo_source",
            "lsp_on_kobo_source",
        ] {
            generated_backend_flags.insert(
                flag,
                manifest["generated_backend"][flag].as_bool().unwrap_or(false),
            );
        }
        Self {
            manifest_path: Some(normalized_display(path)),
            status: "evidence_present",
            inventory_counts: upstream_inventory_counts(&manifest["upstream_inventory"]),
            language_flags: language_surface_flags(&manifest["language_surface"]),
            feature_combinations: json_array_len(&manifest["language_surface"], "feature_matrix"),
            platform_model_grades: platform_model_grades(manifest),
            adapter_decisions: adapter_decisions(manifest),
            generated_backend_flags,
            parity_counts: test_release_parity_counts(&manifest["test_release_parity"]),
            proof_debt_modules: manifest["proof_debt_map"].as_array().map_or(0, Vec::len),
            mutation_tests: json_array_len(&manifest["independent_equivalence"], "mutation_tests"),
            reviewer_reports: json_array_len(&manifest["independent_equivalence"], "reviewer_reports"),
            release_gate: "evidence_visible",
        }
    }

    fn to_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1,
            "status": self.status,
            "manifest": self.manifest_path,
            "upstream_inventory_counts": self.inventory_counts,
            "language_flags": self.language_flags,
            "feature_combinations": self.feature_combinations,
            "platform_model_grades": self.platform_model_grades,
            "adapter_decisions": self.adapter_decisions,
            "generated_backend_flags": self.generated_backend_flags,
            "test_release_parity_counts": self.parity_counts,
            "proof_debt_modules": self.proof_debt_modules,
            "mutation_tests": self.mutation_tests,
            "reviewer_reports": self.reviewer_reports,
            "release_gate": self.release_gate,
        })
    }
}

impl ModuleClassificationSummary {
    fn from_ownership(
        module_ownership: &ModuleOwnershipSummary,
        adapter_debt: &AdapterDebtSummary,
    ) -> Self {
        let mut entries = Vec::new();
        entries.extend(
            module_ownership
                .kobo_owned
                .iter()
                .map(ModuleClassificationEntry::modeled_kobo_module),
        );
        entries.extend(
            module_ownership
                .rust_owned
                .iter()
                .map(ModuleClassificationEntry::adapter_backed_rust_module),
        );
        entries.extend(
            adapter_boundary_debt_modules(adapter_debt)
                .into_iter()
                .map(ModuleClassificationEntry::adapter_boundary),
        );
        entries.sort_by(|left, right| left.module.cmp(&right.module));
        Self { entries }
    }

    fn to_json_value(&self) -> serde_json::Value {
        let mut by_classification = BTreeMap::<&str, Vec<&str>>::new();
        let mut criticality_totals = BTreeMap::<&str, usize>::new();
        let mut release_blocking = Vec::new();
        let mut release_blocking_justifications = Vec::new();
        for entry in &self.entries {
            by_classification
                .entry(entry.classification)
                .or_default()
                .push(entry.module.as_str());
            *criticality_totals.entry(entry.criticality).or_default() += 1;
            if entry.release_blocking {
                release_blocking.push(entry.module.as_str());
                release_blocking_justifications.push(serde_json::json!({
                    "module": entry.module,
                    "justification": entry.justification,
                }));
            }
        }
        serde_json::json!({
            "schema_version": 1,
            "vocabulary": [
                "proved",
                "modeled",
                "adapter-backed",
                "sampled",
                "metadata-only",
                "opaque",
                "debt",
            ],
            "by_classification": by_classification,
            "criticality_totals": criticality_totals,
            "release_blocking": release_blocking,
            "release_blocking_justifications": release_blocking_justifications,
        })
    }
}

impl ModuleClassificationEntry {
    fn modeled_kobo_module(module: &String) -> Self {
        Self {
            module: module.clone(),
            classification: "modeled",
            criticality: "correctness-critical",
            release_blocking: false,
            justification: "Kobo-owned source participates in parser, checker, lowering, diagnostics, replay, debt, and proof reports"
                .to_owned(),
        }
    }

    fn adapter_backed_rust_module(module: &String) -> Self {
        Self {
            module: module.clone(),
            classification: "adapter-backed",
            criticality: "adapter-boundary",
            release_blocking: false,
            justification: "Rust source is kept as an explicit backend or adapter boundary rather than hidden source of truth"
                .to_owned(),
        }
    }

    fn adapter_boundary(module: String) -> Self {
        let classification = adapter_boundary_classification(&module);
        Self {
            release_blocking: classification == "debt",
            criticality: if classification == "debt" {
                "correctness-critical"
            } else {
                "adapter-boundary"
            },
            justification: module.clone(),
            module,
            classification,
        }
    }
}

impl AdapterDebtSummary {
    fn none() -> Self {
        Self {
            watcher: AdapterDebtStatus::none(),
            process: AdapterDebtStatus::none(),
            time: AdapterDebtStatus::none(),
            path_filter: AdapterDebtStatus::none(),
            async_runtime: AdapterDebtStatus::none(),
        }
    }

    fn human_label(&self) -> String {
        format!(
            "adapter debt: watcher={}, process={}, time={}, path_filter={}, async_runtime={}",
            self.watcher.human_label(),
            self.process.human_label(),
            self.time.human_label(),
            self.path_filter.human_label(),
            self.async_runtime.human_label()
        )
    }

    fn to_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "watcher": self.watcher.to_json_value(),
            "process": self.process.to_json_value(),
            "time": self.time.to_json_value(),
            "path_filter": self.path_filter.to_json_value(),
            "async_runtime": self.async_runtime.to_json_value(),
        })
    }
}

impl AdapterDebtStatus {
    fn none() -> Self {
        Self {
            status: "none",
            reason: "no persisted watch evidence".to_owned(),
        }
    }

    fn acceptable(reason: impl Into<String>) -> Self {
        Self {
            status: "acceptable",
            reason: reason.into(),
        }
    }

    fn blocker(reason: impl Into<String>) -> Self {
        Self {
            status: "blocker",
            reason: reason.into(),
        }
    }

    fn human_label(&self) -> String {
        format!("{}({})", self.status, self.reason)
    }

    fn to_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "status": self.status,
            "reason": self.reason,
        })
    }
}

impl ModuleOwnershipSummary {
    fn human_label(&self) -> String {
        format!(
            "modules: kobo-owned={}, rust-owned={}, boundary-debt={}",
            self.kobo_owned.len(),
            self.rust_owned.len(),
            self.boundary_debt.len()
        )
    }

    fn to_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "kobo_owned": self.kobo_owned,
            "rust_owned": self.rust_owned,
            "boundary_debt": self.boundary_debt,
        })
    }
}

fn project_map_digest(
    source_root: &str,
    module_ownership: &ModuleOwnershipSummary,
    module_classification: &ModuleClassificationSummary,
    adapter_debt: &AdapterDebtSummary,
    whole_project_coverage: &WholeProjectCoverageSummary,
) -> anyhow::Result<String> {
    let payload = serde_json::json!({
        "schema_version": PROJECT_MAP_SCHEMA_VERSION,
        "source_root": source_root,
        "module_ownership": module_ownership.to_json_value(),
        "module_classification": module_classification.to_json_value(),
        "adapter_debt": adapter_debt.to_json_value(),
        "whole_project_coverage": whole_project_coverage.to_json_value(),
    });
    Ok(kobo_sim_core::digest::stable_hash(&serde_json::to_string(
        &payload,
    )?))
}

fn whole_project_coverage_for_root(project_root: &Path) -> WholeProjectCoverageSummary {
    let manifest_path = project_root.join(".kobo").join("project-support.json");
    let Ok(source) = fs::read_to_string(&manifest_path) else {
        return WholeProjectCoverageSummary::missing();
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&source) else {
        return WholeProjectCoverageSummary::missing();
    };
    WholeProjectCoverageSummary::from_manifest(&manifest_path, &manifest)
}

fn upstream_inventory_counts(
    inventory: &serde_json::Value,
) -> BTreeMap<&'static str, usize> {
    [
        ("crate_tree", "crate_tree"),
        ("modules", "modules"),
        ("public_types", "public_types"),
        ("cli_surfaces", "cli_surfaces"),
        ("test_fixtures", "test_fixtures"),
        ("platform_paths", "platform_paths"),
        ("feature_combinations", "feature_combinations"),
        ("examples", "examples"),
        ("build_scripts", "build_scripts"),
        ("release_artifacts", "release_artifacts"),
    ]
    .into_iter()
    .map(|(label, field)| (label, json_array_len(inventory, field)))
    .collect()
}

fn language_surface_flags(
    surface: &serde_json::Value,
) -> BTreeMap<&'static str, bool> {
    ["parse", "check", "lower", "source_map", "rare_diagnostics"]
        .into_iter()
        .map(|flag| (flag, surface[flag].as_bool().unwrap_or(false)))
        .collect()
}

fn platform_model_grades(manifest: &serde_json::Value) -> BTreeMap<String, String> {
    manifest["platform_models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|model| {
            Some((
                model["kind"].as_str()?.to_owned(),
                model["replay_grade"].as_str().unwrap_or("missing").to_owned(),
            ))
        })
        .collect()
}

fn adapter_decisions(manifest: &serde_json::Value) -> BTreeMap<String, String> {
    manifest["adapters"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|adapter| {
            Some((
                adapter["kind"].as_str()?.to_owned(),
                adapter_decision(adapter).to_owned(),
            ))
        })
        .collect()
}

fn adapter_decision(adapter: &serde_json::Value) -> &'static str {
    match adapter["crate_source"]["kind"].as_str() {
        Some("project_module") => "kobo_owned_adapter",
        Some("cargo_dependency" | "std") => "formal_adapter",
        Some("generated_backend") => "generated_backend",
        _ => "foreign_boundary",
    }
}

fn test_release_parity_counts(
    parity: &serde_json::Value,
) -> BTreeMap<&'static str, usize> {
    let mut counts = [
        ("upstream_tests", "upstream_tests"),
        ("kobo_replay_tests", "kobo_replay_tests"),
        ("kobo_liveness_tests", "kobo_liveness_tests"),
        ("cli_behavior", "cli_behavior"),
        ("config_behavior", "config_behavior"),
        ("exit_behavior", "exit_behavior"),
        ("logging_behavior", "logging_behavior"),
        ("package_behavior", "package_behavior"),
        ("platform_behavior", "platform_behavior"),
        ("install_behavior", "install_behavior"),
    ]
    .into_iter()
    .map(|(label, field)| (label, usize::from(!parity[field].is_null())))
    .collect::<BTreeMap<_, _>>();
    counts.insert("performance", parity["performance"].as_object().map_or(0, |value| value.len()));
    counts.insert("release_artifacts", json_array_len(parity, "release_artifacts"));
    counts
}

fn adapter_debt_summary_for_file(file: &Path) -> AdapterDebtSummary {
    let Some(state) = read_watch_state_for_file(file) else {
        return AdapterDebtSummary::none();
    };
    AdapterDebtSummary {
        watcher: watcher_adapter_debt(&state),
        process: process_adapter_debt(&state),
        time: time_adapter_debt(&state),
        path_filter: path_filter_adapter_debt(&state),
        async_runtime: async_runtime_adapter_debt(&state),
    }
}

fn read_watch_state_for_file(file: &Path) -> Option<serde_json::Value> {
    for directory in file.parent().into_iter().flat_map(Path::ancestors) {
        let state_path = directory
            .join(".kobo")
            .join("watch")
            .join("source-watch.json");
        let Ok(source) = fs::read_to_string(state_path) else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&source) {
            if value["mode"].as_str() == Some("source_watch_state") {
                return Some(value);
            }
        }
    }
    None
}

fn watcher_adapter_debt(state: &serde_json::Value) -> AdapterDebtStatus {
    let evidence = state["watcher_evidence"].as_str().unwrap_or("missing");
    if evidence == "opaque" || evidence == "missing" {
        return AdapterDebtStatus::blocker(format!("watcher evidence is {evidence}"));
    }
    AdapterDebtStatus::acceptable(evidence)
}

fn process_adapter_debt(state: &serde_json::Value) -> AdapterDebtStatus {
    let Some(lifecycle) = state["child_lifecycle_obligations"].as_array() else {
        return AdapterDebtStatus::blocker("missing child lifecycle evidence");
    };
    if lifecycle.is_empty() {
        return AdapterDebtStatus::blocker("empty child lifecycle evidence");
    }
    if lifecycle.iter().any(|entry| {
        entry["resolution"]
            .as_str()
            .is_some_and(|resolution| resolution.contains("unresolved"))
    }) {
        return AdapterDebtStatus::blocker("unresolved child lifecycle obligation");
    }
    AdapterDebtStatus::acceptable("lifecycle modeled")
}

fn time_adapter_debt(state: &serde_json::Value) -> AdapterDebtStatus {
    let Some(windows) = state["debounce_windows"].as_array() else {
        return AdapterDebtStatus::blocker("missing debounce evidence");
    };
    if windows.is_empty() {
        return AdapterDebtStatus::blocker("empty debounce evidence");
    }
    if windows.iter().any(|window| {
        window["timer_evidence"].as_str() == Some("missing")
            || window["replay_grade"].as_str() == Some("debt")
    }) {
        return AdapterDebtStatus::blocker("incomplete timer evidence");
    }
    AdapterDebtStatus::acceptable("debounce evidence")
}

fn path_filter_adapter_debt(state: &serde_json::Value) -> AdapterDebtStatus {
    let Some(summary) = adapter_summary_by_kind(state, "path_filter") else {
        return AdapterDebtStatus::blocker("missing path filter adapter summary");
    };
    if !summary_has_conformance(summary, "source-watch-path-match") {
        return AdapterDebtStatus::blocker("missing path filter conformance evidence");
    }
    if state_has_path_scope(state) || state_has_path_filter_decision(state) {
        return AdapterDebtStatus::acceptable("path filter summary modeled");
    }
    AdapterDebtStatus::blocker("missing source-visible path filter facts")
}

fn async_runtime_adapter_debt(state: &serde_json::Value) -> AdapterDebtStatus {
    let Some(summary) = adapter_summary_by_kind(state, "async_runtime") else {
        return AdapterDebtStatus::blocker("missing async runtime adapter summary");
    };
    if !summary_has_scheduler_fact(summary, "source-watch-task-order") {
        return AdapterDebtStatus::blocker("missing async scheduler facts");
    }
    if state_has_timer_or_restart_order(state) {
        return AdapterDebtStatus::acceptable("async runtime summary modeled");
    }
    AdapterDebtStatus::blocker("missing source-visible async runtime facts")
}

fn adapter_summary_by_kind<'a>(
    state: &'a serde_json::Value,
    kind: &str,
) -> Option<&'a serde_json::Value> {
    state["adapter_summaries"]
        .as_array()?
        .iter()
        .find(|summary| summary["kind"].as_str() == Some(kind))
}

fn summary_has_conformance(summary: &serde_json::Value, required: &str) -> bool {
    summary["conformance_tests"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|test| test.as_str() == Some(required))
}

fn summary_has_scheduler_fact(summary: &serde_json::Value, required: &str) -> bool {
    summary["scheduler_facts"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|fact| fact.as_str() == Some(required))
}

fn state_has_path_scope(state: &serde_json::Value) -> bool {
    state["scope"]["files"]
        .as_array()
        .is_some_and(|files| !files.is_empty())
}

fn state_has_path_filter_decision(state: &serde_json::Value) -> bool {
    state["event_batches"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|batch| batch["events"].as_array().into_iter().flatten())
        .any(|event| {
            event["filter_decisions"]
                .as_array()
                .is_some_and(|filters| !filters.is_empty())
        })
}

fn state_has_timer_or_restart_order(state: &serde_json::Value) -> bool {
    state["debounce_windows"]
        .as_array()
        .is_some_and(|windows| !windows.is_empty())
        || state["restart_decisions"]
            .as_array()
            .is_some_and(|decisions| !decisions.is_empty())
}

fn module_ownership_summary_for_root(
    source_root: &Path,
    adapter_debt: &AdapterDebtSummary,
) -> anyhow::Result<ModuleOwnershipSummary> {
    let mut summary = ModuleOwnershipSummary {
        kobo_owned: Vec::new(),
        rust_owned: Vec::new(),
        boundary_debt: adapter_boundary_debt_modules(adapter_debt),
    };
    collect_module_ownership_files(source_root, source_root, &mut summary)?;
    summary.kobo_owned.sort();
    summary.rust_owned.sort();
    summary.boundary_debt.sort();
    Ok(summary)
}

fn source_tree_root(file: &Path) -> PathBuf {
    for ancestor in file.ancestors() {
        if ancestor.file_name().and_then(|name| name.to_str()) == Some("src") {
            return ancestor.to_path_buf();
        }
    }
    file.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn project_root_for_source_root(source_root: &Path) -> PathBuf {
    if source_root.file_name().and_then(|name| name.to_str()) == Some("src") {
        return source_root
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| source_root.to_path_buf());
    }
    source_root.to_path_buf()
}

fn json_array_len(value: &serde_json::Value, field: &str) -> usize {
    value[field].as_array().map_or(0, Vec::len)
}

fn collect_module_ownership_files(
    root: &Path,
    directory: &Path,
    summary: &mut ModuleOwnershipSummary,
) -> anyhow::Result<()> {
    if !directory.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(directory)
        .with_context(|| format!("failed to read module directory {}", directory.display()))?
    {
        let entry = entry.context("failed to read module directory entry")?;
        let path = entry.path();
        if path.is_dir() {
            collect_module_ownership_files(root, &path, summary)?;
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        let module_path = path
            .strip_prefix(root)
            .unwrap_or(path.as_path())
            .display()
            .to_string()
            .replace('\\', "/");
        match extension {
            "kobo" => summary.kobo_owned.push(module_path),
            "rs" => summary.rust_owned.push(module_path),
            _ => {}
        }
    }
    Ok(())
}

fn adapter_boundary_debt_modules(adapter_debt: &AdapterDebtSummary) -> Vec<String> {
    [
        ("watcher adapter", &adapter_debt.watcher),
        ("process adapter", &adapter_debt.process),
        ("time adapter", &adapter_debt.time),
        ("path filter adapter", &adapter_debt.path_filter),
        ("async runtime adapter", &adapter_debt.async_runtime),
    ]
    .into_iter()
    .filter_map(|(name, status)| {
        (status.status != "none").then(|| format!("{name}: {}", status.human_label()))
    })
    .collect()
}

fn adapter_boundary_classification(module: &str) -> &'static str {
    if module.contains("blocker(") {
        "debt"
    } else if module.contains("metadata-only") {
        "metadata-only"
    } else if module.contains("opaque") {
        "opaque"
    } else if module.contains("sampled") {
        "sampled"
    } else {
        "adapter-backed"
    }
}

fn normalized_display(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}
