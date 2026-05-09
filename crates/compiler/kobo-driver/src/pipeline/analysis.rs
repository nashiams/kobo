use std::path::Path;

use kobo_analysis::{
    analyze_send_violations, facts_to_diagnostics, run_analysis, scan_source_cancel_safety,
    scan_source_handler_leaks, SpawnSite as AnalysisSpawnSite,
};
use kobo_errors::{resolve_severity, DiagDecision, DiagLabel, KDiagnostic, KErrorCode, Severity};
use kobo_ir::{AsyncViolationKind, Kir, RelaxAttrError, UseEvent, WarnEarlyPattern};
use kobo_transform::{
    strict_async::check_strict_async, strict_async::guard_liveness::detect_guard_across_await,
};

use crate::session::CompileSession;

use super::parse::run_kir_phase;

pub fn run_check_pipeline(session: &mut CompileSession, input: &Path) -> Result<(), ()> {
    let (_, kir) = run_kir_phase(session, input)?;
    run_analysis_phase(session, &kir)
}

/// S-14: Run pipeline ordering heuristic on the parsed file.
///
/// Parses the file, extracts middleware call sites, and checks for common
/// ordering mistakes (auth-after-handler, log-after-response, etc.).
pub fn run_pipeline_ordering_check(
    session: &mut CompileSession,
    input: &Path,
) -> Vec<kobo_analysis::PipelineWarning> {
    let (kobo_file, _kir) = match run_kir_phase(session, input) {
        Ok(pair) => pair,
        Err(()) => return Vec::new(),
    };
    kobo_analysis::check_pipeline_ordering(&kobo_file.inner.items)
}

pub(super) fn run_analysis_phase(session: &mut CompileSession, kir: &Kir) -> Result<(), ()> {
    let facts = run_analysis(kir, session.file_set());
    session.diagnostics.extend(facts_to_diagnostics(
        &facts,
        kir.transform_facts(),
        session.file_set(),
        kir,
        session.mode(),
    ));

    // Emit errors for malformed #[kobo::known_debt] attributes (C07).
    // K0025 malformed attribute is always Error — structural constraint [R6-03].
    for def in kir.struct_defs() {
        if let Some(error_msg) = &def.known_debt_parse_error {
            let span = def.known_debt_span.unwrap_or(def.span);
            let severity =
                resolve_severity(KErrorCode::K0025, session.mode()).unwrap_or(Severity::Error);
            session.diagnostics.push(KDiagnostic::new(
                KErrorCode::K0025,
                severity,
                DiagLabel::primary(span, error_msg.clone()),
                error_msg.clone(),
                DiagDecision(String::new()),
            ));
        }
    }

    // Emit diagnostics for #[kobo::relax] attribute validation errors/warnings [G5].
    // K0026 error path: structural validation errors use hardcoded Error (AC-19 exception).
    // K0026 warning path: use resolve_severity for consistency [BUG-06].
    for RelaxAttrError {
        span,
        message,
        is_error,
    } in kir.relax_attr_errors()
    {
        let severity = if *is_error {
            Severity::Error
        } else {
            resolve_severity(KErrorCode::K0026, session.mode()).unwrap_or(Severity::Warning)
        };
        session.diagnostics.push(KDiagnostic::new(
            KErrorCode::K0026,
            severity,
            DiagLabel::primary(*span, message.clone()),
            message.clone(),
            DiagDecision(String::new()),
        ));
    }
    // Warn when #[kobo::relax] is used in script mode (has no effect).
    if session.mode().is_script() && !session.relaxed_fn_ranges.is_empty() {
        // Emit per-relaxed-fn advisory using the fn span itself.
        let severity =
            resolve_severity(KErrorCode::K0026, session.mode()).unwrap_or(Severity::Warning);
        for &fn_span in &session.relaxed_fn_ranges.clone() {
            session.diagnostics.push(KDiagnostic::new(
                KErrorCode::K0026,
                severity,
                DiagLabel::primary(fn_span, "`#[kobo::relax]` has no effect in script mode"),
                "`#[kobo::relax]` has no effect in script mode".to_owned(),
                DiagDecision(String::new()),
            ));
        }
    }

    // Emit K0080-P advisory notes for non-suppressed structural patterns.
    // Contract C08: these are always `note` severity, never `warning` or `error`.
    for fact in kir.warn_early_facts() {
        if fact.suppressed {
            continue;
        }
        let (code, label_text, explanation) = match &fact.pattern {
            WarnEarlyPattern::BidirectionalRcLinks { struct_name, field_pairs } => {
                let pairs_str = field_pairs
                    .iter()
                    .map(|(a, b)| format!("{a}↔{b}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                (
                    KErrorCode::K0080P1,
                    format!("bidirectional Rc links in `{struct_name}`"),
                    format!(
                        "struct `{struct_name}` has Rc<RefCell<T>> links that form a cycle ({pairs_str})\n   \
                         = this will leak memory unless Weak references are used"
                    ),
                )
            }
            WarnEarlyPattern::ParentChildBackPointer { struct_name, children_field, parent_field } => {
                (
                    KErrorCode::K0080P2,
                    format!("parent↔child back-pointer in `{struct_name}`"),
                    format!(
                        "struct `{struct_name}` has `{children_field}` (children) and `{parent_field}` (parent) — Rc cycle\n   \
                         = this will leak unless parent uses Weak references"
                    ),
                )
            }
            WarnEarlyPattern::SharedMutableAt3PlusSites { site_count, .. } => (
                KErrorCode::K0080P3,
                format!("shared mutable state at {site_count} call sites"),
                format!(
                    "the binding is mutated from {site_count} distinct call sites\n   \
                     = migration will require an architectural decision on ownership"
                ),
            ),
            WarnEarlyPattern::SelfReferentialStruct { struct_name } => (
                KErrorCode::K0080P4,
                format!("self-referential struct `{struct_name}` without indirection"),
                format!(
                    "struct `{struct_name}` contains a direct (non-indirected) field of the same type\n   \
                     = this would have infinite size; use Box<{struct_name}> or Rc<RefCell<{struct_name}>>"
                ),
            ),
        };

        // K0080P1-P4 structural advisories are always Note — not mode-dependent [R6-12].
        let advisory_severity = resolve_severity(code, session.mode()).unwrap_or(Severity::Note);
        session.diagnostics.push(KDiagnostic::new(
            code,
            advisory_severity,
            DiagLabel::primary(fact.span, label_text),
            explanation,
            DiagDecision(
                "advisory only — no automatic fix; see `kobo debt` for migration guidance"
                    .to_owned(),
            ),
        ));
    }

    // Phase 11: Emit K006x diagnostics for async ownership violations.
    // BUG-5 fix: detect executor from config dependencies instead of hardcoding true.
    let has_executor = kobo_codegen::executor::select_executor(&session.config.dependencies)
        != kobo_codegen::executor::ExecutorChoice::None;
    let async_violations = check_strict_async(kir, session.mode(), has_executor);
    for violation in &async_violations {
        let (code, label_text, explanation) = match &violation.kind {
            AsyncViolationKind::NonSendCapture { binding_name, .. } => (
                KErrorCode::K0060,
                format!("binding `{binding_name}` is not Send"),
                format!(
                    "binding `{binding_name}` would be wrapped in Rc (not Send) but the async context requires Send\n   \
                     = kobo decision: refused to generate non-Send wrapper in async context"
                ),
            ),
            AsyncViolationKind::NonSyncShared { binding_name, .. } => (
                KErrorCode::K0061,
                format!("binding `{binding_name}` is not Sync for shared access"),
                format!(
                    "binding `{binding_name}` requires Sync for cross-task sharing but the current wrapper is not Sync\n   \
                     = kobo decision: consider restructuring with channels or an actor pattern"
                ),
            ),
            AsyncViolationKind::MissingExecutor => (
                KErrorCode::K0062,
                "no async executor configured".to_owned(),
                "async code detected but no executor (tokio/async-std) found in dependencies\n   \
                 = add tokio or async-std to [dependencies] in Cargo.toml".to_owned(),
            ),
            AsyncViolationKind::StrictAsyncViolation { binding_name, .. } => (
                KErrorCode::K0063,
                format!("strict mode: async wrapping not permitted for `{binding_name}`"),
                format!(
                    "in @strict mode, binding `{binding_name}` cannot use ownership wrappers in async context\n   \
                     = kobo decision: @strict requires zero-cost ownership — no Rc, Arc, or RefCell"
                ),
            ),
        };
        let severity = resolve_severity(code, session.mode()).unwrap_or(Severity::Error);
        session.diagnostics.push(KDiagnostic::new(
            code,
            severity,
            DiagLabel::primary(violation.span, label_text),
            explanation,
            DiagDecision(String::new()),
        ));
    }

    // S-22: Async Send root-cause diagnostics — pinpoint binding + .await causing non-Send.
    // Build synthetic SpawnSites from bindings with needs_send=true.
    {
        let tf = kir.transform_facts();
        let send_bindings: Vec<_> = tf
            .bindings
            .iter()
            .filter(|b| b.shared_facts.needs_send)
            .map(|b| b.node)
            .collect();
        if !send_bindings.is_empty() {
            let synthetic_site = AnalysisSpawnSite {
                span: tf
                    .bindings
                    .iter()
                    .find(|b| b.shared_facts.needs_send)
                    .map(|b| b.span)
                    .unwrap_or(kobo_ir::KoboSpan::new(0, 0, kobo_ir::FileId(0))),
                captured_bindings: send_bindings,
                await_points: Vec::new(),
            };
            let send_diags = analyze_send_violations(&[synthetic_site], tf, kir);
            for diag in &send_diags {
                let severity =
                    resolve_severity(KErrorCode::K0061, session.mode()).unwrap_or(Severity::Error);
                session.diagnostics.push(KDiagnostic::new(
                    KErrorCode::K0061,
                    severity,
                    DiagLabel::primary(
                        diag.spawn_span,
                        format!(
                            "future requires Send but `{}` uses {}",
                            diag.binding_name, diag.wrapper_type
                        ),
                    ),
                    format!(
                        "binding `{}` cannot cross thread boundary — {}\n   = {}",
                        diag.binding_name, diag.wrapper_type, diag.suggestion
                    ),
                    DiagDecision(diag.suggestion.clone()),
                ));
            }
        }
    }

    // S-54: K0064 GuardHeldAcrossAwait — detect guard bindings live across .await points.
    {
        let guard_violations = detect_guard_across_await(kir);
        for gv in &guard_violations {
            let severity =
                resolve_severity(KErrorCode::K0064, session.mode()).unwrap_or(Severity::Warning);
            let label = format!(
                "{:?} `{}` held across .await",
                gv.guard_kind, gv.binding_name
            );
            let explanation = format!(
                "binding `{}` holds a {:?} guard that is live across a suspend point\n   \
                 = this causes runtime deadlocks and `future is not Send` errors",
                gv.binding_name, gv.guard_kind
            );
            session.diagnostics.push(KDiagnostic::new(
                KErrorCode::K0064,
                severity,
                DiagLabel::primary(gv.guard_span, label),
                explanation,
                DiagDecision(
                    "drop the guard before .await or restructure with a block scope".to_owned(),
                ),
            ));
        }
    }

    // S-55: K0065 SelectBranchNotCancelSafe — scan select blocks for non-cancel-safe methods.
    {
        let mut cancel_diags = Vec::new();
        for (fid, entry) in session.file_set().iter_files() {
            let cancel_warnings = scan_source_cancel_safety(entry.source());
            for cw in &cancel_warnings {
                let severity = resolve_severity(KErrorCode::K0065, session.mode())
                    .unwrap_or(Severity::Warning);
                let span = kobo_ir::KoboSpan::new(
                    cw.source_offset as u32,
                    (cw.source_offset + cw.method_name.len()) as u32,
                    fid,
                );
                cancel_diags.push(KDiagnostic::new(
                    KErrorCode::K0065,
                    severity,
                    DiagLabel::primary(span, format!("`.{}()` is not cancel-safe", cw.method_name)),
                    cw.suggestion.clone(),
                    DiagDecision(
                        "move operation outside select or use a cancel-safe wrapper".to_owned(),
                    ),
                ));
            }
        }
        session.diagnostics.extend(cancel_diags);
    }

    // S-56: K0067 HandlerRequestStateLeak — request parameters must not escape into spawned tasks.
    {
        let mut handler_diags = Vec::new();
        for (fid, entry) in session.file_set().iter_files() {
            let leak_warnings = scan_source_handler_leaks(entry.source());
            for leak in &leak_warnings {
                let severity = resolve_severity(KErrorCode::K0067, session.mode())
                    .unwrap_or(Severity::Warning);
                let span = kobo_ir::KoboSpan::new(
                    leak.source_offset as u32,
                    (leak.source_offset + leak.binding_name.len()) as u32,
                    fid,
                );
                handler_diags.push(KDiagnostic::new(
                    KErrorCode::K0067,
                    severity,
                    DiagLabel::primary(
                        span,
                        format!(
                            "handler `{}` leaks request state `{}` into a spawned task",
                            leak.fn_name, leak.binding_name
                        ),
                    ),
                    format!(
                        "binding `{}` belongs to one handler request but is captured by a spawned async boundary\n   \
                         = clone request-safe state before spawning or move long-lived state into an actor",
                        leak.binding_name
                    ),
                    DiagDecision(
                        "clone request-safe state or move background work behind an actor".to_owned(),
                    ),
                ));
            }
        }
        session.diagnostics.extend(handler_diags);
    }

    if session.has_errors() {
        Err(())
    } else {
        project_live_borrow_liveness_diagnostics(session, kir);
        Ok(())
    }
}

fn project_live_borrow_liveness_diagnostics(session: &mut CompileSession, kir: &Kir) {
    for binding in kir.transform_facts().iter_bindings() {
        if !binding.shared_facts.live_borrow_at_move {
            continue;
        }

        let move_span = binding
            .usage
            .uses
            .iter()
            .find_map(|event| match event {
                UseEvent::Moved { span, .. } => Some(*span),
                _ => None,
            })
            .unwrap_or(binding.span);
        let borrow_span = binding
            .shared_facts
            .borrow_sites
            .first()
            .copied()
            .unwrap_or(binding.span);
        let severity =
            resolve_severity(KErrorCode::K0032, session.mode()).unwrap_or(Severity::Warning);
        session.diagnostics.push(
            KDiagnostic::new(
                KErrorCode::K0032,
                severity,
                DiagLabel::primary(
                    move_span,
                    format!(
                        "`{}` moves while a borrow remains live",
                        binding.binding_name
                    ),
                ),
                "KIR liveness found a borrow that is still used after the move; Kobo must preserve the original value through shared ownership or a clone".to_owned(),
                DiagDecision(
                    "shorten the borrow scope before the move, or keep the generated shared/clone lowering".to_owned(),
                ),
            )
            .with_secondary_label(DiagLabel::secondary(
                borrow_span,
                "borrow starts here and remains live at the move",
            )),
        );
    }
}
