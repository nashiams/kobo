use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Command,
};

use kobo_driver::{run_check_pipeline, run_pipeline_ordering_check};
use kobo_errors::{
    diagnostic_to_json_value, resolve_severity, ColorMode, DiagDecision, DiagLabel,
    DiagnosticOutputFormat, DiagnosticRenderer, DiagnosticSuggestion, DiagnosticSuppression,
    KDiagnostic, KErrorCode, Severity, SuggestionApplicability,
};
use kobo_ir::{FileSetBuilder, GuaranteePolicy, KoboSpan, ScenarioBoundaryPolicy};
use syn::{spanned::Spanned, visit::Visit};

use crate::ProofReplayGradeArg;
use crate::{ErrorFormat, GuaranteeProfileArg, PolicyOutputFormat};

use super::{
    declarations::{self, DeclarationFacts, DeclarationLookup},
    policy,
    session::{build_session, render_diagnostics_with_format},
    summary_validation,
};

pub(super) fn cmd_check(
    file: &Path,
    cli_policy: Option<GuaranteePolicy>,
    guarantee_profile: Option<GuaranteeProfileArg>,
    print_policy: Option<PolicyOutputFormat>,
    pipeline: bool,
    error_format: ErrorFormat,
    color_mode: ColorMode,
    recover_parse: bool,
    replay_critical: bool,
    max_diagnostics: Option<usize>,
    visible_region: Option<&str>,
    include_budgeted: bool,
    emit_proof: Option<ProofReplayGradeArg>,
) -> anyhow::Result<()> {
    reject_invalid_field_capability_views(file, error_format, color_mode)?;
    reject_malformed_scenario_attributes(file, error_format)?;
    reject_invalid_ecosystem_policy(file, error_format)?;
    let mut effective_policy = if guarantee_profile.is_some() || print_policy.is_some() {
        let profile = guarantee_profile.unwrap_or(GuaranteeProfileArg::Dev);
        let loaded = policy::load_effective_policy(Some(file), profile)?;
        if let Some(downgrade) = loaded.downgrade() {
            policy::emit_downgrade(downgrade, error_format)?;
            anyhow::bail!("guarantee policy downgrade requires reason ledger entry");
        }
        if print_policy.is_some() {
            policy::print_policy_json(&loaded)?;
        }
        Some(loaded)
    } else {
        None
    };

    let session_policy = effective_policy
        .as_ref()
        .map(|policy| policy.compiler_policy().clone())
        .or(cli_policy);
    let mut session = build_session(file, session_policy)?;
    if effective_policy.is_none() {
        effective_policy =
            policy::load_configured_release_policy(Some(file), session.guarantee_policy())?;
    }
    session.config.enable_parse_recovery = recover_parse;
    if pipeline {
        eprintln!("[kobo] --pipeline: running full solver pipeline diagnostics");
    }

    match run_check_pipeline(&mut session, file) {
        Ok(()) => {
            let boundary_evidence =
                if replay_critical || boundary_policy_visibility_enabled(&session) {
                    project_boundary_policy_diagnostics(&mut session, file)?
                } else {
                    Vec::new()
                };
            project_contextual_suggestions(&mut session, file)?;
            render_diagnostics_with_format(&session, error_format, color_mode);
            emit_boundary_policy_evidence(&boundary_evidence, error_format)?;
            if replay_critical || !session.config.ecosystem_policy.summaries.is_empty() {
                emit_summary_policy_evidence(&session.config, error_format)?;
            }
            render_budget_summary(
                file,
                error_format,
                max_diagnostics,
                visible_region,
                include_budgeted,
            )?;
            let emitted_machine_checked_diagnostic = error_format == ErrorFormat::Json
                && session.guarantee_policy().is_checked()
                && session.visible_diagnostics().next().is_some();
            let emitted_replay_blocking_diagnostic = has_replay_blocking_diagnostic(&session);

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

            if emitted_machine_checked_diagnostic || emitted_replay_blocking_diagnostic {
                return Err(super::diagnostics_emitted());
            }
            enforce_configured_new_debt_gate(&session, effective_policy.as_ref(), error_format)?;

            if print_policy.is_none() {
                if let Some(policy) = effective_policy.as_ref() {
                    policy::emit_policy_summary(policy);
                }
            }

            if let Some(replay_grade) = emit_proof {
                super::proof::emit_check_proof(file, replay_grade)?;
            }

            Ok(())
        }
        Err(()) => {
            let boundary_evidence =
                if replay_critical || boundary_policy_visibility_enabled(&session) {
                    project_boundary_policy_diagnostics(&mut session, file)?
                } else {
                    Vec::new()
                };
            project_contextual_suggestions(&mut session, file)?;
            render_diagnostics_with_format(&session, error_format, color_mode);
            emit_boundary_policy_evidence(&boundary_evidence, error_format)?;
            if replay_critical || !session.config.ecosystem_policy.summaries.is_empty() {
                emit_summary_policy_evidence(&session.config, error_format)?;
            }
            render_budget_summary(
                file,
                error_format,
                max_diagnostics,
                visible_region,
                include_budgeted,
            )?;
            Err(super::diagnostics_emitted())
        }
    }
}

fn enforce_configured_new_debt_gate(
    session: &kobo_driver::CompileSession,
    effective_policy: Option<&policy::EffectiveGuaranteePolicy>,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    if !effective_policy.is_some_and(policy::EffectiveGuaranteePolicy::denies_new_debt) {
        return Ok(());
    }
    let Some(diagnostic) = session
        .visible_diagnostics()
        .find(|diagnostic| diagnostic.severity != Severity::Note)
    else {
        return Ok(());
    };
    emit_new_debt_gate_failure(diagnostic.code, error_format)?;
    Err(super::diagnostics_emitted())
}

fn emit_new_debt_gate_failure(code: KErrorCode, error_format: ErrorFormat) -> anyhow::Result<()> {
    let message = format!(
        "ci.release deny_new_debt blocked new guarantee debt reported by {}",
        code.as_str()
    );
    match error_format {
        ErrorFormat::Json => println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "kind": "ci_release_gate",
                "gate": "deny_new_debt",
                "code": code.as_str(),
                "message": message,
            }))?
        ),
        ErrorFormat::Human => eprintln!("error: {message}"),
    }
    Ok(())
}

fn boundary_policy_visibility_enabled(session: &kobo_driver::CompileSession) -> bool {
    session.config.ecosystem_policy.default_is_configured
        || !session.config.ecosystem_policy.crates.is_empty()
        || !session.config.ecosystem_policy.types.is_empty()
        || !session.config.ecosystem_policy.adapters.is_empty()
        || !session.config.ecosystem_policy.summaries.is_empty()
}

fn reject_malformed_scenario_attributes(
    file: &Path,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file)?;
    if !source.contains("kobo::scenario()") {
        return Ok(());
    }
    let message =
        "malformed scenario attribute: use #[kobo::scenario(profile = \"async\")] or remove it";
    match error_format {
        ErrorFormat::Json => println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "code": "K0116",
                "message": message,
            }))?
        ),
        ErrorFormat::Human => eprintln!("error: {message}"),
    }
    anyhow::bail!("{message}")
}

fn reject_invalid_ecosystem_policy(file: &Path, error_format: ErrorFormat) -> anyhow::Result<()> {
    let Some(config_path) = nearest_kobo_config(file) else {
        return Ok(());
    };
    let source = std::fs::read_to_string(&config_path)?;
    let parsed: toml::Value = source.parse()?;
    let Some(ecosystem) = parsed.get("ecosystem").and_then(toml::Value::as_table) else {
        return Ok(());
    };
    for (key, value) in [
        ("ecosystem.default", ecosystem.get("default")),
        ("ecosystem.replay_unknown", ecosystem.get("replay_unknown")),
    ] {
        if let Some(value) = value.and_then(toml::Value::as_str) {
            reject_invalid_boundary_policy_value(&config_path, key, value, error_format)?;
        }
    }
    if let Some(crates) = ecosystem.get("crate").and_then(toml::Value::as_array) {
        for crate_policy in crates {
            if let Some(value) = crate_policy.get("policy").and_then(toml::Value::as_str) {
                reject_invalid_boundary_policy_value(
                    &config_path,
                    "ecosystem.crate.policy",
                    value,
                    error_format,
                )?;
            }
        }
    }
    Ok(())
}

fn reject_invalid_boundary_policy_value(
    config_path: &Path,
    key: &str,
    value: &str,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    let policy = ScenarioBoundaryPolicy::from_str(value);
    if !matches!(policy, ScenarioBoundaryPolicy::Unselected) || value == "unselected" {
        return Ok(());
    }
    let message = format!("unknown ecosystem boundary policy `{value}` at {key}");
    match error_format {
        ErrorFormat::Json => println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "code": "K0120",
                "message": message,
                "path": config_path.display().to_string(),
                "key": key,
                "value": value,
            }))?
        ),
        ErrorFormat::Human => eprintln!("error[K0120]: {message} in {}", config_path.display()),
    }
    anyhow::bail!("K0120 ecosystem policy parse error")
}

fn nearest_kobo_config(file: &Path) -> Option<PathBuf> {
    let start = file.parent()?;
    for ancestor in start.ancestors() {
        let candidate = ancestor.join("Kobo.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn has_replay_blocking_diagnostic(session: &kobo_driver::CompileSession) -> bool {
    session.visible_diagnostics().any(|diagnostic| {
        diagnostic.severity == Severity::Error
            || matches!(diagnostic.code, KErrorCode::K0102 | KErrorCode::K0103)
    })
}

fn reject_invalid_field_capability_views(
    file: &Path,
    error_format: ErrorFormat,
    color_mode: ColorMode,
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
                    color_mode,
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
                    color_mode,
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
    color_mode: ColorMode,
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
            let color = super::session::resolve_color_mode(color_mode);
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
) -> anyhow::Result<Vec<BoundaryPolicyEvidence>> {
    let source = std::fs::read_to_string(file)?;
    let boundary = parse_boundary_attr(&source);
    let mut boundary_calls = external_replay_boundaries(&source, &session.config);
    if boundary_calls.is_empty() {
        if let Some(boundary_call) = boundary
            .as_ref()
            .and_then(BoundaryPolicy::as_replay_boundary)
        {
            boundary_calls.push(boundary_call);
        }
    }
    if boundary_calls.is_empty() {
        return Ok(Vec::new());
    }

    let Some((file_id, _)) = session.file_set().iter_files().next() else {
        return Ok(Vec::new());
    };
    let mut evidence = Vec::new();
    for boundary_call in boundary_calls {
        let package_identity =
            package_identity_for(&session.config, file, &boundary_call.crate_name);
        let span = kobo_ir::KoboSpan::new(
            boundary_call.span_start as u32,
            boundary_call.span_end as u32,
            file_id,
        );

        let Some(decision) = boundary_policy_decision(
            &session.config.ecosystem_policy,
            boundary.as_ref(),
            &boundary_call,
        ) else {
            push_boundary_prompt(
                session,
                span,
                format!(
                    "unmodeled external boundary `{}`; choose typed, model, record, activity, stub, outside, opaque, or debt",
                    boundary_call.crate_name
                ),
            );
            continue;
        };

        if matches!(decision.source, BoundaryPolicySource::Source) && decision.reason.is_none() {
            push_boundary_prompt(
                session,
                span,
                format!(
                    "boundary policy `{}` for `{}` requires a reason",
                    decision.policy.as_str(),
                    boundary_call.crate_name
                ),
            );
            continue;
        }

        let reason = decision
            .reason
            .clone()
            .unwrap_or_else(|| match decision.source {
                BoundaryPolicySource::ProjectCrate => "project ecosystem crate policy".to_owned(),
                BoundaryPolicySource::ProjectDefault => {
                    "project ecosystem default policy".to_owned()
                }
                BoundaryPolicySource::Source => "source boundary policy".to_owned(),
            });

        let severity = resolve_severity(KErrorCode::K0108, session.guarantee_policy())
            .unwrap_or(Severity::Warning);
        session.diagnostics.push(
            KDiagnostic::new(
                KErrorCode::K0108,
                severity,
                DiagLabel::primary(
                    span,
                    format!(
                        "boundary policy `{}` recorded for {}",
                        decision.policy.as_str(),
                        boundary_call.crate_name
                    ),
                ),
                format!(
                    "boundary policy `{}` recorded for `{}`: {reason}",
                    decision.policy.as_str(),
                    boundary_call.crate_name
                ),
                DiagDecision("review this policy before claiming exact replay".to_owned()),
            )
            .with_suppression(DiagnosticSuppression::new(span, reason.clone())),
        );

        let declaration = match declarations::load_declaration_with_config(
            file,
            &boundary_call.crate_name,
            package_identity.version.as_deref(),
            &session.config,
        ) {
            DeclarationLookup::Valid(info) => {
                if decision.policy == ScenarioBoundaryPolicy::Typed
                    && !declarations::declaration_covers_call(
                        &info,
                        boundary_call.call_path.as_deref(),
                    )
                {
                    session.diagnostics.push(KDiagnostic::new(
                        KErrorCode::K0121,
                        Severity::Error,
                        DiagLabel::primary(
                            span,
                            format!(
                                "typed declaration for `{}` does not cover boundary call",
                                boundary_call.crate_name
                            ),
                        ),
                        format!(
                            "{} does not declare coverage for `{}`",
                            info.path.display(),
                            boundary_call
                                .call_path
                                .as_deref()
                                .unwrap_or(boundary_call.crate_name.as_str())
                        ),
                        DiagDecision(
                            "add function/type/adapter metadata for this call or choose record/activity/opaque"
                                .to_owned(),
                        ),
                    ));
                }
                if decision.policy == ScenarioBoundaryPolicy::Typed
                    && declarations::typed_call_has_replay_critical_effects(
                        &info,
                        boundary_call.call_path.as_deref(),
                    )
                {
                    session.diagnostics.push(KDiagnostic::new(
                        KErrorCode::K0121,
                        Severity::Error,
                        DiagLabel::primary(
                            span,
                            format!(
                                "typed declaration for `{}` still has replay-critical effects",
                                boundary_call.crate_name
                            ),
                        ),
                        format!(
                            "{} declares replay-critical effects for `{}`; typed exact replay requires no remaining replay-critical effects",
                            info.path.display(),
                            boundary_call
                                .call_path
                                .as_deref()
                                .unwrap_or(boundary_call.crate_name.as_str())
                        ),
                        DiagDecision(
                            "change this boundary to record/activity/model or mark the function pure/deterministic with no effects"
                                .to_owned(),
                        ),
                    ));
                }
                Some(info)
            }
            DeclarationLookup::Missing if decision.policy == ScenarioBoundaryPolicy::Typed => {
                session.diagnostics.push(KDiagnostic::new(
                    KErrorCode::K0122,
                    Severity::Error,
                    DiagLabel::primary(
                        span,
                        format!(
                            "typed boundary for `{}` has no declaration file",
                            boundary_call.crate_name
                        ),
                    ),
                    format!(
                        "`{}` is marked typed, but Kobo could not find a matching kobo.d.toml declaration",
                        boundary_call.crate_name
                    ),
                    DiagDecision(
                        "add a declaration file or change this boundary to record, activity, opaque, or debt"
                            .to_owned(),
                    ),
                ));
                None
            }
            DeclarationLookup::Invalid(error)
                if decision.policy == ScenarioBoundaryPolicy::Typed =>
            {
                session.diagnostics.push(KDiagnostic::new(
                    KErrorCode::K0121,
                    Severity::Error,
                    DiagLabel::primary(
                        span,
                        format!(
                            "invalid declaration metadata for `{}`",
                            boundary_call.crate_name
                        ),
                    ),
                    format!(
                        "{} key `{}` is invalid for `{}`: {}",
                        error.path.display(),
                        error.key,
                        boundary_call.crate_name,
                        error.message
                    ),
                    DiagDecision(
                        "fix the declaration file before using typed ecosystem policy".to_owned(),
                    ),
                ));
                None
            }
            DeclarationLookup::Invalid(_) | DeclarationLookup::Missing => None,
        };
        if decision.policy == ScenarioBoundaryPolicy::Activity
            && !declarations::activity_covers_call(
                declaration.as_ref(),
                boundary_call.call_path.as_deref(),
            )
        {
            session.diagnostics.push(KDiagnostic::new(
                KErrorCode::K0125,
                Severity::Warning,
                DiagLabel::primary(
                    span,
                    format!(
                        "activity boundary for `{}` is missing retry metadata",
                        boundary_call.crate_name
                    ),
                ),
                format!(
                    "`{}` activity metadata should include retry, idempotency, recorded result, and compensation information",
                    boundary_call.crate_name
                ),
                DiagDecision(
                    "add retry/idempotency/result/compensation metadata before using activity evidence for replay review".to_owned(),
                ),
            ));
        }
        let adapter_package = session
            .config
            .ecosystem_policy
            .adapter_for(&boundary_call.crate_name);
        if decision.policy == ScenarioBoundaryPolicy::Model && adapter_package.is_none() {
            session.diagnostics.push(KDiagnostic::new(
                KErrorCode::K0123,
                Severity::Error,
                DiagLabel::primary(
                    span,
                    format!(
                        "model boundary for `{}` has no adapter package",
                        boundary_call.crate_name
                    ),
                ),
                format!(
                    "`{}` is marked model, but no [[ecosystem.adapter]] package was configured",
                    boundary_call.crate_name
                ),
                DiagDecision(
                    "install an adapter package, change the boundary to record/activity/opaque, or accept debt"
                        .to_owned(),
                ),
            ));
        }
        if decision.policy == ScenarioBoundaryPolicy::Model {
            if let Some(adapter) = adapter_package {
                if let Err(message) = super::ecosystem::validate_model_adapter_package(adapter) {
                    session.diagnostics.push(KDiagnostic::new(
                        KErrorCode::K0123,
                        Severity::Error,
                        DiagLabel::primary(
                            span,
                            format!(
                                "model adapter package for `{}` failed validation",
                                boundary_call.crate_name
                            ),
                        ),
                        message,
                        DiagDecision(
                            "install a registry-validated adapter package or downgrade the boundary policy"
                                .to_owned(),
                        ),
                    ));
                }
            }
        }

        evidence.push(BoundaryPolicyEvidence {
            crate_name: boundary_call.crate_name,
            policy: decision.policy,
            reason: decision.reason,
            source: decision.source,
            package_identity,
            declaration,
            span_start: boundary_call.span_start,
            span_end: boundary_call.span_end,
            call_path: boundary_call.call_path,
            adapter_package: adapter_package.map(|adapter| adapter.package.clone()),
        });
    }

    Ok(evidence)
}

fn emit_boundary_policy_evidence(
    evidence: &[BoundaryPolicyEvidence],
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    for entry in evidence {
        match error_format {
            ErrorFormat::Json => println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "category": "boundary-policy-evidence",
                    "crate": entry.crate_name,
                    "policy": entry.policy.as_str(),
                    "reason": entry.reason,
                    "source": entry.source.as_str(),
                    "call_path": entry.call_path.clone(),
                    "package_name": entry.package_identity.name,
                    "package_version": entry.package_identity.version,
                    "package_source": entry.package_identity.source,
                    "package_id": entry.package_identity.package_id,
                    "package_identity_source": entry.package_identity.identity_source,
                    "features": entry.package_identity.features,
                    "declaration_path": entry.declaration.as_ref().map(|info| info.path.display().to_string()),
                    "declaration_version": entry.declaration.as_ref().map(|info| info.version.clone()),
                    "declaration_hash": entry.declaration.as_ref().map(|info| info.hash.clone()),
                    "declaration_metadata_package": entry.declaration.as_ref().and_then(|info| {
                        info.metadata_package.as_ref().map(|package| serde_json::json!({
                            "package": package.package.clone(),
                            "version": package.version.clone(),
                            "path": package.path.display().to_string(),
                            "source": package.source.clone(),
                            "registry": package.registry.clone(),
                            "checksum": package.checksum.clone(),
                            "signed_by": package.signed_by.clone(),
                            "validated": package.validated,
                        }))
                    }),
                    "declaration": entry.declaration.as_ref().map(|info| serde_json::json!({
                        "path": info.path.display().to_string(),
                        "version": info.version.clone(),
                        "schema_version": info.schema_version,
                        "hash": info.hash.clone(),
                    })),
                    "declaration_types": entry.declaration.as_ref().map(|info| info.types.clone()).unwrap_or_default(),
                    "declaration_type_facts": entry.declaration.as_ref().map(|info| {
                        info.type_facts.iter().map(|fact| serde_json::json!({
                            "path": fact.path.clone(),
                            "kind": fact.kind.clone(),
                            "resource": fact.resource,
                            "must_call": fact.must_call.clone(),
                            "ownership": fact.ownership.clone(),
                            "strict_ward": fact.strict_ward.clone(),
                        })).collect::<Vec<_>>()
                    }).unwrap_or_default(),
                    "declaration_obligations": entry.declaration.as_ref().map(|info| {
                        info.type_facts.iter().filter(|fact| fact.resource || !fact.must_call.is_empty()).map(|fact| serde_json::json!({
                            "type": fact.path.clone(),
                            "must_call": fact.must_call.clone(),
                            "resource": fact.resource,
                        })).collect::<Vec<_>>()
                    }).unwrap_or_default(),
                    "declaration_functions": entry.declaration.as_ref().map(|info| info.functions.clone()).unwrap_or_default(),
                    "declaration_function_facts": entry.declaration.as_ref().map(|info| {
                        info.function_facts.iter().map(|fact| serde_json::json!({
                            "path": fact.path.clone(),
                            "effects": fact.effects.clone(),
                            "simulation": fact.simulation.clone(),
                            "determinism": fact.determinism.clone(),
                            "returns": fact.returns.clone(),
                            "obligation": fact.obligation.clone(),
                            "strict_ward": fact.strict_ward.clone(),
                        })).collect::<Vec<_>>()
                    }).unwrap_or_default(),
                    "declaration_effects": entry.declaration.as_ref().map(|info| info.effects.clone()).unwrap_or_default(),
                    "declaration_adapters": entry.declaration.as_ref().map(|info| info.adapters.clone()).unwrap_or_default(),
                    "activity_call_covered": declarations::activity_covers_call(
                        entry.declaration.as_ref(),
                        entry.call_path.as_deref(),
                    ),
                    "adapter": entry.adapter_package.as_ref().map(|package| serde_json::json!({
                        "package": package,
                    })),
                    "span": {
                        "byte_start": entry.span_start,
                        "byte_end": entry.span_end,
                    },
                }))?
            ),
            ErrorFormat::Human => eprintln!(
                "boundary policy: crate={} policy={} source={}{}",
                entry.crate_name,
                entry.policy.as_str(),
                entry.source.as_str(),
                entry
                    .reason
                    .as_deref()
                    .map(|reason| format!(" reason={reason}"))
                    .unwrap_or_default()
            ),
        }
    }
    Ok(())
}

fn emit_summary_policy_evidence(
    config: &kobo_driver::KoboConfig,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    for summary in &config.ecosystem_policy.summaries {
        let valid = summary_validation::load_valid_summary(summary)?;
        let obligations = valid
            .value
            .get("obligations")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let functions = valid
            .value
            .get("functions")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let solver_metadata = valid
            .value
            .get("solver_metadata")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({"engine": "unknown", "outcome": "missing"}));
        match error_format {
            ErrorFormat::Json => println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "category": "summary-policy-evidence",
                    "crate": summary.crate_name,
                    "path": summary.path.display().to_string(),
                    "summary_hash": valid.hash,
                    "schema_version": valid.schema_version,
                    "solver_metadata": solver_metadata,
                    "obligation_count": obligations.len(),
                    "function_count": functions.len(),
                    "obligations": obligations,
                    "functions": functions,
                }))?
            ),
            ErrorFormat::Human => eprintln!(
                "summary policy: crate={} path={} hash={}",
                summary.crate_name,
                summary.path.display(),
                valid.hash
            ),
        }
    }
    Ok(())
}

#[derive(Clone)]
struct BoundaryPolicyDecision {
    policy: ScenarioBoundaryPolicy,
    reason: Option<String>,
    source: BoundaryPolicySource,
}

#[derive(Clone)]
struct BoundaryPolicyEvidence {
    crate_name: String,
    policy: ScenarioBoundaryPolicy,
    reason: Option<String>,
    source: BoundaryPolicySource,
    package_identity: PackageIdentity,
    declaration: Option<DeclarationFacts>,
    span_start: usize,
    span_end: usize,
    call_path: Option<String>,
    adapter_package: Option<String>,
}

#[derive(Copy, Clone)]
enum BoundaryPolicySource {
    Source,
    ProjectCrate,
    ProjectDefault,
}

impl BoundaryPolicySource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::ProjectCrate => "project-crate",
            Self::ProjectDefault => "project-default",
        }
    }
}

fn boundary_policy_decision(
    ecosystem: &kobo_driver::EcosystemPolicyConfig,
    boundary: Option<&BoundaryPolicy>,
    boundary_call: &ExternalReplayBoundary,
) -> Option<BoundaryPolicyDecision> {
    if let Some(BoundaryPolicy {
        crate_name,
        policy,
        reason,
        ..
    }) = boundary
    {
        if crate_name
            .as_deref()
            .map_or(true, |name| name == boundary_call.crate_name.as_str())
        {
            return Some(BoundaryPolicyDecision {
                policy: ScenarioBoundaryPolicy::from_str(policy),
                reason: reason.clone(),
                source: BoundaryPolicySource::Source,
            });
        }
    }

    if let Some(crate_policy) = ecosystem.crate_policy(&boundary_call.crate_name) {
        return Some(BoundaryPolicyDecision {
            policy: crate_policy.policy.clone(),
            reason: crate_policy.reason.clone(),
            source: BoundaryPolicySource::ProjectCrate,
        });
    }

    ecosystem
        .default_is_configured
        .then(|| BoundaryPolicyDecision {
            policy: ecosystem.default.clone(),
            reason: None,
            source: BoundaryPolicySource::ProjectDefault,
        })
}

#[derive(Clone)]
struct PackageIdentity {
    name: String,
    version: Option<String>,
    source: String,
    features: Vec<String>,
    package_id: Option<String>,
    identity_source: &'static str,
}

fn package_identity_for(
    config: &kobo_driver::KoboConfig,
    file: &Path,
    crate_name: &str,
) -> PackageIdentity {
    if let Some(identity) = cargo_metadata_identity(file, crate_name) {
        return identity;
    }
    let Some(value) = config.dependencies.get(crate_name) else {
        return PackageIdentity {
            name: crate_name.to_owned(),
            version: None,
            source: "unknown".to_owned(),
            features: Vec::new(),
            package_id: None,
            identity_source: "manifest-fallback",
        };
    };

    match value {
        toml::Value::String(version) => PackageIdentity {
            name: crate_name.to_owned(),
            version: Some(version.clone()),
            source: "registry".to_owned(),
            features: Vec::new(),
            package_id: None,
            identity_source: "manifest-fallback",
        },
        toml::Value::Table(table) => {
            let name = table
                .get("package")
                .and_then(toml::Value::as_str)
                .unwrap_or(crate_name)
                .to_owned();
            let source = if table.contains_key("path") {
                "path"
            } else if table.contains_key("git") {
                "git"
            } else {
                "registry"
            }
            .to_owned();
            let version = if source == "path" {
                table
                    .get("path")
                    .and_then(toml::Value::as_str)
                    .and_then(|path| path_dependency_version(file, path))
                    .or_else(|| {
                        table
                            .get("version")
                            .and_then(toml::Value::as_str)
                            .map(str::to_owned)
                    })
            } else {
                table
                    .get("version")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned)
            };
            let features = table
                .get("features")
                .and_then(toml::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(toml::Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            PackageIdentity {
                name,
                version,
                source,
                features,
                package_id: None,
                identity_source: "manifest-fallback",
            }
        }
        _ => PackageIdentity {
            name: crate_name.to_owned(),
            version: None,
            source: "unknown".to_owned(),
            features: Vec::new(),
            package_id: None,
            identity_source: "manifest-fallback",
        },
    }
}

fn cargo_metadata_identity(file: &Path, crate_name: &str) -> Option<PackageIdentity> {
    let manifest_dir = nearest_manifest_dir(file)?;
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--offline"])
        .current_dir(&manifest_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let metadata = serde_json::from_slice::<serde_json::Value>(&output.stdout).ok()?;
    let packages = metadata
        .get("packages")
        .and_then(serde_json::Value::as_array)?;
    let package_by_id = packages
        .iter()
        .filter_map(|package| {
            let id = package.get("id")?.as_str()?;
            Some((id.to_owned(), package))
        })
        .collect::<BTreeMap<_, _>>();
    let root_id = metadata
        .get("resolve")
        .and_then(|resolve| resolve.get("root"))
        .and_then(serde_json::Value::as_str);
    let mut feature_map = BTreeMap::<String, Vec<String>>::new();
    let mut dependency_package_id = None;
    if let Some(nodes) = metadata
        .get("resolve")
        .and_then(|resolve| resolve.get("nodes"))
        .and_then(serde_json::Value::as_array)
    {
        for node in nodes {
            let Some(id) = node.get("id").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let features = node
                .get("features")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            feature_map.insert(id.to_owned(), features);
            if root_id == Some(id) {
                if let Some(deps) = node.get("deps").and_then(serde_json::Value::as_array) {
                    for dep in deps {
                        let Some(dep_name) = dep.get("name").and_then(serde_json::Value::as_str)
                        else {
                            continue;
                        };
                        let Some(pkg_id) = dep.get("pkg").and_then(serde_json::Value::as_str)
                        else {
                            continue;
                        };
                        let package_name = package_by_id
                            .get(pkg_id)
                            .and_then(|package| package.get("name"))
                            .and_then(serde_json::Value::as_str);
                        if dep_name == crate_name || package_name == Some(crate_name) {
                            dependency_package_id = Some(pkg_id.to_owned());
                            break;
                        }
                    }
                }
            }
        }
    }
    packages
        .iter()
        .filter(|package| {
            dependency_package_id.as_deref().map_or(true, |id| {
                package.get("id").and_then(serde_json::Value::as_str) == Some(id)
            })
        })
        .find_map(|package| {
            let name = package.get("name")?.as_str()?;
            if dependency_package_id.is_none() && name != crate_name {
                return None;
            }
            let id = package.get("id")?.as_str()?.to_owned();
            Some(PackageIdentity {
                name: name.to_owned(),
                version: package
                    .get("version")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                source: package
                    .get("source")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| "workspace-or-path".to_owned()),
                features: feature_map.remove(&id).unwrap_or_default(),
                package_id: Some(id),
                identity_source: "cargo-metadata",
            })
        })
}

fn path_dependency_version(file: &Path, dependency_path: &str) -> Option<String> {
    let manifest_dir = nearest_manifest_dir(file)?;
    let dependency_dir = if Path::new(dependency_path).is_absolute() {
        PathBuf::from(dependency_path)
    } else {
        manifest_dir.join(dependency_path)
    };
    let manifest = std::fs::read_to_string(dependency_dir.join("Cargo.toml")).ok()?;
    let parsed = manifest.parse::<toml::Value>().ok()?;
    parsed
        .get("package")?
        .get("version")
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
}

fn nearest_manifest_dir(file: &Path) -> Option<PathBuf> {
    let start = file.parent()?;
    for ancestor in start.ancestors() {
        if ancestor.join("Cargo.toml").is_file() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

fn project_contextual_suggestions(
    session: &mut kobo_driver::CompileSession,
    file: &Path,
) -> anyhow::Result<()> {
    if !(session.guarantee_policy().is_checked() || session.guarantee_policy().is_release()) {
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
            let severity = resolve_severity(KErrorCode::K0061, session.guarantee_policy())
                .unwrap_or(Severity::Warning);
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
            let severity = resolve_severity(KErrorCode::K0100, session.guarantee_policy())
                .unwrap_or(Severity::Warning);
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
    let severity = resolve_severity(KErrorCode::K0061, session.guarantee_policy())
        .unwrap_or(Severity::Warning);
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
    let severity = resolve_severity(KErrorCode::K0107, session.guarantee_policy())
        .unwrap_or(Severity::Warning);
    session.diagnostics.push(KDiagnostic::new(
        KErrorCode::K0107,
        severity,
        DiagLabel::primary(span, message.clone()),
        format!(
            "{message}. Available policies: typed, model, record, activity, stub, outside, opaque, debt."
        ),
        DiagDecision("select an explicit boundary policy for replay-critical evidence".to_owned()),
    ));
}

struct BoundaryPolicy {
    crate_name: Option<String>,
    policy: String,
    reason: Option<String>,
    span_start: usize,
    span_end: usize,
}

struct ExternalReplayBoundary {
    crate_name: String,
    call_path: Option<String>,
    span_start: usize,
    span_end: usize,
}

impl BoundaryPolicy {
    fn as_replay_boundary(&self) -> Option<ExternalReplayBoundary> {
        let crate_name = self.crate_name.clone()?;
        Some(ExternalReplayBoundary {
            crate_name,
            call_path: None,
            span_start: self.span_start,
            span_end: self.span_end,
        })
    }
}

fn external_replay_boundaries(
    source: &str,
    config: &kobo_driver::KoboConfig,
) -> Vec<ExternalReplayBoundary> {
    parsed_external_boundaries(source, config)
}

fn parsed_external_boundaries(
    source: &str,
    config: &kobo_driver::KoboConfig,
) -> Vec<ExternalReplayBoundary> {
    let Ok(parsed) = syn::parse_file(source) else {
        return Vec::new();
    };
    let import_index = ImportIndex::from_file(&parsed);
    let dependency_names = dependency_names(config);
    let mut visitor = ExternalBoundaryVisitor {
        source,
        imports: import_index,
        dependency_names,
        bindings: BTreeMap::new(),
        found: Vec::new(),
    };
    visitor.visit_file(&parsed);
    visitor.found
}

fn dependency_names(config: &kobo_driver::KoboConfig) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    extend_dependency_names(&mut names, &config.dependencies);
    extend_dependency_names(&mut names, &config.dev_dependencies);
    extend_dependency_names(&mut names, &config.build_dependencies);
    extend_dependency_names(&mut names, &config.workspace_dependencies);
    for target in &config.target_dependencies {
        extend_dependency_names(&mut names, &target.dependencies);
        extend_dependency_names(&mut names, &target.dev_dependencies);
        extend_dependency_names(&mut names, &target.build_dependencies);
    }
    names.extend(
        config
            .ecosystem_policy
            .crates
            .iter()
            .map(|policy| policy.name.clone()),
    );
    names.extend(
        config
            .ecosystem_policy
            .types
            .iter()
            .map(|policy| policy.crate_name.clone()),
    );
    names.extend(
        config
            .ecosystem_policy
            .adapters
            .iter()
            .map(|policy| policy.crate_name.clone()),
    );
    names.extend(
        config
            .ecosystem_policy
            .summaries
            .iter()
            .map(|policy| policy.crate_name.clone()),
    );
    names
}

fn extend_dependency_names(
    names: &mut BTreeSet<String>,
    dependencies: &std::collections::HashMap<String, toml::Value>,
) {
    for (alias, value) in dependencies {
        names.insert(alias.clone());
        if let toml::Value::Table(table) = value {
            if let Some(package) = table.get("package").and_then(toml::Value::as_str) {
                names.insert(package.to_owned());
            }
        }
    }
}

#[derive(Default)]
struct ImportIndex {
    aliases: BTreeMap<String, String>,
    paths: BTreeMap<String, Vec<String>>,
}

impl ImportIndex {
    fn from_file(file: &syn::File) -> Self {
        let mut index = Self::default();
        for item in &file.items {
            if let syn::Item::Use(item_use) = item {
                collect_use_tree(&item_use.tree, Vec::new(), &mut index);
            }
        }
        index
    }

    fn crate_for_ident(&self, ident: &str) -> Option<&str> {
        self.aliases.get(ident).map(String::as_str)
    }

    fn path_for_ident(&self, ident: &str) -> Option<&[String]> {
        self.paths.get(ident).map(Vec::as_slice)
    }
}

fn collect_use_tree(tree: &syn::UseTree, prefix: Vec<String>, index: &mut ImportIndex) {
    match tree {
        syn::UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            if let Some(root) = next.first() {
                index.aliases.insert(path.ident.to_string(), root.clone());
                index.paths.insert(path.ident.to_string(), next.clone());
            }
            collect_use_tree(&path.tree, next, index);
        }
        syn::UseTree::Name(name) => {
            let mut full = prefix;
            full.push(name.ident.to_string());
            if let Some(root) = full.first() {
                index.aliases.insert(name.ident.to_string(), root.clone());
                index.paths.insert(name.ident.to_string(), full);
            }
        }
        syn::UseTree::Rename(rename) => {
            let mut full = prefix;
            full.push(rename.ident.to_string());
            if let Some(root) = full.first() {
                index
                    .aliases
                    .insert(rename.rename.to_string(), root.clone());
                index.paths.insert(rename.rename.to_string(), full);
            }
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_use_tree(item, prefix.clone(), index);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

struct ExternalBoundaryVisitor<'a> {
    source: &'a str,
    imports: ImportIndex,
    dependency_names: BTreeSet<String>,
    bindings: BTreeMap<String, Vec<String>>,
    found: Vec<ExternalReplayBoundary>,
}

impl<'ast> Visit<'ast> for ExternalBoundaryVisitor<'_> {
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = node.func.as_ref() {
            let segments = path
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>();
            if let Some((crate_name, call_path)) = self.resolve_call_path(&segments) {
                let needle = call_path
                    .as_deref()
                    .and_then(|path| self.source.find(path).map(|_| path.to_owned()))
                    .unwrap_or_else(|| segments.join("::"));
                let (span_start, span_end) = span_offsets(self.source, path.path.span())
                    .unwrap_or_else(|| fallback_span(self.source, &needle, &crate_name));
                self.push_boundary(ExternalReplayBoundary {
                    span_start,
                    span_end,
                    crate_name,
                    call_path,
                });
            }
        }
        syn::visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if let Some((crate_name, call_path)) = self.resolve_method_call_path(node) {
            let needle = call_path.clone().unwrap_or_else(|| crate_name.clone());
            let (span_start, span_end) = span_offsets(self.source, node.span())
                .unwrap_or_else(|| fallback_span(self.source, &needle, &crate_name));
            self.push_boundary(ExternalReplayBoundary {
                span_start,
                span_end,
                crate_name,
                call_path,
            });
            for arg in &node.args {
                self.visit_expr(arg);
            }
            return;
        }
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        if let Some((name, path)) = self.binding_for_local(node) {
            self.bindings.insert(name, path);
        }
        syn::visit::visit_local(self, node);
    }
}

impl ExternalBoundaryVisitor<'_> {
    fn push_boundary(&mut self, boundary: ExternalReplayBoundary) {
        if self.found.iter().any(|existing| {
            existing.crate_name == boundary.crate_name
                && existing.call_path == boundary.call_path
                && existing.span_start == boundary.span_start
                && existing.span_end == boundary.span_end
        }) {
            return;
        }
        self.found.push(boundary);
    }

    fn resolve_call_path(&self, segments: &[String]) -> Option<(String, Option<String>)> {
        let resolved = self.resolve_segments(segments)?;
        let crate_name = resolved.first()?.clone();
        Some((crate_name, Some(resolved.join("::"))))
    }

    fn resolve_method_call_path(
        &self,
        node: &syn::ExprMethodCall,
    ) -> Option<(String, Option<String>)> {
        let receiver_path = match node.receiver.as_ref() {
            syn::Expr::Path(path) => {
                let segments = path
                    .path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .collect::<Vec<_>>();
                if segments.len() == 1 {
                    self.bindings.get(&segments[0]).cloned().unwrap_or(segments)
                } else {
                    segments
                }
            }
            syn::Expr::Call(call) => {
                let syn::Expr::Path(path) = call.func.as_ref() else {
                    return None;
                };
                let mut segments = path
                    .path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .collect::<Vec<_>>();
                segments.pop();
                segments
            }
            _ => return None,
        };
        let mut resolved = self.resolve_segments(&receiver_path)?;
        resolved.push(node.method.to_string());
        let crate_name = resolved.first()?.clone();
        Some((crate_name, Some(resolved.join("::"))))
    }

    fn binding_for_local(&self, node: &syn::Local) -> Option<(String, Vec<String>)> {
        let syn::Pat::Ident(binding) = &node.pat else {
            return None;
        };
        let init = node.init.as_ref()?;
        let syn::Expr::Call(call) = init.expr.as_ref() else {
            return None;
        };
        let syn::Expr::Path(path) = call.func.as_ref() else {
            return None;
        };
        let segments = path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        let mut resolved = self.resolve_segments(&segments)?;
        resolved.pop();
        Some((binding.ident.to_string(), resolved))
    }

    fn resolve_segments(&self, segments: &[String]) -> Option<Vec<String>> {
        let first = segments.first()?;
        if self.dependency_names.contains(first) {
            return Some(segments.to_vec());
        }
        if let Some(path) = self.imports.path_for_ident(first) {
            let mut resolved = path.to_vec();
            resolved.extend(segments.iter().skip(1).cloned());
            if let Some(crate_name) = resolved.first() {
                if self.dependency_names.is_empty() || self.dependency_names.contains(crate_name) {
                    return Some(resolved);
                }
            }
        }
        if let Some(crate_name) = self.imports.crate_for_ident(first) {
            if self.dependency_names.is_empty() || self.dependency_names.contains(crate_name) {
                let mut resolved = vec![crate_name.to_owned()];
                resolved.extend(segments.iter().skip(1).cloned());
                return Some(resolved);
            }
        }
        None
    }
}

fn span_offsets(source: &str, span: proc_macro2::Span) -> Option<(usize, usize)> {
    let start = byte_offset_for_line_column(source, span.start().line, span.start().column)?;
    let end = byte_offset_for_line_column(source, span.end().line, span.end().column)?;
    Some((start, end.max(start + 1)))
}

fn byte_offset_for_line_column(source: &str, target_line: usize, column: usize) -> Option<usize> {
    if target_line == 0 {
        return None;
    }
    let mut offset = 0usize;
    for (line_index, line) in source.split_inclusive('\n').enumerate() {
        if line_index + 1 == target_line {
            return Some(offset + column.min(line.len()));
        }
        offset += line.len();
    }
    (target_line == source.lines().count() + 1).then_some(offset)
}

fn fallback_span(source: &str, needle: &str, crate_name: &str) -> (usize, usize) {
    let start = source
        .find(needle)
        .or_else(|| source.find(crate_name))
        .unwrap_or(0);
    let width = if source
        .get(start..)
        .is_some_and(|tail| tail.starts_with(needle))
    {
        needle.len()
    } else {
        crate_name.len()
    };
    (start, start + width.max(1))
}

fn parse_boundary_attr(source: &str) -> Option<BoundaryPolicy> {
    let parsed = syn::parse_file(source).ok()?;
    for item in &parsed.items {
        for attr in boundary_candidate_attrs(item) {
            if let Some(boundary) = parse_boundary_attribute(attr, source) {
                return Some(boundary);
            }
        }
    }
    None
}

fn boundary_candidate_attrs(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Fn(item) => &item.attrs,
        syn::Item::Impl(item) => &item.attrs,
        syn::Item::Mod(item) => &item.attrs,
        syn::Item::Struct(item) => &item.attrs,
        syn::Item::Trait(item) => &item.attrs,
        syn::Item::Use(item) => &item.attrs,
        _ => &[],
    }
}

fn parse_boundary_attribute(attr: &syn::Attribute, source: &str) -> Option<BoundaryPolicy> {
    let default_policy = if syn_path_ends_with(attr.path(), &["kobo", "boundary"]) {
        None
    } else if syn_path_ends_with(attr.path(), &["kobo", "record"]) {
        Some("record")
    } else if syn_path_ends_with(attr.path(), &["kobo", "activity"]) {
        Some("activity")
    } else {
        return None;
    };
    let syn::Meta::List(list) = &attr.meta else {
        return None;
    };
    let entries = list
        .parse_args_with(
            syn::punctuated::Punctuated::<syn::MetaNameValue, syn::Token![,]>::parse_terminated,
        )
        .ok()?;
    let mut crate_name = None;
    let mut policy = None;
    let mut reason = None;
    for entry in entries {
        let Some(key) = entry
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
        else {
            continue;
        };
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(value),
            ..
        }) = entry.value
        else {
            continue;
        };
        match key.as_str() {
            "crate" => crate_name = Some(value.value()),
            "policy" => policy = Some(value.value()),
            "reason" => reason = Some(value.value()),
            _ => {}
        }
    }
    let (span_start, span_end) = span_offsets(source, attr.span())
        .unwrap_or((0, crate_name.as_ref().map(String::len).unwrap_or(1)));
    Some(BoundaryPolicy {
        crate_name,
        policy: policy
            .or_else(|| default_policy.map(str::to_owned))
            .unwrap_or_else(|| "opaque".to_owned()),
        reason,
        span_start,
        span_end,
    })
}

fn syn_path_ends_with(path: &syn::Path, suffix: &[&str]) -> bool {
    let segments = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    segments.len() >= suffix.len()
        && segments[segments.len() - suffix.len()..]
            .iter()
            .zip(suffix)
            .all(|(left, right)| left == right)
}
