use syn::spanned::Spanned;

use crate::hint::parse_hint;
use kobo_ir::{EscapeKind, FieldTypeShape, KirStructDef, KirStructFieldDef, UseKind};

use super::helpers::{assignment_escape_kind, borrow_kind, is_mutating_method};
use super::{FunctionCtx, PendingHint, TransformFactsBuilder};

impl TransformFactsBuilder<'_> {
    pub(super) fn walk_item(&mut self, item: &syn::Item) {
        match item {
            syn::Item::Fn(function) => self.walk_function(function),
            syn::Item::Struct(item_struct) => self.collect_struct_def(item_struct),
            syn::Item::Const(item_const) => {
                self.emit_declared_binding(&item_const.ident, Some(&item_const.expr), None);
            }
            syn::Item::Static(item_static) => {
                self.emit_declared_binding(&item_static.ident, Some(&item_static.expr), None);
            }
            _ => {}
        }
    }

    /// Collect a struct definition for K0080-P pattern detection.
    ///
    /// Parses `#[kobo::known_debt = "reason"]` attributes and extracts
    /// simplified field type shapes. Only shapes relevant to P1–P4 are kept;
    /// everything else becomes `FieldTypeShape::Other`.
    fn collect_struct_def(&mut self, item: &syn::ItemStruct) {
        let struct_name = item.ident.to_string();
        let span = self.ast.span_from_syn(item.span());

        // Parse #[kobo::known_debt = "reason"] if present.
        let mut known_debt_reason = None;
        let mut known_debt_span = None;
        let mut known_debt_parse_error = None;
        for attr in &item.attrs {
            match parse_known_debt_attr(attr) {
                KnownDebtResult::Valid(reason, attr_span) => {
                    known_debt_reason = Some(reason);
                    known_debt_span = Some(self.ast.span_from_syn(attr_span));
                }
                KnownDebtResult::MissingReason(attr_span) => {
                    known_debt_span = Some(self.ast.span_from_syn(attr_span));
                    known_debt_parse_error = Some(
                        "`#[kobo::known_debt]` requires a reason string".to_owned(),
                    );
                }
                KnownDebtResult::EmptyReason(attr_span) => {
                    known_debt_span = Some(self.ast.span_from_syn(attr_span));
                    known_debt_parse_error = Some(
                        "`#[kobo::known_debt]` reason string must not be empty".to_owned(),
                    );
                }
                KnownDebtResult::NotKnownDebt => {}
            }
        }

        let fields: Vec<KirStructFieldDef> = match &item.fields {
            syn::Fields::Named(named) => named
                .named
                .iter()
                .map(|field| {
                    let name = field
                        .ident
                        .as_ref()
                        .map(|i| i.to_string())
                        .unwrap_or_default();
                    let shape = field_type_shape(&field.ty, &struct_name);
                    KirStructFieldDef { name, shape }
                })
                .collect(),
            _ => Vec::new(),
        };

        self.struct_defs.push(KirStructDef {
            name: struct_name,
            span,
            fields,
            known_debt_reason,
            known_debt_span,
            known_debt_parse_error,
        });
    }

    fn walk_function(&mut self, function: &syn::ItemFn) {
        let generic_params = function
            .sig
            .generics
            .params
            .iter()
            .filter_map(|param| match param {
                syn::GenericParam::Type(ty) => Some(ty.ident.to_string()),
                _ => None,
            })
            .collect();
        let function_hint = parse_hint(&function.attrs).map(|(hint, span)| PendingHint {
            hint,
            span: self.ast.span_from_syn(span),
        });
        self.function_stack.push(FunctionCtx {
            generic_params,
            is_async: function.sig.asyncness.is_some(),
            hint: function_hint,
            hint_consumed: false,
        });

        self.enter_scope(self.ast.span_from_syn(function.span()));
        self.ctx.push_scope();

        for input in &function.sig.inputs {
            if let syn::FnArg::Typed(argument) = input {
                let param_hint = self.take_function_hint();
                self.emit_pat_binding(&argument.pat, None, param_hint);
            }
        }

        self.walk_block_statements(&function.block.stmts);

        self.ctx.pop_scope();
        self.exit_scope(self.ast.span_from_syn(function.span()));
        self.function_stack.pop();
    }

    fn walk_block(&mut self, block: &syn::Block) {
        self.enter_scope(self.ast.span_from_syn(block.span()));
        self.ctx.push_scope();
        self.walk_block_statements(&block.stmts);
        self.ctx.pop_scope();
        self.exit_scope(self.ast.span_from_syn(block.span()));
    }

    fn walk_block_statements(&mut self, statements: &[syn::Stmt]) {
        for stmt in statements {
            self.walk_stmt(stmt);
        }
    }

    fn walk_stmt(&mut self, stmt: &syn::Stmt) {
        match stmt {
            syn::Stmt::Local(local) => self.walk_local(local),
            syn::Stmt::Item(item) => self.walk_item(item),
            syn::Stmt::Expr(expr, _) => self.walk_expr(expr),
            syn::Stmt::Macro(_) => {}
        }
    }

    fn walk_local(&mut self, local: &syn::Local) {
        let init = local.init.as_ref().map(|init| init.expr.as_ref());
        let Some(binding) = self.binding_for_pat(&local.pat).cloned() else {
            if let Some(init_expr) = init {
                self.walk_expr(init_expr);
            }
            return;
        };

        let local_hint = parse_hint(&local.attrs).map(|(hint, span)| PendingHint {
            hint,
            span: self.ast.span_from_syn(span),
        });

        let binding_state = self.emit_binding(&binding, init, local_hint);
        if let Some(init_expr) = init {
            if let Some((mut move_alias, source_state)) =
                self.move_alias_from_initializer(binding_state.decl_id, init_expr)
            {
                let alias_is_candidate = self.pending_elision_candidates.contains_key(&binding.id);
                self.record_pending_elision_candidate(
                    &binding,
                    binding_state.decl_id,
                    source_state.decl_id,
                );
                self.maybe_mark_plain_clone_alias(
                    binding_state.decl_id,
                    source_state,
                    move_alias.span,
                    alias_is_candidate,
                );
                move_alias.plain_clone_alias &= !alias_is_candidate;
                self.push_move_alias(move_alias);
            }
            if let Some(alias) =
                self.borrow_alias_from_initializer(binding_state.decl_id, init_expr)
            {
                self.push_borrow_alias(alias);
            }
            self.walk_local_initializer(init_expr);
        }
        self.ctx.define(&binding.ident, binding_state);
    }

    fn walk_expr(&mut self, expr: &syn::Expr) {
        match expr {
            syn::Expr::Array(array) => {
                for element in &array.elems {
                    self.walk_expr(element);
                }
            }
            syn::Expr::Assign(assign) => {
                let escape_kind = assignment_escape_kind(assign.left.as_ref());
                self.walk_assignment_target(assign.left.as_ref());
                self.walk_move_context_expr(assign.right.as_ref(), escape_kind);
            }
            syn::Expr::Binary(binary) => {
                self.walk_expr(binary.left.as_ref());
                self.walk_expr(binary.right.as_ref());
            }
            syn::Expr::Block(block) => self.walk_block(&block.block),
            syn::Expr::Call(call) => self.walk_call(call),
            syn::Expr::Cast(cast) => self.walk_expr(cast.expr.as_ref()),
            syn::Expr::Closure(closure) => self.walk_expr(closure.body.as_ref()),
            syn::Expr::Field(field) => self.walk_expr(field.base.as_ref()),
            syn::Expr::ForLoop(for_loop) => {
                self.walk_expr(for_loop.expr.as_ref());
                self.walk_block(&for_loop.body);
            }
            syn::Expr::Group(group) => self.walk_expr(group.expr.as_ref()),
            syn::Expr::If(expr_if) => {
                self.walk_expr(expr_if.cond.as_ref());
                self.walk_block(&expr_if.then_branch);
                if let Some((_, else_branch)) = &expr_if.else_branch {
                    self.walk_expr(else_branch.as_ref());
                }
            }
            syn::Expr::Index(index) => {
                self.walk_expr(index.expr.as_ref());
                self.walk_expr(index.index.as_ref());
            }
            syn::Expr::Loop(expr_loop) => self.walk_block(&expr_loop.body),
            syn::Expr::Macro(_) => {}
            syn::Expr::Match(expr_match) => {
                self.walk_expr(expr_match.expr.as_ref());
                for arm in &expr_match.arms {
                    if let Some((_, guard)) = &arm.guard {
                        self.walk_expr(guard.as_ref());
                    }
                    self.walk_expr(arm.body.as_ref());
                }
            }
            syn::Expr::MethodCall(method_call) => {
                self.walk_method_receiver(method_call.receiver.as_ref(), &method_call.method);
                for argument in &method_call.args {
                    self.walk_expr(argument);
                }
            }
            syn::Expr::Paren(paren) => self.walk_expr(paren.expr.as_ref()),
            syn::Expr::Path(path) => self.walk_path_expr(path),
            syn::Expr::Reference(reference) => self.walk_reference_expr(reference),
            syn::Expr::Repeat(repeat) => {
                self.walk_expr(repeat.expr.as_ref());
                self.walk_expr(repeat.len.as_ref());
            }
            syn::Expr::Return(return_expr) => {
                if let Some(expr) = &return_expr.expr {
                    self.walk_move_context_expr(
                        expr.as_ref(),
                        Some(EscapeKind::ReturnedFromFunction),
                    );
                }
            }
            syn::Expr::Struct(expr_struct) => {
                for field in &expr_struct.fields {
                    self.walk_move_context_expr(&field.expr, Some(EscapeKind::StoredInStruct));
                }
                if let Some(rest) = &expr_struct.rest {
                    self.walk_expr(rest.as_ref());
                }
            }
            syn::Expr::Tuple(tuple) => {
                for element in &tuple.elems {
                    self.walk_expr(element);
                }
            }
            syn::Expr::Unary(unary) => self.walk_expr(unary.expr.as_ref()),
            syn::Expr::While(expr_while) => {
                self.walk_expr(expr_while.cond.as_ref());
                self.walk_block(&expr_while.body);
            }
            _ => {}
        }
    }

    fn walk_call(&mut self, call: &syn::ExprCall) {
        self.walk_expr(call.func.as_ref());
        let escape_kind = if self.is_local_function_expr(call.func.as_ref()) {
            None
        } else {
            Some(EscapeKind::PassedToOpaqueCall)
        };
        for argument in &call.args {
            let syn::Expr::Reference(reference) = argument else {
                self.walk_move_context_expr(argument, escape_kind);
                continue;
            };

            if self.emit_ephemeral_borrow(reference.expr.as_ref(), borrow_kind(reference)) {
                continue;
            }

            self.walk_expr(argument);
        }
    }

    pub(super) fn walk_local_initializer(&mut self, expr: &syn::Expr) {
        match expr {
            syn::Expr::Reference(reference) => self.walk_reference_expr(reference),
            _ => self.walk_move_context_expr(expr, None),
        }
    }

    fn walk_move_context_expr(&mut self, expr: &syn::Expr, escape_kind: Option<EscapeKind>) {
        if self.emit_move(expr, escape_kind) {
            return;
        }

        self.walk_expr(expr);
    }

    fn walk_assignment_target(&mut self, expr: &syn::Expr) {
        if self.emit_use(expr, UseKind::Write) {
            return;
        }

        self.walk_expr(expr);
    }

    fn walk_method_receiver(&mut self, receiver: &syn::Expr, method: &syn::Ident) {
        let use_kind = if is_mutating_method(method) {
            UseKind::Write
        } else {
            UseKind::Read
        };

        if self.emit_use(receiver, use_kind) {
            return;
        }

        self.walk_expr(receiver);
    }

    fn walk_path_expr(&mut self, path: &syn::ExprPath) {
        let _ = self.emit_use(&syn::Expr::Path(path.clone()), UseKind::Read);
    }

    fn walk_reference_expr(&mut self, reference: &syn::ExprReference) {
        if self.emit_borrow(reference.expr.as_ref(), borrow_kind(reference)) {
            return;
        }

        self.walk_expr(reference.expr.as_ref());
    }
}

// ---------------------------------------------------------------------------
// Free helpers for struct-def collection
// ---------------------------------------------------------------------------

/// Result of parsing a `#[kobo::known_debt]` attribute.
enum KnownDebtResult {
    /// Attribute is not a `kobo::known_debt` attribute.
    NotKnownDebt,
    /// Valid `#[kobo::known_debt = "reason"]`.
    Valid(String, proc_macro2::Span),
    /// `#[kobo::known_debt]` without a reason string.
    MissingReason(proc_macro2::Span),
    /// `#[kobo::known_debt = ""]` with an empty reason string.
    EmptyReason(proc_macro2::Span),
}

/// Parse `#[kobo::known_debt = "reason"]` from a single attribute.
///
/// Returns `Valid` when the attribute matches with a non-empty reason,
/// `MissingReason` for bare `#[kobo::known_debt]`, `EmptyReason` for
/// `#[kobo::known_debt = ""]`, and `NotKnownDebt` otherwise.
fn parse_known_debt_attr(attr: &syn::Attribute) -> KnownDebtResult {
    match &attr.meta {
        syn::Meta::NameValue(nv) => {
            let segs: Vec<_> = nv.path.segments.iter().map(|s| s.ident.to_string()).collect();
            if segs.len() == 2 && segs[0] == "kobo" && segs[1] == "known_debt" {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(lit_str),
                    ..
                }) = &nv.value
                {
                    let reason = lit_str.value();
                    if reason.is_empty() {
                        return KnownDebtResult::EmptyReason(attr.span());
                    }
                    return KnownDebtResult::Valid(reason, attr.span());
                }
            }
            KnownDebtResult::NotKnownDebt
        }
        syn::Meta::Path(path) => {
            let segs: Vec<_> = path.segments.iter().map(|s| s.ident.to_string()).collect();
            if segs.len() == 2 && segs[0] == "kobo" && segs[1] == "known_debt" {
                return KnownDebtResult::MissingReason(attr.span());
            }
            KnownDebtResult::NotKnownDebt
        }
        _ => KnownDebtResult::NotKnownDebt,
    }
}

/// Map a `syn::Type` to the simplified `FieldTypeShape` used for K0080-P detection.
///
/// `struct_name` is passed so `Self` references can be canonicalized to the
/// containing struct name.
fn field_type_shape(ty: &syn::Type, struct_name: &str) -> FieldTypeShape {
    match ty {
        syn::Type::Path(type_path) => {
            let segs: Vec<String> = type_path
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();

            match segs.as_slice() {
                // Rc<...>
                [rc] if rc == "Rc" => {
                    if let Some(inner) = first_generic_arg(&type_path.path.segments[0]) {
                        if let Some(inner_name) = extract_rc_refcell_inner(inner, struct_name) {
                            return FieldTypeShape::RcRefCellOf(inner_name);
                        }
                    }
                    FieldTypeShape::Other
                }
                // Option<...>
                [opt] if opt == "Option" => {
                    if let Some(inner) = first_generic_arg(&type_path.path.segments[0]) {
                        if let syn::GenericArgument::Type(inner_ty) = inner {
                            if let FieldTypeShape::RcRefCellOf(name) =
                                field_type_shape(inner_ty, struct_name)
                            {
                                return FieldTypeShape::OptionRcRefCellOf(name);
                            }
                        }
                    }
                    FieldTypeShape::Other
                }
                // Vec<...>
                [vec] if vec == "Vec" => {
                    if let Some(inner) = first_generic_arg(&type_path.path.segments[0]) {
                        if let syn::GenericArgument::Type(inner_ty) = inner {
                            match field_type_shape(inner_ty, struct_name) {
                                FieldTypeShape::RcRefCellOf(name) => {
                                    return FieldTypeShape::VecRcRefCellOf(name)
                                }
                                FieldTypeShape::DirectNamed(name) => {
                                    return FieldTypeShape::VecDirectNamed(name)
                                }
                                _ => {}
                            }
                        }
                    }
                    FieldTypeShape::Other
                }
                // Self
                [s] if s == "Self" => FieldTypeShape::DirectNamed(struct_name.to_owned()),
                // Any other single-segment name
                [name] => FieldTypeShape::DirectNamed(name.clone()),
                _ => FieldTypeShape::Other,
            }
        }
        _ => FieldTypeShape::Other,
    }
}

/// Extract the first generic argument from a path segment.
fn first_generic_arg(seg: &syn::PathSegment) -> Option<&syn::GenericArgument> {
    match &seg.arguments {
        syn::PathArguments::AngleBracketed(args) => args.args.first(),
        _ => None,
    }
}

/// If `ty` is `RefCell<X>`, return the canonical name of `X` (with "Self"
/// replaced by `struct_name`).
fn extract_rc_refcell_inner(arg: &syn::GenericArgument, struct_name: &str) -> Option<String> {
    if let syn::GenericArgument::Type(inner_ty) = arg {
        if let syn::Type::Path(tp) = inner_ty {
            let segs: Vec<_> = tp.path.segments.iter().map(|s| s.ident.to_string()).collect();
            if segs.first().map(String::as_str) == Some("RefCell") {
                if let Some(inner_arg) = first_generic_arg(tp.path.segments.first()?) {
                    if let syn::GenericArgument::Type(value_ty) = inner_arg {
                        if let syn::Type::Path(vp) = value_ty {
                            let name = vp
                                .path
                                .segments
                                .last()
                                .map(|s| s.ident.to_string())
                                .unwrap_or_default();
                            let canonical = if name == "Self" {
                                struct_name.to_owned()
                            } else {
                                name
                            };
                            return Some(canonical);
                        }
                    }
                }
            }
        }
    }
    None
}
