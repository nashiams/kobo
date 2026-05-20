use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use kobo_analysis::{
    analyze_send_violations, facts_to_diagnostics, run_analysis, scan_source_cancel_safety,
    scan_source_handler_leaks, scan_source_parallel_warnings,
    scan_source_service_signature_warnings, scan_source_task_local_captures,
    scan_source_task_local_warnings, ParallelWarningKind, ServiceSignatureWarningReason,
    SpawnSite as AnalysisSpawnSite, TaskLocalWarningKind,
};
use kobo_errors::{
    resolve_severity, DiagDecision, DiagLabel, DiagnosticNote, KDiagnostic, KErrorCode, Severity,
};
use kobo_ir::{
    lower_core_program, AsyncViolationKind, CoreBlock, CoreFunction, CoreStatementKind,
    CoreTerminatorKind, Kir, KoboSpan, RelaxAttrError, TransformBindingFacts, UseEvent,
    WarnEarlyPattern,
};
use kobo_transform::{
    strict_async::check_strict_async, strict_async::guard_liveness::detect_guard_across_await,
};

use crate::session::CompileSession;

use super::parse::run_kir_phase;

struct NondeterminismPattern {
    class_name: &'static str,
    operation: &'static str,
}

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

pub(crate) fn run_analysis_phase(session: &mut CompileSession, kir: &Kir) -> Result<(), ()> {
    let downstream_diagnostics_start = session.diagnostics.len();
    let facts = run_analysis(kir, session.file_set());
    session.diagnostics.extend(facts_to_diagnostics(
        &facts,
        kir.transform_facts(),
        session.file_set(),
        kir,
        session.guarantee_policy(),
    ));

    project_known_debt_diagnostics(session, kir);
    project_must_call_attribute_diagnostics(session, kir);
    project_nondeterminism_diagnostics(session);
    project_relax_attribute_diagnostics(session, kir);
    project_warn_early_diagnostics(session, kir);
    project_strict_async_diagnostics(session, kir);
    project_send_root_cause_diagnostics(session, kir);
    project_guard_liveness_diagnostics(session, kir);
    project_cancel_safety_diagnostics(session);
    project_handler_leak_diagnostics(session);
    project_service_signature_diagnostics(session);
    project_parallel_diagnostics(session);
    project_task_local_diagnostics(session);
    project_strict_liveness_diagnostics(session, kir);
    session.suppress_diagnostics_from(downstream_diagnostics_start);

    if session.has_errors() {
        Err(())
    } else {
        project_live_borrow_liveness_diagnostics(session, kir);
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct DriverActiveObligation {
    binding: String,
    span: KoboSpan,
}

type DriverObligationEnv = BTreeMap<String, DriverActiveObligation>;

fn project_strict_liveness_diagnostics(session: &mut CompileSession, kir: &Kir) {
    if !session.guarantee_policy().is_release() {
        return;
    }
    let severity =
        resolve_severity(KErrorCode::K0100, session.guarantee_policy()).unwrap_or(Severity::Error);
    for program in kir.scenario_programs() {
        let core = lower_core_program(program);
        let recursive_functions = strict_liveness_recursive_functions(program);
        for function in &core.functions {
            for (span, binding, exit_kind) in
                strict_liveness_failures(function, &recursive_functions)
            {
                if session.diagnostics.iter().any(|diagnostic| {
                    diagnostic.code == KErrorCode::K0100 && diagnostic.primary.span == span
                }) {
                    continue;
                }
                let message = format!(
                    "strict liveness: unresolved obligation `{binding}` reaches {exit_kind}"
                );
                session.diagnostics.push(KDiagnostic::new(
                    KErrorCode::K0100,
                    severity,
                    DiagLabel::primary(span, "unresolved strict liveness obligation"),
                    message,
                    DiagDecision(
                        "discharge, return, transfer, suppress with reason, or mark the boundary explicit"
                            .to_owned(),
                    ),
                ));
            }
        }
    }
}

fn strict_liveness_failures(
    function: &CoreFunction,
    recursive_functions: &BTreeSet<String>,
) -> Vec<(KoboSpan, String, &'static str)> {
    let blocks = function
        .blocks
        .iter()
        .map(|block| (block.id.clone(), block))
        .collect::<BTreeMap<_, _>>();
    let Some(entry) = function.blocks.first() else {
        return Vec::new();
    };
    let mut failures = Vec::new();
    let mut in_states: BTreeMap<String, DriverObligationEnv> = BTreeMap::new();
    let mut worklist = VecDeque::from([entry.id.clone()]);
    in_states.insert(entry.id.clone(), BTreeMap::new());

    while let Some(block_id) = worklist.pop_front() {
        let Some(block) = blocks.get(&block_id) else {
            continue;
        };
        let mut active = in_states.get(&block_id).cloned().unwrap_or_default();
        apply_driver_block_liveness(block, recursive_functions, &mut active, &mut failures);
        if block.successors.is_empty() {
            failures.extend(
                active
                    .values()
                    .map(|obligation| (obligation.span, obligation.binding.clone(), "normal_exit")),
            );
            continue;
        }
        for successor in &block.successors {
            if successor == &block.id {
                continue;
            }
            let target = in_states.entry(successor.clone()).or_default();
            let mut changed = false;
            for (binding, obligation) in active.clone() {
                changed |= target.insert(binding, obligation).is_none();
            }
            if changed && !worklist.iter().any(|candidate| candidate == successor) {
                worklist.push_back(successor.clone());
            }
        }
    }
    dedupe_strict_liveness_failures(failures)
}

fn apply_driver_block_liveness(
    block: &CoreBlock,
    recursive_functions: &BTreeSet<String>,
    active: &mut DriverObligationEnv,
    failures: &mut Vec<(KoboSpan, String, &'static str)>,
) {
    for statement in &block.statements {
        match statement.kind {
            CoreStatementKind::ObligationCreate => {
                if let Some(binding) = statement.binding.clone() {
                    active.insert(
                        binding.clone(),
                        DriverActiveObligation {
                            binding,
                            span: statement.source_span,
                        },
                    );
                }
            }
            CoreStatementKind::ObligationDischarge | CoreStatementKind::ObligationEscape => {
                if let Some(binding) = statement.binding.as_ref() {
                    active.remove(binding);
                }
            }
            CoreStatementKind::ObligationTransfer => {
                if driver_transfer_is_summary_proved(statement, recursive_functions) {
                    if let Some(binding) = statement.binding.as_ref() {
                        active.remove(binding);
                    }
                }
            }
            CoreStatementKind::ObligationBranchUnresolved => {
                if let Some(binding) = statement.binding.as_ref() {
                    let span = active
                        .get(binding)
                        .map(|obligation| obligation.span)
                        .unwrap_or(statement.source_span);
                    failures.push((span, binding.clone(), "branch_exit"));
                }
            }
            CoreStatementKind::ObligationMove
            | CoreStatementKind::UnsupportedContainer
            | CoreStatementKind::Call => {}
        }
    }
    for terminator in &block.terminators {
        let exit_kind = match terminator.kind {
            CoreTerminatorKind::Return => Some("return"),
            CoreTerminatorKind::ErrorExit => Some("error_exit"),
            CoreTerminatorKind::Panic => Some("panic"),
            CoreTerminatorKind::OpaqueBoundary => Some("opaque_boundary"),
            CoreTerminatorKind::Await | CoreTerminatorKind::Goto | CoreTerminatorKind::Branch => {
                None
            }
        };
        if let Some(exit_kind) = exit_kind {
            failures.extend(
                active
                    .values()
                    .map(|obligation| (obligation.span, obligation.binding.clone(), exit_kind)),
            );
        }
    }
}

fn driver_transfer_is_summary_proved(
    statement: &kobo_ir::CoreStatement,
    recursive_functions: &BTreeSet<String>,
) -> bool {
    let Some(callee) = statement.action.as_deref() else {
        return false;
    };
    !callee.starts_with("unproven:") && !recursive_functions.contains(callee)
}

fn strict_liveness_recursive_functions(program: &kobo_ir::ScenarioProgram) -> BTreeSet<String> {
    program
        .coverage
        .call_graph_sccs
        .iter()
        .filter(|scc| scc.is_recursive)
        .flat_map(|scc| scc.functions.iter().cloned())
        .collect()
}

fn dedupe_strict_liveness_failures(
    failures: Vec<(KoboSpan, String, &'static str)>,
) -> Vec<(KoboSpan, String, &'static str)> {
    let mut deduped = Vec::new();
    for failure in failures {
        if !deduped.iter().any(|existing| existing == &failure) {
            deduped.push(failure);
        }
    }
    deduped
}

fn project_known_debt_diagnostics(session: &mut CompileSession, kir: &Kir) {
    for def in kir.struct_defs() {
        if let Some(error_msg) = &def.known_debt_parse_error {
            let span = def.known_debt_span.unwrap_or(def.span);
            let severity = resolve_severity(KErrorCode::K0025, session.guarantee_policy())
                .unwrap_or(Severity::Error);
            session.diagnostics.push(KDiagnostic::new(
                KErrorCode::K0025,
                severity,
                DiagLabel::primary(span, error_msg.clone()),
                error_msg.clone(),
                DiagDecision(String::new()),
            ));
        }
    }
}

fn project_must_call_attribute_diagnostics(session: &mut CompileSession, kir: &Kir) {
    for error in kir.must_call_attr_errors() {
        let severity = resolve_severity(KErrorCode::K0114, session.guarantee_policy())
            .unwrap_or(Severity::Error);
        session.diagnostics.push(KDiagnostic::new(
            KErrorCode::K0114,
            severity,
            DiagLabel::primary(error.span, error.message.clone()),
            error.message.clone(),
            DiagDecision(
                "write `#[kobo::must_call(commit | rollback)]` with named actions".to_owned(),
            ),
        ));
    }
}

fn project_relax_attribute_diagnostics(session: &mut CompileSession, kir: &Kir) {
    for RelaxAttrError {
        span,
        message,
        is_error,
    } in kir.relax_attr_errors()
    {
        let severity = if *is_error {
            Severity::Error
        } else {
            resolve_severity(KErrorCode::K0026, session.guarantee_policy())
                .unwrap_or(Severity::Warning)
        };
        session.diagnostics.push(KDiagnostic::new(
            KErrorCode::K0026,
            severity,
            DiagLabel::primary(*span, message.clone()),
            message.clone(),
            DiagDecision(String::new()),
        ));
    }

    if session.guarantee_policy().is_dev() && !session.relaxed_fn_ranges.is_empty() {
        let severity = resolve_severity(KErrorCode::K0026, session.guarantee_policy())
            .unwrap_or(Severity::Warning);
        for &fn_span in &session.relaxed_fn_ranges.clone() {
            session.diagnostics.push(KDiagnostic::new(
                KErrorCode::K0026,
                severity,
                DiagLabel::primary(fn_span, "`#[kobo::relax]` has no effect in the dev profile"),
                "`#[kobo::relax]` has no effect while the dev guarantee policy is already advisory"
                    .to_owned(),
                DiagDecision("remove the marker or use a stricter guarantee profile".to_owned()),
            ));
        }
    }
}

fn project_warn_early_diagnostics(session: &mut CompileSession, kir: &Kir) {
    for fact in kir.warn_early_facts() {
        if fact.suppressed {
            continue;
        }
        let (code, label_text, explanation) = warn_early_diagnostic_parts(&fact.pattern);
        let severity = resolve_severity(code, session.guarantee_policy()).unwrap_or(Severity::Note);
        session.diagnostics.push(KDiagnostic::new(
            code,
            severity,
            DiagLabel::primary(fact.span, label_text),
            explanation,
            DiagDecision(
                "advisory only; no automatic fix; see `kobo debt` for migration guidance"
                    .to_owned(),
            ),
        ));
    }
}

fn warn_early_diagnostic_parts(pattern: &WarnEarlyPattern) -> (KErrorCode, String, String) {
    match pattern {
        WarnEarlyPattern::BidirectionalRcLinks {
            struct_name,
            field_pairs,
        } => {
            let pairs = field_pairs
                .iter()
                .map(|(left, right)| format!("{left}<->{right}"))
                .collect::<Vec<_>>()
                .join(", ");
            (
                KErrorCode::K0080P1,
                format!("bidirectional shared links in `{struct_name}`"),
                format!(
                    "struct `{struct_name}` has shared mutable links that form a cycle ({pairs}); this can leak memory unless back-links use weak references"
                ),
            )
        }
        WarnEarlyPattern::ParentChildBackPointer {
            struct_name,
            children_field,
            parent_field,
        } => (
            KErrorCode::K0080P2,
            format!("parent-child back-pointer in `{struct_name}`"),
            format!(
                "struct `{struct_name}` has `{children_field}` children and `{parent_field}` parent links; this can leak unless the parent link uses weak references"
            ),
        ),
        WarnEarlyPattern::SharedMutableAt3PlusSites { site_count, .. } => (
            KErrorCode::K0080P3,
            format!("shared mutable state at {site_count} call sites"),
            format!(
                "the binding is mutated from {site_count} distinct call sites; migration will require an architectural ownership decision"
            ),
        ),
        WarnEarlyPattern::SelfReferentialStruct { struct_name } => (
            KErrorCode::K0080P4,
            format!("self-referential struct `{struct_name}` without indirection"),
            format!(
                "struct `{struct_name}` contains a direct field of the same type; this would have infinite size, so use Box<{struct_name}> or another explicit indirection"
            ),
        ),
    }
}

fn project_strict_async_diagnostics(session: &mut CompileSession, kir: &Kir) {
    let has_executor = kobo_codegen::executor::select_executor(&session.config.dependencies)
        != kobo_codegen::executor::ExecutorChoice::None;
    let async_violations = check_strict_async(kir, session.guarantee_policy(), has_executor);
    for violation in &async_violations {
        if let AsyncViolationKind::NonSendCapture { binding_name, .. } = &violation.kind {
            if binding_is_captured_by_explicit_local_spawn(session, binding_name) {
                continue;
            }
        }
        let (code, label_text, explanation, decision) =
            async_violation_diagnostic_parts(&violation.kind);
        let severity =
            resolve_severity(code, session.guarantee_policy()).unwrap_or(Severity::Error);
        session.diagnostics.push(KDiagnostic::new(
            code,
            severity,
            DiagLabel::primary(violation.span, label_text),
            explanation,
            decision,
        ));
    }
}

fn binding_is_captured_by_explicit_local_spawn(
    session: &CompileSession,
    binding_name: &str,
) -> bool {
    session.file_set().iter_files().any(|(_, entry)| {
        scan_source_task_local_captures(entry.source())
            .iter()
            .any(|capture| capture.is_explicit_local && capture.binding_name == binding_name)
    })
}

fn async_violation_diagnostic_parts(
    kind: &AsyncViolationKind,
) -> (KErrorCode, String, String, String) {
    match kind {
        AsyncViolationKind::NonSendCapture { binding_name, .. } => (
            KErrorCode::K0060,
            format!("binding `{binding_name}` is not Send"),
            format!(
                "binding `{binding_name}` would use single-thread sharing, but this async context requires Send"
            ),
            "use message passing, clone owned data, or keep the task on a local executor".to_owned(),
        ),
        AsyncViolationKind::NonSyncShared { binding_name, .. } => (
            KErrorCode::K0061,
            format!("binding `{binding_name}` is not Sync for shared access"),
            format!(
                "binding `{binding_name}` requires Sync for cross-task sharing, but the current ownership shape is not Sync"
            ),
            "restructure with channels, actor ownership, or an explicit async shared state policy".to_owned(),
        ),
        AsyncViolationKind::MissingExecutor => (
            KErrorCode::K0062,
            "no async executor configured".to_owned(),
            "async code was detected but no executor dependency such as tokio or async-std was found"
                .to_owned(),
            "add tokio or async-std to [dependencies] in Cargo.toml; if a guard is live across an await, drop the guard before .await or restructure with a block scope".to_owned(),
        ),
        AsyncViolationKind::StrictAsyncViolation { binding_name, .. } => (
            KErrorCode::K0063,
            format!("release profile: async wrapping not permitted for `{binding_name}`"),
            format!(
                "inside @strict enforcement, binding `{binding_name}` cannot use ownership wrappers in async context"
            ),
            "use @strict async fn when the async strict protocol is intentional, or move this work into a synchronous helper".to_owned(),
        ),
    }
}

fn project_send_root_cause_diagnostics(session: &mut CompileSession, kir: &Kir) {
    let transform_facts = kir.transform_facts();
    let send_bindings = transform_facts
        .bindings
        .iter()
        .filter(|binding| binding.shared_facts.needs_send)
        .map(|binding| binding.node)
        .collect::<Vec<_>>();
    if send_bindings.is_empty() {
        return;
    }

    let synthetic_site = AnalysisSpawnSite {
        span: transform_facts
            .bindings
            .iter()
            .find(|binding| binding.shared_facts.needs_send)
            .map(|binding| binding.span)
            .unwrap_or(kobo_ir::KoboSpan::new(0, 0, kobo_ir::FileId(0))),
        captured_bindings: send_bindings,
        await_points: Vec::new(),
    };
    let send_diagnostics = analyze_send_violations(&[synthetic_site], transform_facts, kir);
    for diagnostic in &send_diagnostics {
        if binding_is_captured_by_explicit_local_spawn(session, &diagnostic.binding_name) {
            continue;
        }
        let severity = resolve_severity(KErrorCode::K0061, session.guarantee_policy())
            .unwrap_or(Severity::Error);
        session.diagnostics.push(KDiagnostic::new(
            KErrorCode::K0061,
            severity,
            DiagLabel::primary(
                diagnostic.spawn_span,
                format!(
                    "future requires Send but `{}` uses {}",
                    diagnostic.binding_name, diagnostic.wrapper_type
                ),
            ),
            format!(
                "binding `{}` cannot cross thread boundary — {}\n   = {}",
                diagnostic.binding_name, diagnostic.wrapper_type, diagnostic.suggestion
            ),
            DiagDecision(diagnostic.suggestion.clone()),
        ));
    }
}

fn project_guard_liveness_diagnostics(session: &mut CompileSession, kir: &Kir) {
    let guard_violations = detect_guard_across_await(kir);
    for violation in &guard_violations {
        let severity = resolve_severity(KErrorCode::K0064, session.guarantee_policy())
            .unwrap_or(Severity::Warning);
        let label = format!(
            "{:?} `{}` held across .await",
            violation.guard_kind, violation.binding_name
        );
        let explanation = format!(
            "binding `{}` holds a {:?} guard that is live across a suspend point\n   \
             = this causes runtime deadlocks and `future is not Send` errors",
            violation.binding_name, violation.guard_kind
        );
        session.diagnostics.push(KDiagnostic::new(
            KErrorCode::K0064,
            severity,
            DiagLabel::primary(violation.guard_span, label),
            explanation,
            DiagDecision(
                "drop the guard before .await or restructure with a block scope".to_owned(),
            ),
        ));
    }
}

fn project_cancel_safety_diagnostics(session: &mut CompileSession) {
    let mut diagnostics = Vec::new();
    for (file_id, entry) in session.file_set().iter_files() {
        let cancel_warnings = scan_source_cancel_safety(entry.source());
        for warning in &cancel_warnings {
            let severity = resolve_severity(KErrorCode::K0065, session.guarantee_policy())
                .unwrap_or(Severity::Warning);
            let span = kobo_ir::KoboSpan::new(
                warning.source_offset as u32,
                (warning.source_offset + warning.method_name.len()) as u32,
                file_id,
            );
            diagnostics.push(KDiagnostic::new(
                KErrorCode::K0065,
                severity,
                DiagLabel::primary(
                    span,
                    format!("`.{}()` is not cancel-safe", warning.method_name),
                ),
                warning.suggestion.clone(),
                DiagDecision(
                    "move operation outside select or use a cancel-safe wrapper".to_owned(),
                ),
            ));
        }
    }
    session.diagnostics.extend(diagnostics);
}

fn project_handler_leak_diagnostics(session: &mut CompileSession) {
    let mut diagnostics = Vec::new();
    for (file_id, entry) in session.file_set().iter_files() {
        let leak_warnings = scan_source_handler_leaks(entry.source());
        for leak in &leak_warnings {
            let severity = resolve_severity(KErrorCode::K0067, session.guarantee_policy())
                .unwrap_or(Severity::Warning);
            let span = kobo_ir::KoboSpan::new(
                leak.source_offset as u32,
                (leak.source_offset + leak.binding_name.len()) as u32,
                file_id,
            );
            diagnostics.push(KDiagnostic::new(
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
    session.diagnostics.extend(diagnostics);
}

fn project_service_signature_diagnostics(session: &mut CompileSession) {
    let mut diagnostics = Vec::new();
    for (file_id, entry) in session.file_set().iter_files() {
        let warnings = scan_source_service_signature_warnings(entry.source());
        for warning in &warnings {
            let severity = resolve_severity(KErrorCode::K0061, session.guarantee_policy())
                .unwrap_or(Severity::Error);
            let span = KoboSpan::new(
                warning.source_offset as u32,
                (warning.source_offset + warning.span_len).max(warning.source_offset + 1) as u32,
                file_id,
            );
            let method = warning
                .method_name
                .as_deref()
                .unwrap_or(warning.service_name.as_str());
            let (label, why, decision) =
                service_signature_warning_message(&warning.reason, &warning.service_name, method);
            diagnostics.push(KDiagnostic::new(
                KErrorCode::K0061,
                severity,
                DiagLabel::primary(span, label),
                why,
                DiagDecision(decision),
            ));
        }
    }
    session.diagnostics.extend(diagnostics);
}

fn service_signature_warning_message(
    reason: &ServiceSignatureWarningReason,
    service_name: &str,
    method: &str,
) -> (String, String, String) {
    match reason {
        ServiceSignatureWarningReason::GenericService => (
            format!("service signature `{service_name}` uses unsupported generics"),
            format!(
                "service signature `{service_name}` is generic; Kobo cannot generate a bounded typed message enum until the service type is monomorphic"
            ),
            "specialize the service type before applying #[kobo::service], or move the generic API behind a typed wrapper".to_owned(),
        ),
        ServiceSignatureWarningReason::GenericMethod => (
            format!("service signature `{method}` uses unsupported method generics"),
            format!(
                "service method `{method}` has generic parameters; generated service messages need concrete owned field and reply types"
            ),
            "specialize the method arguments before crossing the service channel".to_owned(),
        ),
        ServiceSignatureWarningReason::BorrowedMessageType => (
            format!("service signature `{method}` uses borrowed channel data"),
            format!(
                "service method `{method}` uses borrowed message or reply data; bounded service channels require owned values with a supported lifetime model"
            ),
            "pass owned data such as String/Arc<T>, or keep the method outside #[kobo::service] until a borrow-safe protocol is modeled".to_owned(),
        ),
        ServiceSignatureWarningReason::UnsupportedReceiver => (
            format!("service signature `{method}` has unsupported receiver"),
            format!(
                "service method `{method}` must use &self or &mut self so the generated worker does not move the service out of its dispatch loop"
            ),
            "change the service method receiver to &self or &mut self".to_owned(),
        ),
    }
}

fn project_task_local_diagnostics(session: &mut CompileSession) {
    let mut diagnostics = Vec::new();
    for (file_id, entry) in session.file_set().iter_files() {
        let warnings = scan_source_task_local_warnings(entry.source());
        for warning in &warnings {
            match &warning.kind {
                TaskLocalWarningKind::NormalSpawnNonSendCapture {
                    binding_name,
                    type_name,
                } => {
                    let severity = resolve_severity(KErrorCode::K0061, session.guarantee_policy())
                        .unwrap_or(Severity::Error);
                    let span = KoboSpan::new(
                        warning.source_offset as u32,
                        (warning.source_offset + "spawn".len()) as u32,
                        file_id,
                    );
                    diagnostics.push(KDiagnostic::new(
                        KErrorCode::K0061,
                        severity,
                        DiagLabel::primary(
                            span,
                            format!("normal spawn captures non-Send `{binding_name}`"),
                        ),
                        format!(
                            "binding `{binding_name}` uses {type_name}, which cannot cross the Send boundary required by normal spawn"
                        ),
                        DiagDecision(
                            "use spawn local for intentional task-local work, or change the captured state to a Send type such as Arc".to_owned(),
                        ),
                    ));
                }
                TaskLocalWarningKind::LocalFutureEscape { binding_name } => {
                    let severity = resolve_severity(KErrorCode::K0067, session.guarantee_policy())
                        .unwrap_or(Severity::Error);
                    let span = KoboSpan::new(
                        warning.source_offset as u32,
                        (warning.source_offset + binding_name.len()).max(warning.source_offset + 1)
                            as u32,
                        file_id,
                    );
                    diagnostics.push(KDiagnostic::new(
                        KErrorCode::K0067,
                        severity,
                        DiagLabel::primary(
                            span,
                            format!("task-local future `{binding_name}` escapes its LocalSet"),
                        ),
                        format!(
                            "task-local handle `{binding_name}` must stay inside the LocalSet that owns its non-Send execution context"
                        ),
                        DiagDecision(
                            "await or drop the task-local handle inside the spawn local zone".to_owned(),
                        ),
                    ));
                }
            }
        }
    }
    session.diagnostics.extend(diagnostics);
}

fn project_parallel_diagnostics(session: &mut CompileSession) {
    let mut diagnostics = Vec::new();
    for (file_id, entry) in session.file_set().iter_files() {
        let warnings = scan_source_parallel_warnings(entry.source());
        for warning in &warnings {
            let severity = resolve_severity(KErrorCode::K0061, session.guarantee_policy())
                .unwrap_or(Severity::Warning);
            let (label, why, decision) = parallel_warning_message(&warning.kind);
            let span_len = parallel_warning_span_len(&warning.kind);
            let span = KoboSpan::new(
                warning.source_offset as u32,
                (warning.source_offset + span_len).max(warning.source_offset + 1) as u32,
                file_id,
            );
            diagnostics.push(KDiagnostic::new(
                KErrorCode::K0061,
                severity,
                DiagLabel::primary(span, label),
                why,
                DiagDecision(decision),
            ));
        }
    }
    session.diagnostics.extend(diagnostics);
}

fn parallel_warning_message(kind: &ParallelWarningKind) -> (String, String, String) {
    match kind {
        ParallelWarningKind::NonSendCapture {
            binding_name,
            type_name,
        } => (
            format!("parallel loop captures non-Send `{binding_name}`"),
            format!(
                "`{binding_name}` uses `{type_name}`, which cannot safely cross Rayon worker threads"
            ),
            "change the captured state to a Send + Sync type such as Arc, or keep the loop serial"
                .to_owned(),
        ),
        ParallelWarningKind::SharedMutation { binding_name } => (
            format!("parallel loop mutates shared `{binding_name}`"),
            format!("`{binding_name}` is mutated inside the parallel body and would race"),
            "collect per-item results or protect shared mutation behind an explicit synchronization boundary"
                .to_owned(),
        ),
        ParallelWarningKind::MissingBoundaryPolicy => (
            "parallel loop near ward boundary needs an explicit policy".to_owned(),
            "ward boundaries can affect replay ordering; choose whether the loop is outside or inside that boundary"
                .to_owned(),
            "write #[kobo::parallel(policy = \"outside\")] or keep this path serial".to_owned(),
        ),
    }
}

fn parallel_warning_span_len(kind: &ParallelWarningKind) -> usize {
    match kind {
        ParallelWarningKind::NonSendCapture { binding_name, .. }
        | ParallelWarningKind::SharedMutation { binding_name } => binding_name.len(),
        ParallelWarningKind::MissingBoundaryPolicy => "ward".len(),
    }
}
fn project_live_borrow_liveness_diagnostics(session: &mut CompileSession, kir: &Kir) {
    for binding in kir.transform_facts().iter_bindings() {
        if !binding.shared_facts.live_borrow_at_move {
            continue;
        }

        let move_span = source_move_span_for_binding(session, binding).unwrap_or_else(|| {
            binding
                .usage
                .uses
                .iter()
                .find_map(|event| match event {
                    UseEvent::Moved { span, .. } => Some(*span),
                    _ => None,
                })
                .unwrap_or(binding.span)
        });
        let borrow_span = binding
            .shared_facts
            .borrow_sites
            .first()
            .copied()
            .unwrap_or(binding.span);
        let severity = resolve_severity(KErrorCode::K0032, session.guarantee_policy())
            .unwrap_or(Severity::Warning);
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

fn source_move_span_for_binding(
    session: &CompileSession,
    binding: &TransformBindingFacts,
) -> Option<KoboSpan> {
    let file_id = binding
        .usage
        .uses
        .iter()
        .find_map(|event| match event {
            UseEvent::Moved { span, .. } => Some(span.file_id),
            _ => None,
        })
        .or_else(|| {
            binding
                .shared_facts
                .borrow_sites
                .first()
                .map(|span| span.file_id)
        })
        .unwrap_or(binding.span.file_id);
    let entry = session.file_set().get(file_id)?;
    let source = entry.source();
    let needle = binding.binding_name.as_str();
    let mut offset = 0usize;

    for line in source.lines() {
        if let Some(index) = line.find(needle) {
            let before = &line[..index];
            let after = &line[index + needle.len()..];
            if before.trim_end().ends_with('=')
                && !before.trim_end().ends_with("&=")
                && !before.trim_end().ends_with('&')
                && after.trim_start().starts_with(';')
            {
                let start = (offset + index) as u32;
                return Some(KoboSpan::new(start, start + needle.len() as u32, file_id));
            }
        }
        offset += line.len() + 1;
    }

    None
}

fn project_nondeterminism_diagnostics(session: &mut CompileSession) {
    let mut diagnostics = Vec::new();
    for (file_id, entry) in session.file_set().iter_files() {
        let source = entry.source();
        if !is_replay_risk_zone(source) {
            continue;
        }

        for pattern in nondeterminism_patterns() {
            let occurrences = source.matches(pattern.operation).count();
            if occurrences == 0 {
                continue;
            }
            let offset = source.find(pattern.operation).unwrap_or(0) as u32;
            let span =
                kobo_ir::KoboSpan::new(offset, offset + pattern.operation.len() as u32, file_id);
            let severity = resolve_severity(KErrorCode::K0102, session.guarantee_policy())
                .unwrap_or(Severity::Warning);
            diagnostics.push(
                KDiagnostic::new(
                    KErrorCode::K0102,
                    severity,
                    DiagLabel::primary(
                        span,
                        format!("raw {} nondeterminism: {}", pattern.class_name, pattern.operation),
                    ),
                    format!(
                        "raw {} nondeterminism appears in a scenario or future replay zone through `{}`",
                        pattern.class_name, pattern.operation
                    ),
                    DiagDecision(
                        "wrap this operation behind a modeled policy before claiming replay"
                            .to_owned(),
                    ),
                )
                .with_note(DiagnosticNote::new(format!(
                    "occurrences: {occurrences}"
                ))),
            );
        }
    }
    session.diagnostics.extend(diagnostics);
}

fn is_replay_risk_zone(source: &str) -> bool {
    source.contains("kobo::scenario") || source.contains("replay") || source.contains("ward")
}

fn nondeterminism_patterns() -> &'static [NondeterminismPattern] {
    &[
        NondeterminismPattern {
            class_name: "time",
            operation: "SystemTime::now",
        },
        NondeterminismPattern {
            class_name: "time",
            operation: "Instant::now",
        },
        NondeterminismPattern {
            class_name: "random",
            operation: "rand::random",
        },
        NondeterminismPattern {
            class_name: "random",
            operation: "thread_rng",
        },
        NondeterminismPattern {
            class_name: "filesystem",
            operation: "std::fs::",
        },
        NondeterminismPattern {
            class_name: "scheduler",
            operation: "tokio::spawn",
        },
        NondeterminismPattern {
            class_name: "scheduler",
            operation: "select!",
        },
        NondeterminismPattern {
            class_name: "process",
            operation: "std::process::",
        },
        NondeterminismPattern {
            class_name: "environment",
            operation: "std::env::",
        },
        NondeterminismPattern {
            class_name: "network",
            operation: "TcpStream",
        },
        NondeterminismPattern {
            class_name: "network",
            operation: "reqwest::",
        },
    ]
}
