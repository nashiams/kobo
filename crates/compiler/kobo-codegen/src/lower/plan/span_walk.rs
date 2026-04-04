use kobo_ir::KoboSpan;
use kobo_parser::KoboFile;
use syn::spanned::Spanned;

/// Collect all spans that represent conditional or loop bodies in the AST.
/// Used by the plan builder to determine whether mutation sites are conditional.
pub(super) fn collect_conditional_body_spans(ast: &KoboFile) -> Vec<KoboSpan> {
    let mut spans = Vec::new();
    for item in &ast.inner.items {
        collect_from_item(item, ast, &mut spans);
    }
    spans
}

fn collect_from_item(item: &syn::Item, ast: &KoboFile, spans: &mut Vec<KoboSpan>) {
    match item {
        syn::Item::Fn(function) => collect_from_block(&function.block, ast, spans),
        syn::Item::Const(item_const) => {
            collect_from_expr(item_const.expr.as_ref(), ast, spans);
        }
        syn::Item::Static(item_static) => {
            collect_from_expr(item_static.expr.as_ref(), ast, spans);
        }
        _ => {}
    }
}

fn collect_from_block(block: &syn::Block, ast: &KoboFile, spans: &mut Vec<KoboSpan>) {
    for statement in &block.stmts {
        collect_from_stmt(statement, ast, spans);
    }
}

fn collect_from_stmt(statement: &syn::Stmt, ast: &KoboFile, spans: &mut Vec<KoboSpan>) {
    match statement {
        syn::Stmt::Local(local) => {
            if let Some(init) = &local.init {
                collect_from_expr(init.expr.as_ref(), ast, spans);
            }
        }
        syn::Stmt::Item(item) => collect_from_item(item, ast, spans),
        syn::Stmt::Expr(expr, _) => collect_from_expr(expr, ast, spans),
        syn::Stmt::Macro(_) => {}
    }
}

fn collect_from_expr(expr: &syn::Expr, ast: &KoboFile, spans: &mut Vec<KoboSpan>) {
    match expr {
        syn::Expr::Array(array) => {
            for element in &array.elems {
                collect_from_expr(element, ast, spans);
            }
        }
        syn::Expr::Assign(assign) => {
            collect_from_expr(assign.left.as_ref(), ast, spans);
            collect_from_expr(assign.right.as_ref(), ast, spans);
        }
        syn::Expr::Binary(binary) => {
            collect_from_expr(binary.left.as_ref(), ast, spans);
            collect_from_expr(binary.right.as_ref(), ast, spans);
        }
        syn::Expr::Block(block) => collect_from_block(&block.block, ast, spans),
        syn::Expr::Call(call) => {
            collect_from_expr(call.func.as_ref(), ast, spans);
            for argument in &call.args {
                collect_from_expr(argument, ast, spans);
            }
        }
        syn::Expr::Cast(cast) => {
            collect_from_expr(cast.expr.as_ref(), ast, spans);
        }
        syn::Expr::Closure(closure) => {
            collect_from_expr(closure.body.as_ref(), ast, spans);
        }
        syn::Expr::Field(field) => {
            collect_from_expr(field.base.as_ref(), ast, spans);
        }
        syn::Expr::ForLoop(for_loop) => {
            collect_from_expr(for_loop.expr.as_ref(), ast, spans);
            spans.push(ast.span_from_syn(for_loop.body.span()));
            collect_from_block(&for_loop.body, ast, spans);
        }
        syn::Expr::Group(group) => {
            collect_from_expr(group.expr.as_ref(), ast, spans);
        }
        syn::Expr::If(expr_if) => {
            collect_from_expr(expr_if.cond.as_ref(), ast, spans);
            spans.push(ast.span_from_syn(expr_if.then_branch.span()));
            collect_from_block(&expr_if.then_branch, ast, spans);
            if let Some((_, else_branch)) = &expr_if.else_branch {
                spans.push(ast.span_from_syn(else_branch.span()));
                collect_from_expr(else_branch.as_ref(), ast, spans);
            }
        }
        syn::Expr::Index(index) => {
            collect_from_expr(index.expr.as_ref(), ast, spans);
            collect_from_expr(index.index.as_ref(), ast, spans);
        }
        syn::Expr::Loop(expr_loop) => {
            spans.push(ast.span_from_syn(expr_loop.body.span()));
            collect_from_block(&expr_loop.body, ast, spans);
        }
        syn::Expr::Match(expr_match) => {
            collect_from_expr(expr_match.expr.as_ref(), ast, spans);
            for arm in &expr_match.arms {
                if let Some((_, guard)) = &arm.guard {
                    collect_from_expr(guard.as_ref(), ast, spans);
                }
                spans.push(ast.span_from_syn(arm.body.span()));
                collect_from_expr(arm.body.as_ref(), ast, spans);
            }
        }
        syn::Expr::MethodCall(method_call) => {
            collect_from_expr(method_call.receiver.as_ref(), ast, spans);
            for argument in &method_call.args {
                collect_from_expr(argument, ast, spans);
            }
        }
        syn::Expr::Paren(paren) => {
            collect_from_expr(paren.expr.as_ref(), ast, spans);
        }
        syn::Expr::Reference(reference) => {
            collect_from_expr(reference.expr.as_ref(), ast, spans);
        }
        syn::Expr::Repeat(repeat) => {
            collect_from_expr(repeat.expr.as_ref(), ast, spans);
            collect_from_expr(repeat.len.as_ref(), ast, spans);
        }
        syn::Expr::Return(return_expr) => {
            if let Some(inner) = &return_expr.expr {
                collect_from_expr(inner.as_ref(), ast, spans);
            }
        }
        syn::Expr::Struct(expr_struct) => {
            for field in &expr_struct.fields {
                collect_from_expr(&field.expr, ast, spans);
            }
            if let Some(rest) = &expr_struct.rest {
                collect_from_expr(rest.as_ref(), ast, spans);
            }
        }
        syn::Expr::Tuple(tuple) => {
            for element in &tuple.elems {
                collect_from_expr(element, ast, spans);
            }
        }
        syn::Expr::Unary(unary) => {
            collect_from_expr(unary.expr.as_ref(), ast, spans);
        }
        syn::Expr::While(expr_while) => {
            collect_from_expr(expr_while.cond.as_ref(), ast, spans);
            spans.push(ast.span_from_syn(expr_while.body.span()));
            collect_from_block(&expr_while.body, ast, spans);
        }
        _ => {}
    }
}
