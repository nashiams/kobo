mod evidence;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::session::{build_session, render_diagnostics};
use evidence::{
    DebounceWindowInput, DuplicateStatus, RestartPolicyBranch, WatchEventInput, WatchEventKind,
    WatchEventPathInput, WatchEvidence, WatchExecutionMode, WatchPathRole, WatchRerunOutcome,
    WatchRerunReport, DEBOUNCE_INTERVAL_MS,
};
use kobo_driver::{run_check_pipeline, run_codegen_pipeline};

/// File-watcher re-run on save.
///
/// In `--simple` mode: polls the file's modification time, and when it changes,
/// triggers a recompile. This avoids pulling in platform-specific inotify/FSEvents
/// dependencies — suitable for development workflows.
///
/// In `--build` mode: runs the full codegen pipeline on each change, producing
/// generated Rust output — suitable for continuous build feedback (S-29).
pub(super) fn cmd_watch(
    file: Option<&Path>,
    simple: bool,
    build: bool,
    plan: bool,
    changed: Option<&Path>,
) -> anyhow::Result<()> {
    if plan {
        return cmd_watch_plan(file, changed);
    }
    let Some(file) = file else {
        anyhow::bail!("unscoped workspace watch is disabled; pass a FILE or use --plan FILE");
    };
    if !simple && !build {
        println!("Use --simple for basic save-and-recheck mode, or --build for codegen+compile.");
        return Ok(());
    }

    let mode_label = if build { "build" } else { "simple" };
    let run_once = std::env::var_os("KOBO_WATCH_ONCE").is_some();
    let max_windows = watch_max_windows();
    let scope = watch_scope(file)?;
    let state_path = source_watch_state_path()?;
    let persisted_state_loaded = state_path.is_file();
    persist_source_watch_state(file, &scope, persisted_state_loaded, &[])?;
    println!(
        "Watching {} ({mode_label} mode, scoped-persist-reload, Ctrl+C to stop)",
        file.display(),
    );
    println!("scope: {}", relative_display(file));
    println!("persisted state: {}", state_path.display());
    println!("persisted state loaded: {persisted_state_loaded}");
    println!("reload checkpoint: source-map-and-diagnostics");
    println!("restartable: true");

    let mut scope = scope;
    let mut watched = watched_files(&scope)?;
    let mut change_sequence = 0;
    let mut evidence_history = Vec::new();

    loop {
        std::thread::sleep(Duration::from_millis(DEBOUNCE_INTERVAL_MS));

        let Some(window) = collect_debounce_window(&mut scope, &mut watched)? else {
            continue;
        };
        change_sequence += 1;
        let mut evidence = WatchEvidence::for_window(
            change_sequence,
            relative_display(file),
            window
                .changes
                .into_iter()
                .map(WatchChange::into_event_input)
                .collect(),
            DebounceWindowInput {
                has_timer_extension: window.has_timer_extension,
                extension_cause_paths: window.extension_cause_paths,
            },
            if build {
                WatchExecutionMode::Build
            } else {
                WatchExecutionMode::Simple
            },
        );

        let changed_paths = evidence.changed_paths();
        println!(
            "[kobo-watch] Change detected in {}, recompiling...",
            changed_paths.join(", ")
        );
        for changed_path in changed_paths {
            println!("changed file: {changed_path}");
        }
        let report = if build {
            on_file_changed_build(file)
        } else {
            on_file_changed(file)
        };
        evidence.record_rerun_report(report);
        for line in evidence.human_lines() {
            println!("{line}");
        }
        evidence_history.push(evidence);
        persist_source_watch_state(file, &scope, persisted_state_loaded, &evidence_history)?;
        if run_once || max_windows.is_some_and(|limit| evidence_history.len() >= limit) {
            break;
        }
    }

    Ok(())
}

fn cmd_watch_plan(file: Option<&Path>, changed: Option<&Path>) -> anyhow::Result<()> {
    let Some(file) = file else {
        anyhow::bail!(
            "unscoped workspace watch is disabled; pass an explicit FILE to plan invalidation"
        );
    };
    let scope = watch_scope(file)?;
    let target = relative_display(file);
    let changed_display = changed.map(relative_display);
    let plan_branch = changed
        .map(|path| plan_change_branch(&scope, path))
        .unwrap_or(RestartPolicyBranch::PlanOnly);
    let evidence = WatchEvidence::for_plan(1, target.clone(), changed_display.clone(), plan_branch);
    println!("Watch plan");
    println!("mode: scoped-persist-reload");
    println!("scope: {target}");
    println!("persisted scope: {target}");
    println!("reload checkpoint: source-map-and-diagnostics");
    println!("restartable: true");
    println!("files:");
    for file in &scope.files {
        println!("- {}", relative_display(file));
    }
    println!("rerun targets:");
    println!("rerun target: kobo check {target}");
    println!("rerun target: kobo inspect {target}");
    for line in evidence.human_lines() {
        println!("{line}");
    }
    if let Some(changed) = changed_display {
        if matches!(plan_branch, RestartPolicyBranch::ChangedInScope) {
            println!("invalidated: {changed}");
            println!("reason: changed file belongs to scoped watch plan");
            println!("rerun target: kobo check {target}");
            println!("rerun target: kobo inspect {target}");
        } else {
            println!("ignored: {changed}");
            println!("reason: changed file is outside scoped watch plan");
        }
    }
    Ok(())
}

struct WatchScope {
    root: PathBuf,
    root_file: PathBuf,
    files: Vec<PathBuf>,
}

struct WatchedFile {
    path: PathBuf,
    snapshot: Option<FileSnapshot>,
}

#[derive(Clone, Copy)]
struct FileSnapshot {
    modified: SystemTime,
    len: u64,
    is_readonly: bool,
}

struct WatchChange {
    display: String,
    event_kind: WatchEventKind,
    previous_modified_ms: Option<u128>,
    current_modified_ms: Option<u128>,
    duplicate_status: DuplicateStatus,
    additional_paths: Vec<WatchEventPathInput>,
}

struct DebounceWindow {
    changes: Vec<WatchChange>,
    has_timer_extension: bool,
    extension_cause_paths: Vec<String>,
}

fn watch_scope(file: &Path) -> anyhow::Result<WatchScope> {
    let root = file.parent().unwrap_or_else(|| Path::new("."));
    let mut files = Vec::new();
    collect_kobo_watch_files(root, &mut files)?;
    if !files.iter().any(|candidate| same_path(candidate, file)) {
        files.push(file.to_path_buf());
    }
    files.sort();
    Ok(WatchScope {
        root: root.to_path_buf(),
        root_file: file.to_path_buf(),
        files,
    })
}

fn collect_kobo_watch_files(dir: &Path, files: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    let entries = std::fs::read_dir(dir)
        .map_err(|error| anyhow::anyhow!("cannot read watch scope {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| anyhow::anyhow!("cannot read watch entry: {error}"))?;
        let path = entry.path();
        if path.is_dir() {
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| matches!(name, "target" | ".git" | ".kobo"))
            {
                continue;
            }
            collect_kobo_watch_files(&path, files)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("kobo") {
            files.push(path);
        }
    }
    Ok(())
}

fn same_path(left: &Path, right: &Path) -> bool {
    normalized_path(left) == normalized_path(right)
}

fn normalized_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| absolute_path(path))
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

fn plan_change_branch(scope: &WatchScope, changed: &Path) -> RestartPolicyBranch {
    if path_in_scope(scope, changed) {
        RestartPolicyBranch::ChangedInScope
    } else {
        RestartPolicyBranch::IgnoredOutOfScope
    }
}

fn path_in_scope(scope: &WatchScope, changed: &Path) -> bool {
    scope.files.iter().any(|file| same_path(file, changed))
}

fn relative_display(path: &Path) -> String {
    let absolute = absolute_path(path);
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    absolute
        .strip_prefix(&cwd)
        .unwrap_or(&absolute)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn source_watch_state_path() -> anyhow::Result<PathBuf> {
    let root = std::env::current_dir()
        .map_err(|error| anyhow::anyhow!("failed to determine current directory: {error}"))?;
    let state_dir = root.join(".kobo").join("watch");
    std::fs::create_dir_all(&state_dir)
        .map_err(|error| anyhow::anyhow!("failed to create {}: {error}", state_dir.display()))?;
    Ok(state_dir.join("source-watch.json"))
}

fn watch_max_windows() -> Option<usize> {
    std::env::var("KOBO_WATCH_MAX_WINDOWS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|limit| *limit > 0)
}

fn persist_source_watch_state(
    root_file: &Path,
    scope: &WatchScope,
    persisted_state_loaded: bool,
    evidence_history: &[WatchEvidence],
) -> anyhow::Result<()> {
    let state_path = source_watch_state_path()?;
    let scope_files = scope
        .files
        .iter()
        .map(|file| relative_display(file))
        .collect::<Vec<_>>();
    let event_batches = evidence_history
        .iter()
        .map(WatchEvidence::event_batch_json)
        .collect::<Vec<_>>();
    let debounce_windows = evidence_history
        .iter()
        .map(WatchEvidence::debounce_window_json)
        .collect::<Vec<_>>();
    let restart_decisions = evidence_history
        .iter()
        .map(WatchEvidence::restart_decision_json)
        .collect::<Vec<_>>();
    let child_lifecycle_obligations = evidence_history
        .iter()
        .map(WatchEvidence::child_lifecycle_json)
        .collect::<Vec<_>>();
    let changes = evidence_history
        .last()
        .map(WatchEvidence::changes_json)
        .unwrap_or_default();
    let watcher_evidence = evidence_history
        .last()
        .map(WatchEvidence::watcher_evidence)
        .unwrap_or("metadata-only");
    let value = serde_json::json!({
        "schema_version": 1,
        "mode": "source_watch_state",
        "scope": {
            "root": relative_display(root_file),
            "files": scope_files,
        },
        "reload_checkpoint": "source-map-and-diagnostics",
        "restartable": true,
        "persisted_state_loaded": persisted_state_loaded,
        "watcher_evidence": watcher_evidence,
        "rerun_targets": [
            format!("kobo check {}", relative_display(root_file)),
            format!("kobo inspect {}", relative_display(root_file)),
        ],
        "changes": changes,
        "event_batches": event_batches,
        "debounce_windows": debounce_windows,
        "restart_decisions": restart_decisions,
        "child_lifecycle_obligations": child_lifecycle_obligations,
    });
    std::fs::write(&state_path, serde_json::to_vec_pretty(&value)?)
        .map_err(|error| anyhow::anyhow!("failed to write {}: {error}", state_path.display()))
}

fn watched_files(scope: &WatchScope) -> anyhow::Result<Vec<WatchedFile>> {
    scope
        .files
        .iter()
        .map(|path| {
            Ok(WatchedFile {
                path: path.clone(),
                snapshot: Some(get_file_snapshot(path)?),
            })
        })
        .collect()
}

fn collect_debounce_window(
    scope: &mut WatchScope,
    watched: &mut Vec<WatchedFile>,
) -> anyhow::Result<Option<DebounceWindow>> {
    let mut changes = collect_watch_changes(scope, watched)?;
    if changes.is_empty() {
        return Ok(None);
    }
    let mut has_timer_extension = false;
    let mut extension_cause_paths = Vec::new();

    loop {
        std::thread::sleep(Duration::from_millis(DEBOUNCE_INTERVAL_MS));
        let later_changes = collect_watch_changes(scope, watched)?;
        if later_changes.is_empty() {
            changes = normalize_rename_events(changes);
            changes.sort_by(|left, right| left.display.cmp(&right.display));
            extension_cause_paths.sort();
            extension_cause_paths.dedup();
            return Ok(Some(DebounceWindow {
                changes,
                has_timer_extension,
                extension_cause_paths,
            }));
        }
        has_timer_extension = true;
        extension_cause_paths.extend(later_changes.iter().map(|change| change.display.clone()));
        merge_window_changes(&mut changes, later_changes);
    }
}

fn normalize_rename_events(changes: Vec<WatchChange>) -> Vec<WatchChange> {
    let mut remaining = changes;
    let remove_index = remaining
        .iter()
        .position(|change| matches!(change.event_kind, WatchEventKind::Remove));
    let create_index = remaining
        .iter()
        .position(|change| matches!(change.event_kind, WatchEventKind::Create));

    let (Some(remove_index), Some(create_index)) = (remove_index, create_index) else {
        return remaining;
    };
    if remove_index == create_index {
        return remaining;
    }

    let create = remaining.remove(create_index);
    let adjusted_remove_index = if create_index < remove_index {
        remove_index - 1
    } else {
        remove_index
    };
    let remove = remaining.remove(adjusted_remove_index);

    if !same_parent_display(&remove.display, &create.display) {
        remaining.push(remove);
        remaining.push(create);
        return remaining;
    }

    remaining.push(WatchChange::renamed(remove, create));
    remaining
}

fn same_parent_display(left: &str, right: &str) -> bool {
    Path::new(left).parent() == Path::new(right).parent()
}

fn collect_watch_changes(
    scope: &mut WatchScope,
    watched: &mut Vec<WatchedFile>,
) -> anyhow::Result<Vec<WatchChange>> {
    let mut changes = changed_existing_files(watched);
    refresh_watch_scope(scope)?;
    changes.extend(created_watch_files(scope, watched));
    Ok(changes)
}

fn changed_existing_files(watched: &mut [WatchedFile]) -> Vec<WatchChange> {
    let mut changes = Vec::new();
    for file in watched {
        match get_file_snapshot(&file.path) {
            Ok(current) => match file.snapshot {
                Some(previous) if current.has_modified_change(previous) => {
                    file.snapshot = Some(current);
                    changes.push(WatchChange::modified(
                        &file.path,
                        previous.modified,
                        current.modified,
                    ));
                }
                Some(previous) if current.has_metadata_change(previous) => {
                    file.snapshot = Some(current);
                    changes.push(WatchChange::metadata(
                        &file.path,
                        previous.modified,
                        current.modified,
                    ));
                }
                None => {
                    file.snapshot = Some(current);
                    changes.push(WatchChange::created(&file.path, current.modified));
                }
                _ => {}
            },
            Err(_) if file.snapshot.is_some() => {
                let previous = file.snapshot.take();
                changes.push(WatchChange::removed(&file.path, previous));
            }
            Err(_) => {}
        }
    }
    changes
}

fn refresh_watch_scope(scope: &mut WatchScope) -> anyhow::Result<()> {
    let mut files = Vec::new();
    collect_kobo_watch_files(&scope.root, &mut files)?;
    if !files
        .iter()
        .any(|candidate| same_path(candidate, &scope.root_file))
    {
        files.push(scope.root_file.clone());
    }
    files.sort();
    scope.files = files;
    Ok(())
}

fn created_watch_files(scope: &WatchScope, watched: &mut Vec<WatchedFile>) -> Vec<WatchChange> {
    let mut changes = Vec::new();
    for file in &scope.files {
        if watched
            .iter()
            .any(|watched_file| same_path(&watched_file.path, file))
        {
            continue;
        }
        match get_file_snapshot(file) {
            Ok(current) => {
                watched.push(WatchedFile {
                    path: file.clone(),
                    snapshot: Some(current),
                });
                changes.push(WatchChange::created(file, current.modified));
            }
            Err(_) => {
                watched.push(WatchedFile {
                    path: file.clone(),
                    snapshot: None,
                });
                changes.push(WatchChange::unknown(file));
            }
        }
    }
    changes
}

fn merge_window_changes(changes: &mut Vec<WatchChange>, later_changes: Vec<WatchChange>) {
    for later_change in later_changes {
        if let Some(existing) = changes
            .iter_mut()
            .find(|existing| existing.display == later_change.display)
        {
            existing.event_kind = later_change.event_kind;
            existing.current_modified_ms = later_change.current_modified_ms;
            existing.duplicate_status = DuplicateStatus::Coalesced;
        } else {
            changes.push(later_change);
        }
    }
}

impl WatchChange {
    fn modified(path: &Path, previous: SystemTime, current: SystemTime) -> Self {
        Self {
            display: relative_display(path),
            event_kind: WatchEventKind::Modify,
            previous_modified_ms: Some(system_time_millis(previous)),
            current_modified_ms: Some(system_time_millis(current)),
            duplicate_status: DuplicateStatus::Unique,
            additional_paths: Vec::new(),
        }
    }

    fn created(path: &Path, current: SystemTime) -> Self {
        Self {
            display: relative_display(path),
            event_kind: WatchEventKind::Create,
            previous_modified_ms: None,
            current_modified_ms: Some(system_time_millis(current)),
            duplicate_status: DuplicateStatus::Unique,
            additional_paths: Vec::new(),
        }
    }

    fn metadata(path: &Path, previous: SystemTime, current: SystemTime) -> Self {
        Self {
            display: relative_display(path),
            event_kind: WatchEventKind::Metadata,
            previous_modified_ms: Some(system_time_millis(previous)),
            current_modified_ms: Some(system_time_millis(current)),
            duplicate_status: DuplicateStatus::Unique,
            additional_paths: Vec::new(),
        }
    }

    fn removed(path: &Path, previous: Option<FileSnapshot>) -> Self {
        Self {
            display: relative_display(path),
            event_kind: WatchEventKind::Remove,
            previous_modified_ms: previous.map(|snapshot| system_time_millis(snapshot.modified)),
            current_modified_ms: None,
            duplicate_status: DuplicateStatus::Unique,
            additional_paths: Vec::new(),
        }
    }

    fn unknown(path: &Path) -> Self {
        Self {
            display: relative_display(path),
            event_kind: WatchEventKind::Unknown,
            previous_modified_ms: None,
            current_modified_ms: None,
            duplicate_status: DuplicateStatus::Unknown,
            additional_paths: Vec::new(),
        }
    }

    fn renamed(remove: WatchChange, create: WatchChange) -> Self {
        let parent_path = Path::new(&create.display)
            .parent()
            .map(|path| {
                path.components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .unwrap_or_default();
        Self {
            display: remove.display,
            event_kind: WatchEventKind::Rename,
            previous_modified_ms: remove.previous_modified_ms,
            current_modified_ms: create.current_modified_ms,
            duplicate_status: DuplicateStatus::Coalesced,
            additional_paths: vec![
                WatchEventPathInput {
                    role: WatchPathRole::DestinationPath,
                    path: create.display,
                },
                WatchEventPathInput {
                    role: WatchPathRole::ParentPath,
                    path: parent_path,
                },
            ],
        }
    }

    fn into_event_input(self) -> WatchEventInput {
        let mut paths = vec![WatchEventPathInput {
            role: WatchPathRole::SourcePath,
            path: self.display,
        }];
        paths.extend(self.additional_paths);
        WatchEventInput {
            paths,
            event_kind: self.event_kind,
            previous_modified_ms: self.previous_modified_ms,
            current_modified_ms: self.current_modified_ms,
            duplicate_status: self.duplicate_status,
        }
    }
}

fn system_time_millis(value: SystemTime) -> u128 {
    value
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

/// Get file modification time.
fn get_mtime(file: &Path) -> anyhow::Result<SystemTime> {
    let metadata = std::fs::metadata(file)
        .map_err(|e| anyhow::anyhow!("cannot stat {}: {}", file.display(), e))?;
    metadata
        .modified()
        .map_err(|e| anyhow::anyhow!("cannot get mtime for {}: {}", file.display(), e))
}

fn get_file_snapshot(file: &Path) -> anyhow::Result<FileSnapshot> {
    let metadata = std::fs::metadata(file)
        .map_err(|e| anyhow::anyhow!("cannot stat {}: {}", file.display(), e))?;
    let modified = metadata
        .modified()
        .map_err(|e| anyhow::anyhow!("cannot get mtime for {}: {}", file.display(), e))?;
    Ok(FileSnapshot {
        modified,
        len: metadata.len(),
        is_readonly: metadata.permissions().readonly(),
    })
}

impl FileSnapshot {
    fn has_modified_change(self, previous: Self) -> bool {
        self.modified != previous.modified
    }

    fn has_metadata_change(self, previous: Self) -> bool {
        self.len != previous.len || self.is_readonly != previous.is_readonly
    }
}

/// Called when a file change is detected — runs the check pipeline.
fn on_file_changed(file: &Path) -> WatchRerunReport {
    println!("[kobo-watch] Triggering rebuild for {}", file.display());
    match build_session(file, None) {
        Ok(mut session) => match run_check_pipeline(&mut session, file) {
            Ok(()) => {
                render_diagnostics(&session);
                if session.diagnostics.is_empty() {
                    println!("[kobo-watch] OK — no diagnostics");
                } else {
                    println!("[kobo-watch] {} diagnostic(s)", session.diagnostics.len());
                }
                WatchRerunReport {
                    outcome: WatchRerunOutcome::Succeeded,
                    diagnostic_count: session.diagnostics.len(),
                }
            }
            Err(()) => {
                render_diagnostics(&session);
                eprintln!("[kobo-watch] check failed");
                WatchRerunReport {
                    outcome: WatchRerunOutcome::Failed,
                    diagnostic_count: session.diagnostics.len(),
                }
            }
        },
        Err(e) => {
            eprintln!("[kobo-watch] session error: {e}");
            WatchRerunReport {
                outcome: WatchRerunOutcome::Failed,
                diagnostic_count: 0,
            }
        }
    }
}

/// S-29: Called when a file change is detected in --build mode — runs full codegen pipeline.
fn on_file_changed_build(file: &Path) -> WatchRerunReport {
    println!(
        "[kobo-watch] Triggering codegen build for {}",
        file.display()
    );
    match build_session(file, None) {
        Ok(mut session) => match run_codegen_pipeline(&mut session, file) {
            Ok(artifacts) => {
                render_diagnostics(&session);
                println!(
                    "[kobo-watch] codegen OK — wrote {}",
                    artifacts.rs_path.display()
                );
                WatchRerunReport {
                    outcome: WatchRerunOutcome::Succeeded,
                    diagnostic_count: session.diagnostics.len(),
                }
            }
            Err(()) => {
                render_diagnostics(&session);
                eprintln!("[kobo-watch] codegen failed");
                WatchRerunReport {
                    outcome: WatchRerunOutcome::Failed,
                    diagnostic_count: session.diagnostics.len(),
                }
            }
        },
        Err(e) => {
            eprintln!("[kobo-watch] session error: {e}");
            WatchRerunReport {
                outcome: WatchRerunOutcome::Failed,
                diagnostic_count: 0,
            }
        }
    }
}

/// Detect whether a file has been modified since a given timestamp.
/// Used for testing the watch detection logic without entering the loop.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn detect_change(file: &Path, since: SystemTime) -> anyhow::Result<bool> {
    let current = get_mtime(file)?;
    Ok(current != since)
}

/// Debounce interval in milliseconds.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const DEBOUNCE_MS: u64 = DEBOUNCE_INTERVAL_MS;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn watch_detects_file_change() {
        let dir = std::env::temp_dir().join("kobo_watch_test");
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("test.kobo");

        // Create initial file
        std::fs::write(&file, "fn main() {}").unwrap();
        let t0 = get_mtime(&file).unwrap();

        // No change yet
        assert!(!detect_change(&file, t0).unwrap());

        // Wait to ensure filesystem timestamp granularity
        std::thread::sleep(std::time::Duration::from_millis(DEBOUNCE_MS + 100));

        // Modify the file
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&file)
            .unwrap();
        f.write_all(b"fn main() { println!(\"updated\"); }")
            .unwrap();
        f.flush().unwrap();
        drop(f);

        // Now detect change
        assert!(detect_change(&file, t0).unwrap());

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn watch_no_change_same_content() {
        let dir = std::env::temp_dir().join("kobo_watch_test2");
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("test2.kobo");

        std::fs::write(&file, "fn main() {}").unwrap();
        let t0 = get_mtime(&file).unwrap();

        // No modification → no change
        assert!(!detect_change(&file, t0).unwrap());

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn debounce_interval_is_200ms() {
        assert_eq!(DEBOUNCE_MS, 200);
    }

    #[test]
    fn unknown_watch_change_persists_partial_unknown_evidence() {
        let change = WatchChange::unknown(Path::new("src/raced.kobo"));
        let evidence = WatchEvidence::for_window(
            1,
            "src/main.kobo".to_owned(),
            vec![change.into_event_input()],
            DebounceWindowInput {
                has_timer_extension: false,
                extension_cause_paths: Vec::new(),
            },
            WatchExecutionMode::Simple,
        );
        let batch = evidence.event_batch_json();

        assert_eq!(batch["events"][0]["kind"], "unknown");
        assert_eq!(batch["replay_grade"], "partial");
        assert_eq!(batch["events"][0]["duplicate_or_coalesced"], "unknown");
    }
}
