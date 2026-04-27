use syn::spanned::Spanned;

use kobo_ir::{FieldTypeShape, KirStructDef, KirStructFieldDef};

use super::attrs::{parse_known_debt_attr, KnownDebtResult};
use super::TransformFactsBuilder;

impl TransformFactsBuilder<'_> {
    /// Collect a struct definition for K0080-P pattern detection.
    ///
    /// Parses `#[kobo::known_debt = "reason"]` attributes and extracts
    /// simplified field type shapes. Only shapes relevant to P1–P4 are kept;
    /// everything else becomes `FieldTypeShape::Other`.
    pub(super) fn collect_struct_def(&mut self, item: &syn::ItemStruct) {
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
                    known_debt_parse_error =
                        Some("`#[kobo::known_debt]` requires a reason string".to_owned());
                }
                KnownDebtResult::EmptyReason(attr_span) => {
                    known_debt_span = Some(self.ast.span_from_syn(attr_span));
                    known_debt_parse_error =
                        Some("`#[kobo::known_debt]` reason string must not be empty".to_owned());
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
}

// ---------------------------------------------------------------------------
// Free helpers for struct-def type shape analysis
// ---------------------------------------------------------------------------

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
                    if let Some(syn::GenericArgument::Type(inner_ty)) =
                        first_generic_arg(&type_path.path.segments[0])
                    {
                        if let FieldTypeShape::RcRefCellOf(name) =
                            field_type_shape(inner_ty, struct_name)
                        {
                            return FieldTypeShape::OptionRcRefCellOf(name);
                        }
                    }
                    FieldTypeShape::Other
                }
                // Vec<...>
                [vec] if vec == "Vec" => {
                    if let Some(syn::GenericArgument::Type(inner_ty)) =
                        first_generic_arg(&type_path.path.segments[0])
                    {
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
    if let syn::GenericArgument::Type(syn::Type::Path(tp)) = arg {
        let segs: Vec<_> = tp
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();
        if segs.first().map(String::as_str) == Some("RefCell") {
            if let Some(syn::GenericArgument::Type(syn::Type::Path(vp))) =
                first_generic_arg(tp.path.segments.first()?)
            {
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
    None
}
