use std::fs;
use std::path::Path;

use anyhow::Context;
use kobo_errors::{format_diagnostic, render_k0020, render_k0021, DiagOwnerStats};
use kobo_ir::{FileId, FileSetBuilder, KoboSpan};

use super::ownership_analysis;

#[derive(Copy, Clone)]
struct SourceSpanContext<'a> {
    file_id: FileId,
    source: &'a str,
}

pub(super) fn cmd_perf(
    file: &Path,
    from: Option<&Path>,
    threshold: Option<u64>,
    format: Option<&str>,
    evidence: &str,
    session: Option<&Path>,
) -> anyhow::Result<()> {
    let threshold = threshold
        .or_else(|| std::env::var("KOBO_DIAG_THRESHOLD").ok()?.parse().ok())
        .unwrap_or(10_000_u64);

    match evidence {
        "estimate" => {}
        "real" => {
            let session = session.context("--evidence real requires --session <DIR>")?;
            print!("{}", session_perf_json(file, session, threshold)?);
            return Ok(());
        }
        other => anyhow::bail!("invalid perf evidence `{other}`; expected estimate or real"),
    }

    if format == Some("json") {
        if let Some(log) = from {
            let diag_text = fs::read_to_string(log)
                .with_context(|| format!("failed to read diag log {}", log.display()))?;
            print!("{}", diag_log_perf_json(file, &diag_text, threshold)?);
        } else {
            let source = fs::read_to_string(file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            print!("{}", source_perf_json(file, &source, threshold)?);
        }
        return Ok(());
    }

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

    let source =
        fs::read_to_string(file).with_context(|| format!("failed to read {}", file.display()))?;
    let mut file_set_builder = FileSetBuilder::new();
    let file_id = file_set_builder.add_file(file.to_path_buf(), source.clone());
    let source_context = SourceSpanContext {
        file_id,
        source: &source,
    };
    let entries = parse_diag_log(&diag_text, threshold, Some(source_context));
    if entries.is_empty() {
        println!("No [kobo-diag] entries found in log (threshold: {threshold}).");
        return Ok(());
    }

    // Emit diagnostics to stdout as plain text; no session needed for perf.
    let file_set = file_set_builder.as_file_set();
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

fn session_perf_json(file: &Path, session: &Path, threshold: u64) -> anyhow::Result<String> {
    let path = session.join("diagowner.jsonl");
    let text = fs::read_to_string(&path)
        .with_context(|| format!("failed to read structured session artifact {}", path.display()))?;
    let mut borrow_count = 0_u64;
    let mut mut_borrow_count = 0_u64;
    let mut contention = 0_u64;
    let mut hot_paths = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let value: serde_json::Value = serde_json::from_str(line)
            .with_context(|| format!("invalid diagowner JSON line in {}", path.display()))?;
        let entry_borrow = value["borrow_count"].as_u64().unwrap_or(0);
        let entry_mut = value["mut_borrow_count"].as_u64().unwrap_or(0);
        let entry_contention = value["contention"]
            .as_u64()
            .or_else(|| value["contention_count"].as_u64())
            .unwrap_or(0);
        borrow_count += entry_borrow;
        mut_borrow_count += entry_mut;
        contention += entry_contention;
        hot_paths.push(serde_json::json!({
            "binding": value["binding"].as_str().unwrap_or("<unknown>"),
            "borrow_count": entry_borrow,
            "mut_borrow_count": entry_mut,
            "contention": entry_contention,
            "line": value["span"]["line"].as_u64().unwrap_or(0),
            "column": value["span"]["column"].as_u64().unwrap_or(0),
            "hot": entry_borrow > threshold || entry_mut > threshold,
        }));
    }
    let value = serde_json::json!({
        "file": cli_relative_path(file)?,
        "source": "session_diagowner",
        "borrow_count": borrow_count,
        "mut_borrow_count": mut_borrow_count,
        "contention": contention,
        "hot_paths": hot_paths,
        "threshold": threshold,
    });
    Ok(format!("{}\n", serde_json::to_string(&value)?))
}

fn source_perf_json(file: &Path, source: &str, threshold: u64) -> anyhow::Result<String> {
    let analysis = ownership_analysis::analyze_source(source);
    let hot_paths = analysis
        .perf
        .hot_paths
        .iter()
        .map(|path| {
            serde_json::json!({
                "line": path.line,
                "kind": path.kind,
                "binding": path.binding,
                "field": path.field,
            })
        })
        .collect::<Vec<_>>();
    let value = serde_json::json!({
        "file": cli_relative_path(file)?,
        "source": "source_estimate",
        "evidence": "source-derived estimate; pass --from diag.log for prior-run DiagOwner stats",
        "borrow_count": analysis.perf.borrow_count,
        "mut_borrow_count": analysis.perf.mut_borrow_count,
        "contention": analysis.perf.contention,
        "hot_paths": hot_paths,
        "threshold": threshold,
    });
    Ok(format!("{}\n", serde_json::to_string(&value)?))
}

fn diag_log_perf_json(file: &Path, diag_text: &str, threshold: u64) -> anyhow::Result<String> {
    let entries = parse_diag_log(diag_text, threshold, None);
    let borrow_count = entries.iter().map(|stats| stats.borrow_count).sum::<u64>();
    let mut_borrow_count = entries
        .iter()
        .map(|stats| stats.mut_borrow_count)
        .sum::<u64>();
    let contention = entries
        .iter()
        .map(|stats| stats.contention_count)
        .sum::<u64>();
    let hot_paths = entries
        .iter()
        .map(|stats| {
            serde_json::json!({
                "binding": stats.binding_name,
                "borrow_count": stats.borrow_count,
                "mut_borrow_count": stats.mut_borrow_count,
                "contention": stats.contention_count,
                "saturated": stats.saturated,
                "hot": stats.borrow_count > threshold || stats.mut_borrow_count > threshold,
            })
        })
        .collect::<Vec<_>>();
    let value = serde_json::json!({
        "file": cli_relative_path(file)?,
        "source": "diag_log",
        "borrow_count": borrow_count,
        "mut_borrow_count": mut_borrow_count,
        "contention": contention,
        "hot_paths": hot_paths,
        "threshold": threshold,
    });
    Ok(format!("{}\n", serde_json::to_string(&value)?))
}

fn cli_relative_path(file: &Path) -> anyhow::Result<String> {
    let absolute = if file.is_absolute() {
        file.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to determine current directory")?
            .join(file)
    };
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let display_path = absolute.strip_prefix(&cwd).unwrap_or(&absolute);
    Ok(display_path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

/// Parse all `[kobo-diag]` blocks from a diag log capture.
///
/// Each block starts with a `[kobo-diag] <loc> — (Rc<RefCell<T>>)` header line
/// and contains indented key: value lines until the next block or EOF.
fn parse_diag_log(
    text: &str,
    threshold: u64,
    source_context: Option<SourceSpanContext<'_>>,
) -> Vec<DiagOwnerStats> {
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
        let loc_part = loc_part
            .split(" - ")
            .next()
            .unwrap_or(&loc_part)
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

        let source_location = source_context
            .and_then(|context| span_from_log_location(&loc_part, context))
            .unwrap_or_else(|| KoboSpan::new(0, 0, FileId(0)));

        results.push(DiagOwnerStats {
            source_location,
            binding_name: loc_part,
            borrow_count,
            mut_borrow_count,
            contention_count,
            saturated,
            threshold,
        });
    }

    results
}

fn span_from_log_location(
    location: &str,
    source_context: SourceSpanContext<'_>,
) -> Option<KoboSpan> {
    let (line, column) = line_column_from_location(location)?;
    let offset = byte_offset_for_line_column(source_context.source, line, column)?;
    let end = (offset + 1).min(source_context.source.len());
    Some(KoboSpan::new(
        offset as u32,
        end.max(offset) as u32,
        source_context.file_id,
    ))
}

fn line_column_from_location(location: &str) -> Option<(usize, usize)> {
    let mut parts = location.rsplitn(3, ':');
    let column = parts.next()?.trim().parse().ok()?;
    let line = parts.next()?.trim().parse().ok()?;
    Some((line, column))
}

fn byte_offset_for_line_column(source: &str, line: usize, column: usize) -> Option<usize> {
    if line == 0 || column == 0 {
        return None;
    }

    let mut current_line = 1usize;
    let mut line_start = 0usize;
    for (index, byte) in source.bytes().enumerate() {
        if current_line == line {
            break;
        }
        if byte == b'\n' {
            current_line += 1;
            line_start = index + 1;
        }
    }

    if current_line != line {
        return None;
    }

    let line_end = source[line_start..]
        .find('\n')
        .map(|relative_end| line_start + relative_end)
        .unwrap_or(source.len());
    Some((line_start + column - 1).min(line_end).min(source.len()))
}
