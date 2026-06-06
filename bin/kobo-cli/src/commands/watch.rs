mod evidence;
mod trace;

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::session::{build_session, render_diagnostics};
use evidence::{
    DebounceWindowInput, DuplicateStatus, EventEvidenceGrade, RawWatchEventInput,
    RestartPolicyBranch, WatchEventInput, WatchEventKind, WatchEventPathInput, WatchEvidence,
    WatchExecutionMode, WatchPathRole, WatchRerunOutcome, WatchRerunReport, DEBOUNCE_INTERVAL_MS,
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
    import_trace: Option<&Path>,
    witness_out: Option<&Path>,
) -> anyhow::Result<()> {
    if let Some(trace_path) = import_trace {
        return trace::cmd_import_trace(trace_path, witness_out);
    }
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
    let snapshot_source = FsWatchSnapshotSource;
    let mut watched = watched_files(&scope, &snapshot_source)?;
    let mut change_sequence = 0;
    let mut evidence_history = Vec::new();

    loop {
        std::thread::sleep(Duration::from_millis(DEBOUNCE_INTERVAL_MS));

        let Some(window) = collect_debounce_window(&mut scope, &mut watched, &snapshot_source)?
        else {
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

pub(super) fn is_watch_trace_witness(witness: &serde_json::Value) -> bool {
    trace::is_watch_trace_witness(witness)
}

pub(super) fn replay_watch_trace_witness(
    witness: &serde_json::Value,
    error_format: crate::ErrorFormat,
) -> anyhow::Result<()> {
    trace::replay_watch_trace_witness(witness, error_format)
}

pub(super) fn enforce_release_lifecycle_gate(
    file: &Path,
    error_format: crate::ErrorFormat,
) -> anyhow::Result<()> {
    let Some(state) = source_watch_state_for_file(file)? else {
        return Ok(());
    };
    let Some(reason) = release_lifecycle_gate_reason(&state) else {
        return Ok(());
    };
    emit_release_lifecycle_gate(&reason, error_format)?;
    anyhow::bail!("release watch lifecycle gate blocked {reason}")
}

fn source_watch_state_for_file(file: &Path) -> anyhow::Result<Option<serde_json::Value>> {
    for directory in file.parent().into_iter().flat_map(Path::ancestors) {
        let state_path = directory
            .join(".kobo")
            .join("watch")
            .join("source-watch.json");
        if !state_path.is_file() {
            continue;
        }
        let source = std::fs::read_to_string(&state_path)
            .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", state_path.display()))?;
        let value = serde_json::from_str::<serde_json::Value>(&source).map_err(|error| {
            anyhow::anyhow!("failed to parse {}: {error}", state_path.display())
        })?;
        if value["mode"].as_str() == Some("source_watch_state") {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

fn release_lifecycle_gate_reason(state: &serde_json::Value) -> Option<String> {
    let Some(lifecycle) = state["child_lifecycle_obligations"].as_array() else {
        return Some("missing child lifecycle evidence".to_owned());
    };
    if lifecycle.is_empty() {
        return Some("empty child lifecycle evidence".to_owned());
    }
    for entry in lifecycle {
        let resolution = entry["resolution"].as_str().unwrap_or("missing");
        if !is_release_lifecycle_resolution(resolution) {
            return Some(format!(
                "unresolved child lifecycle obligation: {resolution}"
            ));
        }
    }

    let Some(debounce_windows) = state["debounce_windows"].as_array() else {
        return Some("missing debounce evidence".to_owned());
    };
    if debounce_windows.is_empty() {
        return Some("empty debounce evidence".to_owned());
    }
    if debounce_windows.iter().any(|window| {
        window["timer_evidence"].as_str() == Some("missing")
            || window["replay_grade"].as_str() == Some("debt")
    }) {
        return Some("incomplete debounce shutdown evidence".to_owned());
    }
    None
}

fn is_release_lifecycle_resolution(resolution: &str) -> bool {
    matches!(
        resolution,
        "no_child_started"
            | "in_process_rerun_finished"
            | "exited"
            | "signaled"
            | "killed"
            | "detached"
    )
}

fn emit_release_lifecycle_gate(
    reason: &str,
    error_format: crate::ErrorFormat,
) -> anyhow::Result<()> {
    let message = format!("release watch lifecycle gate blocked {reason}");
    match error_format {
        crate::ErrorFormat::Human => eprintln!("error: {message}"),
        crate::ErrorFormat::Json => println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "kind": "ci_release_gate",
                "gate": "watch_lifecycle",
                "message": message,
            }))?
        ),
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
    unresolved_error: Option<WatchErrorFingerprint>,
    pending_startup_error: Option<WatchErrorFingerprint>,
}

#[derive(Clone, Copy)]
struct FileSnapshot {
    modified: SystemTime,
    len: u64,
    is_readonly: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct WatchErrorFingerprint {
    kind: io::ErrorKind,
    raw_os_error: Option<i32>,
    message: String,
}

trait WatchSnapshotSource {
    fn snapshot(&self, file: &Path) -> io::Result<FileSnapshot>;
}

struct FsWatchSnapshotSource;

impl WatchErrorFingerprint {
    fn from_error(error: &io::Error) -> Self {
        Self {
            kind: error.kind(),
            raw_os_error: error.raw_os_error(),
            message: error.to_string(),
        }
    }

    fn as_label(&self) -> String {
        let kind = match self.kind {
            io::ErrorKind::NotFound => "not_found",
            io::ErrorKind::PermissionDenied => "permission_denied",
            io::ErrorKind::AlreadyExists => "already_exists",
            io::ErrorKind::InvalidInput => "invalid_input",
            io::ErrorKind::InvalidData => "invalid_data",
            io::ErrorKind::TimedOut => "timed_out",
            io::ErrorKind::Interrupted => "interrupted",
            io::ErrorKind::WouldBlock => "would_block",
            io::ErrorKind::UnexpectedEof => "unexpected_eof",
            io::ErrorKind::OutOfMemory => "out_of_memory",
            _ => "other",
        };
        let identity = self
            .raw_os_error
            .map(|code| format!("os:{code}"))
            .unwrap_or_else(|| format!("message:{}", self.message));
        format!("{kind}:{identity}")
    }
}

struct WatchChange {
    display: String,
    event_kind: WatchEventKind,
    previous_modified_ms: Option<u128>,
    current_modified_ms: Option<u128>,
    duplicate_status: DuplicateStatus,
    evidence_grade: EventEvidenceGrade,
    error_fingerprint: Option<String>,
    additional_paths: Vec<WatchEventPathInput>,
    raw_events: Vec<RawWatchEventInput>,
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
    let value = source_watch_state_json(root_file, scope, persisted_state_loaded, evidence_history);
    std::fs::write(&state_path, serde_json::to_vec_pretty(&value)?)
        .map_err(|error| anyhow::anyhow!("failed to write {}: {error}", state_path.display()))
}

fn source_watch_state_json(
    root_file: &Path,
    scope: &WatchScope,
    persisted_state_loaded: bool,
    evidence_history: &[WatchEvidence],
) -> serde_json::Value {
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
    serde_json::json!({
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
    })
}

fn watched_files(
    scope: &WatchScope,
    snapshot_source: &impl WatchSnapshotSource,
) -> anyhow::Result<Vec<WatchedFile>> {
    scope
        .files
        .iter()
        .map(|path| {
            let (snapshot, pending_startup_error) = match snapshot_source.snapshot(path) {
                Ok(snapshot) => (Some(snapshot), None),
                Err(error) => (None, Some(WatchErrorFingerprint::from_error(&error))),
            };
            Ok(WatchedFile {
                path: path.clone(),
                snapshot,
                unresolved_error: None,
                pending_startup_error,
            })
        })
        .collect()
}

fn collect_debounce_window(
    scope: &mut WatchScope,
    watched: &mut Vec<WatchedFile>,
    snapshot_source: &impl WatchSnapshotSource,
) -> anyhow::Result<Option<DebounceWindow>> {
    let mut changes = collect_watch_changes(scope, watched, snapshot_source)?;
    if changes.is_empty() {
        return Ok(None);
    }
    let mut has_timer_extension = false;
    let mut extension_cause_paths = Vec::new();

    loop {
        std::thread::sleep(Duration::from_millis(DEBOUNCE_INTERVAL_MS));
        let later_changes = collect_watch_changes(scope, watched, snapshot_source)?;
        if later_changes.is_empty() {
            return Ok(Some(finalize_debounce_window(
                changes,
                has_timer_extension,
                extension_cause_paths,
            )));
        }
        if repeated_unknown_changes(&changes, &later_changes) {
            has_timer_extension = true;
            extension_cause_paths.extend(later_changes.iter().map(|change| change.display.clone()));
            merge_window_changes(&mut changes, later_changes);
            return Ok(Some(finalize_debounce_window(
                changes,
                has_timer_extension,
                extension_cause_paths,
            )));
        }
        has_timer_extension = true;
        extension_cause_paths.extend(later_changes.iter().map(|change| change.display.clone()));
        merge_window_changes(&mut changes, later_changes);
    }
}

fn finalize_debounce_window(
    mut changes: Vec<WatchChange>,
    has_timer_extension: bool,
    mut extension_cause_paths: Vec<String>,
) -> DebounceWindow {
    changes = normalize_rename_events(changes);
    changes.sort_by(|left, right| left.display.cmp(&right.display));
    extension_cause_paths.sort();
    extension_cause_paths.dedup();
    DebounceWindow {
        changes,
        has_timer_extension,
        extension_cause_paths,
    }
}

fn repeated_unknown_changes(existing: &[WatchChange], later: &[WatchChange]) -> bool {
    !later.is_empty()
        && later.iter().all(|later_change| {
            matches!(later_change.event_kind, WatchEventKind::Unknown)
                && existing
                    .iter()
                    .any(|existing_change| existing_change.can_coalesce_with(later_change))
        })
}

fn normalize_rename_events(changes: Vec<WatchChange>) -> Vec<WatchChange> {
    let mut remaining = changes;
    let remove_count = remaining
        .iter()
        .filter(|change| matches!(change.event_kind, WatchEventKind::Remove))
        .count();
    let create_count = remaining
        .iter()
        .filter(|change| matches!(change.event_kind, WatchEventKind::Create))
        .count();
    if remove_count != 1 || create_count != 1 {
        return remaining;
    }

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
    if remaining[remove_index].display == remaining[create_index].display {
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

    remaining.push(WatchChange::rename_candidate(remove, create));
    remaining
}

fn same_parent_display(left: &str, right: &str) -> bool {
    Path::new(left).parent() == Path::new(right).parent()
}

fn collect_watch_changes(
    scope: &mut WatchScope,
    watched: &mut Vec<WatchedFile>,
    snapshot_source: &impl WatchSnapshotSource,
) -> anyhow::Result<Vec<WatchChange>> {
    let mut changes = changed_existing_files(watched, snapshot_source);
    refresh_watch_scope(scope)?;
    changes.extend(created_watch_files(scope, watched, snapshot_source));
    Ok(changes)
}

fn changed_existing_files(
    watched: &mut [WatchedFile],
    snapshot_source: &impl WatchSnapshotSource,
) -> Vec<WatchChange> {
    let mut changes = Vec::new();
    for file in watched {
        let pending_startup_error = file.pending_startup_error.take();
        match snapshot_source.snapshot(&file.path) {
            Ok(current) => {
                if let Some(fingerprint) = pending_startup_error {
                    changes.push(WatchChange::unknown(
                        &file.path,
                        file.snapshot,
                        Some(&fingerprint),
                    ));
                }
                file.unresolved_error = None;
                match file.snapshot {
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
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let fingerprint = WatchErrorFingerprint::from_error(&error);
                if let Some(pending) =
                    pending_startup_error.filter(|pending| pending.kind != io::ErrorKind::NotFound)
                {
                    changes.push(WatchChange::unknown(
                        &file.path,
                        file.snapshot,
                        Some(&pending),
                    ));
                }
                if file.unresolved_error.as_ref() == Some(&fingerprint) {
                    continue;
                }
                let previous = file.snapshot.take();
                file.unresolved_error = Some(fingerprint.clone());
                changes.push(WatchChange::removed(
                    &file.path,
                    previous,
                    Some(&fingerprint),
                ));
            }
            Err(error) => {
                let fingerprint = WatchErrorFingerprint::from_error(&error);
                if let Some(pending) = pending_startup_error {
                    changes.push(WatchChange::unknown(
                        &file.path,
                        file.snapshot,
                        Some(&pending),
                    ));
                    if pending == fingerprint {
                        file.unresolved_error = Some(fingerprint);
                        continue;
                    }
                }
                if file.unresolved_error.as_ref() == Some(&fingerprint) {
                    continue;
                }
                file.unresolved_error = Some(fingerprint.clone());
                changes.push(WatchChange::unknown(
                    &file.path,
                    file.snapshot,
                    Some(&fingerprint),
                ));
            }
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

fn created_watch_files(
    scope: &WatchScope,
    watched: &mut Vec<WatchedFile>,
    snapshot_source: &impl WatchSnapshotSource,
) -> Vec<WatchChange> {
    let mut changes = Vec::new();
    for file in &scope.files {
        if watched
            .iter()
            .any(|watched_file| same_path(&watched_file.path, file))
        {
            continue;
        }
        match snapshot_source.snapshot(file) {
            Ok(current) => {
                watched.push(WatchedFile {
                    path: file.clone(),
                    snapshot: Some(current),
                    unresolved_error: None,
                    pending_startup_error: None,
                });
                changes.push(WatchChange::created(file, current.modified));
            }
            Err(error) => {
                let fingerprint = WatchErrorFingerprint::from_error(&error);
                watched.push(WatchedFile {
                    path: file.clone(),
                    snapshot: None,
                    unresolved_error: Some(fingerprint.clone()),
                    pending_startup_error: None,
                });
                if error.kind() == io::ErrorKind::NotFound {
                    changes.push(WatchChange::removed(file, None, Some(&fingerprint)));
                } else {
                    changes.push(WatchChange::unknown(file, None, Some(&fingerprint)));
                }
            }
        }
    }
    changes
}

fn merge_window_changes(changes: &mut Vec<WatchChange>, later_changes: Vec<WatchChange>) {
    for later_change in later_changes {
        if let Some(existing) = changes
            .iter_mut()
            .find(|existing| existing.can_coalesce_with(&later_change))
        {
            existing.event_kind = later_change.event_kind;
            existing.current_modified_ms = later_change.current_modified_ms;
            existing.duplicate_status = DuplicateStatus::Coalesced;
            existing.evidence_grade = later_change.evidence_grade;
            existing.error_fingerprint = later_change.error_fingerprint;
            existing.additional_paths = later_change.additional_paths;
            existing.raw_events = later_change.raw_events;
        } else {
            changes.push(later_change);
        }
    }
}

impl WatchChange {
    fn can_coalesce_with(&self, other: &Self) -> bool {
        self.display == other.display
            && self.event_kind.as_str() == other.event_kind.as_str()
            && self.error_fingerprint == other.error_fingerprint
    }

    fn modified(path: &Path, previous: SystemTime, current: SystemTime) -> Self {
        Self {
            display: relative_display(path),
            event_kind: WatchEventKind::Modify,
            previous_modified_ms: Some(system_time_millis(previous)),
            current_modified_ms: Some(system_time_millis(current)),
            duplicate_status: DuplicateStatus::Unique,
            evidence_grade: EventEvidenceGrade::MetadataOnly,
            error_fingerprint: None,
            additional_paths: Vec::new(),
            raw_events: Vec::new(),
        }
    }

    fn created(path: &Path, current: SystemTime) -> Self {
        Self {
            display: relative_display(path),
            event_kind: WatchEventKind::Create,
            previous_modified_ms: None,
            current_modified_ms: Some(system_time_millis(current)),
            duplicate_status: DuplicateStatus::Unique,
            evidence_grade: EventEvidenceGrade::MetadataOnly,
            error_fingerprint: None,
            additional_paths: Vec::new(),
            raw_events: Vec::new(),
        }
    }

    fn metadata(path: &Path, previous: SystemTime, current: SystemTime) -> Self {
        Self {
            display: relative_display(path),
            event_kind: WatchEventKind::Metadata,
            previous_modified_ms: Some(system_time_millis(previous)),
            current_modified_ms: Some(system_time_millis(current)),
            duplicate_status: DuplicateStatus::Unique,
            evidence_grade: EventEvidenceGrade::MetadataOnly,
            error_fingerprint: None,
            additional_paths: Vec::new(),
            raw_events: Vec::new(),
        }
    }

    fn removed(
        path: &Path,
        previous: Option<FileSnapshot>,
        fingerprint: Option<&WatchErrorFingerprint>,
    ) -> Self {
        Self {
            display: relative_display(path),
            event_kind: WatchEventKind::Remove,
            previous_modified_ms: previous.map(|snapshot| system_time_millis(snapshot.modified)),
            current_modified_ms: None,
            duplicate_status: DuplicateStatus::Unique,
            evidence_grade: EventEvidenceGrade::MetadataOnly,
            error_fingerprint: fingerprint.map(WatchErrorFingerprint::as_label),
            additional_paths: Vec::new(),
            raw_events: Vec::new(),
        }
    }

    fn unknown(
        path: &Path,
        previous: Option<FileSnapshot>,
        fingerprint: Option<&WatchErrorFingerprint>,
    ) -> Self {
        Self {
            display: relative_display(path),
            event_kind: WatchEventKind::Unknown,
            previous_modified_ms: previous.map(|snapshot| system_time_millis(snapshot.modified)),
            current_modified_ms: None,
            duplicate_status: DuplicateStatus::Unknown,
            evidence_grade: EventEvidenceGrade::Unknown,
            error_fingerprint: fingerprint.map(WatchErrorFingerprint::as_label),
            additional_paths: Vec::new(),
            raw_events: Vec::new(),
        }
    }

    fn rename_candidate(remove: WatchChange, create: WatchChange) -> Self {
        let parent_path = Path::new(&create.display)
            .parent()
            .map(|path| {
                path.components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .unwrap_or_default();
        let source_path = remove.display;
        let destination_path = create.display;
        Self {
            display: source_path.clone(),
            event_kind: WatchEventKind::RenameCandidate,
            previous_modified_ms: remove.previous_modified_ms,
            current_modified_ms: create.current_modified_ms,
            duplicate_status: DuplicateStatus::Coalesced,
            evidence_grade: EventEvidenceGrade::ModelledFromMetadata,
            error_fingerprint: None,
            additional_paths: vec![
                WatchEventPathInput {
                    role: WatchPathRole::DestinationPath,
                    path: destination_path.clone(),
                },
                WatchEventPathInput {
                    role: WatchPathRole::ParentPath,
                    path: parent_path,
                },
            ],
            raw_events: vec![
                RawWatchEventInput {
                    event_kind: WatchEventKind::Remove,
                    paths: vec![WatchEventPathInput {
                        role: WatchPathRole::SourcePath,
                        path: source_path,
                    }],
                    evidence_grade: remove.evidence_grade,
                    error_fingerprint: remove.error_fingerprint,
                },
                RawWatchEventInput {
                    event_kind: WatchEventKind::Create,
                    paths: vec![WatchEventPathInput {
                        role: WatchPathRole::DestinationPath,
                        path: destination_path,
                    }],
                    evidence_grade: create.evidence_grade,
                    error_fingerprint: create.error_fingerprint,
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
            evidence_grade: self.evidence_grade,
            error_fingerprint: self.error_fingerprint,
            raw_events: self.raw_events,
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

impl WatchSnapshotSource for FsWatchSnapshotSource {
    fn snapshot(&self, file: &Path) -> io::Result<FileSnapshot> {
        get_file_snapshot(file)
    }
}

fn get_file_snapshot(file: &Path) -> io::Result<FileSnapshot> {
    let metadata = std::fs::metadata(file)?;
    let modified = metadata.modified()?;
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

    struct ErrorSnapshotSource {
        kind: io::ErrorKind,
        message: &'static str,
    }

    impl WatchSnapshotSource for ErrorSnapshotSource {
        fn snapshot(&self, _file: &Path) -> io::Result<FileSnapshot> {
            Err(io::Error::new(self.kind, self.message))
        }
    }

    struct OkSnapshotSource {
        snapshot: FileSnapshot,
    }

    impl WatchSnapshotSource for OkSnapshotSource {
        fn snapshot(&self, _file: &Path) -> io::Result<FileSnapshot> {
            Ok(self.snapshot)
        }
    }

    struct SequenceSnapshotSource {
        steps: std::sync::Mutex<Vec<io::Result<FileSnapshot>>>,
    }

    impl SequenceSnapshotSource {
        fn new(mut steps: Vec<io::Result<FileSnapshot>>) -> Self {
            steps.reverse();
            Self {
                steps: std::sync::Mutex::new(steps),
            }
        }
    }

    impl WatchSnapshotSource for SequenceSnapshotSource {
        fn snapshot(&self, _file: &Path) -> io::Result<FileSnapshot> {
            self.steps
                .lock()
                .unwrap()
                .pop()
                .unwrap_or_else(|| Ok(test_snapshot(99)))
        }
    }

    fn test_snapshot(seconds: u64) -> FileSnapshot {
        FileSnapshot {
            modified: UNIX_EPOCH + Duration::from_secs(seconds),
            len: 13,
            is_readonly: false,
        }
    }

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
        let change = WatchChange::unknown(Path::new("src/raced.kobo"), None, None);
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
        assert_eq!(batch["events"][0]["evidence_grade"], "unknown");
    }

    #[test]
    fn scanner_snapshot_error_persists_unknown_event() {
        let root = temp_watch_root("unknown-snapshot");
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let root_file = src.join("main.kobo");
        std::fs::write(&root_file, "fn main() {}\n").unwrap();
        let mut scope = WatchScope {
            root: root.clone(),
            root_file: root_file.clone(),
            files: vec![root_file.clone()],
        };
        let mut watched = vec![WatchedFile {
            path: root_file.clone(),
            snapshot: Some(FileSnapshot {
                modified: UNIX_EPOCH,
                len: 13,
                is_readonly: false,
            }),
            unresolved_error: None,
            pending_startup_error: None,
        }];

        let changes = collect_watch_changes(
            &mut scope,
            &mut watched,
            &ErrorSnapshotSource {
                kind: io::ErrorKind::PermissionDenied,
                message: "permission denied first",
            },
        )
        .unwrap();
        assert_eq!(changes[0].event_kind.as_str(), "unknown");
        assert!(
            watched[0].snapshot.is_some(),
            "non-NotFound snapshot failures should keep the prior snapshot instead of inventing a remove",
        );

        let evidence = WatchEvidence::for_window(
            1,
            relative_display(&root_file),
            changes
                .into_iter()
                .map(WatchChange::into_event_input)
                .collect(),
            DebounceWindowInput {
                has_timer_extension: false,
                extension_cause_paths: Vec::new(),
            },
            WatchExecutionMode::Simple,
        );
        let state = source_watch_state_json(&root_file, &scope, false, &[evidence]);

        assert_eq!(state["event_batches"][0]["events"][0]["kind"], "unknown");
        assert_eq!(
            state["event_batches"][0]["events"][0]["evidence_grade"],
            "unknown"
        );
        assert_eq!(state["changes"][0]["event_kind"], "unknown");
        assert_eq!(state["changes"][0]["replay_grade"], "partial");
        assert_eq!(
            state["event_batches"][0]["events"][0]["error_fingerprint"],
            "permission_denied:message:permission denied first"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn persistent_unknown_snapshot_error_closes_debounce_window() {
        let root = temp_watch_root("persistent-unknown-snapshot");
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let root_file = src.join("main.kobo");
        std::fs::write(&root_file, "fn main() {}\n").unwrap();
        let mut scope = WatchScope {
            root: root.clone(),
            root_file: root_file.clone(),
            files: vec![root_file.clone()],
        };
        let mut watched = vec![WatchedFile {
            path: root_file.clone(),
            snapshot: Some(FileSnapshot {
                modified: UNIX_EPOCH,
                len: 13,
                is_readonly: false,
            }),
            unresolved_error: None,
            pending_startup_error: None,
        }];

        let window = collect_debounce_window(
            &mut scope,
            &mut watched,
            &ErrorSnapshotSource {
                kind: io::ErrorKind::PermissionDenied,
                message: "permission denied first",
            },
        )
        .unwrap()
        .expect("persistent unknown evidence should still close a debounce window");
        assert_eq!(window.changes.len(), 1);
        assert_eq!(window.changes[0].event_kind.as_str(), "unknown");
        assert!(matches!(
            window.changes[0].duplicate_status,
            DuplicateStatus::Unknown
        ));
        assert!(
            !window.has_timer_extension,
            "remembered identical unknown evidence should make the next scan quiet"
        );
        assert!(
            watched[0].snapshot.is_some(),
            "persistent unknown should preserve the last usable snapshot for recovery",
        );

        let evidence = WatchEvidence::for_window(
            1,
            relative_display(&root_file),
            window
                .changes
                .into_iter()
                .map(WatchChange::into_event_input)
                .collect(),
            DebounceWindowInput {
                has_timer_extension: window.has_timer_extension,
                extension_cause_paths: window.extension_cause_paths,
            },
            WatchExecutionMode::Simple,
        );
        let state = source_watch_state_json(&root_file, &scope, false, &[evidence]);

        assert_eq!(state["event_batches"][0]["events"][0]["kind"], "unknown");
        assert_eq!(
            state["event_batches"][0]["events"][0]["duplicate_or_coalesced"],
            "unknown"
        );
        assert_eq!(state["debounce_windows"][0]["timer_cancelled"], false);
        assert_eq!(state["changes"][0]["event_kind"], "unknown");
        assert_eq!(
            state["event_batches"][0]["events"][0]["error_fingerprint"],
            "permission_denied:message:permission denied first"
        );

        let second_window = collect_debounce_window(
            &mut scope,
            &mut watched,
            &ErrorSnapshotSource {
                kind: io::ErrorKind::PermissionDenied,
                message: "permission denied first",
            },
        )
        .unwrap();
        assert!(
            second_window.is_none(),
            "same unresolved snapshot error should not create another restart window",
        );

        let changed_error_window = collect_debounce_window(
            &mut scope,
            &mut watched,
            &ErrorSnapshotSource {
                kind: io::ErrorKind::PermissionDenied,
                message: "permission denied second",
            },
        )
        .unwrap()
        .expect("changed snapshot error identity should reopen watcher evidence");
        assert_eq!(
            changed_error_window.changes[0].event_kind.as_str(),
            "unknown"
        );
        assert_eq!(
            changed_error_window.changes[0].error_fingerprint.as_deref(),
            Some("permission_denied:message:permission denied second")
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn startup_snapshot_error_enters_unknown_state_and_suppresses_repeat() {
        let root = temp_watch_root("startup-unknown-snapshot");
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let root_file = src.join("main.kobo");
        std::fs::write(&root_file, "fn main() {}\n").unwrap();
        let mut scope = WatchScope {
            root: root.clone(),
            root_file: root_file.clone(),
            files: vec![root_file.clone()],
        };
        let mut watched = watched_files(
            &scope,
            &ErrorSnapshotSource {
                kind: io::ErrorKind::PermissionDenied,
                message: "startup denied",
            },
        )
        .unwrap();
        assert_eq!(watched.len(), 1);
        assert!(
            watched[0].snapshot.is_none(),
            "startup snapshot failures should not abort watch setup",
        );
        assert!(
            watched[0].pending_startup_error.is_some(),
            "startup errors should wait as pending typed evidence for the first scan",
        );

        let first_window = collect_debounce_window(
            &mut scope,
            &mut watched,
            &ErrorSnapshotSource {
                kind: io::ErrorKind::PermissionDenied,
                message: "startup denied",
            },
        )
        .unwrap()
        .expect("startup snapshot failure should become unknown evidence");
        assert_eq!(first_window.changes[0].event_kind.as_str(), "unknown");
        assert_eq!(
            first_window.changes[0].error_fingerprint.as_deref(),
            Some("permission_denied:message:startup denied")
        );

        let repeated_window = collect_debounce_window(
            &mut scope,
            &mut watched,
            &ErrorSnapshotSource {
                kind: io::ErrorKind::PermissionDenied,
                message: "startup denied",
            },
        )
        .unwrap();
        assert!(
            repeated_window.is_none(),
            "same startup snapshot failure should not trigger another restart window",
        );

        let changed_window = collect_debounce_window(
            &mut scope,
            &mut watched,
            &ErrorSnapshotSource {
                kind: io::ErrorKind::PermissionDenied,
                message: "startup denied but different",
            },
        )
        .unwrap()
        .expect("same error kind with changed identity should reopen evidence");
        assert_eq!(
            changed_window.changes[0].error_fingerprint.as_deref(),
            Some("permission_denied:message:startup denied but different")
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn startup_snapshot_error_then_recovery_preserves_unknown_evidence() {
        let root = temp_watch_root("startup-unknown-recovery");
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let root_file = src.join("main.kobo");
        std::fs::write(&root_file, "fn main() {}\n").unwrap();
        let mut scope = WatchScope {
            root: root.clone(),
            root_file: root_file.clone(),
            files: vec![root_file.clone()],
        };
        let mut watched = watched_files(
            &scope,
            &ErrorSnapshotSource {
                kind: io::ErrorKind::PermissionDenied,
                message: "startup denied",
            },
        )
        .unwrap();

        let changes = collect_watch_changes(
            &mut scope,
            &mut watched,
            &OkSnapshotSource {
                snapshot: FileSnapshot {
                    modified: UNIX_EPOCH + Duration::from_secs(1),
                    len: 13,
                    is_readonly: false,
                },
            },
        )
        .unwrap();
        let kinds = changes
            .iter()
            .map(|change| change.event_kind.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec!["unknown", "create"],
            "startup unknown evidence should be kept before the recovery create",
        );
        assert_eq!(
            changes[0].error_fingerprint.as_deref(),
            Some("permission_denied:message:startup denied"),
        );
        assert!(watched[0].pending_startup_error.is_none());
        assert!(watched[0].unresolved_error.is_none());

        let evidence = WatchEvidence::for_window(
            1,
            relative_display(&root_file),
            changes
                .into_iter()
                .map(WatchChange::into_event_input)
                .collect(),
            DebounceWindowInput {
                has_timer_extension: false,
                extension_cause_paths: Vec::new(),
            },
            WatchExecutionMode::Simple,
        );
        let state = source_watch_state_json(&root_file, &scope, false, &[evidence]);
        assert_eq!(state["event_batches"][0]["events"][0]["kind"], "unknown");
        assert_eq!(state["event_batches"][0]["events"][1]["kind"], "create");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn startup_unknown_then_create_within_debounce_window_stays_visible() {
        let root = temp_watch_root("startup-unknown-window-recovery");
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let root_file = src.join("main.kobo");
        std::fs::write(&root_file, "fn main() {}\n").unwrap();
        let mut scope = WatchScope {
            root: root.clone(),
            root_file: root_file.clone(),
            files: vec![root_file.clone()],
        };
        let mut watched = watched_files(
            &scope,
            &ErrorSnapshotSource {
                kind: io::ErrorKind::PermissionDenied,
                message: "startup denied",
            },
        )
        .unwrap();

        let window = collect_debounce_window(
            &mut scope,
            &mut watched,
            &SequenceSnapshotSource::new(vec![Ok(test_snapshot(1)), Ok(test_snapshot(1))]),
        )
        .unwrap()
        .expect("startup unknown followed by create should produce one window");
        let kinds = window
            .changes
            .iter()
            .map(|change| change.event_kind.as_str())
            .collect::<Vec<_>>();
        assert_eq!(kinds, vec!["unknown", "create"]);
        assert_eq!(
            window
                .changes
                .iter()
                .find(|change| change.event_kind.as_str() == "unknown")
                .and_then(|change| change.error_fingerprint.as_deref()),
            Some("permission_denied:message:startup denied"),
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn changed_unknown_then_create_within_debounce_window_stays_visible() {
        let root = temp_watch_root("changed-unknown-window-recovery");
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let root_file = src.join("main.kobo");
        std::fs::write(&root_file, "fn main() {}\n").unwrap();
        let mut scope = WatchScope {
            root: root.clone(),
            root_file: root_file.clone(),
            files: vec![root_file.clone()],
        };
        let mut watched = watched_files(
            &scope,
            &ErrorSnapshotSource {
                kind: io::ErrorKind::PermissionDenied,
                message: "first denied",
            },
        )
        .unwrap();

        let window = collect_debounce_window(
            &mut scope,
            &mut watched,
            &SequenceSnapshotSource::new(vec![
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "second denied",
                )),
                Ok(test_snapshot(1)),
                Ok(test_snapshot(1)),
            ]),
        )
        .unwrap()
        .expect("changed unknown followed by create should produce one window");
        let event_inputs = window
            .changes
            .into_iter()
            .map(WatchChange::into_event_input)
            .collect::<Vec<_>>();
        let evidence = WatchEvidence::for_window(
            1,
            relative_display(&root_file),
            event_inputs,
            DebounceWindowInput {
                has_timer_extension: window.has_timer_extension,
                extension_cause_paths: window.extension_cause_paths,
            },
            WatchExecutionMode::Simple,
        );
        let batch = evidence.event_batch_json();
        let events = batch["events"]
            .as_array()
            .expect("events should be an array");
        let kinds = events
            .iter()
            .map(|event| event["kind"].as_str().unwrap_or_default())
            .collect::<Vec<_>>();

        assert_eq!(kinds, vec!["unknown", "unknown", "create"]);
        assert_eq!(
            events[0]["error_fingerprint"],
            "permission_denied:message:first denied"
        );
        assert_eq!(
            events[1]["error_fingerprint"],
            "permission_denied:message:second denied"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn startup_not_found_then_create_within_debounce_window_stays_visible() {
        let root = temp_watch_root("startup-not-found-window-recovery");
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let root_file = src.join("main.kobo");
        std::fs::write(&root_file, "fn main() {}\n").unwrap();
        let mut scope = WatchScope {
            root: root.clone(),
            root_file: root_file.clone(),
            files: vec![root_file.clone()],
        };
        let mut watched = watched_files(
            &scope,
            &ErrorSnapshotSource {
                kind: io::ErrorKind::NotFound,
                message: "startup missing",
            },
        )
        .unwrap();

        let window = collect_debounce_window(
            &mut scope,
            &mut watched,
            &SequenceSnapshotSource::new(vec![
                Err(io::Error::new(io::ErrorKind::NotFound, "startup missing")),
                Ok(test_snapshot(1)),
                Ok(test_snapshot(1)),
            ]),
        )
        .unwrap()
        .expect("startup NotFound followed by create should produce one window");
        let kinds = window
            .changes
            .iter()
            .map(|change| change.event_kind.as_str())
            .collect::<Vec<_>>();
        assert_eq!(kinds, vec!["remove", "create"]);
        assert_eq!(
            window
                .changes
                .iter()
                .find(|change| change.event_kind.as_str() == "remove")
                .and_then(|change| change.error_fingerprint.as_deref()),
            Some("not_found:message:startup missing"),
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn same_path_remove_then_create_within_debounce_window_stays_visible() {
        let root = temp_watch_root("same-path-remove-create-window");
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let root_file = src.join("main.kobo");
        std::fs::write(&root_file, "fn main() {}\n").unwrap();
        let mut scope = WatchScope {
            root: root.clone(),
            root_file: root_file.clone(),
            files: vec![root_file.clone()],
        };
        let mut watched = vec![WatchedFile {
            path: root_file.clone(),
            snapshot: Some(test_snapshot(0)),
            unresolved_error: None,
            pending_startup_error: None,
        }];

        let window = collect_debounce_window(
            &mut scope,
            &mut watched,
            &SequenceSnapshotSource::new(vec![
                Err(io::Error::new(io::ErrorKind::NotFound, "gone")),
                Ok(test_snapshot(1)),
                Ok(test_snapshot(1)),
            ]),
        )
        .unwrap()
        .expect("same-path remove/create should produce one window");
        let kinds = window
            .changes
            .iter()
            .map(|change| change.event_kind.as_str())
            .collect::<Vec<_>>();
        assert_eq!(kinds, vec!["remove", "create"]);
        assert_eq!(
            window
                .changes
                .iter()
                .find(|change| change.event_kind.as_str() == "remove")
                .and_then(|change| change.error_fingerprint.as_deref()),
            Some("not_found:message:gone"),
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_candidate_raw_remove_keeps_not_found_fingerprint() {
        let fingerprint = WatchErrorFingerprint {
            kind: io::ErrorKind::NotFound,
            raw_os_error: None,
            message: "startup missing".to_owned(),
        };
        let window = finalize_debounce_window(
            vec![
                WatchChange::removed(Path::new("src/service.kobo"), None, Some(&fingerprint)),
                WatchChange::created(
                    Path::new("src/moved_service.kobo"),
                    UNIX_EPOCH + Duration::from_secs(1),
                ),
            ],
            false,
            Vec::new(),
        );
        assert_eq!(window.changes.len(), 1);
        assert_eq!(window.changes[0].event_kind.as_str(), "rename_candidate");

        let evidence = WatchEvidence::for_window(
            1,
            "src/main.kobo".to_owned(),
            window
                .changes
                .into_iter()
                .map(WatchChange::into_event_input)
                .collect(),
            DebounceWindowInput {
                has_timer_extension: false,
                extension_cause_paths: Vec::new(),
            },
            WatchExecutionMode::Simple,
        );
        let batch = evidence.event_batch_json();
        let raw_events = batch["events"][0]["raw_events"]
            .as_array()
            .expect("rename candidate raw events should be an array");

        assert_eq!(raw_events[0]["kind"], "remove");
        assert_eq!(
            raw_events[0]["error_fingerprint"],
            "not_found:message:startup missing",
        );
        assert_eq!(raw_events[0]["evidence_grade"], "metadata_only");
        assert_eq!(raw_events[1]["kind"], "create");
    }

    #[test]
    fn startup_not_found_becomes_fingerprinted_remove_evidence() {
        let root = temp_watch_root("startup-not-found-remove");
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let root_file = src.join("main.kobo");
        std::fs::write(&root_file, "fn main() {}\n").unwrap();
        let mut scope = WatchScope {
            root: root.clone(),
            root_file: root_file.clone(),
            files: vec![root_file.clone()],
        };
        let mut watched = watched_files(
            &scope,
            &ErrorSnapshotSource {
                kind: io::ErrorKind::NotFound,
                message: "startup missing",
            },
        )
        .unwrap();

        let first_window = collect_debounce_window(
            &mut scope,
            &mut watched,
            &ErrorSnapshotSource {
                kind: io::ErrorKind::NotFound,
                message: "startup missing",
            },
        )
        .unwrap()
        .expect("startup NotFound should become remove evidence");
        assert_eq!(first_window.changes.len(), 1);
        assert_eq!(first_window.changes[0].event_kind.as_str(), "remove");
        assert_eq!(
            first_window.changes[0].error_fingerprint.as_deref(),
            Some("not_found:message:startup missing"),
        );

        let changed_window = collect_debounce_window(
            &mut scope,
            &mut watched,
            &ErrorSnapshotSource {
                kind: io::ErrorKind::NotFound,
                message: "startup missing but different",
            },
        )
        .unwrap()
        .expect("changed startup NotFound identity should reopen remove evidence");
        assert_eq!(changed_window.changes[0].event_kind.as_str(), "remove");
        assert_eq!(
            changed_window.changes[0].error_fingerprint.as_deref(),
            Some("not_found:message:startup missing but different"),
        );

        let repeated_window = collect_debounce_window(
            &mut scope,
            &mut watched,
            &ErrorSnapshotSource {
                kind: io::ErrorKind::NotFound,
                message: "startup missing but different",
            },
        )
        .unwrap();
        assert!(
            repeated_window.is_none(),
            "same startup NotFound identity should not repeat remove evidence",
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn snapshot_none_not_found_and_recovery_are_tracked() {
        let root = temp_watch_root("snapshot-none-transitions");
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let root_file = src.join("main.kobo");
        std::fs::write(&root_file, "fn main() {}\n").unwrap();
        let mut scope = WatchScope {
            root: root.clone(),
            root_file: root_file.clone(),
            files: vec![root_file.clone()],
        };
        let mut watched = vec![WatchedFile {
            path: root_file.clone(),
            snapshot: None,
            unresolved_error: None,
            pending_startup_error: None,
        }];

        let not_found = collect_watch_changes(
            &mut scope,
            &mut watched,
            &ErrorSnapshotSource {
                kind: io::ErrorKind::NotFound,
                message: "missing",
            },
        )
        .unwrap();
        assert_eq!(not_found[0].event_kind.as_str(), "remove");
        assert_eq!(
            not_found[0].error_fingerprint.as_deref(),
            Some("not_found:message:missing"),
        );
        assert!(watched[0].snapshot.is_none());
        assert_eq!(
            watched[0]
                .unresolved_error
                .as_ref()
                .map(WatchErrorFingerprint::as_label)
                .as_deref(),
            Some("not_found:message:missing"),
        );

        let repeated_not_found = collect_watch_changes(
            &mut scope,
            &mut watched,
            &ErrorSnapshotSource {
                kind: io::ErrorKind::NotFound,
                message: "missing",
            },
        )
        .unwrap();
        assert!(
            repeated_not_found.is_empty(),
            "same snapshot-none NotFound should not repeat remove evidence",
        );

        let changed_not_found = collect_watch_changes(
            &mut scope,
            &mut watched,
            &ErrorSnapshotSource {
                kind: io::ErrorKind::NotFound,
                message: "still missing but different",
            },
        )
        .unwrap();
        assert_eq!(changed_not_found[0].event_kind.as_str(), "remove");
        assert_eq!(
            changed_not_found[0].error_fingerprint.as_deref(),
            Some("not_found:message:still missing but different"),
        );

        let recovered = collect_watch_changes(
            &mut scope,
            &mut watched,
            &OkSnapshotSource {
                snapshot: FileSnapshot {
                    modified: UNIX_EPOCH + Duration::from_secs(1),
                    len: 13,
                    is_readonly: false,
                },
            },
        )
        .unwrap();
        assert_eq!(recovered[0].event_kind.as_str(), "create");
        assert!(watched[0].snapshot.is_some());
        assert!(
            watched[0].unresolved_error.is_none(),
            "successful snapshot should clear unresolved error state",
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scanner_not_found_snapshot_persists_remove_event() {
        let root = temp_watch_root("not-found-snapshot");
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let root_file = src.join("main.kobo");
        std::fs::write(&root_file, "fn main() {}\n").unwrap();
        let mut scope = WatchScope {
            root: root.clone(),
            root_file: root_file.clone(),
            files: vec![root_file.clone()],
        };
        let mut watched = vec![WatchedFile {
            path: root_file.clone(),
            snapshot: Some(FileSnapshot {
                modified: UNIX_EPOCH,
                len: 13,
                is_readonly: false,
            }),
            unresolved_error: None,
            pending_startup_error: None,
        }];

        let changes = collect_watch_changes(
            &mut scope,
            &mut watched,
            &ErrorSnapshotSource {
                kind: io::ErrorKind::NotFound,
                message: "gone",
            },
        )
        .unwrap();
        assert_eq!(changes[0].event_kind.as_str(), "remove");
        assert!(
            watched[0].snapshot.is_none(),
            "NotFound is the snapshot failure that clears the watched file state",
        );

        let evidence = WatchEvidence::for_window(
            1,
            relative_display(&root_file),
            changes
                .into_iter()
                .map(WatchChange::into_event_input)
                .collect(),
            DebounceWindowInput {
                has_timer_extension: false,
                extension_cause_paths: Vec::new(),
            },
            WatchExecutionMode::Simple,
        );
        let state = source_watch_state_json(&root_file, &scope, false, &[evidence]);

        assert_eq!(state["event_batches"][0]["events"][0]["kind"], "remove");
        assert_eq!(
            state["event_batches"][0]["events"][0]["evidence_grade"],
            "metadata_only"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    fn temp_watch_root(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "kobo-watch-{label}-{}-{suffix}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        root
    }
}
