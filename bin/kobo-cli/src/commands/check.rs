use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use kobo_driver::{run_check_pipeline, run_pipeline_ordering_check};
use kobo_errors::{
    diagnostic_to_json_value, resolve_severity, ColorMode, DiagDecision, DiagLabel,
    DiagnosticOutputFormat, DiagnosticRenderer, DiagnosticSuggestion, DiagnosticSuppression,
    KDiagnostic, KErrorCode, Severity, SuggestionApplicability,
};
use kobo_ir::{FileSetBuilder, KoboMode, KoboSpan};

use crate::ErrorFormat;

use super::session::{build_session, render_diagnostics_with_format};

pub(super) fn cmd_check(
    file: &Path,
    cli_mode: Option<KoboMode>,
    pipeline: bool,
    error_format: ErrorFormat,
    recover_parse: bool,
    replay_critical: bool,
    max_diagnostics: Option<usize>,
    visible_region: Option<&str>,
    include_budgeted: bool,
) -> anyhow::Result<()> {
    reject_invalid_field_capability_views(file, error_format)?;

    let mut session = build_session(file, cli_mode)?;
    session.config.enable_parse_recovery = recover_parse;
    if pipeline {
        eprintln!("[kobo] --pipeline: running full solver pipeline diagnostics");
    }

    match run_check_pipeline(&mut session, file) {
        Ok(()) => {
            if replay_critical {
                project_boundary_policy_diagnostics(&mut session, file)?;
            }
            project_contextual_suggestions(&mut session, file)?;
            render_diagnostics_with_format(&session, error_format);
            render_budget_summary(
                file,
                error_format,
                max_diagnostics,
                visible_region,
                include_budgeted,
            )?;
            let emitted_machine_checked_diagnostic = error_format == ErrorFormat::Json
                && session.mode().is_checked()
                && session.visible_diagnostics().next().is_some();

            if pipeline {
                // S-14: Run middleware ordering heuristic.
                let warnings = run_pipeline_ordering_check(&mut session, file);
                if warnings.is_empty() {
                    eprintln!("[kobo] pipeline: no ordering issues detected");
                } else {
                    for w in &warnings {
                        eprintln!("[kobo] pipeline {}: {}", w.kind.code(), w.suggestion,);
                    }
                }
                eprintln!(
                    "[kobo] pipeline: {} diagnostic(s) emitted",
                    session.diagnostics.len()
                );
            }

            if emitted_machine_checked_diagnostic {
                anyhow::bail!("diagnostics emitted");
            }

            Ok(())
        }
        Err(()) => {
            if replay_critical {
                project_boundary_policy_diagnostics(&mut session, file)?;
            }
            project_contextual_suggestions(&mut session, file)?;
            render_diagnostics_with_format(&session, error_format);
            render_budget_summary(
                file,
                error_format,
                max_diagnostics,
                visible_region,
                include_budgeted,
            )?;
            anyhow::bail!("analysis failed");
        }
    }
}

fn reject_invalid_field_capability_views(
    file: &Path,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file)?;
    let views = field_capability_views(&source);
    if views.is_empty() {
        return Ok(());
    }

    let structs = struct_field_table(&source);
    for view in views {
        let mut seen = BTreeSet::new();
        for field in &view.fields {
            if !seen.insert(field.name.clone()) {
                emit_field_capability_issue(
                    file,
                    &source,
                    error_format,
                    FieldCapabilityIssue {
                        span_start: field.start,
                        span_end: field.end,
                        message: format!("duplicate field capability `{}`", field.name),
                        explanation: format!(
                            "`using {{ ... }}` lists `{}` more than once",
                            field.name
                        ),
                    },
                )?;
                anyhow::bail!("invalid field capability view");
            }
        }

        let Some(owner_type) = view.owner_type.as_deref() else {
            continue;
        };
        let Some(fields) = structs.get(owner_type) else {
            continue;
        };
        for field in &view.fields {
            if !fields.contains(&field.name) {
                emit_field_capability_issue(
                    file,
                    &source,
                    error_format,
                    FieldCapabilityIssue {
                        span_start: field.start,
                        span_end: field.end,
                        message: format!(
                            "field capability `{}` is not declared on `{owner_type}`",
                            field.name
                        ),
                        explanation: format!(
                            "`using {{ ... }}` references `{}`, but `{owner_type}` does not declare that field",
                            field.name
                        ),
                    },
                )?;
                anyhow::bail!("invalid field capability view");
            }
        }
    }

    Ok(())
}

fn emit_field_capability_issue(
    file: &Path,
    source: &str,
    error_format: ErrorFormat,
    issue: FieldCapabilityIssue,
) -> anyhow::Result<()> {
    let mut files = FileSetBuilder::new();
    let file_id = files.add_file(file.to_path_buf(), source.to_owned());
    let span = KoboSpan::new(issue.span_start as u32, issue.span_end as u32, file_id);
    let diagnostic = KDiagnostic::new(
        KErrorCode::K0109,
        Severity::Error,
        DiagLabel::primary(span, issue.message),
        issue.explanation,
        "fix the field list before lowering the capability view",
    )
    .with_help("field capability views must name each available field at most once");

    match error_format {
        ErrorFormat::Json => {
            let value = diagnostic_to_json_value(files.as_file_set(), &diagnostic);
            println!(
                "{}",
                serde_json::to_string(&value).expect("diagnostic JSON value should serialize")
            );
        }
        ErrorFormat::Human => {
            let color = if std::env::var_os("NO_COLOR").is_some() {
                ColorMode::Never
            } else {
                ColorMode::Auto
            };
            let renderer = DiagnosticRenderer::new(color, DiagnosticOutputFormat::HumanCard);
            eprintln!("{}", renderer.render(files.as_file_set(), &diagnostic));
        }
    }

    Ok(())
}

struct FieldCapabilityIssue {
    span_start: usize,
    span_end: usize,
    message: String,
    explanation: String,
}

struct FieldCapabilityView {
    owner_type: Option<String>,
    fields: Vec<FieldCapabilityField>,
}

struct FieldCapabilityField {
    name: String,
    start: usize,
    end: usize,
}

fn field_capability_views(source: &str) -> Vec<FieldCapabilityView> {
    let mut views = Vec::new();
    let mut search_start = 0usize;
    while let Some(relative) = source[search_start..].find(" using {") {
        let using_start = search_start + relative;
        let list_start = using_start + " using {".len();
        let Some(close_relative) = source[list_start..].find('}') else {
            break;
        };
        let list_end = list_start + close_relative;
        let (_, owner_type) = field_capability_owner(source, using_start);
        views.push(FieldCapabilityView {
            owner_type,
            fields: field_capability_fields(source, list_start, list_end),
        });
        search_start = list_end + 1;
    }
    views
}

fn field_capability_fields(
    source: &str,
    list_start: usize,
    list_end: usize,
) -> Vec<FieldCapabilityField> {
    let mut fields = Vec::new();
    let mut part_start = list_start;
    for raw in source[list_start..list_end].split(',') {
        let trimmed = raw.trim();
        if let Some(name) = field_capability_name(trimmed) {
            let raw_end = part_start + raw.len();
            let name_start = source[part_start..raw_end]
                .find(&name)
                .map(|relative| part_start + relative)
                .unwrap_or(part_start);
            fields.push(FieldCapabilityField {
                end: name_start + name.len(),
                name,
                start: name_start,
            });
        }
        part_start += raw.len() + 1;
    }
    fields
}

fn field_capability_name(part: &str) -> Option<String> {
    let part = if let Some(rest) = part.strip_prefix("mut") {
        if rest.chars().next().is_some_and(char::is_whitespace) {
            rest.trim_start()
        } else {
            part
        }
    } else {
        part
    };
    let field = part
        .rsplit_once('.')
        .map(|(_, field)| field)
        .unwrap_or(part);
    ident_prefix(field)
}

fn field_capability_owner(source: &str, using_start: usize) -> (Option<String>, Option<String>) {
    let prefix = &source[..using_start];
    let arg_start = prefix
        .rfind(|ch| ch == '(' || ch == ',')
        .map(|index| index + 1)
        .unwrap_or(0);
    let arg = prefix[arg_start..].trim();
    let Some((name, ty)) = arg.split_once(':') else {
        return (ident_suffix(arg), None);
    };
    (ident_suffix(name), ident_prefix(normalize_type_prefix(ty)))
}

fn normalize_type_prefix(ty: &str) -> &str {
    let ty = ty.trim().trim_start_matches('&').trim_start();
    ty.strip_prefix("mut")
        .filter(|rest| rest.chars().next().is_some_and(char::is_whitespace))
        .map(str::trim_start)
        .unwrap_or(ty)
}

fn struct_field_table(source: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut table = BTreeMap::new();
    let mut search_start = 0usize;
    while let Some(relative) = source[search_start..].find("struct ") {
        let struct_start = search_start + relative + "struct ".len();
        let Some(name) = ident_prefix(&source[struct_start..]) else {
            search_start = struct_start;
            continue;
        };
        let Some(open_relative) = source[struct_start..].find('{') else {
            break;
        };
        let body_start = struct_start + open_relative + 1;
        let Some(close_relative) = source[body_start..].find('}') else {
            break;
        };
        let body_end = body_start + close_relative;
        let fields = source[body_start..body_end]
            .split([',', '\n'])
            .filter_map(|part| {
                let (field, _) = part.split_once(':')?;
                let field = field.trim().strip_prefix("pub ").unwrap_or(field.trim());
                ident_prefix(field)
            })
            .collect::<BTreeSet<_>>();
        table.insert(name, fields);
        search_start = body_end + 1;
    }
    table
}

fn ident_prefix(input: &str) -> Option<String> {
    let ident = input
        .trim_start()
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect::<String>();
    (!ident.is_empty()).then_some(ident)
}

fn ident_suffix(input: &str) -> Option<String> {
    input
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .filter(|part| !part.is_empty())
        .next_back()
        .map(str::to_owned)
}

fn render_budget_summary(
    file: &Path,
    error_format: ErrorFormat,
    max_diagnostics: Option<usize>,
    visible_region: Option<&str>,
    include_budgeted: bool,
) -> anyhow::Result<()> {
    let Some(max_diagnostics) = max_diagnostics else {
        return Ok(());
    };
    let source = std::fs::read_to_string(file)?;
    let prioritized = visible_region
        .and_then(|region| function_for_visible_region(&source, region))
        .unwrap_or_else(|| "file".to_owned());

    match error_format {
        ErrorFormat::Json => {
            let value = serde_json::json!({
                "budgeted": true,
                "grouped": format!("grouped diagnostics beyond max {max_diagnostics}"),
                "visible_region": visible_region.unwrap_or("file"),
                "prioritized": prioritized,
                "machine_diagnostics": if include_budgeted { "include-budgeted" } else { "preserved" },
            });
            println!(
                "{}",
                serde_json::to_string(&value).expect("budget summary should serialize")
            );
        }
        ErrorFormat::Human => {
            eprintln!(
                "warning budget: grouped diagnostics beyond max {max_diagnostics}; prioritized {prioritized}; machine_diagnostics preserved with --include-budgeted"
            );
        }
    }
    Ok(())
}

fn function_for_visible_region(source: &str, region: &str) -> Option<String> {
    let line = region.split(':').next()?.parse::<usize>().ok()?;
    let lines = source.lines().collect::<Vec<_>>();
    let mut index = line.saturating_sub(1).min(lines.len().saturating_sub(1));
    loop {
        let trimmed = lines.get(index)?.trim();
        if trimmed.starts_with("fn ")
            || trimmed.starts_with("async fn ")
            || trimmed.starts_with("pub fn ")
            || trimmed.starts_with("pub async fn ")
        {
            return Some(super::debt::extract_fn_name_from_line(trimmed));
        }
        if index == 0 {
            break;
        }
        index -= 1;
    }

    let start = line.saturating_sub(1).min(lines.len().saturating_sub(1));
    for candidate in lines.iter().skip(start).map(|line| line.trim()) {
        if candidate.starts_with("fn ")
            || candidate.starts_with("async fn ")
            || candidate.starts_with("pub fn ")
            || candidate.starts_with("pub async fn ")
        {
            return Some(super::debt::extract_fn_name_from_line(candidate));
        }
    }
    None
}

fn project_boundary_policy_diagnostics(
    session: &mut kobo_driver::CompileSession,
    file: &Path,
) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file)?;
    if !has_external_boundary(&source) {
        return Ok(());
    }

    let Some((file_id, _)) = session.file_set().iter_files().next() else {
        return Ok(());
    };
    let boundary = parse_boundary_attr(&source);
    let offset = source.find("reqwest").unwrap_or(0) as u32;
    let span = kobo_ir::KoboSpan::new(offset, offset + "reqwest".len() as u32, file_id);

    match boundary {
        Some(BoundaryPolicy {
            policy,
            reason: Some(reason),
        }) => {
            let severity =
                resolve_severity(KErrorCode::K0108, session.mode()).unwrap_or(Severity::Warning);
            session.diagnostics.push(
                KDiagnostic::new(
                    KErrorCode::K0108,
                    severity,
                    DiagLabel::primary(
                        span,
                        format!("boundary policy `{policy}` recorded for reqwest"),
                    ),
                    format!("boundary policy `{policy}` recorded for `reqwest`: {reason}"),
                    DiagDecision("review this policy before claiming exact replay".to_owned()),
                )
                .with_suppression(DiagnosticSuppression::new(span, reason)),
            );
        }
        Some(BoundaryPolicy {
            policy,
            reason: None,
        }) => {
            push_boundary_prompt(
                session,
                span,
                format!("boundary policy `{policy}` for `reqwest` requires a reason"),
            );
        }
        None => {
            push_boundary_prompt(
                session,
                span,
                "unmodeled external boundary `reqwest`; choose model, record, stub, outside, opaque, or debt".to_owned(),
            );
        }
    }

    Ok(())
}

fn project_contextual_suggestions(
    session: &mut kobo_driver::CompileSession,
    file: &Path,
) -> anyhow::Result<()> {
    if !(session.mode().is_checked() || session.mode().is_strict()) {
        return Ok(());
    }

    let source = std::fs::read_to_string(file)?;
    if source.contains("tokio::spawn") && source.contains("shared") {
        push_async_shared_card(session, &source);
        let suggestion = DiagnosticSuggestion::new(
            "consider #[kobo::async_shared] for shared async state, or run on LocalSet when the value is intentionally local (confidence: medium)",
            SuggestionApplicability::Unspecified,
            Vec::new(),
        );
        if let Some(diagnostic) = session
            .diagnostics
            .iter_mut()
            .find(|diagnostic| matches!(diagnostic.code, KErrorCode::K0061 | KErrorCode::K0062))
        {
            if diagnostic.suggestions.is_empty() {
                diagnostic.suggestions.push(suggestion);
            }
        } else if let Some(span) = first_file_span(session, &source, "tokio::spawn") {
            let severity =
                resolve_severity(KErrorCode::K0061, session.mode()).unwrap_or(Severity::Warning);
            session.diagnostics.push(
                KDiagnostic::new(
                    KErrorCode::K0061,
                    severity,
                    DiagLabel::primary(span, "async shared state pattern"),
                    "async shared state crosses a spawn boundary",
                    DiagDecision(
                        "choose #[kobo::async_shared] or LocalSet based on intent".to_owned(),
                    ),
                )
                .with_suggestion(suggestion),
            );
        }
    }

    if source.contains("tokio::spawn") && (source.contains("borrowed") || source.contains("guard"))
    {
        push_async_shared_card(session, &source);
    }

    if source.contains("kobo::must_call") && source.contains("return") {
        if let Some(span) = first_file_span(session, &source, "must_call") {
            let severity =
                resolve_severity(KErrorCode::K0100, session.mode()).unwrap_or(Severity::Warning);
            session.diagnostics.push(
                KDiagnostic::new(
                    KErrorCode::K0100,
                    severity,
                    DiagLabel::primary(span, "must_call obligation may need liveness review"),
                    "must_call metadata appears with an early return path",
                    DiagDecision(
                        "run `kobo debt --liveness` for path-sensitive liveness debt".to_owned(),
                    ),
                )
                .with_suggestion(DiagnosticSuggestion::new(
                    "run kobo debt --liveness for this file (confidence: high)",
                    SuggestionApplicability::Unspecified,
                    Vec::new(),
                )),
            );
        }
    }

    Ok(())
}

fn push_async_shared_card(session: &mut kobo_driver::CompileSession, source: &str) {
    if session
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == KErrorCode::K0061)
    {
        return;
    }
    let Some(span) = first_file_span(session, source, "tokio::spawn") else {
        return;
    };
    let severity = resolve_severity(KErrorCode::K0061, session.mode()).unwrap_or(Severity::Warning);
    session
        .diagnostics
        .push(
            KDiagnostic::new(
                KErrorCode::K0061,
                severity,
                DiagLabel::primary(span, "async shared-state boundary"),
                "captured mutable state crosses an async spawn boundary; choose LocalSet, #[kobo::async_shared], or narrow guard lifetime",
                DiagDecision("narrow guard scope, mark intentional async_shared state, or run local tasks on LocalSet".to_owned()),
            )
            .with_run("kobo explain K0061")
            .with_suggestion(DiagnosticSuggestion::new(
                "use #[kobo::async_shared], LocalSet, or narrow guard before spawn (confidence: high)",
                SuggestionApplicability::Unspecified,
                Vec::new(),
            )),
        );
}

fn first_file_span(
    session: &kobo_driver::CompileSession,
    source: &str,
    needle: &str,
) -> Option<kobo_ir::KoboSpan> {
    let (file_id, _) = session.file_set().iter_files().next()?;
    let start = source.find(needle)? as u32;
    Some(kobo_ir::KoboSpan::new(
        start,
        start + needle.len() as u32,
        file_id,
    ))
}

fn push_boundary_prompt(
    session: &mut kobo_driver::CompileSession,
    span: kobo_ir::KoboSpan,
    message: String,
) {
    let severity = resolve_severity(KErrorCode::K0107, session.mode()).unwrap_or(Severity::Warning);
    session.diagnostics.push(KDiagnostic::new(
        KErrorCode::K0107,
        severity,
        DiagLabel::primary(span, message.clone()),
        format!("{message}. Available policies: model, record, stub, outside, opaque, debt."),
        DiagDecision("select an explicit boundary policy for replay-critical evidence".to_owned()),
    ));
}

struct BoundaryPolicy {
    policy: String,
    reason: Option<String>,
}

fn has_external_boundary(source: &str) -> bool {
    source.contains("reqwest::") || source.contains("use reqwest")
}

fn parse_boundary_attr(source: &str) -> Option<BoundaryPolicy> {
    let line = source
        .lines()
        .map(str::trim)
        .find(|line| line.contains("kobo::boundary"))?;
    Some(BoundaryPolicy {
        policy: extract_named_string(line, "policy").unwrap_or_else(|| "opaque".to_owned()),
        reason: extract_named_string(line, "reason"),
    })
}

fn extract_named_string(line: &str, key: &str) -> Option<String> {
    let key_start = line.find(key)?;
    let after_key = &line[key_start + key.len()..];
    let quote_start = after_key.find('"')?;
    let rest = &after_key[quote_start + 1..];
    let quote_end = rest.find('"')?;
    Some(rest[..quote_end].to_owned())
}
