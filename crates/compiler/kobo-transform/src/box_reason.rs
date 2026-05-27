use std::collections::HashMap;

use kobo_ir::BoxReason;
use kobo_parser::KoboFile;

pub(crate) fn collect_box_reasons(
    ast: &KoboFile,
    small_struct_clone_threshold_bytes: usize,
) -> HashMap<String, BoxReason> {
    let mut reasons = HashMap::new();

    for item in &ast.inner.items {
        let syn::Item::Struct(item_struct) = item else {
            continue;
        };

        // Recursive type handling belongs to type-definition validation, not
        // Binding-level BoxOwned lowering keeps the enum variant reserved
        // Leave this reason reserved; transform does not synthesize it today.
        if struct_size_bytes(item_struct)
            .is_some_and(|size| size >= small_struct_clone_threshold_bytes)
            && !struct_has_heap_fields(item_struct)
        {
            reasons.insert(item_struct.ident.to_string(), BoxReason::StackSizeHeuristic);
        }
    }

    reasons
}

pub(crate) fn type_box_reason(
    ty: &syn::Type,
    box_reasons: &HashMap<String, BoxReason>,
) -> Option<BoxReason> {
    match ty {
        syn::Type::TraitObject(_) => Some(BoxReason::TraitObject),
        syn::Type::Path(path) => {
            let ident = path.path.segments.last()?.ident.to_string();
            box_reasons.get(&ident).copied()
        }
        _ => None,
    }
}

fn struct_size_bytes(item_struct: &syn::ItemStruct) -> Option<usize> {
    let mut size = 0usize;
    for ty in struct_field_types(item_struct) {
        size += primitive_type_size(ty)?;
    }
    Some(size)
}

fn struct_has_heap_fields(item_struct: &syn::ItemStruct) -> bool {
    struct_field_types(item_struct).any(type_is_heap_like)
}

fn struct_field_types(item_struct: &syn::ItemStruct) -> impl Iterator<Item = &syn::Type> {
    item_struct.fields.iter().map(|field| &field.ty)
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

    use super::collect_box_reasons;

    #[test]
    fn recursive_structs_do_not_produce_box_reason_in_v0_3() {
        let source = r#"
struct Node {
    next: Node,
}
"#;
        let mut id_gen = NodeIdGen::new();
        let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");

        let reasons = collect_box_reasons(&ast, 512);

        assert!(reasons.is_empty());
    }

    #[test]
    fn large_plain_structs_still_get_stack_size_reason() {
        let source = r#"
struct Large {
    a: u64,
    b: u64,
    c: u64,
}
"#;
        let mut id_gen = NodeIdGen::new();
        let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");

        let reasons = collect_box_reasons(&ast, 16);

        assert_eq!(
            reasons.get("Large").copied(),
            Some(kobo_ir::BoxReason::StackSizeHeuristic)
        );
    }
}
