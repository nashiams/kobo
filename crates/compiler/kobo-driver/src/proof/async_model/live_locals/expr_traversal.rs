use std::collections::BTreeSet;

use super::collect_ident_uses_in_stmt;
use super::collect_pat_bindings;
use super::collect_pat_bindings_in_stmt;
use super::expr_await_count;
use super::ident_uses_in_expr;

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
            await_live_locals_in_await(await_expr.base.as_ref(), later_uses, declared_before)
        }
        syn::Expr::Array(_)
        | syn::Expr::Assign(_)
        | syn::Expr::Binary(_)
        | syn::Expr::Call(_)
        | syn::Expr::Index(_)
        | syn::Expr::MethodCall(_)
        | syn::Expr::Range(_)
        | syn::Expr::Repeat(_)
        | syn::Expr::Struct(_)
        | syn::Expr::Tuple(_) => {
            await_live_locals_in_sequence_expr(expr, later_uses, declared_before)
        }
        syn::Expr::Block(_) | syn::Expr::TryBlock(_) | syn::Expr::Unsafe(_) => {
            await_live_locals_in_block_expr(expr, later_uses, declared_before)
        }
        syn::Expr::Break(_)
        | syn::Expr::Cast(_)
        | syn::Expr::Closure(_)
        | syn::Expr::Field(_)
        | syn::Expr::Group(_)
        | syn::Expr::Let(_)
        | syn::Expr::Paren(_)
        | syn::Expr::Reference(_)
        | syn::Expr::Return(_)
        | syn::Expr::Try(_)
        | syn::Expr::Unary(_)
        | syn::Expr::Yield(_) => {
            await_live_locals_in_single_child_expr(expr, later_uses, declared_before)
        }
        syn::Expr::ForLoop(_) | syn::Expr::While(_) => {
            await_live_locals_in_loop_expr(expr, later_uses, declared_before)
        }
        syn::Expr::If(expr_if) => await_live_locals_in_if(expr_if, later_uses, declared_before),
        syn::Expr::Match(expr_match) => {
            await_live_locals_in_match(expr_match, later_uses, declared_before)
        }
        _ => await_live_locals_in_fallback_expr(expr, later_uses, declared_before),
    }
}

fn await_live_locals_in_await(
    base: &syn::Expr,
    later_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    let (mut awaits, uses) = await_live_locals_in_expr(base, later_uses, declared_before);
    awaits.push(live_declared_locals(later_uses, declared_before));
    (awaits, uses)
}

fn await_live_locals_in_sequence_expr(
    expr: &syn::Expr,
    later_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    match expr {
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
        syn::Expr::Call(call) => await_live_locals_in_expr_sequence(
            std::iter::once(call.func.as_ref()).chain(call.args.iter()),
            later_uses,
            declared_before,
        ),
        syn::Expr::Index(index) => await_live_locals_in_expr_sequence(
            [index.expr.as_ref(), index.index.as_ref()].into_iter(),
            later_uses,
            declared_before,
        ),
        syn::Expr::MethodCall(call) => await_live_locals_in_expr_sequence(
            std::iter::once(call.receiver.as_ref()).chain(call.args.iter()),
            later_uses,
            declared_before,
        ),
        syn::Expr::Range(range) => {
            let children = range
                .start
                .iter()
                .chain(range.end.iter())
                .map(|expr| expr.as_ref());
            await_live_locals_in_expr_sequence(children, later_uses, declared_before)
        }
        syn::Expr::Repeat(repeat) => await_live_locals_in_expr_sequence(
            [repeat.expr.as_ref(), repeat.len.as_ref()].into_iter(),
            later_uses,
            declared_before,
        ),
        syn::Expr::Struct(expr_struct) => {
            let children = expr_struct
                .fields
                .iter()
                .map(|field| &field.expr)
                .chain(expr_struct.rest.iter().map(|expr| expr.as_ref()));
            await_live_locals_in_expr_sequence(children, later_uses, declared_before)
        }
        syn::Expr::Tuple(tuple) => {
            await_live_locals_in_expr_sequence(tuple.elems.iter(), later_uses, declared_before)
        }
        _ => await_live_locals_in_fallback_expr(expr, later_uses, declared_before),
    }
}

fn await_live_locals_in_block_expr(
    expr: &syn::Expr,
    later_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    match expr {
        syn::Expr::Block(block) => {
            await_live_locals_in_block(&block.block, later_uses, declared_before)
        }
        syn::Expr::TryBlock(try_block) => {
            await_live_locals_in_block(&try_block.block, later_uses, declared_before)
        }
        syn::Expr::Unsafe(expr_unsafe) => {
            await_live_locals_in_block(&expr_unsafe.block, later_uses, declared_before)
        }
        _ => await_live_locals_in_fallback_expr(expr, later_uses, declared_before),
    }
}

fn await_live_locals_in_single_child_expr(
    expr: &syn::Expr,
    later_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    let child = match expr {
        syn::Expr::Break(expr_break) => expr_break.expr.as_deref(),
        syn::Expr::Cast(cast) => Some(cast.expr.as_ref()),
        syn::Expr::Closure(closure) => Some(closure.body.as_ref()),
        syn::Expr::Field(field) => Some(field.base.as_ref()),
        syn::Expr::Group(group) => Some(group.expr.as_ref()),
        syn::Expr::Let(expr_let) => Some(expr_let.expr.as_ref()),
        syn::Expr::Paren(paren) => Some(paren.expr.as_ref()),
        syn::Expr::Reference(reference) => Some(reference.expr.as_ref()),
        syn::Expr::Return(expr_return) => expr_return.expr.as_deref(),
        syn::Expr::Try(expr_try) => Some(expr_try.expr.as_ref()),
        syn::Expr::Unary(unary) => Some(unary.expr.as_ref()),
        syn::Expr::Yield(expr_yield) => expr_yield.expr.as_deref(),
        _ => None,
    };
    child
        .map(|value| await_live_locals_in_expr(value, later_uses, declared_before))
        .unwrap_or_default()
}

fn await_live_locals_in_loop_expr(
    expr: &syn::Expr,
    later_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    match expr {
        syn::Expr::ForLoop(expr_for) => {
            await_live_locals_in_for_loop(expr_for, later_uses, declared_before)
        }
        syn::Expr::While(expr_while) => {
            await_live_locals_in_while(expr_while, later_uses, declared_before)
        }
        _ => await_live_locals_in_fallback_expr(expr, later_uses, declared_before),
    }
}

fn await_live_locals_in_for_loop(
    expr_for: &syn::ExprForLoop,
    later_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    let mut body_declared = declared_before.clone();
    collect_pat_bindings(&expr_for.pat, &mut body_declared);
    let (body_awaits, body_uses) =
        await_live_locals_in_block(&expr_for.body, later_uses, &body_declared);
    let mut iter_later_uses = later_uses.clone();
    iter_later_uses.extend(body_uses.iter().cloned());
    let (mut awaits, iter_uses) =
        await_live_locals_in_expr(expr_for.expr.as_ref(), &iter_later_uses, declared_before);
    awaits.extend(body_awaits);
    let mut uses = iter_uses;
    uses.extend(body_uses);
    (awaits, uses)
}

fn await_live_locals_in_while(
    expr_while: &syn::ExprWhile,
    later_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
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

fn await_live_locals_in_fallback_expr(
    expr: &syn::Expr,
    later_uses: &BTreeSet<String>,
    declared_before: &BTreeSet<String>,
) -> (Vec<BTreeSet<String>>, BTreeSet<String>) {
    let uses = ident_uses_in_expr(expr);
    (
        vec![live_declared_locals(later_uses, declared_before); expr_await_count(expr)],
        uses,
    )
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
