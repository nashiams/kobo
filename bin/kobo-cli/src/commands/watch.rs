use std::path::Path;
use std::time::SystemTime;

use kobo_driver::run_check_pipeline;
use super::session::{build_session, render_diagnostics};

/// File-watcher re-run on save.
///
/// In `--simple` mode: polls the file's modification time, and when it changes,
/// triggers a recompile. This avoids pulling in platform-specific inotify/FSEvents
/// dependencies — suitable for development workflows.
pub(super) fn cmd_watch(file: &Path, simple: bool) -> anyhow::Result<()> {
    if !simple {
        println!("Use --simple for basic save-and-rerun mode.");
        return Ok(());
    }

    println!("Watching {} (simple mode, Ctrl+C to stop)", file.display());

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
            // In production this would invoke the compiler pipeline.
            // For now, report the detection.
            on_file_changed(file);
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
