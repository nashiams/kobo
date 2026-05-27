use super::*;

pub(super) fn boundaries_from_operations(operations: &[ScenarioOp]) -> Vec<ScenarioBoundary> {
    let mut boundaries = Vec::new();
    for operation in operations {
        let ScenarioOpKind::ExternalBoundary {
            crate_name,
            call_path,
            policy,
            ..
        } = &operation.kind
        else {
            continue;
        };
        let boundary_name = call_path.as_ref().unwrap_or(crate_name);
        if boundaries
            .iter()
            .any(|boundary: &ScenarioBoundary| boundary.name == *boundary_name)
        {
            continue;
        }
        boundaries.push(ScenarioBoundary {
            span: operation.span,
            name: boundary_name.clone(),
            decision: policy.as_str().to_owned(),
        });
    }
    boundaries
}

pub(super) fn matches_bool_pat(pat: &Pat, value: Option<bool>) -> bool {
    match (pat, value) {
        (
            Pat::Lit(ExprLit {
                lit: Lit::Bool(candidate),
                ..
            }),
            Some(value),
        ) => candidate.value == value,
        (Pat::Wild(_), _) => true,
        _ => false,
    }
}

pub(super) fn function_returns_bool_literal(function: &ItemFn) -> Option<bool> {
    function
        .block
        .stmts
        .iter()
        .find_map(|statement| match statement {
            Stmt::Expr(
                Expr::Lit(ExprLit {
                    lit: Lit::Bool(value),
                    ..
                }),
                _,
            ) => Some(value.value),
            _ => None,
        })
}
pub(super) fn local_suppression_reason(local: &Local) -> Option<String> {
    local.attrs.iter().find_map(|attr| {
        if !attr.path().segments.iter().any(|segment| {
            segment.ident == "suppress_liveness" || segment.ident == "allow_liveness_escape"
        }) {
            return None;
        }
        let rendered = match &attr.meta {
            Meta::List(list) => list.tokens.to_string(),
            other => other.to_token_stream().to_string(),
        };
        extract_quoted_value(&rendered, "reason")
            .or_else(|| Some("reasoned suppression".to_owned()))
    })
}

pub(super) fn extract_quoted_value(rendered: &str, key: &str) -> Option<String> {
    let marker = format!("{key} = \"");
    let start = rendered.find(&marker)? + marker.len();
    let end = rendered[start..].find('"')? + start;
    Some(rendered[start..end].to_owned())
}
pub(super) fn modeled_boundary(call: &ExprMethodCall) -> Option<ScenarioModeledBoundary> {
    if receiver_has_ward_member(call.receiver.as_ref(), "time") {
        return Some(ScenarioModeledBoundary::WardTime);
    }
    if receiver_has_ward_member(call.receiver.as_ref(), "random") {
        return Some(ScenarioModeledBoundary::WardRandom);
    }
    if receiver_has_ward_member(call.receiver.as_ref(), "task")
        || receiver_is_ward_method(call.receiver.as_ref(), "task")
    {
        return Some(ScenarioModeledBoundary::WardTask);
    }
    if receiver_is_ward_path(call.receiver.as_ref()) && call.method == "task" {
        return Some(ScenarioModeledBoundary::WardTask);
    }
    None
}
pub(super) fn receiver_has_ward_member(expr: &Expr, member: &str) -> bool {
    match expr {
        Expr::Field(field) => {
            member_name(&field.member).as_deref() == Some(member)
                && receiver_is_ward_path(field.base.as_ref())
        }
        Expr::MethodCall(call) => receiver_has_ward_member(call.receiver.as_ref(), member),
        _ => false,
    }
}

pub(super) fn receiver_is_ward_method(expr: &Expr, method: &str) -> bool {
    matches!(expr, Expr::MethodCall(call) if call.method == method && receiver_is_ward_path(call.receiver.as_ref()))
}

pub(super) fn receiver_is_ward_path(expr: &Expr) -> bool {
    expr_path_ident(expr).as_deref() == Some("ward")
}

pub(super) fn member_name(member: &syn::Member) -> Option<String> {
    match member {
        syn::Member::Named(ident) => Some(ident.to_string()),
        syn::Member::Unnamed(_) => None,
    }
}

pub(super) fn must_call_actions(attr: &syn::Attribute) -> Option<Vec<String>> {
    let syn::Meta::List(list) = &attr.meta else {
        return None;
    };
    let actions = list
        .tokens
        .to_string()
        .split('|')
        .map(str::trim)
        .map(|part| part.trim_matches(','))
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    (!actions.is_empty()).then_some(actions)
}

pub(super) fn pat_ident(pat: &Pat) -> Option<String> {
    match pat {
        Pat::Ident(PatIdent { ident, .. }) => Some(ident.to_string()),
        Pat::Type(PatType { pat, .. }) => pat_ident(pat),
        Pat::Wild(_) => None,
        _ => None,
    }
}

pub(super) fn fn_arg_ident(arg: &syn::FnArg) -> Option<String> {
    match arg {
        syn::FnArg::Typed(pat_type) => pat_ident(pat_type.pat.as_ref()),
        syn::FnArg::Receiver(_) => None,
    }
}

pub(super) fn is_handler_obligation_argument(argument: &PatType) -> bool {
    handler_argument_type_name(argument)
        .as_deref()
        .is_some_and(|type_name| !is_non_obligation_handler_type(type_name))
}

pub(super) fn handler_argument_type_name(argument: &PatType) -> Option<String> {
    let syn::Type::Path(type_path) = argument.ty.as_ref() else {
        return None;
    };
    type_path
        .path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

pub(super) fn is_non_obligation_handler_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "bool"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "f32"
            | "f64"
            | "String"
            | "str"
    )
}

pub(super) fn expr_path_ident(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Path(path) => path_last_ident(&path.path),
        Expr::Reference(reference) => expr_path_ident(reference.expr.as_ref()),
        Expr::Paren(paren) => expr_path_ident(paren.expr.as_ref()),
        _ => None,
    }
}

pub(super) fn receiver_ident(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Path(path) => path_last_ident(&path.path),
        Expr::Reference(reference) => receiver_ident(reference.expr.as_ref()),
        Expr::Paren(paren) => receiver_ident(paren.expr.as_ref()),
        _ => None,
    }
}

pub(super) fn path_first_ident(path: &Path) -> Option<String> {
    path.segments
        .first()
        .map(|segment| segment.ident.to_string())
}

pub(super) fn path_last_ident(path: &Path) -> Option<String> {
    path.segments
        .last()
        .map(|segment| segment.ident.to_string())
}

pub(super) fn path_to_string(path: &Path) -> String {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

pub(super) fn path_starts_with(path: &[String], prefix: &[&str]) -> bool {
    path.len() >= prefix.len()
        && path
            .iter()
            .zip(prefix.iter())
            .all(|(segment, expected)| segment == expected)
}

pub(super) fn path_ends_with_segments(path: &[String], suffix: &[&str]) -> bool {
    path.len() >= suffix.len()
        && path[path.len() - suffix.len()..]
            .iter()
            .zip(suffix.iter())
            .all(|(segment, expected)| segment == expected)
}

pub(super) fn path_ends_with(path: &Path, suffix: &[&str]) -> bool {
    let segments = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    if suffix.len() > segments.len() {
        return false;
    }
    segments[segments.len() - suffix.len()..]
        .iter()
        .zip(suffix)
        .all(|(segment, expected)| segment == expected)
}

pub(super) fn select_branch_count(mac: &Macro) -> u32 {
    let branch_count = mac.tokens.to_string().matches("=>").count();
    branch_count.max(1) as u32
}
