use std::collections::{HashMap, HashSet};

use kobo_parser::KoboFile;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SmallCloneProfile {
    Eligible,
    KnownHeapField,
    OpaqueField,
    TooLarge,
    MissingCloneImpl,
}

pub(crate) fn collect_small_clone_profiles(
    ast: &KoboFile,
    threshold_bytes: usize,
) -> HashMap<String, SmallCloneProfile> {
    let struct_defs = ast
        .inner
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Struct(item_struct) => Some((item_struct.ident.to_string(), item_struct)),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    let mut cache = HashMap::new();

    for name in struct_defs.keys() {
        let _ = analyze_struct(
            name,
            &struct_defs,
            threshold_bytes,
            &mut cache,
            &mut HashSet::new(),
        );
    }

    cache
}

pub(crate) fn type_small_clone_profile(
    ty: &syn::Type,
    profiles: &HashMap<String, SmallCloneProfile>,
) -> Option<SmallCloneProfile> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    let ident = path.path.segments.last()?.ident.to_string();
    profiles.get(&ident).copied()
}

fn analyze_struct(
    name: &str,
    struct_defs: &HashMap<String, &syn::ItemStruct>,
    threshold_bytes: usize,
    cache: &mut HashMap<String, SmallCloneProfile>,
    visiting: &mut HashSet<String>,
) -> SmallCloneProfile {
    if let Some(profile) = cache.get(name).copied() {
        return profile;
    }

    if !visiting.insert(name.to_owned()) {
        return SmallCloneProfile::OpaqueField;
    }

    let Some(item_struct) = struct_defs.get(name).copied() else {
        visiting.remove(name);
        return SmallCloneProfile::OpaqueField;
    };

    let mut total_size = 0usize;
    let mut profile = SmallCloneProfile::Eligible;

    for field in &item_struct.fields {
        let field_profile =
            analyze_field_type(&field.ty, struct_defs, threshold_bytes, cache, visiting);
        total_size = total_size.saturating_add(field_profile.size_bytes);
        profile = merge_profile(profile, field_profile.profile);
        if matches!(
            profile,
            SmallCloneProfile::KnownHeapField | SmallCloneProfile::OpaqueField
        ) {
            break;
        }
    }

    if profile == SmallCloneProfile::Eligible && total_size > threshold_bytes {
        profile = SmallCloneProfile::TooLarge;
    }

    if profile == SmallCloneProfile::Eligible && !struct_has_clone_like_derive(item_struct) {
        profile = SmallCloneProfile::MissingCloneImpl;
    }

    visiting.remove(name);
    cache.insert(name.to_owned(), profile);
    profile
}

struct FieldAnalysis {
    profile: SmallCloneProfile,
    size_bytes: usize,
}

fn analyze_field_type(
    ty: &syn::Type,
    struct_defs: &HashMap<String, &syn::ItemStruct>,
    threshold_bytes: usize,
    cache: &mut HashMap<String, SmallCloneProfile>,
    visiting: &mut HashSet<String>,
) -> FieldAnalysis {
    if type_is_heap_like(ty) {
        return FieldAnalysis {
            profile: SmallCloneProfile::KnownHeapField,
            size_bytes: 0,
        };
    }

    match ty {
        syn::Type::Array(array) => {
            let len = array_len(array).unwrap_or(0);
            let inner =
                analyze_field_type(&array.elem, struct_defs, threshold_bytes, cache, visiting);
            FieldAnalysis {
                profile: inner.profile,
                size_bytes: inner.size_bytes.saturating_mul(len),
            }
        }
        syn::Type::Paren(paren) => {
            analyze_field_type(&paren.elem, struct_defs, threshold_bytes, cache, visiting)
        }
        syn::Type::Path(path) => {
            let Some(segment) = path.path.segments.last() else {
                return opaque_field();
            };
            if let Some(size_bytes) = primitive_type_size(ty) {
                return FieldAnalysis {
                    profile: SmallCloneProfile::Eligible,
                    size_bytes,
                };
            }
            if path.path.segments.len() > 1 {
                return opaque_field();
            }
            let ident = segment.ident.to_string();
            if let Some(item_struct) = struct_defs.get(&ident) {
                let nested_profile =
                    analyze_struct(&ident, struct_defs, threshold_bytes, cache, visiting);
                let nested_size = item_struct
                    .fields
                    .iter()
                    .map(|field| {
                        analyze_field_type(&field.ty, struct_defs, threshold_bytes, cache, visiting)
                            .size_bytes
                    })
                    .fold(0usize, usize::saturating_add);
                return FieldAnalysis {
                    profile: nested_profile,
                    size_bytes: nested_size,
                };
            }

            opaque_field()
        }
        syn::Type::Reference(_) => FieldAnalysis {
            profile: SmallCloneProfile::Eligible,
            size_bytes: 8,
        },
        syn::Type::Tuple(tuple) => {
            let mut total_size = 0usize;
            let mut profile = SmallCloneProfile::Eligible;
            for elem in &tuple.elems {
                let analysis =
                    analyze_field_type(elem, struct_defs, threshold_bytes, cache, visiting);
                total_size = total_size.saturating_add(analysis.size_bytes);
                profile = merge_profile(profile, analysis.profile);
            }
            FieldAnalysis {
                profile,
                size_bytes: total_size,
            }
        }
        _ => opaque_field(),
    }
}

fn opaque_field() -> FieldAnalysis {
    FieldAnalysis {
        profile: SmallCloneProfile::OpaqueField,
        size_bytes: 0,
    }
}

fn merge_profile(current: SmallCloneProfile, next: SmallCloneProfile) -> SmallCloneProfile {
    use SmallCloneProfile::{Eligible, KnownHeapField, MissingCloneImpl, OpaqueField, TooLarge};

    match (current, next) {
        (KnownHeapField, _) | (_, KnownHeapField) => KnownHeapField,
        (OpaqueField, _) | (_, OpaqueField) => OpaqueField,
        (TooLarge, _) | (_, TooLarge) => TooLarge,
        (MissingCloneImpl, _) | (_, MissingCloneImpl) => MissingCloneImpl,
        _ => Eligible,
    }
}

fn struct_has_clone_like_derive(item_struct: &syn::ItemStruct) -> bool {
    item_struct.attrs.iter().any(|attr| {
        if !attr.path().is_ident("derive") {
            return false;
        }
        let mut has_clone_like = false;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("Clone") || meta.path.is_ident("Copy") {
                has_clone_like = true;
            }
            Ok(())
        });
        has_clone_like
    })
}

fn array_len(array: &syn::TypeArray) -> Option<usize> {
    let syn::Expr::Lit(lit) = &array.len else {
        return None;
    };
    let syn::Lit::Int(int_lit) = &lit.lit else {
        return None;
    };
    int_lit.base10_parse::<usize>().ok()
}

fn primitive_type_size(ty: &syn::Type) -> Option<usize> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    let ident = path.path.segments.last()?.ident.to_string();
    let size = match ident.as_str() {
        "u8" | "i8" | "bool" => 1,
        "u16" | "i16" => 2,
        "u32" | "i32" | "f32" | "char" => 4,
        "u64" | "i64" | "f64" | "usize" | "isize" => 8,
        "u128" | "i128" => 16,
        _ => return None,
    };
    Some(size)
}

fn type_is_heap_like(ty: &syn::Type) -> bool {
    let syn::Type::Path(path) = ty else {
        return false;
    };
    let Some(segment) = path.path.segments.last() else {
        return false;
    };

    matches!(
        segment.ident.to_string().as_str(),
        "String" | "Vec" | "Box" | "Rc" | "Arc"
    )
}

#[cfg(test)]
mod tests {
    use kobo_ir::{FileId, NodeIdGen};
    use kobo_parser::parse_file;

    use super::{collect_small_clone_profiles, SmallCloneProfile};

    fn profiles(source: &str) -> std::collections::HashMap<String, SmallCloneProfile> {
        let mut id_gen = NodeIdGen::new();
        let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
        collect_small_clone_profiles(&ast, 128)
    }

    #[test]
    fn small_clone_profile_marks_small_cloneable_structs_as_eligible() {
        let source = r#"
#[derive(Clone, Copy)]
struct SmallCopy {
    a: u64,
    b: u64,
}
"#;

        assert_eq!(
            profiles(source).get("SmallCopy"),
            Some(&SmallCloneProfile::Eligible)
        );
    }

    #[test]
    fn small_clone_profile_rejects_heap_fields() {
        let source = r#"
#[derive(Clone)]
struct DeepClone {
    name: String,
}
"#;

        assert_eq!(
            profiles(source).get("DeepClone"),
            Some(&SmallCloneProfile::KnownHeapField)
        );
    }

    #[test]
    fn small_clone_profile_rejects_large_structs() {
        let source = r#"
#[derive(Clone, Copy)]
struct LargeCopy {
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    a4: u64,
    a5: u64,
    a6: u64,
    a7: u64,
    a8: u64,
    a9: u64,
    a10: u64,
    a11: u64,
    a12: u64,
    a13: u64,
    a14: u64,
    a15: u64,
    a16: u64,
}
"#;

        assert_eq!(
            profiles(source).get("LargeCopy"),
            Some(&SmallCloneProfile::TooLarge)
        );
    }

    #[test]
    fn small_clone_profile_marks_imported_fields_as_opaque() {
        let source = r#"
#[derive(Clone)]
struct MaybeAlloc {
    path: std::path::PathBuf,
}
"#;

        assert_eq!(
            profiles(source).get("MaybeAlloc"),
            Some(&SmallCloneProfile::OpaqueField)
        );
    }
}
