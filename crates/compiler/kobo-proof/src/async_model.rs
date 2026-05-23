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
    let skippable_lifecycle_method_awaits =
        skippable_lifecycle_method_initializer_awaits_by_function(source);
    let mut actual = BTreeSet::<(&str, &str)>::new();
    for local in &certificate.core.async_model.future_state_locals {
        if !actual.insert((local.suspension_state.as_str(), local.binding.as_str())) {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "future_state_locals".to_owned(),
                id: format!("{}:{}", local.suspension_state, local.binding),
            });
        }
    }

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

    let summary_functions = certificate
        .function_summaries
        .iter()
        .map(|summary| summary.function.as_str())
        .collect::<BTreeSet<_>>();
    let modeled_functions = certificate
        .core
        .cfg_nodes
        .iter()
        .map(|node| node.function.as_str())
        .chain(suspensions_by_function.keys().copied())
        .collect::<BTreeSet<_>>();
    for function in &modeled_functions {
        if !summary_functions.contains(function) {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "function_summaries".to_owned(),
                id: (*function).to_owned(),
            });
        }
    }

    for function in modeled_functions {
        let expected_by_await = expected_by_function
            .get(function)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let actual_count = suspensions_by_function
            .get(function)
            .map(Vec::len)
            .unwrap_or_default();
        let skippable_count = skippable_lifecycle_method_awaits
            .get(function)
            .copied()
            .unwrap_or_default();
        let minimum_expected = expected_by_await.len().saturating_sub(skippable_count);
        if actual_count > expected_by_await.len() || actual_count < minimum_expected {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "suspension_states".to_owned(),
                id: function.to_owned(),
            });
        }
    }

    let mut expected = BTreeSet::<(String, String)>::new();
    for (function, suspensions) in suspensions_by_function {
        let Some(expected_by_await) = expected_by_function.get(function) else {
            continue;
        };
        for (index, suspension) in suspensions.iter().enumerate() {
            let Some(expected_locals) = expected_by_await.get(index) else {
                continue;
            };
            for binding in expected_locals {
                expected.insert((suspension.id.clone(), binding.clone()));
            }
        }
    }
    for (suspension_state, binding) in &expected {
        if !actual.contains(&(suspension_state.as_str(), binding.as_str())) {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "future_state_locals".to_owned(),
                id: binding.clone(),
            });
        }
    }
    for (suspension_state, binding) in actual {
        if !expected.contains(&(suspension_state.to_owned(), binding.to_owned())) {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "future_state_locals".to_owned(),
                id: format!("{suspension_state}:{binding}"),
            });
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

fn skippable_lifecycle_method_initializer_awaits_by_function(
    source: &str,
) -> BTreeMap<String, usize> {
    let Ok(file) = syn::parse_file(source) else {
        return BTreeMap::new();
    };
    let lifecycle_types = lifecycle_like_types(&file);
    let method_returns = method_return_types(&file);
    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Fn(function) => Some((
                function.sig.ident.to_string(),
                skippable_lifecycle_method_initializer_awaits(
                    function,
                    &method_returns,
                    &lifecycle_types,
                ),
            )),
            _ => None,
        })
        .filter(|(_, count)| *count > 0)
        .collect()
}

fn lifecycle_like_types(file: &syn::File) -> BTreeSet<String> {
    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(item_impl) => type_path_last(item_impl.self_ty.as_ref()).map(|name| {
                let has_lifecycle_action = item_impl.items.iter().any(|item| {
                    matches!(
                        item,
                        syn::ImplItem::Fn(method)
                            if matches!(
                                method.sig.ident.to_string().as_str(),
                                "ack" | "nack" | "requeue" | "close" | "commit" | "rollback"
                            )
                    )
                });
                (name, has_lifecycle_action)
            }),
            _ => None,
        })
        .filter_map(|(name, has_lifecycle_action)| has_lifecycle_action.then_some(name))
        .collect()
}

fn method_return_types(file: &syn::File) -> BTreeMap<String, String> {
    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(item_impl) => Some(item_impl),
            _ => None,
        })
        .flat_map(|item_impl| item_impl.items.iter())
        .filter_map(|item| match item {
            syn::ImplItem::Fn(method) => return_type_name(&method.sig.output)
                .map(|return_type| (method.sig.ident.to_string(), return_type)),
            _ => None,
        })
        .collect()
}

fn return_type_name(output: &syn::ReturnType) -> Option<String> {
    let syn::ReturnType::Type(_, ty) = output else {
        return None;
    };
    type_path_last(ty.as_ref())
}

fn type_path_last(ty: &syn::Type) -> Option<String> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    path.path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

fn skippable_lifecycle_method_initializer_awaits(
    function: &syn::ItemFn,
    method_returns: &BTreeMap<String, String>,
    lifecycle_types: &BTreeSet<String>,
) -> usize {
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
        .filter_map(|(index, statement)| {
            let syn::Stmt::Local(local) = statement else {
                return None;
            };
            let init = local.init.as_ref()?;
            let method = awaited_initializer_method(init.expr.as_ref())?;
            let return_type = method_returns.get(&method)?;
            lifecycle_types.contains(return_type).then(|| {
                let declared_before = locals_before_statement
                    .get(index)
                    .cloned()
                    .unwrap_or_default();
                await_live_locals_in_expr(
                    init.expr.as_ref(),
                    &later_statement_uses[index],
                    &declared_before,
                )
                .0
            })
        })
        .filter(|locals_by_await| locals_by_await.iter().all(BTreeSet::is_empty))
        .map(|locals_by_await| locals_by_await.len())
        .sum()
}

fn awaited_initializer_method(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Await(await_expr) => match await_expr.base.as_ref() {
            syn::Expr::MethodCall(call) => Some(call.method.to_string()),
            _ => None,
        },
        syn::Expr::Group(group) => awaited_initializer_method(group.expr.as_ref()),
        syn::Expr::Paren(paren) => awaited_initializer_method(paren.expr.as_ref()),
        _ => None,
    }
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
            let declared_before = locals_before_statement
                .get(index)
                .cloned()
                .unwrap_or_default();
            await_live_locals_in_stmt(statement, &later_statement_uses[index], &declared_before)
                .into_iter()
        })
        .collect()
}

fn await_live_locals_in_stmt(
    statement: &syn::Stmt,
    later_statement_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> Vec<BTreeSet<String>> {
    match statement {
        syn::Stmt::Local(local) => local
            .init
            .as_ref()
            .map(|init| {
                await_live_locals_in_expr(init.expr.as_ref(), later_statement_uses, declared_before)
                    .0
            })
            .unwrap_or_default(),
        syn::Stmt::Expr(expr, _) => {
            await_live_locals_in_expr(expr, later_statement_uses, declared_before).0
        }
        syn::Stmt::Macro(_) | syn::Stmt::Item(_) => Vec::new(),
    }
}

fn await_live_locals_in_expr(
    expr: &syn::Expr,
    later_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    match expr {
        syn::Expr::Await(await_expr) => {
            let (mut awaits, uses) =
                await_live_locals_in_expr(await_expr.base.as_ref(), later_uses, declared_before);
            awaits.push(live_declared_locals(later_uses, declared_before));
            (awaits, uses)
        }
        syn::Expr::Array(array) => {
            await_live_locals_in_expr_sequence(array.elems.iter(), later_uses, declared_before)
        }
        syn::Expr::Assign(assign) => await_live_locals_in_expr_sequence(
            [assign.left.as_ref(), assign.right.as_ref()].into_iter(),
            later_uses,
            declared_before,
        ),
        syn::Expr::Binary(binary) => await_live_locals_in_expr_sequence(
            [binary.left.as_ref(), binary.right.as_ref()].into_iter(),
            later_uses,
            declared_before,
        ),
        syn::Expr::Block(block) => {
            await_live_locals_in_block(&block.block, later_uses, declared_before)
        }
        syn::Expr::Break(expr_break) => expr_break
            .expr
            .as_deref()
            .map(|value| await_live_locals_in_expr(value, later_uses, declared_before))
            .unwrap_or_default(),
        syn::Expr::Call(call) => await_live_locals_in_expr_sequence(
            std::iter::once(call.func.as_ref()).chain(call.args.iter()),
            later_uses,
            declared_before,
        ),
        syn::Expr::Cast(cast) => {
            await_live_locals_in_expr(cast.expr.as_ref(), later_uses, declared_before)
        }
        syn::Expr::Closure(closure) => {
            await_live_locals_in_expr(closure.body.as_ref(), later_uses, declared_before)
        }
        syn::Expr::Field(field) => {
            await_live_locals_in_expr(field.base.as_ref(), later_uses, declared_before)
        }
        syn::Expr::ForLoop(expr_for) => {
            let mut body_declared = declared_before.clone();
            collect_pat_bindings(&expr_for.pat, &mut body_declared);
            let (body_awaits, body_uses) =
                await_live_locals_in_block(&expr_for.body, later_uses, &body_declared);
            let mut iter_later_uses = later_uses.clone();
            iter_later_uses.extend(body_uses.iter().cloned());
            let (mut awaits, iter_uses) = await_live_locals_in_expr(
                expr_for.expr.as_ref(),
                &iter_later_uses,
                declared_before,
            );
            awaits.extend(body_awaits);
            let mut uses = iter_uses;
            uses.extend(body_uses);
            (awaits, uses)
        }
        syn::Expr::Group(group) => {
            await_live_locals_in_expr(group.expr.as_ref(), later_uses, declared_before)
        }
        syn::Expr::If(expr_if) => await_live_locals_in_if(expr_if, later_uses, declared_before),
        syn::Expr::Index(index) => await_live_locals_in_expr_sequence(
            [index.expr.as_ref(), index.index.as_ref()].into_iter(),
            later_uses,
            declared_before,
        ),
        syn::Expr::Let(expr_let) => {
            await_live_locals_in_expr(expr_let.expr.as_ref(), later_uses, declared_before)
        }
        syn::Expr::Match(expr_match) => {
            await_live_locals_in_match(expr_match, later_uses, declared_before)
        }
        syn::Expr::MethodCall(call) => await_live_locals_in_expr_sequence(
            std::iter::once(call.receiver.as_ref()).chain(call.args.iter()),
            later_uses,
            declared_before,
        ),
        syn::Expr::Paren(paren) => {
            await_live_locals_in_expr(paren.expr.as_ref(), later_uses, declared_before)
        }
        syn::Expr::Range(range) => {
            let children = range
                .start
                .iter()
                .chain(range.end.iter())
                .map(|expr| expr.as_ref());
            await_live_locals_in_expr_sequence(children, later_uses, declared_before)
        }
        syn::Expr::Reference(reference) => {
            await_live_locals_in_expr(reference.expr.as_ref(), later_uses, declared_before)
        }
        syn::Expr::Repeat(repeat) => await_live_locals_in_expr_sequence(
            [repeat.expr.as_ref(), repeat.len.as_ref()].into_iter(),
            later_uses,
            declared_before,
        ),
        syn::Expr::Return(expr_return) => expr_return
            .expr
            .as_deref()
            .map(|value| await_live_locals_in_expr(value, later_uses, declared_before))
            .unwrap_or_default(),
        syn::Expr::Struct(expr_struct) => {
            let children = expr_struct
                .fields
                .iter()
                .map(|field| &field.expr)
                .chain(expr_struct.rest.iter().map(|expr| expr.as_ref()));
            await_live_locals_in_expr_sequence(children, later_uses, declared_before)
        }
        syn::Expr::Try(expr_try) => {
            await_live_locals_in_expr(expr_try.expr.as_ref(), later_uses, declared_before)
        }
        syn::Expr::TryBlock(try_block) => {
            await_live_locals_in_block(&try_block.block, later_uses, declared_before)
        }
        syn::Expr::Tuple(tuple) => {
            await_live_locals_in_expr_sequence(tuple.elems.iter(), later_uses, declared_before)
        }
        syn::Expr::Unary(unary) => {
            await_live_locals_in_expr(unary.expr.as_ref(), later_uses, declared_before)
        }
        syn::Expr::Unsafe(expr_unsafe) => {
            await_live_locals_in_block(&expr_unsafe.block, later_uses, declared_before)
        }
        syn::Expr::While(expr_while) => {
            let (body_awaits, body_uses) =
                await_live_locals_in_block(&expr_while.body, later_uses, declared_before);
            let mut condition_later_uses = later_uses.clone();
            condition_later_uses.extend(body_uses.iter().cloned());
            let (mut awaits, condition_uses) = await_live_locals_in_expr(
                expr_while.cond.as_ref(),
                &condition_later_uses,
                declared_before,
            );
            awaits.extend(body_awaits);
            let mut uses = condition_uses;
            uses.extend(body_uses);
            (awaits, uses)
        }
        syn::Expr::Yield(expr_yield) => expr_yield
            .expr
            .as_deref()
            .map(|value| await_live_locals_in_expr(value, later_uses, declared_before))
            .unwrap_or_default(),
        _ => {
            let uses = ident_uses_in_expr(expr);
            (
                vec![live_declared_locals(later_uses, declared_before); expr_await_count(expr)],
                uses,
            )
        }
    }
}

fn live_declared_locals(
    live_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> BTreeSet<String> {
    declared_before
        .iter()
        .filter(|binding| !binding.starts_with('_'))
        .filter(|binding| live_uses.contains(*binding))
        .cloned()
        .collect()
}

fn await_live_locals_in_block(
    block: &syn::Block,
    later_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    let mut later_statement_uses = vec![BTreeSet::<String>::new(); block.stmts.len()];
    let mut suffix_uses = later_uses.clone();
    for (index, statement) in block.stmts.iter().enumerate().rev() {
        later_statement_uses[index] = suffix_uses.clone();
        collect_ident_uses_in_stmt(statement, &mut suffix_uses);
    }
    let mut declared = declared_before.clone();
    let mut awaits = Vec::new();
    let mut uses = BTreeSet::new();
    for (index, statement) in block.stmts.iter().enumerate() {
        awaits.extend(await_live_locals_in_stmt(
            statement,
            &later_statement_uses[index],
            &declared,
        ));
        collect_ident_uses_in_stmt(statement, &mut uses);
        collect_pat_bindings_in_stmt(statement, &mut declared);
    }
    (awaits, uses)
}

fn await_live_locals_in_if(
    expr_if: &syn::ExprIf,
    later_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    let (then_awaits, then_uses) =
        await_live_locals_in_block(&expr_if.then_branch, later_uses, declared_before);
    let (else_awaits, else_uses) = expr_if
        .else_branch
        .as_ref()
        .map(|(_, else_expr)| {
            await_live_locals_in_expr(else_expr.as_ref(), later_uses, declared_before)
        })
        .unwrap_or_default();
    let mut condition_later_uses = later_uses.clone();
    condition_later_uses.extend(then_uses.iter().cloned());
    condition_later_uses.extend(else_uses.iter().cloned());
    let (condition_awaits, condition_uses) = await_live_locals_in_expr(
        expr_if.cond.as_ref(),
        &condition_later_uses,
        declared_before,
    );
    let mut awaits = condition_awaits;
    awaits.extend(then_awaits);
    awaits.extend(else_awaits);
    let mut uses = condition_uses;
    uses.extend(then_uses);
    uses.extend(else_uses);
    (awaits, uses)
}

fn await_live_locals_in_match(
    expr_match: &syn::ExprMatch,
    later_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    let mut arm_awaits = Vec::new();
    let mut arm_uses = BTreeSet::new();
    for arm in &expr_match.arms {
        let mut arm_declared = declared_before.clone();
        collect_pat_bindings(&arm.pat, &mut arm_declared);
        let (body_awaits, body_uses) =
            await_live_locals_in_expr(arm.body.as_ref(), later_uses, &arm_declared);
        let mut guard_later_uses = later_uses.clone();
        guard_later_uses.extend(body_uses.iter().cloned());
        let (guard_awaits, guard_uses) = arm
            .guard
            .as_ref()
            .map(|(_, guard)| {
                await_live_locals_in_expr(guard.as_ref(), &guard_later_uses, &arm_declared)
            })
            .unwrap_or_default();
        arm_awaits.extend(guard_awaits);
        arm_awaits.extend(body_awaits);
        arm_uses.extend(guard_uses);
        arm_uses.extend(body_uses);
    }
    let mut scrutinee_later_uses = later_uses.clone();
    scrutinee_later_uses.extend(arm_uses.iter().cloned());
    let (mut awaits, scrutinee_uses) = await_live_locals_in_expr(
        expr_match.expr.as_ref(),
        &scrutinee_later_uses,
        declared_before,
    );
    awaits.extend(arm_awaits);
    let mut uses = scrutinee_uses;
    uses.extend(arm_uses);
    (awaits, uses)
}

fn await_live_locals_in_expr_sequence<'a>(
    children: impl Iterator<Item = &'a syn::Expr>,
    later_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    let children = children.collect::<Vec<_>>();
    let mut suffix = later_uses.clone();
    let mut awaits_reversed = Vec::<BTreeSet<String>>::new();
    let mut uses = BTreeSet::<String>::new();
    for child in children.into_iter().rev() {
        let (child_awaits, child_uses) = await_live_locals_in_expr(child, &suffix, declared_before);
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
