use std::path::Path;

use anyhow::Context;
use kobo_debt::borrow_report::{build_borrow_report, BorrowReport};
use kobo_debt::patterns::{detect_migration_patterns, format_patterns};
use kobo_debt::{build_debt_report, format_warn_early};
use kobo_driver::{lifetime_erasure_debt_report, run_kir_phase};
use kobo_migrate::{greedy_resolve, GreedyConfig};

use super::session::build_session;

pub(super) fn cmd_debt(file: &Path, json: bool, summary: bool) -> anyhow::Result<()> {
    let mut session = build_session(file, None)?;
    let (_, kir) = run_kir_phase(&mut session, file)
        .map_err(|()| anyhow::anyhow!("failed to build KIR for {}", file.display()))?;

    let (file_count, line_count) = count_files_and_lines(session.file_set());
    let report = build_debt_report(&kir, file_count, line_count);

    if json {
        let json_str = serde_json::to_string_pretty(&report)
            .context("failed to serialize debt report to JSON")?;
        println!("{json_str}");
        return Ok(());
    }

    if summary {
        println!(
            "{} file(s), {} line(s) — {} RcMutShared site(s) [T1:{} T2:{} T3:{}] — {} warning(s)",
            report.file_count,
            report.line_count,
            report.inventory.rc_mut_shared,
            report.complexity.tier1,
            report.complexity.tier2,
            report.complexity.tier3,
            report.warn_early.len(),
        );
        return Ok(());
    }

    // Human-readable output.
    let human = format_warn_early(&report);
    if human.is_empty() {
        println!(
            "Debt report: {} RcMutShared site(s). No active structural warnings.",
            report.inventory.rc_mut_shared
        );
    } else {
        println!("{human}");
    }

    Ok(())
}

fn count_files_and_lines(file_set: &kobo_ir::FileSet) -> (usize, usize) {
    let mut file_count = 0usize;
    let mut line_count = 0usize;
    for (_id, entry) in file_set.iter_files() {
        file_count += 1;
        line_count += entry.source.lines().count();
    }
    (file_count, line_count)
}

pub(super) fn cmd_debt_borrows(file: &Path, json: bool) -> anyhow::Result<()> {
    let mut session = build_session(file, None)?;
    let (_, kir) = run_kir_phase(&mut session, file)
        .map_err(|()| anyhow::anyhow!("failed to build KIR for {}", file.display()))?;
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;
    let lifetime_debt = lifetime_erasure_debt_report(&source, session.mode());

    let tf = kir.transform_facts();
    let mut all_overlaps = Vec::new();
    let total_bindings = tf.bindings.len();
    let mut bindings_with_overlaps = 0usize;

    for (i, binding) in tf.bindings.iter().enumerate() {
        let usage = &tf.usages[i];
        let shared = &tf.shared_facts[i];
        let report = build_borrow_report(&binding.binding_name, usage, shared);
        if !report.overlapping_sites.is_empty() {
            bindings_with_overlaps += 1;
        }
        all_overlaps.extend(report.overlapping_sites);
    }

    let combined = BorrowReport {
        schema_version: 1,
        total_bindings_analyzed: total_bindings,
        bindings_with_overlaps,
        overlapping_sites: all_overlaps,
    };

    if json {
        let json_str = serde_json::to_string_pretty(&combined)
            .context("failed to serialize borrow report to JSON")?;
        println!("{json_str}");
        return Ok(());
    }

    // Human-readable output.
    if combined.overlapping_sites.is_empty() {
        println!("No borrow overlaps detected.");
    } else {
        for overlap in &combined.overlapping_sites {
            println!(
                "Borrow overlap in `{}`: {} immutable, {} mutable span(s). Fix: {:?}",
                overlap.binding_name,
                overlap.immutable_spans.len(),
                overlap.mutable_spans.len(),
                overlap.fix_pattern,
            );
        }
        println!(
            "\n{} borrow overlap(s) found.",
            combined.overlapping_sites.len()
        );
    }

    if !lifetime_debt.is_empty() {
        println!("\n{lifetime_debt}");
    }

    Ok(())
}

pub(super) fn cmd_debt_patterns(file: &Path, json: bool) -> anyhow::Result<()> {
    let mut session = build_session(file, None)?;
    let (_, kir) = run_kir_phase(&mut session, file)
        .map_err(|()| anyhow::anyhow!("failed to build KIR for {}", file.display()))?;

    let config = GreedyConfig {
        solver_cluster_limit: session.config.solver_cluster_limit,
        solver_budget_seconds: session.config.solver_budget_seconds,
        mutable_sites_threshold: GreedyConfig::default().mutable_sites_threshold,
    };
    let result = greedy_resolve(&kir, &config);
    let patterns = detect_migration_patterns(&kir, &result.resolved);

    if json {
        let json_str = serde_json::to_string_pretty(&patterns)
            .context("failed to serialize patterns to JSON")?;
        println!("{json_str}");
        return Ok(());
    }

    let output = format_patterns(&patterns);
    println!("{output}");
    Ok(())
}

/// S-62: Report functions where typed errors would replace boxed/dynamic errors.
///
/// Scans generated code for `Box<dyn Error>`, `anyhow::Error`, and `?` usage
/// in functions that return `Result`. Suggests where typed error enums would help.
pub(super) fn cmd_debt_errors(file: &Path, json: bool) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;

    // Find functions returning Result and count `?` usage
    let mut entries: Vec<ErrorDebtEntry> = Vec::new();
    let lines: Vec<&str> = source.lines().collect();

    let mut current_fn: Option<String> = None;
    let mut question_marks: usize = 0;
    let mut brace_depth: i32 = 0;

    for line in &lines {
        let trimmed = line.trim();

        // Detect function start
        if (trimmed.starts_with("fn ")
            || trimmed.starts_with("async fn ")
            || trimmed.starts_with("pub fn ")
            || trimmed.starts_with("pub async fn "))
            && trimmed.contains("->")
        {
            let fn_name = extract_fn_name_from_line(trimmed);
            if trimmed.contains("Result") {
                current_fn = Some(fn_name);
                question_marks = 0;
                brace_depth = 0;
            }
        }

        if current_fn.is_some() {
            brace_depth += trimmed.matches('{').count() as i32;
            brace_depth -= trimmed.matches('}').count() as i32;
            question_marks += trimmed.matches('?').count();

            if brace_depth <= 0 && trimmed.contains('}') {
                if let Some(fn_name) = current_fn.take() {
                    if question_marks > 0 {
                        let uses_boxed = trimmed.contains("Box<dyn")
                            || source.contains(&format!("fn {fn_name}"))
                                && source.contains("Box<dyn Error");
                        entries.push(ErrorDebtEntry {
                            fn_name,
                            question_mark_count: question_marks,
                            uses_boxed_error: uses_boxed,
                            suggestion: if question_marks > 3 {
                                "consider a typed error enum".to_string()
                            } else {
                                "boxed error is acceptable".to_string()
                            },
                        });
                    }
                }
            }
        }
    }

    if json {
        let values: Vec<serde_json::Value> = entries.iter().map(|e| e.to_json_value()).collect();
        let json_str = serde_json::to_string_pretty(&values)
            .context("failed to serialize error debt to JSON")?;
        println!("{json_str}");
        return Ok(());
    }

    if entries.is_empty() {
        println!("No error debt found — no Result-returning functions with `?` usage.");
    } else {
        println!("Error Debt Report");
        println!("═════════════════");
        for entry in &entries {
            println!(
                "  {} — {}x `?` — {}",
                entry.fn_name, entry.question_mark_count, entry.suggestion,
            );
        }
        println!("\n{} function(s) with error debt.", entries.len());
    }

    Ok(())
}

#[derive(Debug)]
struct ErrorDebtEntry {
    fn_name: String,
    question_mark_count: usize,
    uses_boxed_error: bool,
    suggestion: String,
}

impl ErrorDebtEntry {
    fn to_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "fn_name": self.fn_name,
            "question_mark_count": self.question_mark_count,
            "uses_boxed_error": self.uses_boxed_error,
            "suggestion": self.suggestion,
        })
    }
}

fn extract_fn_name_from_line(line: &str) -> String {
    let rest = line
        .strip_prefix("pub async fn ")
        .or_else(|| line.strip_prefix("async fn "))
        .or_else(|| line.strip_prefix("pub fn "))
        .or_else(|| line.strip_prefix("fn "))
        .unwrap_or(line);
    rest.split('(')
        .next()
        .unwrap_or("unknown")
        .trim()
        .to_string()
}
