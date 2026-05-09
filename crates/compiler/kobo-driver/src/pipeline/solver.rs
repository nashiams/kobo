use kobo_codegen::{KoboSourceMap, SolverBudgetJson, SolverEvidenceJson};
use kobo_errors::{resolve_severity, DiagDecision, DiagLabel, KDiagnostic, KErrorCode, Severity};
use kobo_ir::{Kir, SolutionMap};
use kobo_migrate::{
    solve_modular, solve_modular_with_evidence, SolveOutcome, SolverBudget, SolverEvidence,
};

use crate::session::CompileSession;

pub(super) fn resolve_solution(kir: &Kir) -> (SolverEvidence, SolveOutcome) {
    let budget = SolverBudget::default();
    let modular = solve_modular_with_evidence(kir, &budget);
    let outcome = solve_modular(kir, &budget);
    (modular.solver_evidence, outcome)
}

/// Project non-Unique solver outcomes to K-code diagnostics (Phase 00/05).
pub(super) fn project_solver_diagnostics(
    session: &mut CompileSession,
    kir: &Kir,
    outcome: &SolveOutcome,
) {
    match outcome {
        SolveOutcome::Unique(_) => {}
        SolveOutcome::MultiSolution(candidates) => {
            if candidates.is_empty() {
                return;
            }
            let first_node = candidates[0]
                .solution
                .iter()
                .map(|(id, _)| id)
                .min_by_key(|id| id.0);
            let span = first_node
                .and_then(|id| kir.get_node(id))
                .map(|n| n.span)
                .unwrap_or(kobo_ir::KoboSpan::new(0, 0, kobo_ir::FileId(0)));
            let severity =
                resolve_severity(KErrorCode::K0083, session.mode()).unwrap_or(Severity::Warning);
            session.diagnostics.push(KDiagnostic::new(
                KErrorCode::K0083,
                severity,
                DiagLabel::primary(
                    span,
                    format!(
                        "ownership has {} valid solver candidate(s)",
                        candidates.len()
                    ),
                ),
                "solver found multiple valid ownership assignments; using the lowest-risk candidate for code generation"
                    .to_owned(),
                DiagDecision("run `kobo migrate --review` to inspect and lock a candidate".to_owned()),
            ));
        }
        SolveOutcome::NoSolution(report) => {
            let span = report
                .conflicting_nodes
                .first()
                .and_then(|id| kir.get_node(*id))
                .map(|n| n.span)
                .unwrap_or(kobo_ir::KoboSpan::new(0, 0, kobo_ir::FileId(0)));
            let severity =
                resolve_severity(KErrorCode::K0080, session.mode()).unwrap_or(Severity::Warning);
            session.diagnostics.push(KDiagnostic::new(
                KErrorCode::K0080,
                severity,
                DiagLabel::primary(
                    span,
                    format!("ownership conflict: {}", report.conflict_reason),
                ),
                format!(
                    "solver found no valid ownership assignment for {} node(s)",
                    report.conflicting_nodes.len()
                ),
                DiagDecision("review sharing patterns or add ownership hints".to_owned()),
            ));
        }
        SolveOutcome::ClusterTooLarge(report) => {
            let span = report
                .member_nodes
                .first()
                .and_then(|id| kir.get_node(*id))
                .map(|n| n.span)
                .unwrap_or(kobo_ir::KoboSpan::new(0, 0, kobo_ir::FileId(0)));
            let severity =
                resolve_severity(KErrorCode::K0081, session.mode()).unwrap_or(Severity::Warning);
            session.diagnostics.push(KDiagnostic::new(
                KErrorCode::K0081,
                severity,
                DiagLabel::primary(
                    span,
                    format!(
                        "constraint cluster too large ({} nodes, limit {})",
                        report.cluster_size, report.limit
                    ),
                ),
                format!(
                    "cluster of {} interdependent bindings exceeds solver limit of {}",
                    report.cluster_size, report.limit
                ),
                DiagDecision(
                    "split large functions or reduce sharing to lower cluster size".to_owned(),
                ),
            ));
        }
        SolveOutcome::BudgetExceeded(report) => {
            let span = kobo_ir::KoboSpan::new(0, 0, kobo_ir::FileId(0));
            let severity =
                resolve_severity(KErrorCode::K0082, session.mode()).unwrap_or(Severity::Warning);
            session.diagnostics.push(KDiagnostic::new(
                KErrorCode::K0082,
                severity,
                DiagLabel::primary(
                    span,
                    format!(
                        "solver budget exceeded ({:.1}s of {:.1}s, {}/{} resolved)",
                        report.elapsed_seconds,
                        report.budget_seconds,
                        report.resolved_count,
                        report.total_count,
                    ),
                ),
                format!(
                    "solver resolved {}/{} nodes before {:.1}s budget expired",
                    report.resolved_count, report.total_count, report.budget_seconds
                ),
                DiagDecision(
                    "increase solver_budget_seconds or reduce constraint complexity".to_owned(),
                ),
            ));
        }
        SolveOutcome::BoundaryStop(report) => {
            let span = kir
                .get_node(report.crossing_node)
                .map(|n| n.span)
                .unwrap_or(kobo_ir::KoboSpan::new(0, 0, kobo_ir::FileId(0)));
            let severity =
                resolve_severity(KErrorCode::K0090, session.mode()).unwrap_or(Severity::Warning);
            session.diagnostics.push(KDiagnostic::new(
                KErrorCode::K0090,
                severity,
                DiagLabel::primary(
                    span,
                    format!(
                        "ownership depends on external crate `{}`",
                        report.external_crate
                    ),
                ),
                format!(
                    "binding crosses into `{}::{}` — solver cannot infer ownership across crate boundaries",
                    report.external_crate, report.external_function
                ),
                DiagDecision(
                    "add an explicit ownership hint or use #[kobo::migrate]".to_owned(),
                ),
            ));
        }
    }
}

/// S-3: Cap engine struct bindings to PlainOwned.
///
/// Bindings whose declared type matches an `#[kobo::engine]` struct are framework-managed
/// and must not be promoted to Rc/Arc. Walk nodes marked as engine-ceiling and downgrade
/// any that were solver-assigned above PlainOwned.
pub(super) fn apply_engine_ceiling(
    kir: &Kir,
    solution: &mut SolutionMap,
) -> Vec<EngineCeilingAdjustment> {
    use kobo_ir::{NodeKind, OwnershipTier};

    let mut adjustments = Vec::new();
    for node in kir.iter_decl_nodes() {
        debug_assert_eq!(node.kind, NodeKind::Decl);
        if !kir.is_engine_ceiling(node.id) {
            continue;
        }
        let resolved = solution.resolve(node.id, node.ownership);
        if resolved.priority() > OwnershipTier::PlainOwned.priority() {
            solution.insert(node.id, OwnershipTier::PlainOwned);
            adjustments.push(EngineCeilingAdjustment {
                span: node.span,
                binding_name: binding_name_for_node(kir, node.id),
                requested_tier: resolved,
            });
        }
    }
    adjustments
}

#[derive(Debug, Clone)]
pub(super) struct EngineCeilingAdjustment {
    span: kobo_ir::KoboSpan,
    binding_name: String,
    requested_tier: kobo_ir::OwnershipTier,
}

fn binding_name_for_node(kir: &Kir, node_id: kobo_ir::KirNodeId) -> String {
    kir.transform_facts()
        .iter_bindings()
        .find(|binding| binding.node == node_id)
        .map(|binding| binding.binding_name.clone())
        .unwrap_or_else(|| format!("node_{}", node_id.0))
}

pub(super) fn project_engine_ceiling_diagnostics(
    session: &mut CompileSession,
    adjustments: &[EngineCeilingAdjustment],
) {
    for adjustment in adjustments {
        let severity =
            resolve_severity(KErrorCode::K0031, session.mode()).unwrap_or(Severity::Warning);
        session.diagnostics.push(KDiagnostic::new(
            KErrorCode::K0031,
            severity,
            DiagLabel::primary(
                adjustment.span,
                format!(
                    "`{}` is #[kobo::engine]-managed and cannot use {:?}",
                    adjustment.binding_name, adjustment.requested_tier
                ),
            ),
            format!(
                "solver selected {:?}, but engine structs are framework-managed; Kobo emitted PlainOwned instead of silently downgrading",
                adjustment.requested_tier
            ),
            DiagDecision(
                "keep engine state owned by the framework or remove #[kobo::engine] if shared ownership is intentional".to_owned(),
            ),
        ));
    }
}

pub(super) fn inject_solver_evidence(
    mut source_map: KoboSourceMap,
    evidence: &SolverEvidence,
) -> KoboSourceMap {
    // Set top-level solver evidence.
    source_map.solver_evidence = Some(SolverEvidenceJson {
        outcome: evidence.outcome_name.clone(),
        graph_fingerprint: evidence.graph_fingerprint.clone(),
        node_count: evidence.node_count as u64,
        edge_count: evidence.edge_count as u64,
        budget: SolverBudgetJson {
            max_cluster_size: evidence.budget.max_cluster_size as u64,
            budget_seconds: evidence.budget.budget_seconds,
        },
    });

    // Annotate each mapping with solver provenance.
    for entry in &mut source_map.x_kobo_mappings {
        entry.solver_outcome = Some(evidence.outcome_name.clone());
        entry.decision_source = Some("solver".to_owned());
    }

    source_map
}
