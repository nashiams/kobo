use std::collections::{BTreeMap, BTreeSet};

use crate::{
    stable_hash, CancelEdgeEvidence, ObligationEventKind, ObligationStatus, ProofCertificate,
    SelectPathEvidence, SuspensionStateEvidence, VerificationError,
};

use crate::obligation::{block_obligation_envs, event_state_map};

pub(crate) fn verify_cancel_edges(certificate: &ProofCertificate) -> Result<(), VerificationError> {
    let mut await_blocks = BTreeSet::<String>::new();
    let mut cancel_blocks = BTreeSet::<String>::new();
    for edge in &certificate.core.cfg_edges {
        if edge.kind == "await" {
            await_blocks.insert(edge.from.clone());
            if edge.to == "await_cancel" {
                cancel_blocks.insert(edge.from.clone());
            }
        }
    }
    for block in await_blocks {
        if !cancel_blocks.contains(&block) {
            return Err(VerificationError::MissingCancelEdge { block });
        }
    }
    Ok(())
}

pub(crate) fn verify_async_model(
    certificate: &ProofCertificate,
    source: &str,
) -> Result<(), VerificationError> {
    let await_edges = await_edges(certificate);
    let suspensions = suspensions_by_id(certificate);
    let suspension_blocks = suspensions
        .values()
        .map(|state| state.block.as_str())
        .collect::<BTreeSet<_>>();

    for (function, block, edge_targets) in &await_edges {
        let Some(suspension) = suspensions
            .values()
            .find(|state| state.function == *function && state.block == *block)
        else {
            return Err(VerificationError::MissingAsyncCancelEvidence {
                block: block.clone(),
            });
        };
        if suspension.terminator_kind != "await"
            || !edge_targets.contains(suspension.resume_edge.as_str())
            || !edge_targets.contains(suspension.cancel_edge.as_str())
        {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "suspension_states".to_owned(),
                id: suspension.id.clone(),
            });
        }
    }

    let cancel_evidence = certificate
        .core
        .async_model
        .cancel_edges
        .iter()
        .map(cancel_edge_key)
        .collect::<BTreeSet<_>>();
    for (function, block, edge_targets) in &await_edges {
        if edge_targets.contains("await_cancel")
            && !cancel_evidence.contains(&(function.as_str(), block.as_str(), "await_cancel"))
        {
            return Err(VerificationError::MissingAsyncCancelEvidence {
                block: block.clone(),
            });
        }
    }

    for local in &certificate.core.async_model.future_state_locals {
        if !suspensions.contains_key(local.suspension_state.as_str()) {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "future_state_locals".to_owned(),
                id: local.binding.clone(),
            });
        }
    }

    verify_future_state_locals(certificate, source, &suspensions)?;
    verify_future_state_obligations(certificate, &suspensions)?;
    verify_select_paths(certificate)?;
    verify_timeout_cancel_edges(certificate, &suspensions)?;
    verify_spawned_task_obligations(certificate)?;

    for suspension in suspensions.values() {
        if !suspension_blocks.contains(suspension.block.as_str()) {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "suspension_states".to_owned(),
                id: suspension.id.clone(),
            });
        }
    }

    Ok(())
}

fn verify_future_state_obligations(
    certificate: &ProofCertificate,
    suspensions: &BTreeMap<&str, &SuspensionStateEvidence>,
) -> Result<(), VerificationError> {
    let block_envs = block_obligation_envs(certificate)?;
    let mut actual = BTreeMap::<(&str, &str), ObligationStatus>::new();
    for obligation in &certificate.core.async_model.future_state_obligations {
        if !suspensions.contains_key(obligation.suspension_state.as_str()) {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "future_state_obligations".to_owned(),
                id: obligation.binding.clone(),
            });
        }
        actual.insert(
            (
                obligation.suspension_state.as_str(),
                obligation.binding.as_str(),
            ),
            obligation.state.clone(),
        );
    }

    for suspension in suspensions.values() {
        let env = block_envs
            .get(&suspension.block)
            .cloned()
            .unwrap_or_default();
        for (binding, expected_state) in env {
            if expected_state != ObligationStatus::Owned {
                continue;
            }
            let observed = actual.get(&(suspension.id.as_str(), binding.as_str()));
            if observed != Some(&ObligationStatus::Owned) {
                return Err(VerificationError::MissingFutureStateObligation {
                    binding,
                    suspension_state: suspension.id.clone(),
                });
            }
        }
    }
    Ok(())
}

fn verify_future_state_locals(
    certificate: &ProofCertificate,
    source: &str,
    suspensions: &BTreeMap<&str, &SuspensionStateEvidence>,
) -> Result<(), VerificationError> {
    let expected_by_function = parsed_live_locals_by_function(source);
    let actual = certificate
        .core
        .async_model
        .future_state_locals
        .iter()
        .map(|local| (local.suspension_state.as_str(), local.binding.as_str()))
        .collect::<BTreeSet<_>>();

    let mut suspensions_by_function = BTreeMap::<&str, Vec<&SuspensionStateEvidence>>::new();
    for suspension in suspensions.values() {
        suspensions_by_function
            .entry(suspension.function.as_str())
            .or_default()
            .push(*suspension);
    }
    for suspensions in suspensions_by_function.values_mut() {
        suspensions.sort_by(|left, right| {
            (
                left.source_span.line,
                left.source_span.start,
                left.block.as_str(),
                left.id.as_str(),
            )
                .cmp(&(
                    right.source_span.line,
                    right.source_span.start,
                    right.block.as_str(),
                    right.id.as_str(),
                ))
        });
    }

    for (function, suspensions) in suspensions_by_function {
        let Some(expected_by_await) = expected_by_function.get(function) else {
            continue;
        };
        for (index, suspension) in suspensions.iter().enumerate() {
            let Some(expected_locals) = expected_by_await.get(index) else {
                continue;
            };
            for binding in expected_locals {
                if !actual.contains(&(suspension.id.as_str(), binding.as_str())) {
                    return Err(VerificationError::AsyncEvidenceMismatch {
                        field: "future_state_locals".to_owned(),
                        id: binding.clone(),
                    });
                }
            }
        }
    }
    Ok(())
}

fn verify_select_paths(certificate: &ProofCertificate) -> Result<(), VerificationError> {
    let block_envs = block_obligation_envs(certificate)?;
    let mut branch_edges = BTreeSet::<(&str, &str, &str)>::new();
    for edge in &certificate.core.cfg_edges {
        if edge.kind == "branch" && edge.to.starts_with("bb") {
            branch_edges.insert((edge.function.as_str(), edge.from.as_str(), edge.to.as_str()));
        }
    }
    let paths = certificate
        .core
        .async_model
        .select_paths
        .iter()
        .collect::<Vec<_>>();
    for (function, block, target) in branch_edges {
        for path_kind in ["winner", "loser_cancel"] {
            let Some(path) = paths.iter().copied().find(|path| {
                path.function == function
                    && path.branch_block == block
                    && path.branch_target == target
                    && path.path_kind == path_kind
            }) else {
                return Err(VerificationError::MissingSelectPathEvidence {
                    block: block.to_owned(),
                    path_kind: path_kind.to_owned(),
                });
            };
            let Some(expected_results) = block_envs.get(target) else {
                return Err(VerificationError::AsyncEvidenceMismatch {
                    field: "select_paths.branch_target".to_owned(),
                    id: path.id.clone(),
                });
            };
            let expected_cancelled = if path_kind == "loser_cancel" {
                block_envs
                    .get(block)
                    .map(|env| {
                        env.iter()
                            .filter(|(_, state)| **state == ObligationStatus::Owned)
                            .map(|(binding, state)| crate::ObligationState {
                                binding: binding.clone(),
                                state: state.clone(),
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            verify_select_path_results(path, expected_results, &expected_cancelled)?;
        }
    }
    Ok(())
}

fn verify_timeout_cancel_edges(
    certificate: &ProofCertificate,
    suspensions: &BTreeMap<&str, &SuspensionStateEvidence>,
) -> Result<(), VerificationError> {
    let timeout_by_suspension = certificate
        .core
        .async_model
        .timeout_cancel_edges
        .iter()
        .map(|edge| edge.suspension_state.as_str())
        .collect::<BTreeSet<_>>();
    for suspension in suspensions.values() {
        if suspension.boundary.as_deref() == Some("tokio::time::timeout")
            && !timeout_by_suspension.contains(suspension.id.as_str())
        {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "timeout_cancel_edges".to_owned(),
                id: suspension.id.clone(),
            });
        }
    }

    let cancel_evidence = certificate
        .core
        .async_model
        .cancel_edges
        .iter()
        .map(cancel_edge_key)
        .collect::<BTreeSet<_>>();
    for timeout in &certificate.core.async_model.timeout_cancel_edges {
        let Some(suspension) = suspensions.get(timeout.suspension_state.as_str()) else {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "timeout_cancel_edges".to_owned(),
                id: timeout.id.clone(),
            });
        };
        if timeout.source != "tokio::time::timeout"
            || timeout.cancel_edge != suspension.cancel_edge
            || !cancel_evidence.contains(&(
                suspension.function.as_str(),
                suspension.block.as_str(),
                timeout.cancel_edge.as_str(),
            ))
        {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "timeout_cancel_edges".to_owned(),
                id: timeout.id.clone(),
            });
        }
    }
    Ok(())
}

fn verify_spawned_task_obligations(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    let exit_env = event_state_map(&certificate.exit_env);
    for task in &certificate.core.async_model.spawned_task_obligations {
        let has_resolution_policy = task.required_resolution.iter().any(|resolution| {
            matches!(
                resolution.as_str(),
                "await" | "join" | "abort" | "detach" | "detach-with-policy" | "transfer"
            )
        });
        if !has_resolution_policy {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "spawned_task_obligations".to_owned(),
                id: task.binding.clone(),
            });
        }
        let has_resolution_event = certificate.obligation_events.iter().any(|event| {
            event.binding.as_deref() == Some(task.binding.as_str())
                && matches!(
                    event.kind,
                    ObligationEventKind::Discharge | ObligationEventKind::Transfer
                )
                && event.action.as_deref().is_some_and(|action| {
                    matches!(
                        action,
                        "await" | "join" | "abort" | "detach" | "detach-with-policy" | "transfer"
                    )
                })
        });
        if !has_resolution_event {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "spawned_task_obligations.resolution_event".to_owned(),
                id: task.binding.clone(),
            });
        }
        if exit_env
            .get(&task.binding)
            .is_some_and(ObligationStatus::is_unresolved_exit)
        {
            return Err(VerificationError::UnresolvedExitObligation {
                binding: task.binding.clone(),
            });
        }
    }
    Ok(())
}

fn verify_select_path_results(
    path: &SelectPathEvidence,
    expected_results: &BTreeMap<String, ObligationStatus>,
    expected_cancelled: &[crate::ObligationState],
) -> Result<(), VerificationError> {
    if event_state_map(&path.obligation_results) != *expected_results {
        return Err(VerificationError::AsyncEvidenceMismatch {
            field: "select_paths.obligation_results".to_owned(),
            id: path.id.clone(),
        });
    }
    if path.cancelled_obligations != expected_cancelled {
        return Err(VerificationError::AsyncEvidenceMismatch {
            field: "select_paths.cancelled_obligations".to_owned(),
            id: path.id.clone(),
        });
    }
    let observed = canonical_select_result_hash(
        &path.branch_target,
        &path.path_kind,
        &path.obligation_results,
        &path.cancelled_obligations,
    );
    if observed != path.obligation_result_hash {
        return Err(VerificationError::AsyncEvidenceMismatch {
            field: "select_paths.obligation_result_hash".to_owned(),
            id: path.id.clone(),
        });
    }
    Ok(())
}

fn canonical_select_result_hash(
    branch_target: &str,
    path_kind: &str,
    states: &[crate::ObligationState],
    cancelled_obligations: &[crate::ObligationState],
) -> String {
    let mut statuses = states
        .iter()
        .map(|state| format!("{}:{}", state.binding, state.state.as_str()))
        .collect::<Vec<_>>();
    statuses.sort_unstable();
    let mut cancelled = cancelled_obligations
        .iter()
        .map(|state| format!("{}:{}", state.binding, state.state.as_str()))
        .collect::<Vec<_>>();
    cancelled.sort_unstable();
    stable_hash(&format!(
        "select-result:{branch_target}:{path_kind}:{statuses:?}:cancelled:{cancelled:?}"
    ))
}

fn await_edges<'a>(certificate: &'a ProofCertificate) -> Vec<(String, String, BTreeSet<&'a str>)> {
    let mut grouped = BTreeMap::<(String, String), BTreeSet<&'a str>>::new();
    for edge in &certificate.core.cfg_edges {
        if edge.kind == "await" {
            grouped
                .entry((edge.function.clone(), edge.from.clone()))
                .or_default()
                .insert(edge.to.as_str());
        }
    }
    grouped
        .into_iter()
        .map(|((function, block), targets)| (function, block, targets))
        .collect()
}

fn suspensions_by_id(certificate: &ProofCertificate) -> BTreeMap<&str, &SuspensionStateEvidence> {
    certificate
        .core
        .async_model
        .suspension_states
        .iter()
        .map(|state| (state.id.as_str(), state))
        .collect()
}

fn cancel_edge_key(edge: &CancelEdgeEvidence) -> (&str, &str, &str) {
    (edge.function.as_str(), edge.from.as_str(), edge.to.as_str())
}

fn parsed_live_locals_by_function(source: &str) -> BTreeMap<String, Vec<BTreeSet<String>>> {
    let Ok(file) = syn::parse_file(source) else {
        return BTreeMap::new();
    };
    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Fn(function) => Some((
                function.sig.ident.to_string(),
                parsed_live_locals_by_await(function),
            )),
            _ => None,
        })
        .filter(|(_, locals_by_await)| !locals_by_await.is_empty())
        .collect()
}

fn parsed_live_locals_by_await(function: &syn::ItemFn) -> Vec<BTreeSet<String>> {
    let mut locals_before_statement = Vec::<BTreeSet<String>>::new();
    let mut declared = BTreeSet::<String>::new();
    for statement in &function.block.stmts {
        locals_before_statement.push(declared.clone());
        collect_pat_bindings_in_stmt(statement, &mut declared);
    }

    let mut later_statement_uses = vec![BTreeSet::<String>::new(); function.block.stmts.len()];
    let mut suffix_uses = BTreeSet::<String>::new();
    for (index, statement) in function.block.stmts.iter().enumerate().rev() {
        later_statement_uses[index] = suffix_uses.clone();
        collect_ident_uses_in_stmt(statement, &mut suffix_uses);
    }

    function
        .block
        .stmts
        .iter()
        .enumerate()
        .flat_map(|(index, statement)| {
            let mut declared_before = locals_before_statement
                .get(index)
                .cloned()
                .unwrap_or_default();
            collect_pre_await_pat_bindings_in_stmt(statement, &mut declared_before);
            await_live_uses_in_stmt(statement, &later_statement_uses[index])
                .into_iter()
                .map(move |after_await_uses| {
                    declared_before
                        .iter()
                        .filter(|binding| !binding.starts_with('_'))
                        .filter(|binding| after_await_uses.contains(*binding))
                        .cloned()
                        .collect::<BTreeSet<_>>()
                })
        })
        .collect()
}

fn await_live_uses_in_stmt(
    statement: &syn::Stmt,
    later_statement_uses: &BTreeSet<String>,
) -> Vec<BTreeSet<String>> {
    match statement {
        syn::Stmt::Local(local) => local
            .init
            .as_ref()
            .map(|init| await_live_uses_in_expr(init.expr.as_ref(), later_statement_uses).0)
            .unwrap_or_default(),
        syn::Stmt::Expr(expr, _) => await_live_uses_in_expr(expr, later_statement_uses).0,
        syn::Stmt::Macro(_) | syn::Stmt::Item(_) => Vec::new(),
    }
}

fn await_live_uses_in_expr(
    expr: &syn::Expr,
    later_uses: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    match expr {
        syn::Expr::Await(await_expr) => {
            let (mut awaits, uses) = await_live_uses_in_expr(await_expr.base.as_ref(), later_uses);
            awaits.push(later_uses.clone());
            (awaits, uses)
        }
        syn::Expr::Array(array) => await_live_uses_in_expr_sequence(array.elems.iter(), later_uses),
        syn::Expr::Assign(assign) => await_live_uses_in_expr_sequence(
            [assign.left.as_ref(), assign.right.as_ref()].into_iter(),
            later_uses,
        ),
        syn::Expr::Binary(binary) => await_live_uses_in_expr_sequence(
            [binary.left.as_ref(), binary.right.as_ref()].into_iter(),
            later_uses,
        ),
        syn::Expr::Call(call) => await_live_uses_in_expr_sequence(
            std::iter::once(call.func.as_ref()).chain(call.args.iter()),
            later_uses,
        ),
        syn::Expr::Block(block) => await_live_uses_in_block(&block.block, later_uses),
        syn::Expr::Cast(cast) => await_live_uses_in_expr(cast.expr.as_ref(), later_uses),
        syn::Expr::Closure(closure) => await_live_uses_in_expr(closure.body.as_ref(), later_uses),
        syn::Expr::Field(field) => await_live_uses_in_expr(field.base.as_ref(), later_uses),
        syn::Expr::Group(group) => await_live_uses_in_expr(group.expr.as_ref(), later_uses),
        syn::Expr::If(expr_if) => await_live_uses_in_if(expr_if, later_uses),
        syn::Expr::Index(index) => await_live_uses_in_expr_sequence(
            [index.expr.as_ref(), index.index.as_ref()].into_iter(),
            later_uses,
        ),
        syn::Expr::MethodCall(call) => await_live_uses_in_expr_sequence(
            std::iter::once(call.receiver.as_ref()).chain(call.args.iter()),
            later_uses,
        ),
        syn::Expr::Paren(paren) => await_live_uses_in_expr(paren.expr.as_ref(), later_uses),
        syn::Expr::Range(range) => {
            let children = range
                .start
                .iter()
                .chain(range.end.iter())
                .map(|expr| expr.as_ref());
            await_live_uses_in_expr_sequence(children, later_uses)
        }
        syn::Expr::Reference(reference) => {
            await_live_uses_in_expr(reference.expr.as_ref(), later_uses)
        }
        syn::Expr::Repeat(repeat) => await_live_uses_in_expr_sequence(
            [repeat.expr.as_ref(), repeat.len.as_ref()].into_iter(),
            later_uses,
        ),
        syn::Expr::Struct(expr_struct) => {
            let children = expr_struct
                .fields
                .iter()
                .map(|field| &field.expr)
                .chain(expr_struct.rest.iter().map(|expr| expr.as_ref()));
            await_live_uses_in_expr_sequence(children, later_uses)
        }
        syn::Expr::Tuple(tuple) => await_live_uses_in_expr_sequence(tuple.elems.iter(), later_uses),
        syn::Expr::Unary(unary) => await_live_uses_in_expr(unary.expr.as_ref(), later_uses),
        _ => {
            let uses = ident_uses_in_expr(expr);
            (vec![later_uses.clone(); expr_await_count(expr)], uses)
        }
    }
}

fn await_live_uses_in_block(
    block: &syn::Block,
    later_uses: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    let mut later_statement_uses = vec![BTreeSet::<String>::new(); block.stmts.len()];
    let mut suffix_uses = later_uses.clone();
    for (index, statement) in block.stmts.iter().enumerate().rev() {
        later_statement_uses[index] = suffix_uses.clone();
        collect_ident_uses_in_stmt(statement, &mut suffix_uses);
    }
    let mut awaits = Vec::new();
    let mut uses = BTreeSet::new();
    for (index, statement) in block.stmts.iter().enumerate() {
        awaits.extend(await_live_uses_in_stmt(
            statement,
            &later_statement_uses[index],
        ));
        collect_ident_uses_in_stmt(statement, &mut uses);
    }
    (awaits, uses)
}

fn await_live_uses_in_if(
    expr_if: &syn::ExprIf,
    later_uses: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    let (then_awaits, then_uses) = await_live_uses_in_block(&expr_if.then_branch, later_uses);
    let (else_awaits, else_uses) = expr_if
        .else_branch
        .as_ref()
        .map(|(_, else_expr)| await_live_uses_in_expr(else_expr.as_ref(), later_uses))
        .unwrap_or_default();
    let mut condition_later_uses = later_uses.clone();
    condition_later_uses.extend(then_uses.iter().cloned());
    condition_later_uses.extend(else_uses.iter().cloned());
    let (condition_awaits, condition_uses) =
        await_live_uses_in_expr(expr_if.cond.as_ref(), &condition_later_uses);
    let mut awaits = condition_awaits;
    awaits.extend(then_awaits);
    awaits.extend(else_awaits);
    let mut uses = condition_uses;
    uses.extend(then_uses);
    uses.extend(else_uses);
    (awaits, uses)
}

fn await_live_uses_in_expr_sequence<'a>(
    children: impl Iterator<Item = &'a syn::Expr>,
    later_uses: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    let children = children.collect::<Vec<_>>();
    let mut suffix = later_uses.clone();
    let mut awaits_reversed = Vec::<BTreeSet<String>>::new();
    let mut uses = BTreeSet::<String>::new();
    for child in children.into_iter().rev() {
        let (child_awaits, child_uses) = await_live_uses_in_expr(child, &suffix);
        suffix.extend(child_uses.iter().cloned());
        uses.extend(child_uses);
        awaits_reversed.extend(child_awaits.into_iter().rev());
    }
    awaits_reversed.reverse();
    (awaits_reversed, uses)
}

fn ident_uses_in_expr(expr: &syn::Expr) -> BTreeSet<String> {
    let mut uses = BTreeSet::new();
    collect_ident_uses_in_expr(expr, &mut uses);
    uses
}

fn expr_await_count(expr: &syn::Expr) -> usize {
    struct AwaitVisitor {
        count: usize,
    }

    impl<'ast> syn::visit::Visit<'ast> for AwaitVisitor {
        fn visit_expr_await(&mut self, expr: &'ast syn::ExprAwait) {
            self.count += 1;
            syn::visit::visit_expr_await(self, expr);
        }
    }

    let mut visitor = AwaitVisitor { count: 0 };
    syn::visit::visit_expr(&mut visitor, expr);
    visitor.count
}

fn collect_pat_bindings_in_stmt(statement: &syn::Stmt, bindings: &mut BTreeSet<String>) {
    let syn::Stmt::Local(local) = statement else {
        return;
    };
    collect_pat_bindings(&local.pat, bindings);
}

fn collect_pre_await_pat_bindings_in_stmt(
    statement: &syn::Stmt,
    bindings: &mut BTreeSet<String>,
) -> bool {
    match statement {
        syn::Stmt::Local(local) => {
            if local.init.as_ref().is_some_and(|init| {
                collect_pre_await_pat_bindings_in_expr(init.expr.as_ref(), bindings)
            }) {
                return true;
            }
            collect_pat_bindings(&local.pat, bindings);
            false
        }
        syn::Stmt::Expr(expr, _) => collect_pre_await_pat_bindings_in_expr(expr, bindings),
        syn::Stmt::Macro(_) | syn::Stmt::Item(_) => false,
    }
}

fn collect_pre_await_pat_bindings_in_expr(
    expr: &syn::Expr,
    bindings: &mut BTreeSet<String>,
) -> bool {
    match expr {
        syn::Expr::Await(_) => true,
        syn::Expr::Block(block) => block
            .block
            .stmts
            .iter()
            .any(|statement| collect_pre_await_pat_bindings_in_stmt(statement, bindings)),
        syn::Expr::If(expr_if) => {
            collect_pre_await_pat_bindings_in_expr(expr_if.cond.as_ref(), bindings)
                || expr_if
                    .then_branch
                    .stmts
                    .iter()
                    .any(|statement| collect_pre_await_pat_bindings_in_stmt(statement, bindings))
                || expr_if.else_branch.as_ref().is_some_and(|(_, else_expr)| {
                    collect_pre_await_pat_bindings_in_expr(else_expr.as_ref(), bindings)
                })
        }
        syn::Expr::Tuple(tuple) => tuple
            .elems
            .iter()
            .any(|expr| collect_pre_await_pat_bindings_in_expr(expr, bindings)),
        syn::Expr::Array(array) => array
            .elems
            .iter()
            .any(|expr| collect_pre_await_pat_bindings_in_expr(expr, bindings)),
        syn::Expr::Call(call) => std::iter::once(call.func.as_ref())
            .chain(call.args.iter())
            .any(|expr| collect_pre_await_pat_bindings_in_expr(expr, bindings)),
        syn::Expr::MethodCall(call) => std::iter::once(call.receiver.as_ref())
            .chain(call.args.iter())
            .any(|expr| collect_pre_await_pat_bindings_in_expr(expr, bindings)),
        syn::Expr::Binary(binary) => {
            collect_pre_await_pat_bindings_in_expr(binary.left.as_ref(), bindings)
                || collect_pre_await_pat_bindings_in_expr(binary.right.as_ref(), bindings)
        }
        syn::Expr::Paren(paren) => {
            collect_pre_await_pat_bindings_in_expr(paren.expr.as_ref(), bindings)
        }
        syn::Expr::Group(group) => {
            collect_pre_await_pat_bindings_in_expr(group.expr.as_ref(), bindings)
        }
        syn::Expr::Reference(reference) => {
            collect_pre_await_pat_bindings_in_expr(reference.expr.as_ref(), bindings)
        }
        syn::Expr::Unary(unary) => {
            collect_pre_await_pat_bindings_in_expr(unary.expr.as_ref(), bindings)
        }
        _ => false,
    }
}

fn collect_pat_bindings(pattern: &syn::Pat, bindings: &mut BTreeSet<String>) {
    match pattern {
        syn::Pat::Ident(ident) => {
            bindings.insert(ident.ident.to_string());
        }
        syn::Pat::Tuple(tuple) => {
            for element in &tuple.elems {
                collect_pat_bindings(element, bindings);
            }
        }
        syn::Pat::Struct(pattern) => {
            for field in &pattern.fields {
                collect_pat_bindings(&field.pat, bindings);
            }
        }
        syn::Pat::TupleStruct(pattern) => {
            for element in &pattern.elems {
                collect_pat_bindings(element, bindings);
            }
        }
        syn::Pat::Slice(pattern) => {
            for element in &pattern.elems {
                collect_pat_bindings(element, bindings);
            }
        }
        syn::Pat::Reference(pattern) => collect_pat_bindings(&pattern.pat, bindings),
        syn::Pat::Type(pattern) => collect_pat_bindings(&pattern.pat, bindings),
        syn::Pat::Or(pattern) => {
            for case in &pattern.cases {
                collect_pat_bindings(case, bindings);
            }
        }
        _ => {}
    }
}

fn collect_ident_uses_in_stmt(statement: &syn::Stmt, uses: &mut BTreeSet<String>) {
    struct UseVisitor<'a> {
        uses: &'a mut BTreeSet<String>,
    }

    impl<'a, 'ast> syn::visit::Visit<'ast> for UseVisitor<'a> {
        fn visit_expr_path(&mut self, expr: &'ast syn::ExprPath) {
            if expr.qself.is_none() && expr.path.segments.len() == 1 {
                if let Some(segment) = expr.path.segments.first() {
                    self.uses.insert(segment.ident.to_string());
                }
            }
            syn::visit::visit_expr_path(self, expr);
        }
    }

    let mut visitor = UseVisitor { uses };
    syn::visit::visit_stmt(&mut visitor, statement);
}

fn collect_ident_uses_in_expr(expr: &syn::Expr, uses: &mut BTreeSet<String>) {
    struct UseVisitor<'a> {
        uses: &'a mut BTreeSet<String>,
    }

    impl<'a, 'ast> syn::visit::Visit<'ast> for UseVisitor<'a> {
        fn visit_expr_path(&mut self, expr: &'ast syn::ExprPath) {
            if expr.qself.is_none() && expr.path.segments.len() == 1 {
                if let Some(segment) = expr.path.segments.first() {
                    self.uses.insert(segment.ident.to_string());
                }
            }
            syn::visit::visit_expr_path(self, expr);
        }
    }

    let mut visitor = UseVisitor { uses };
    syn::visit::visit_expr(&mut visitor, expr);
}
