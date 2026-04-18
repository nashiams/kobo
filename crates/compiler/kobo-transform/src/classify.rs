use std::collections::{HashMap, HashSet};

use kobo_ir::{KirNodeId, ResourceKind};
use kobo_parser::{KoboBinding, KoboBindingKind};

use crate::small_clone::{type_small_clone_profile, SmallCloneProfile};

/// All Rust primitive types that implement `Copy`. Used by `is_copy_type` to
/// avoid wrapping stack-value types.
pub(crate) const PRIMITIVE_COPY_TYPES: &[&str] = &[
    "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32",
    "u64", "u128", "usize",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BindingMetadata {
    pub resource_kind: Option<ResourceKind>,
    pub is_copy_known: bool,
    pub is_generic: bool,
    pub small_clone_profile: Option<SmallCloneProfile>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BindingState {
    pub decl_id: KirNodeId,
    pub kind: KoboBindingKind,
    pub resource_kind: Option<ResourceKind>,
    pub is_copy_known: bool,
    pub is_generic: bool,
    pub small_clone_profile: Option<SmallCloneProfile>,
}

/// Scope-aware context used while transform resolves concrete binding identity.
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

pub(crate) fn binding_metadata(
    binding: &KoboBinding,
    init: Option<&syn::Expr>,
    ctx: &TransformCtx,
    generic_params: &HashSet<String>,
    small_clone_profiles: &HashMap<String, SmallCloneProfile>,
) -> BindingMetadata {
    BindingMetadata {
        resource_kind: detect_resource_kind(binding.ty.as_ref(), init, ctx),
        is_copy_known: binding.ty.as_ref().is_some_and(is_copy_type)
            || init.is_some_and(|expr| is_copy_expr(expr, ctx)),
        is_generic: binding
            .ty
            .as_ref()
            .is_some_and(|ty| is_generic_type(ty, generic_params))
            || init.is_some_and(|expr| is_generic_expr(expr, ctx)),
        small_clone_profile: binding
            .ty
            .as_ref()
            .and_then(|ty| type_small_clone_profile(ty, small_clone_profiles))
            .or_else(|| {
                init.and_then(|expr| small_clone_profile_from_expr(expr, ctx, small_clone_profiles))
            }),
    }
}

/// Returns `true` if `ty` is a `Copy` type that should never be wrapped.
pub(crate) fn is_copy_type(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Path(path) => {
            let Some(segment) = path.path.segments.last() else {
                return false;
            };

            PRIMITIVE_COPY_TYPES.contains(&segment.ident.to_string().as_str())
        }
        syn::Type::Tuple(tuple) if tuple.elems.is_empty() => true,
        syn::Type::Tuple(tuple) => tuple.elems.iter().all(is_copy_type),
        syn::Type::Array(array) => is_copy_type(&array.elem),
        syn::Type::Reference(r) => r.mutability.is_none(),
        _ => false,
    }
}

pub(crate) fn detect_resource_kind(
    ty: Option<&syn::Type>,
    init: Option<&syn::Expr>,
    ctx: &TransformCtx,
) -> Option<ResourceKind> {
    detect_resource_kind_from_type(ty)
        .or_else(|| init.and_then(|expr| detect_resource_kind_from_expr(expr, ctx)))
}

pub(crate) fn is_generic_type(ty: &syn::Type, generic_params: &HashSet<String>) -> bool {
    let syn::Type::Path(path) = ty else {
        return false;
    };

    path.qself.is_none()
        && path.path.segments.len() == 1
        && generic_params.contains(&path.path.segments[0].ident.to_string())
}

fn is_copy_expr(expr: &syn::Expr, ctx: &TransformCtx) -> bool {
    match expr {
        syn::Expr::Lit(lit) => matches!(
            lit.lit,
            syn::Lit::Int(_)
                | syn::Lit::Float(_)
                | syn::Lit::Bool(_)
                | syn::Lit::Char(_)
                | syn::Lit::Byte(_)
        ),
        syn::Expr::Path(path) => {
            let Some(ident) = single_ident(path.path.segments.iter().map(|segment| &segment.ident))
            else {
                return false;
            };

            ctx.lookup(ident)
                .is_some_and(|binding| binding.is_copy_known)
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

fn is_generic_expr(expr: &syn::Expr, ctx: &TransformCtx) -> bool {
    match expr {
        syn::Expr::Path(path) => {
            let Some(ident) = single_ident(path.path.segments.iter().map(|segment| &segment.ident))
            else {
                return false;
            };

            ctx.lookup(ident).is_some_and(|binding| binding.is_generic)
        }
        syn::Expr::Paren(paren) => is_generic_expr(&paren.expr, ctx),
        syn::Expr::Group(group) => is_generic_expr(&group.expr, ctx),
        _ => false,
    }
}

fn small_clone_profile_from_expr(
    expr: &syn::Expr,
    ctx: &TransformCtx,
    small_clone_profiles: &HashMap<String, SmallCloneProfile>,
) -> Option<SmallCloneProfile> {
    match expr {
        syn::Expr::Struct(expr_struct) => {
            let ident = expr_struct.path.segments.last()?.ident.to_string();
            small_clone_profiles.get(&ident).copied()
        }
        syn::Expr::Path(path) => {
            let ident = single_ident(path.path.segments.iter().map(|segment| &segment.ident))?;
            ctx.lookup(ident)?.small_clone_profile
        }
        syn::Expr::Paren(paren) => {
            small_clone_profile_from_expr(&paren.expr, ctx, small_clone_profiles)
        }
        syn::Expr::Group(group) => {
            small_clone_profile_from_expr(&group.expr, ctx, small_clone_profiles)
        }
        _ => None,
    }
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
