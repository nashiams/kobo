use syn::spanned::Spanned;

pub(super) struct ParsedMustCallAction {
    pub(super) name: String,
    pub(super) span: proc_macro2::Span,
}

pub(super) enum MustCallAttrResult {
    NotMustCall,
    Valid {
        actions: Vec<ParsedMustCallAction>,
        attr_span: proc_macro2::Span,
    },
    Invalid {
        message: String,
        attr_span: proc_macro2::Span,
    },
}

// ---------------------------------------------------------------------------
// Shared helper
// ---------------------------------------------------------------------------

/// Check if a `syn::Path` matches `kobo::<name>`.
fn is_kobo_path(path: &syn::Path, name: &str) -> bool {
    let segs: Vec<_> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    segs.len() == 2 && segs[0] == "kobo" && segs[1] == name
}

pub(super) fn parse_must_call_attr(attr: &syn::Attribute) -> MustCallAttrResult {
    match &attr.meta {
        syn::Meta::List(list) if is_kobo_path(&list.path, "must_call") => {
            parse_must_call_actions(list.tokens.clone(), attr.span())
        }
        syn::Meta::Path(path) if is_kobo_path(path, "must_call") => MustCallAttrResult::Invalid {
            message: "`#[kobo::must_call]` requires an action list".to_owned(),
            attr_span: attr.span(),
        },
        syn::Meta::NameValue(nv) if is_kobo_path(&nv.path, "must_call") => {
            MustCallAttrResult::Invalid {
                message: "`#[kobo::must_call]` uses `action | action` syntax".to_owned(),
                attr_span: attr.span(),
            }
        }
        _ => MustCallAttrResult::NotMustCall,
    }
}

fn parse_must_call_actions(
    tokens: proc_macro2::TokenStream,
    attr_span: proc_macro2::Span,
) -> MustCallAttrResult {
    let mut actions = Vec::new();
    let mut expect_action = true;

    for token in tokens {
        if expect_action {
            match token {
                proc_macro2::TokenTree::Ident(ident) => {
                    actions.push(ParsedMustCallAction {
                        name: ident.to_string(),
                        span: ident.span(),
                    });
                    expect_action = false;
                }
                _ => {
                    return MustCallAttrResult::Invalid {
                        message: "`#[kobo::must_call]` expects an action name".to_owned(),
                        attr_span,
                    };
                }
            }
        } else {
            match token {
                proc_macro2::TokenTree::Punct(punct)
                    if punct.as_char() == '|' && punct.spacing() == proc_macro2::Spacing::Alone =>
                {
                    expect_action = true;
                }
                _ => {
                    return MustCallAttrResult::Invalid {
                        message: "`#[kobo::must_call]` separates alternatives with a single `|`"
                            .to_owned(),
                        attr_span,
                    };
                }
            }
        }
    }

    if actions.is_empty() {
        return MustCallAttrResult::Invalid {
            message: "`#[kobo::must_call]` requires at least one action".to_owned(),
            attr_span,
        };
    }

    if expect_action {
        return MustCallAttrResult::Invalid {
            message: "`#[kobo::must_call]` is missing an action after `|`".to_owned(),
            attr_span,
        };
    }

    MustCallAttrResult::Valid { actions, attr_span }
}

// ---------------------------------------------------------------------------
// #[kobo::relax] parsing [G5]
// ---------------------------------------------------------------------------

/// Result of parsing a `#[kobo::relax]` attribute from an `syn::Attribute`.
pub(super) enum RelaxAttrResult {
    /// Attribute is not a `kobo::relax` attribute.
    NotRelax,
    /// Valid bare `#[kobo::relax]`.
    Valid(proc_macro2::Span),
    /// Malformed `#[kobo::relax = "..."]` or `#[kobo::relax(...)]` — takes no arguments.
    HasArguments(proc_macro2::Span),
}

/// Parse `#[kobo::relax]` from a single attribute.
pub(super) fn parse_relax_attr(attr: &syn::Attribute) -> RelaxAttrResult {
    match &attr.meta {
        syn::Meta::Path(path) if is_kobo_path(path, "relax") => RelaxAttrResult::Valid(attr.span()),
        syn::Meta::NameValue(nv) if is_kobo_path(&nv.path, "relax") => {
            RelaxAttrResult::HasArguments(attr.span())
        }
        syn::Meta::List(list) if is_kobo_path(&list.path, "relax") => {
            RelaxAttrResult::HasArguments(attr.span())
        }
        _ => RelaxAttrResult::NotRelax,
    }
}

// ---------------------------------------------------------------------------
// #[kobo::migrate] parsing [G6]
// ---------------------------------------------------------------------------

/// Result of parsing a `#[kobo::migrate]` attribute.
pub(super) enum MigrateAttrResult {
    /// Attribute is not a `kobo::migrate` attribute.
    NotMigrate,
    /// Valid bare `#[kobo::migrate]` (no reason).
    Valid(proc_macro2::Span),
    /// Valid `#[kobo::migrate = "reason"]` with a reason string.
    ValidWithReason(String, proc_macro2::Span),
    /// Malformed `#[kobo::migrate(...)]` — arguments not allowed in list form.
    HasArguments(proc_macro2::Span),
}

/// Parse `#[kobo::migrate]` or `#[kobo::migrate = "reason"]` from a single attribute.
pub(super) fn parse_migrate_attr(attr: &syn::Attribute) -> MigrateAttrResult {
    match &attr.meta {
        syn::Meta::Path(path) if is_kobo_path(path, "migrate") => {
            MigrateAttrResult::Valid(attr.span())
        }
        syn::Meta::NameValue(nv) if is_kobo_path(&nv.path, "migrate") => {
            if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(lit_str),
                ..
            }) = &nv.value
            {
                MigrateAttrResult::ValidWithReason(lit_str.value(), attr.span())
            } else {
                MigrateAttrResult::HasArguments(attr.span())
            }
        }
        syn::Meta::List(list) if is_kobo_path(&list.path, "migrate") => {
            MigrateAttrResult::HasArguments(attr.span())
        }
        _ => MigrateAttrResult::NotMigrate,
    }
}

// ---------------------------------------------------------------------------
// #[kobo::known_debt] parsing
// ---------------------------------------------------------------------------

/// Result of parsing a `#[kobo::known_debt]` attribute.
pub(super) enum KnownDebtResult {
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
pub(super) fn parse_known_debt_attr(attr: &syn::Attribute) -> KnownDebtResult {
    match &attr.meta {
        syn::Meta::NameValue(nv) if is_kobo_path(&nv.path, "known_debt") => {
            if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(lit_str),
                ..
            }) = &nv.value
            {
                let reason = lit_str.value();
                if reason.is_empty() {
                    KnownDebtResult::EmptyReason(attr.span())
                } else {
                    KnownDebtResult::Valid(reason, attr.span())
                }
            } else {
                KnownDebtResult::NotKnownDebt
            }
        }
        syn::Meta::Path(path) if is_kobo_path(path, "known_debt") => {
            KnownDebtResult::MissingReason(attr.span())
        }
        _ => KnownDebtResult::NotKnownDebt,
    }
}

// ---------------------------------------------------------------------------
// #[kobo::async_shared] parsing [BUG 7]
// ---------------------------------------------------------------------------

/// Result of parsing a `#[kobo::async_shared]` attribute.
pub(super) enum AsyncSharedAttrResult {
    /// Attribute is not a `kobo::async_shared` attribute.
    NotAsyncShared,
    /// Valid bare `#[kobo::async_shared]`.
    Valid,
    /// Malformed `#[kobo::async_shared = ...]` or `#[kobo::async_shared(...)]` — takes no args.
    HasArguments,
}

/// Parse `#[kobo::async_shared]` from a single attribute.
pub(super) fn parse_async_shared_attr(attr: &syn::Attribute) -> AsyncSharedAttrResult {
    match &attr.meta {
        syn::Meta::Path(path) if is_kobo_path(path, "async_shared") => AsyncSharedAttrResult::Valid,
        syn::Meta::NameValue(nv) if is_kobo_path(&nv.path, "async_shared") => {
            AsyncSharedAttrResult::HasArguments
        }
        syn::Meta::List(list) if is_kobo_path(&list.path, "async_shared") => {
            AsyncSharedAttrResult::HasArguments
        }
        _ => AsyncSharedAttrResult::NotAsyncShared,
    }
}
