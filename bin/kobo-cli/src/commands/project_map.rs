use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;

const PROJECT_MAP_SCHEMA_VERSION: u64 = 1;

pub(super) struct ProjectMapReport {
    source_root: String,
    digest: String,
    module_ownership: ModuleOwnershipSummary,
    adapter_debt: AdapterDebtSummary,
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
        let adapter_debt = adapter_debt_summary_for_file(file);
        let module_ownership = module_ownership_summary_for_root(&source_root, &adapter_debt)?;
        let source_root = normalized_display(&source_root);
        let digest = project_map_digest(&source_root, &module_ownership, &adapter_debt)?;
        Ok(Self {
            source_root,
            digest,
            module_ownership,
            adapter_debt,
        })
    }

    pub(super) fn to_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "schema_version": PROJECT_MAP_SCHEMA_VERSION,
            "source_root": self.source_root,
            "digest": self.digest,
            "module_ownership": self.module_ownership.to_json_value(),
            "adapter_debt": self.adapter_debt.to_json_value(),
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
            self.module_ownership.boundary_debt.len()
        )
    }

    pub(super) fn inspect_comment(&self) -> String {
        format!("// kobo-project-map: {}", self.human_label_fields())
    }

    fn human_label_fields(&self) -> String {
        format!(
            "digest={} kobo-owned={} rust-owned={} boundary-debt={}",
            self.digest,
            self.module_ownership.kobo_owned.len(),
            self.module_ownership.rust_owned.len(),
            self.module_ownership.boundary_debt.len()
        )
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
    adapter_debt: &AdapterDebtSummary,
) -> anyhow::Result<String> {
    let payload = serde_json::json!({
        "schema_version": PROJECT_MAP_SCHEMA_VERSION,
        "source_root": source_root,
        "module_ownership": module_ownership.to_json_value(),
        "adapter_debt": adapter_debt.to_json_value(),
    });
    Ok(kobo_sim_core::digest::stable_hash(&serde_json::to_string(
        &payload,
    )?))
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

fn normalized_display(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}
