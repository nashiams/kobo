use std::collections::HashMap;

use kobo_ir::{KirNodeId, OwnershipTier, ResourceKind};
use kobo_parser::KoboBinding;

/// Classification result for a single binding.
#[derive(Clone, Copy)]
pub(crate) struct BindingClassification {
    pub ownership: OwnershipTier,
    pub resource_kind: Option<ResourceKind>,
}

/// Ownership state already known in the current lexical scope.
#[derive(Clone, Copy)]
pub(crate) struct BindingState {
    pub decl_id: KirNodeId,
    pub ownership: OwnershipTier,
    pub resource_kind: Option<ResourceKind>,
}

/// Scope-aware context used during the v0.1 transform.
pub(crate) struct TransformCtx {
    scopes: Vec<HashMap<String, BindingState>>,
}

impl TransformCtx {
    pub(crate) fn new() -> Self {
        Self { scopes: Vec::new() }
    }

    pub(crate) fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub(crate) fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub(crate) fn define(&mut self, ident: &syn::Ident, state: BindingState) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(ident.to_string(), state);
        }
    }

    pub(crate) fn lookup(&self, ident: &syn::Ident) -> Option<BindingState> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(&ident.to_string()).copied())
    }
}

/// Returns `true` if `ty` is a `Copy` type that should never be wrapped.
///
/// Covers primitives, `bool`, `char`, and tuples and arrays of `Copy` types.
/// Returns `false` (wrap conservatively) when uncertain. Full trait-based
/// determination requires `rustc` integration (future).
pub(crate) fn is_copy_type(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Path(path) => {
            let Some(segment) = path.path.segments.last() else {
                return false;
            };

            matches!(
                segment.ident.to_string().as_str(),
                "i8" | "i16"
                    | "i32"
                    | "i64"
                    | "i128"
                    | "isize"
                    | "u8"
                    | "u16"
                    | "u32"
                    | "u64"
                    | "u128"
                    | "usize"
                    | "f32"
                    | "f64"
                    | "bool"
                    | "char"
            )
        }
        syn::Type::Tuple(tuple) if tuple.elems.is_empty() => true,
        syn::Type::Tuple(tuple) => tuple.elems.iter().all(is_copy_type),
        syn::Type::Array(array) => is_copy_type(&array.elem),
        syn::Type::Reference(_) => true,
        _ => false,
    }
}

/// Classifies a binding according to the v0.1 script-mode rules.
pub(crate) fn classify_binding(
    binding: &KoboBinding,
    init: Option<&syn::Expr>,
    ctx: &TransformCtx,
) -> BindingClassification {
    if let Some(resource_kind) = detect_resource_kind(binding.ty.as_ref(), init, ctx) {
        return BindingClassification {
            ownership: OwnershipTier::Scoped,
            resource_kind: Some(resource_kind),
        };
    }

    if binding.ty.as_ref().is_some_and(is_copy_type)
        || init.is_some_and(|expr| is_copy_expr(expr, ctx))
    {
        return BindingClassification {
            ownership: OwnershipTier::PlainOwned,
            resource_kind: None,
        };
    }

    BindingClassification {
        ownership: OwnershipTier::RcMutShared,
        resource_kind: None,
    }
}

fn is_copy_expr(expr: &syn::Expr, ctx: &TransformCtx) -> bool {
    match expr {
        syn::Expr::Lit(lit) => matches!(
            lit.lit,
            syn::Lit::Int(_)
                | syn::Lit::Float(_)
                | syn::Lit::Bool(_)
                | syn::Lit::Char(_)
                | syn::Lit::Str(_)
                | syn::Lit::ByteStr(_)
                | syn::Lit::Byte(_)
        ),
        syn::Expr::Path(path) => {
            let Some(ident) = single_ident(path.path.segments.iter().map(|segment| &segment.ident))
            else {
                return false;
            };

            ctx.lookup(ident)
                .is_some_and(|binding| binding.ownership == OwnershipTier::PlainOwned)
        }
        syn::Expr::Paren(paren) => is_copy_expr(&paren.expr, ctx),
        syn::Expr::Group(group) => is_copy_expr(&group.expr, ctx),
        syn::Expr::Tuple(tuple) => tuple.elems.iter().all(|expr| is_copy_expr(expr, ctx)),
        syn::Expr::Array(array) => array.elems.iter().all(|expr| is_copy_expr(expr, ctx)),
        syn::Expr::Reference(_) => true,
        syn::Expr::Unary(unary) => is_copy_expr(&unary.expr, ctx),
        syn::Expr::Cast(cast) => is_copy_expr(&cast.expr, ctx),
        _ => false,
    }
}

fn detect_resource_kind(
    ty: Option<&syn::Type>,
    init: Option<&syn::Expr>,
    ctx: &TransformCtx,
) -> Option<ResourceKind> {
    detect_resource_kind_from_type(ty)
        .or_else(|| init.and_then(|expr| detect_resource_kind_from_expr(expr, ctx)))
}

fn detect_resource_kind_from_type(ty: Option<&syn::Type>) -> Option<ResourceKind> {
    let syn::Type::Path(path) = ty? else {
        return None;
    };

    resource_kind_for_ident(path.path.segments.last()?.ident.to_string().as_str())
}

fn detect_resource_kind_from_expr(expr: &syn::Expr, ctx: &TransformCtx) -> Option<ResourceKind> {
    match expr {
        syn::Expr::Call(call) => detect_resource_kind_from_callable(call.func.as_ref()),
        syn::Expr::MethodCall(method_call) => {
            if matches!(method_call.method.to_string().as_str(), "unwrap" | "expect") {
                return detect_resource_kind_from_expr(&method_call.receiver, ctx);
            }

            match method_call.method.to_string().as_str() {
                "open" => Some(ResourceKind::File),
                "connect" => Some(ResourceKind::Network),
                _ => None,
            }
        }
        syn::Expr::Path(path) => {
            let ident = single_ident(path.path.segments.iter().map(|segment| &segment.ident))?;
            ctx.lookup(ident)?.resource_kind
        }
        syn::Expr::Paren(paren) => detect_resource_kind_from_expr(&paren.expr, ctx),
        syn::Expr::Group(group) => detect_resource_kind_from_expr(&group.expr, ctx),
        _ => None,
    }
}

fn detect_resource_kind_from_callable(expr: &syn::Expr) -> Option<ResourceKind> {
    let syn::Expr::Path(path) = expr else {
        return None;
    };

    let mut segments = path
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string());
    let first = segments.next()?;
    let second = segments.next();
    let third = segments.next();

    match (first.as_str(), second.as_deref(), third.as_deref()) {
        ("File", Some("open"), None) => Some(ResourceKind::File),
        ("std", Some("fs"), Some("File")) => Some(ResourceKind::File),
        ("TcpStream", Some("connect"), None) => Some(ResourceKind::Network),
        ("UdpSocket", Some("bind"), None) => Some(ResourceKind::Network),
        _ => None,
    }
}

fn resource_kind_for_ident(ident: &str) -> Option<ResourceKind> {
    match ident {
        "File" => Some(ResourceKind::File),
        "TcpStream" | "UdpSocket" => Some(ResourceKind::Network),
        "Mutex" | "RwLock" => Some(ResourceKind::Lock),
        "Child" | "Command" => Some(ResourceKind::Os),
        _ => None,
    }
}

fn single_ident<'a>(mut idents: impl Iterator<Item = &'a syn::Ident>) -> Option<&'a syn::Ident> {
    let ident = idents.next()?;
    if idents.next().is_some() {
        return None;
    }
    Some(ident)
}
