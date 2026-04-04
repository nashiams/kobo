use std::fs;
use std::path::Path;

use anyhow::Context;
use kobo_errors::{format_diagnostic, render_k0020, render_k0021, DiagOwnerStats};
use kobo_ir::{FileId, KoboSpan};

pub(super) fn cmd_perf(
    file: &Path,
    from: Option<&Path>,
    threshold: Option<u64>,
) -> anyhow::Result<()> {
    let threshold = threshold
        .or_else(|| std::env::var("KOBO_DIAG_THRESHOLD").ok()?.parse().ok())
        .unwrap_or(10_000_u64);

    let diag_text = match from {
        Some(log) => fs::read_to_string(log)
            .with_context(|| format!("failed to read diag log {}", log.display()))?,
        None => {
            eprintln!(
                "advisory: no --from <FILE> supplied.\n\
                 Re-run with `KOBO_DIAG=1 kobo run {} 2>diag.log` then pass `--from diag.log`.",
                file.display()
            );
            return Ok(());
        }
    };

    let entries = parse_diag_log(&diag_text);
    if entries.is_empty() {
        println!("No [kobo-diag] entries found in log (threshold: {threshold}).");
        return Ok(());
    }

    // Emit diagnostics to stdout as plain text; no session needed for perf.
    let file_set = kobo_ir::FileSet::new();
    let mut hot_count = 0usize;

    for stats in &entries {
        let is_hot = stats.borrow_count > threshold || stats.mut_borrow_count > threshold;
        if is_hot {
            hot_count += 1;
            let diag = render_k0020(stats, &file_set);
            println!("{}", format_diagnostic(&file_set, &diag));
        }
        if stats.saturated {
            let diag = render_k0021(stats);
            println!("{}", format_diagnostic(&file_set, &diag));
        }
    }

    if hot_count == 0 {
        println!("No hot paths detected above threshold {threshold}.");
    }

    Ok(())
}

/// Parse all `[kobo-diag]` blocks from a diag log capture.
///
/// Each block starts with a `[kobo-diag] <loc> — (Rc<RefCell<T>>)` header line
/// and contains indented key: value lines until the next block or EOF.
fn parse_diag_log(text: &str) -> Vec<DiagOwnerStats> {
    let mut results = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();
        if !line.starts_with("[kobo-diag]") {
            i += 1;
            continue;
        }

        // Extract the binding location from the header line.
        // Format: "[kobo-diag] <loc> — (Rc<RefCell<T>>)"
        let loc_part = line
            .trim_start_matches("[kobo-diag]")
            .split(" — ")
            .next()
            .unwrap_or("")
            .trim()
            .to_owned();

        let mut borrow_count = 0u64;
        let mut mut_borrow_count = 0u64;
        let mut contention_count = 0u64;
        let mut saturated = false;

        i += 1;
        while i < lines.len() {
            let field_line = lines[i].trim();
            if field_line.starts_with("[kobo-diag]") {
                break; // start of next block
            }
            if let Some(val) = field_line.strip_prefix("borrow_count:") {
                borrow_count = val.trim().parse().unwrap_or(0);
            } else if let Some(val) = field_line.strip_prefix("mut_borrow_count:") {
                mut_borrow_count = val.trim().parse().unwrap_or(0);
            } else if let Some(val) = field_line.strip_prefix("contention_count:") {
                contention_count = val.trim().parse().unwrap_or(0);
            } else if let Some(val) = field_line.strip_prefix("saturated:") {
                saturated = val.trim() == "true";
            }
            i += 1;
        }

        results.push(DiagOwnerStats {
            source_location: KoboSpan::new(0, 0, FileId(0)),
            binding_name: loc_part,
            borrow_count,
            mut_borrow_count,
            contention_count,
            saturated,
            threshold: 10_000,
        });
    }

    results
}
