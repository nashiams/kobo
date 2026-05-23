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

    for suspension in suspensions.values() {
        let Some(expected_locals) = expected_by_function.get(suspension.function.as_str()) else {
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
            verify_select_path_results(path, expected_results)?;
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
) -> Result<(), VerificationError> {
    if event_state_map(&path.obligation_results) != *expected_results {
        return Err(VerificationError::AsyncEvidenceMismatch {
            field: "select_paths.obligation_results".to_owned(),
            id: path.id.clone(),
        });
    }
    let observed = canonical_select_result_hash(
        &path.branch_target,
        &path.path_kind,
        &path.obligation_results,
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
) -> String {
    let mut statuses = states
        .iter()
        .map(|state| format!("{}:{}", state.binding, state.state.as_str()))
        .collect::<Vec<_>>();
    statuses.sort_unstable();
    stable_hash(&format!(
        "select-result:{branch_target}:{path_kind}:{statuses:?}"
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

fn parsed_live_locals_by_function(source: &str) -> BTreeMap<String, BTreeSet<String>> {
    let Ok(file) = syn::parse_file(source) else {
        return BTreeMap::new();
    };
    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Fn(function) => Some((
                function.sig.ident.to_string(),
                parsed_live_locals_across_first_await(function),
            )),
            _ => None,
        })
        .filter(|(_, locals)| !locals.is_empty())
        .collect()
}

fn parsed_live_locals_across_first_await(function: &syn::ItemFn) -> BTreeSet<String> {
    let mut before_await = BTreeSet::<String>::new();
    let mut after_await_uses = BTreeSet::<String>::new();
    let mut seen_await = false;
    for statement in &function.block.stmts {
        if seen_await {
            collect_ident_uses_in_stmt(statement, &mut after_await_uses);
            continue;
        }
        if stmt_contains_await(statement) {
            seen_await = true;
            continue;
        }
        collect_pat_bindings_in_stmt(statement, &mut before_await);
    }

    before_await
        .into_iter()
        .filter(|binding| !binding.starts_with('_'))
        .filter(|binding| after_await_uses.contains(binding))
        .collect()
}

fn stmt_contains_await(statement: &syn::Stmt) -> bool {
    struct AwaitVisitor {
        found: bool,
    }

    impl<'ast> syn::visit::Visit<'ast> for AwaitVisitor {
        fn visit_expr_await(&mut self, expr: &'ast syn::ExprAwait) {
            self.found = true;
            syn::visit::visit_expr_await(self, expr);
        }
    }

    let mut visitor = AwaitVisitor { found: false };
    syn::visit::visit_stmt(&mut visitor, statement);
    visitor.found
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
