use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::Context;
use kobo_driver::{run_and_compile_for_debt_probe, CompileSession};
use kobo_errors::{resolve_color_mode_from_parts, ColorMode, KDiagnostic};
use kobo_ir::debt::{DebtReport, OwnershipDebtRecord};
use kobo_ir::{FileSet, Kir, KoboSpan};

use crate::commands::session::build_session;

pub(super) fn ownership_debt_records(
    kir: &Kir,
    session: &CompileSession,
) -> Vec<OwnershipDebtRecord> {
    let facts = kobo_analysis::run_analysis(kir, session.file_set());
    kobo_analysis::facts_to_ownership_debt(&facts, kir.transform_facts(), session.file_set())
}

pub(super) fn rustc_escape_debt_records<'a>(
    diagnostics: impl IntoIterator<Item = &'a KDiagnostic>,
) -> Vec<OwnershipDebtRecord> {
    diagnostics
        .into_iter()
        .filter_map(|diagnostic| diagnostic.ownership_debt.clone())
        .collect()
}

pub(super) fn rustc_escape_debt_for_file(file: &Path) -> anyhow::Result<Vec<OwnershipDebtRecord>> {
    let mut session = build_session(file, None)?;
    let output_dir = rustc_escape_probe_output_dir(file);
    std::fs::create_dir_all(&output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;
    session.config.output_dir = Some(output_dir.clone());

    let _ = run_and_compile_for_debt_probe(&mut session, file);
    let records = rustc_escape_debt_records(session.visible_diagnostics());
    let _ = std::fs::remove_dir_all(&output_dir);
    Ok(records)
}

fn rustc_escape_probe_output_dir(file: &Path) -> PathBuf {
    let stem = file
        .file_stem()
        .map(|stem| stem.to_string_lossy())
        .unwrap_or_else(|| "source".into());
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "kobo-debt-rustc-escape-{}-{stem}-{nonce}",
        std::process::id()
    ))
}

pub(super) fn extend_unique_ownership_debt(
    records: &mut Vec<OwnershipDebtRecord>,
    additional: impl IntoIterator<Item = OwnershipDebtRecord>,
) {
    for record in additional {
        if records
            .iter()
            .any(|existing| same_debt_site(existing, &record))
        {
            continue;
        }
        records.push(record);
    }
}

fn same_debt_site(left: &OwnershipDebtRecord, right: &OwnershipDebtRecord) -> bool {
    left.code == right.code
        && left.span.file_id == right.span.file_id
        && left.span.start == right.span.start
        && left.span.end == right.span.end
}

pub(super) fn format_ownership_debt(file_set: &FileSet, report: &DebtReport) -> String {
    let mut output = String::new();
    for record in &report.ownership_debt {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&format!(
            "{}[{}] {}: {}\n",
            record.severity.as_str(),
            record.code.as_str(),
            debt_location(file_set, record.span),
            record.message
        ));
        output.push_str("  hint: ");
        output.push_str(&record.hint);
    }
    output
}

fn debt_location(file_set: &FileSet, span: KoboSpan) -> String {
    let Some(file) = file_set.get(span.file_id) else {
        return "unknown:1:1".to_owned();
    };
    let (line, column) = file.line_col(span.start);
    format!("{}:{line}:{column}", file.path.display())
}

pub(super) fn resolve_debt_color_mode(requested: ColorMode) -> ColorMode {
    resolve_color_mode_from_parts(
        requested,
        std::env::var_os("NO_COLOR").is_some(),
        std::io::stdout().is_terminal(),
    )
}

pub(super) fn colorize_debt_output(input: &str, color_mode: ColorMode) -> String {
    if color_mode != ColorMode::Always {
        return input.to_owned();
    }

    let mut output = String::new();
    for (index, line) in input.lines().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        output.push_str(&colorize_debt_line(line));
    }
    if input.ends_with('\n') {
        output.push('\n');
    }
    output
}

fn colorize_debt_line(line: &str) -> String {
    let trimmed = line.trim_start();
    if line.starts_with("warning[") || trimmed.starts_with("note[") {
        format!("\x1b[1;33m{line}\x1b[0m")
    } else if trimmed.starts_with("hint:") || trimmed.starts_with("= migration:") {
        format!("\x1b[2;33m{line}\x1b[0m")
    } else {
        line.to_owned()
    }
}
