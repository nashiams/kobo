use super::{BindingEnv, BlockFlow, Expr, ScenarioLowerer};

impl<'a> ScenarioLowerer<'a> {
    pub(super) fn execute_structural_expr(
        &mut self,
        expr: &'a Expr,
        env: &mut BindingEnv,
    ) -> BlockFlow {
        match expr {
            Expr::Array(array) => {
                for element in &array.elems {
                    self.execute_expr(element, env);
                }
            }
            Expr::Assign(assign) => {
                self.execute_expr(assign.left.as_ref(), env);
                self.execute_expr(assign.right.as_ref(), env);
            }
            Expr::Binary(binary) => {
                self.execute_expr(binary.left.as_ref(), env);
                self.execute_expr(binary.right.as_ref(), env);
            }
            Expr::Cast(cast) => {
                self.execute_expr(cast.expr.as_ref(), env);
            }
            Expr::Closure(closure) => {
                self.execute_expr(closure.body.as_ref(), env);
            }
            Expr::Field(field) => {
                self.execute_expr(field.base.as_ref(), env);
            }
            Expr::Group(group) => {
                self.execute_expr(group.expr.as_ref(), env);
            }
            Expr::Index(index) => {
                self.execute_expr(index.expr.as_ref(), env);
                self.execute_expr(index.index.as_ref(), env);
            }
            Expr::Let(expr_let) => {
                self.execute_expr(expr_let.expr.as_ref(), env);
            }
            Expr::Paren(paren) => {
                self.execute_expr(paren.expr.as_ref(), env);
            }
            Expr::Range(range) => {
                if let Some(start) = range.start.as_deref() {
                    self.execute_expr(start, env);
                }
                if let Some(end) = range.end.as_deref() {
                    self.execute_expr(end, env);
                }
            }
            Expr::Reference(reference) => {
                self.execute_expr(reference.expr.as_ref(), env);
            }
            Expr::Repeat(repeat) => {
                self.execute_expr(repeat.expr.as_ref(), env);
                self.execute_expr(repeat.len.as_ref(), env);
            }
            Expr::Struct(expr_struct) => {
                for field in &expr_struct.fields {
                    self.execute_expr(&field.expr, env);
                }
                if let Some(rest) = expr_struct.rest.as_deref() {
                    self.execute_expr(rest, env);
                }
            }
            Expr::Tuple(tuple) => {
                for element in &tuple.elems {
                    self.execute_expr(element, env);
                }
            }
            Expr::Unary(unary) => {
                self.execute_expr(unary.expr.as_ref(), env);
            }
            Expr::Unsafe(expr_unsafe) => {
                self.execute_block(&expr_unsafe.block, env);
            }
            Expr::Yield(expr_yield) => {
                if let Some(value) = expr_yield.expr.as_deref() {
                    self.execute_expr(value, env);
                }
            }
            _ => {}
        }
        BlockFlow::Fallthrough
    }
}
