use std::path::Path;
use std::time::SystemTime;

use kobo_driver::{run_check_pipeline, run_codegen_pipeline};
use super::session::{build_session, render_diagnostics};

/// File-watcher re-run on save.
///
/// In `--simple` mode: polls the file's modification time, and when it changes,
/// triggers a recompile. This avoids pulling in platform-specific inotify/FSEvents
/// dependencies — suitable for development workflows.
///
/// In `--build` mode: runs the full codegen pipeline on each change, producing
/// generated Rust output — suitable for continuous build feedback (S-29).
pub(super) fn cmd_watch(file: &Path, simple: bool, build: bool) -> anyhow::Result<()> {
    if !simple && !build {
        println!("Use --simple for basic save-and-recheck mode, or --build for codegen+compile.");
        return Ok(());
    }

    let mode_label = if build { "build" } else { "simple" };
    println!("Watching {} ({mode_label} mode, Ctrl+C to stop)", file.display());

    let mut last_modified = get_mtime(file)?;

    loop {
        std::thread::sleep(std::time::Duration::from_millis(200));

        let current = match get_mtime(file) {
            Ok(t) => t,
            Err(_) => continue, // file temporarily unavailable
        };

        if current != last_modified {
            last_modified = current;
            println!("[kobo-watch] Change detected in {}, recompiling...", file.display());
            if build {
                on_file_changed_build(file);
            } else {
                on_file_changed(file);
            }
        }
    }
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
        Ok(mut session) => {
            match run_check_pipeline(&mut session, file) {
                Ok(()) => {
                    render_diagnostics(&session);
                    if session.diagnostics.is_empty() {
                        println!("[kobo-watch] OK — no diagnostics");
                    } else {
                        println!(
                            "[kobo-watch] {} diagnostic(s)",
                            session.diagnostics.len()
                        );
                    }
                }
                Err(()) => {
                    render_diagnostics(&session);
                    eprintln!("[kobo-watch] check failed");
                }
            }
        }
        Err(e) => {
            eprintln!("[kobo-watch] session error: {e}");
        }
    }
}

/// S-29: Called when a file change is detected in --build mode — runs full codegen pipeline.
fn on_file_changed_build(file: &Path) {
    println!("[kobo-watch] Triggering codegen build for {}", file.display());
    match build_session(file, None) {
        Ok(mut session) => {
            match run_codegen_pipeline(&mut session, file) {
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
            }
        }
        Err(e) => {
            eprintln!("[kobo-watch] session error: {e}");
        }
    }
}

/// Detect whether a file has been modified since a given timestamp.
/// Used for testing the watch detection logic without entering the loop.
pub(crate) fn detect_change(file: &Path, since: SystemTime) -> anyhow::Result<bool> {
    let current = get_mtime(file)?;
    Ok(current != since)
}

/// Debounce interval in milliseconds.
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
        f.write_all(b"fn main() { println!(\"updated\"); }").unwrap();
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
