use super::{
    path_ends_with, path_last_ident, BoundaryPolicyFact, BoundaryPolicyMap, ExprLit, File, HashMap,
    Item, ItemUse, Lit, MetaNameValue, Punctuated, ScenarioBoundaryPolicy,
};
pub(super) fn collect_boundary_policies(file: &File) -> BoundaryPolicyMap {
    let mut policies = HashMap::new();
    for item in &file.items {
        let Item::Use(item_use) = item else {
            continue;
        };
        if let Some(policy) = boundary_policy_from_use(item_use) {
            policies.insert(policy.0, policy.1);
        }
    }
    policies
}

pub(super) fn boundary_policy_from_use(item_use: &ItemUse) -> Option<(String, BoundaryPolicyFact)> {
    item_use.attrs.iter().find_map(boundary_policy_from_attr)
}

pub(super) fn boundary_policy_from_attr(
    attr: &syn::Attribute,
) -> Option<(String, BoundaryPolicyFact)> {
    if path_ends_with(attr.path(), &["kobo", "boundary"]) {
        return parse_boundary_attr(attr, ScenarioBoundaryPolicy::Unselected);
    }
    if path_ends_with(attr.path(), &["kobo", "record"]) {
        return parse_boundary_attr(attr, ScenarioBoundaryPolicy::Record);
    }
    if path_ends_with(attr.path(), &["kobo", "activity"]) {
        return parse_boundary_attr(attr, ScenarioBoundaryPolicy::Activity);
    }
    None
}

pub(super) fn parse_boundary_attr(
    attr: &syn::Attribute,
    default_policy: ScenarioBoundaryPolicy,
) -> Option<(String, BoundaryPolicyFact)> {
    let syn::Meta::List(list) = &attr.meta else {
        return None;
    };
    let entries = list
        .parse_args_with(Punctuated::<MetaNameValue, syn::Token![,]>::parse_terminated)
        .ok()?;
    let mut crate_name = None;
    let mut policy = default_policy;
    let mut reason = None;

    for entry in entries {
        let Some(key) = path_last_ident(&entry.path) else {
            continue;
        };
        let syn::Expr::Lit(ExprLit {
            lit: Lit::Str(value),
            ..
        }) = entry.value
        else {
            continue;
        };
        match key.as_str() {
            "crate" => crate_name = Some(value.value()),
            "policy" => policy = ScenarioBoundaryPolicy::from_str(&value.value()),
            "reason" => reason = Some(value.value()),
            _ => {}
        }
    }

    let crate_name = crate_name?;
    Some((crate_name, BoundaryPolicyFact { policy, reason }))
}
