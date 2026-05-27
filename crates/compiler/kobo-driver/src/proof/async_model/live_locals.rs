use std::collections::BTreeSet;

use kobo_ir::FileId;
use kobo_parser::{
    preprocess_bridge_blocks, preprocess_concurrent_sugar, preprocess_kobo_keywords,
    preprocess_spawn_blocks, strict_keyword_configs,
};

pub(super) fn parsed_live_locals_by_await(source: &str, target: &str) -> Vec<Vec<String>> {
    let Some(file) = parse_source_for_async_model(source) else {
        return Vec::new();
    };
    let Some(function) = file.items.iter().find_map(|item| match item {
        syn::Item::Fn(function) if function.sig.ident == target => Some(function),
        _ => None,
    }) else {
        return Vec::new();
    };

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
                .map(|locals| locals.into_iter().collect::<Vec<_>>())
        })
        .collect()
}

fn parse_source_for_async_model(source: &str) -> Option<syn::File> {
    syn::parse_file(source)
        .or_else(|_| syn::parse_file(&preprocess_kobo_source_for_syn(source)))
        .ok()
}

fn preprocess_kobo_source_for_syn(source: &str) -> String {
    let (rewritten, _) = preprocess_concurrent_sugar(source);
    let configs = strict_keyword_configs();
    let (rewritten, _) = preprocess_kobo_keywords(&rewritten, &configs);
    let (rewritten, _) = preprocess_spawn_blocks(&rewritten, FileId(0));
    let (rewritten, _) = preprocess_bridge_blocks(&rewritten);
    rewritten
}

pub(super) fn await_live_locals_in_stmt(
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

pub(super) fn await_live_locals_in_expr(
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

pub(super) fn live_declared_locals(
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

pub(super) fn await_live_locals_in_block(
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

pub(super) fn await_live_locals_in_if(
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

pub(super) fn await_live_locals_in_match(
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

pub(super) fn await_live_locals_in_expr_sequence<'a>(
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

pub(super) fn ident_uses_in_expr(expr: &syn::Expr) -> BTreeSet<String> {
    let mut uses = BTreeSet::new();
    collect_ident_uses_in_expr(expr, &mut uses);
    uses
}

pub(super) fn expr_await_count(expr: &syn::Expr) -> usize {
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

pub(super) fn collect_pat_bindings_in_stmt(statement: &syn::Stmt, bindings: &mut BTreeSet<String>) {
    let syn::Stmt::Local(local) = statement else {
        return;
    };
    collect_pat_bindings(&local.pat, bindings);
}

pub(super) fn collect_pat_bindings(pattern: &syn::Pat, bindings: &mut BTreeSet<String>) {
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

pub(super) fn collect_ident_uses_in_stmt(statement: &syn::Stmt, uses: &mut BTreeSet<String>) {
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

pub(super) fn collect_ident_uses_in_expr(expr: &syn::Expr, uses: &mut BTreeSet<String>) {
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
