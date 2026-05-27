use super::parallel_gate::ParallelSafetyGate;
use super::receivers::{block_mutates_binding, iterator_source_ident};
use super::spawn_captures::collect_spawn_captures;
use super::*;

impl<'a> Lowerer<'a> {
    pub(super) fn lower_block_statements(
        &mut self,
        block: &mut syn::Block,
        scopes: &mut ScopeStack,
    ) {
        let mut index = 0usize;
        while index < block.stmts.len() {
            if self.try_materialize_iterator_mutation(block, index, scopes) {
                self.lower_stmt(&mut block.stmts[index], scopes);
                index += 1;
                continue;
            }

            if self.try_shrink_borrow_alias(block, index, scopes) {
                index += 1;
                continue;
            }

            self.lower_stmt(&mut block.stmts[index], scopes);
            index += 1;
        }
    }

    fn try_materialize_iterator_mutation(
        &mut self,
        block: &mut syn::Block,
        index: usize,
        _scopes: &mut ScopeStack,
    ) -> bool {
        let Some(syn::Stmt::Expr(syn::Expr::ForLoop(for_loop), _)) = block.stmts.get(index) else {
            return false;
        };
        let Some(source_ident) = iterator_source_ident(for_loop.expr.as_ref()) else {
            return false;
        };
        if !block_mutates_binding(&for_loop.body, &source_ident) {
            return false;
        }
        self.record_iterator_materialization_note(&source_ident);

        let snapshot_ident =
            quote::format_ident!("__kobo_iter_snapshot_{}", self.iter_snapshot_counter);
        self.iter_snapshot_counter += 1;
        let source_expr = (*for_loop.expr).clone();
        let snapshot_stmt: syn::Stmt = parse_quote! {
            let #snapshot_ident = #source_expr.cloned().collect::<Vec<_>>();
        };
        let mut materialized_loop = for_loop.clone();
        materialized_loop.expr = Box::new(parse_quote!(#snapshot_ident));
        let loop_stmt = syn::Stmt::Expr(syn::Expr::ForLoop(materialized_loop), None);
        let materialized_block = syn::Expr::Block(syn::ExprBlock {
            attrs: Vec::new(),
            label: None,
            block: syn::Block {
                brace_token: syn::token::Brace::default(),
                stmts: vec![snapshot_stmt, loop_stmt],
            },
        });
        block.stmts[index] = syn::Stmt::Expr(materialized_block, None);
        true
    }

    fn record_iterator_materialization_note(&mut self, source_ident: &syn::Ident) {
        let Some(binding) = self
            .ast
            .iter_bindings()
            .find(|binding| binding.ident == *source_ident)
        else {
            return;
        };
        let Some(node) = self.plan.node_for_binding(binding) else {
            return;
        };
        if self.annotation_notes.iter().any(|note| {
            note.node == node && note.reason.starts_with("iterator-materialization-debt")
        }) {
            return;
        }
        let (kobo_line, _) = self.ast.line_col(binding.span);
        self.annotation_notes.push(AnnotationNote {
            node,
            binding_name: binding.ident.to_string(),
            kobo_line,
            reason: "iterator-materialization-debt: materialized iter() snapshot before mutation"
                .to_owned(),
        });
    }

    pub(super) fn lower_nested_block(&mut self, block: &mut syn::Block, scopes: &mut ScopeStack) {
        scopes.push();
        self.lower_block_statements(block, scopes);
        scopes.pop();
    }

    fn lower_stmt(&mut self, stmt: &mut syn::Stmt, scopes: &mut ScopeStack) {
        match stmt {
            syn::Stmt::Local(local) => self.lower_local(local, scopes),
            syn::Stmt::Item(item) => self.lower_item(item),
            syn::Stmt::Expr(expr, semi) => {
                if let syn::Expr::ForLoop(for_loop) = expr {
                    let policy = parallel::policy_value(&for_loop.attrs)
                        .unwrap_or_else(|| "outside".to_owned());
                    let has_explicit_policy = parallel::policy_value(&for_loop.attrs).is_some();
                    let source_line = self.source_line_for_expr_for_loop(for_loop);
                    let safety_gate =
                        self.parallel_safety_gate(for_loop, scopes, has_explicit_policy);
                    match parallel::lower_for_loop(for_loop, safety_gate.accepted) {
                        parallel::ParallelLowering::Parallel => {
                            self.needs_rayon = true;
                            if has_explicit_policy {
                                parallel::mark_boundary_policy(for_loop, &policy);
                            }
                            self.lower_expr(for_loop.expr.as_mut(), scopes);
                            self.lower_nested_block(&mut for_loop.body, scopes);
                            self.parallel_evidence.push(self.parallel_loop_evidence(
                                for_loop,
                                scopes,
                                source_line,
                                "rayon-par-iter",
                                &policy,
                                safety_gate,
                            ));
                            *expr = parallel::for_each_adapter_expr(for_loop);
                            *semi = Some(syn::token::Semi::default());
                            return;
                        }
                        parallel::ParallelLowering::SerialPolicy => {
                            parallel::mark_serial_policy(for_loop);
                            self.parallel_evidence.push(self.parallel_loop_evidence(
                                for_loop,
                                scopes,
                                source_line,
                                "serial",
                                "serial-order",
                                ParallelSafetyGate::policy("policy-gate:serial-order"),
                            ));
                        }
                        parallel::ParallelLowering::BoundaryPolicy => {
                            parallel::mark_boundary_policy(for_loop, &policy);
                            self.parallel_evidence.push(self.parallel_loop_evidence(
                                for_loop,
                                scopes,
                                source_line,
                                "boundary-policy",
                                &policy,
                                ParallelSafetyGate::policy("policy-gate:ward-boundary"),
                            ));
                        }
                        parallel::ParallelLowering::SafetyBlocked => {
                            parallel::mark_safety_blocked(for_loop, &safety_gate.blockers);
                            self.parallel_evidence.push(self.parallel_loop_evidence(
                                for_loop,
                                scopes,
                                source_line,
                                "blocked-safety",
                                "safety-gate",
                                safety_gate,
                            ));
                        }
                        parallel::ParallelLowering::None => {}
                    }
                }
                if semi.is_none() && self.lower_owned_value_expr(expr, scopes) {
                    return;
                }
                self.lower_expr(expr, scopes);
            }
            syn::Stmt::Macro(stmt_macro) => {
                // Check for spawn block marker macro — wire clone injection.
                // S-53: Determine spawn strategy based on captured bindings' ownership tiers.
                if !spawn::is_any_spawn_block_macro(&stmt_macro.mac) {
                    self.lower_macro_tokens(&mut stmt_macro.mac.tokens, scopes);
                    return;
                }

                let captured = collect_spawn_captures(&stmt_macro.mac.tokens, scopes);
                let use_spawn_local = spawn::is_spawn_local_block_macro(&stmt_macro.mac);
                if use_spawn_local {
                    self.needs_local_set = true;
                    let captured_bindings = captured
                        .iter()
                        .map(|binding| format!("{}:{:?}", binding.name, binding.tier))
                        .collect::<Vec<_>>();
                    self.task_local_evidence.push(TaskLocalEvidence {
                        source_line: self.source_line_for_macro(&stmt_macro.mac),
                        strategy: "tokio-spawn-local-localset".to_owned(),
                        proof: if spawn::is_spawn_local_block_macro(&stmt_macro.mac) {
                            "explicit-spawn-local-zone".to_owned()
                        } else {
                            "non-send-capture-tier".to_owned()
                        },
                        captured_bindings,
                        safety_checks: vec![
                            "localset-scope".to_owned(),
                            "no-local-handle-escape".to_owned(),
                            "non-send-capture-contained".to_owned(),
                        ],
                    });
                }
                if let Some(spawn_expr) =
                    spawn::lower_spawn_macro_with_strategy(&stmt_macro.mac, use_spawn_local)
                {
                    let semi = stmt_macro.semi_token;
                    if captured.is_empty() {
                        *stmt = syn::Stmt::Expr(spawn_expr, semi);
                    } else {
                        // Inject clone stmts before the spawn, wrap in a block.
                        let mut block_stmts: Vec<syn::Stmt> = Vec::new();
                        for cap in &captured {
                            let action = clone_inject::capture_action(cap);
                            if action == clone_inject::CaptureAction::Clone {
                                let clone_name = clone_inject::clone_var_name(&cap.name);
                                let clone_ident =
                                    syn::Ident::new(&clone_name, proc_macro2::Span::call_site());
                                let orig_ident =
                                    syn::Ident::new(&cap.name, proc_macro2::Span::call_site());
                                let clone_stmt: syn::Stmt = parse_quote! {
                                    let #clone_ident = #orig_ident.clone();
                                };
                                block_stmts.push(clone_stmt);
                            }
                        }
                        block_stmts.push(syn::Stmt::Expr(spawn_expr, semi));
                        let block: syn::Expr = syn::Expr::Block(syn::ExprBlock {
                            attrs: Vec::new(),
                            label: None,
                            block: syn::Block {
                                brace_token: syn::token::Brace::default(),
                                stmts: block_stmts,
                            },
                        });
                        *stmt = syn::Stmt::Expr(block, None);
                    }
                    return;
                }
            }
        }
    }

    fn try_shrink_borrow_alias(
        &mut self,
        block: &mut syn::Block,
        index: usize,
        scopes: &mut ScopeStack,
    ) -> bool {
        let Some(syn::Stmt::Local(local)) = block.stmts.get(index) else {
            return false;
        };
        let Some(alias) = simple_borrow_alias(local, scopes) else {
            return false;
        };
        let Some(next_stmt) = block.stmts.get(index + 1) else {
            self.record_borrow_scope_conservative(local);
            return false;
        };
        if has_later_alias_use(&block.stmts[index + 2..], &alias.alias_ident) {
            self.record_borrow_scope_conservative(local);
            return false;
        }

        let Some((mut method_call, had_semi)) =
            rewritable_method_call(next_stmt, &alias.alias_ident)
        else {
            self.record_borrow_scope_conservative(local);
            return false;
        };

        for argument in &mut method_call.args {
            self.lower_expr(argument, scopes);
        }

        let source_ident = alias.source_ident;
        method_call.receiver = if alias.is_mutable {
            Box::new(parse_quote!(#source_ident.borrow_mut()))
        } else {
            Box::new(parse_quote!(#source_ident.borrow()))
        };

        let replacement = util::build_borrow_scope_block_stmt(method_call, had_semi);
        block.stmts[index] = replacement;
        block.stmts.remove(index + 1);
        true
    }

    fn record_borrow_scope_conservative(&mut self, local: &syn::Local) {
        let Some(binding) = binding_for_pat(self.ast, &local.pat) else {
            return;
        };
        let Some(node) = self.plan.node_for_binding(binding) else {
            return;
        };
        let (kobo_line, _) = self.ast.line_col(binding.span);
        if self
            .annotation_notes
            .iter()
            .any(|note| note.node == node && note.reason == "borrow-scope-conservative")
        {
            return;
        }

        self.annotation_notes.push(AnnotationNote {
            node,
            binding_name: binding.ident.to_string(),
            kobo_line,
            reason: "borrow-scope-conservative".to_owned(),
        });
    }
}
