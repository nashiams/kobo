use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::session::{build_session, render_diagnostics};
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
    println!(
        "Watching {} ({mode_label} mode, Ctrl+C to stop)",
        file.display()
    );

    let mut last_modified = get_mtime(file)?;

    loop {
        std::thread::sleep(std::time::Duration::from_millis(200));

        let current = match get_mtime(file) {
            Ok(t) => t,
            Err(_) => continue, // file temporarily unavailable
        };

        if current != last_modified {
            last_modified = current;
            println!(
                "[kobo-watch] Change detected in {}, recompiling...",
                file.display()
            );
            if build {
                on_file_changed_build(file);
            } else {
                on_file_changed(file);
            }
            if run_once {
                break;
            }
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
    let changed = changed.map(relative_display);
    println!("Watch plan");
    println!("scope: {target}");
    println!("files:");
    for file in &scope.files {
        println!("- {}", relative_display(file));
    }
    println!("rerun targets:");
    println!("rerun target: kobo check {target}");
    println!("rerun target: kobo inspect {target}");
    if let Some(changed) = changed {
        println!("invalidated: {changed}");
        println!("reason: changed file belongs to scoped watch plan");
        println!("rerun target: kobo check {target}");
        println!("rerun target: kobo inspect {target}");
    }
    Ok(())
}

struct WatchScope {
    files: Vec<PathBuf>,
}

fn watch_scope(file: &Path) -> anyhow::Result<WatchScope> {
    let root = file.parent().unwrap_or_else(|| Path::new("."));
    let mut files = Vec::new();
    collect_kobo_watch_files(root, &mut files)?;
    if !files.iter().any(|candidate| same_path(candidate, file)) {
        files.push(file.to_path_buf());
    }
    files.sort();
    Ok(WatchScope { files })
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
    left.canonicalize().ok() == right.canonicalize().ok()
}

fn relative_display(path: &Path) -> String {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    absolute
        .strip_prefix(&cwd)
        .unwrap_or(&absolute)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Get file modification time.
fn get_mtime(file: &Path) -> anyhow::Result<SystemTime> {
    let metadata = std::fs::metadata(file)
        .map_err(|e| anyhow::anyhow!("cannot stat {}: {}", file.display(), e))?;
    metadata
        .modified()
        .map_err(|e| anyhow::anyhow!("cannot get mtime for {}: {}", file.display(), e))
}

/// Called when a file change is detected — runs the check pipeline.
fn on_file_changed(file: &Path) {
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
            }
            Err(()) => {
                render_diagnostics(&session);
                eprintln!("[kobo-watch] check failed");
            }
        },
        Err(e) => {
            eprintln!("[kobo-watch] session error: {e}");
        }
    }
}

/// S-29: Called when a file change is detected in --build mode — runs full codegen pipeline.
fn on_file_changed_build(file: &Path) {
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
            }
            Err(()) => {
                render_diagnostics(&session);
                eprintln!("[kobo-watch] codegen failed");
            }
        },
        Err(e) => {
            eprintln!("[kobo-watch] session error: {e}");
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
pub(crate) const DEBOUNCE_MS: u64 = 200;

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
}
