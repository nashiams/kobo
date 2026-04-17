use std::path::Path;

use anyhow::Context;
use kobo_debt::{build_debt_report, format_warn_early};
use kobo_debt::borrow_report::build_borrow_report;
use kobo_driver::run_kir_phase;

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

pub(super) fn cmd_debt_borrows(file: &Path) -> anyhow::Result<()> {
    let mut session = build_session(file, None)?;
    let (_, kir) = run_kir_phase(&mut session, file)
        .map_err(|()| anyhow::anyhow!("failed to build KIR for {}", file.display()))?;

    let tf = kir.transform_facts();
    let mut total_overlaps = 0usize;

    for (i, binding) in tf.bindings.iter().enumerate() {
        let usage = &tf.usages[i];
        let shared = &tf.shared_facts[i];
        let report = build_borrow_report(&binding.binding_name, usage, shared);
        for overlap in &report.overlapping_sites {
            total_overlaps += 1;
            println!(
                "Borrow overlap in `{}`: {} immutable, {} mutable span(s). Fix: {:?}",
                overlap.binding_name,
                overlap.immutable_spans.len(),
                overlap.mutable_spans.len(),
                overlap.fix_pattern,
            );
        }
    }

    if total_overlaps == 0 {
        println!("No borrow overlaps detected.");
    } else {
        println!("\n{total_overlaps} borrow overlap(s) found.");
    }

    Ok(())
}
