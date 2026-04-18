/// Edge-case tests for `is_copy_type` — TRAP-03 and P0-4 invariant audit.
///
/// BUG: `is_copy_type` treats ALL references as Copy via
///   `syn::Type::Reference(_) => true`
/// but `&mut T` is NOT Copy in Rust (exclusive borrow cannot be duplicated).
/// These tests document the expected correct behavior and expose the bug.
use syn::parse_quote;

use super::classify::is_copy_type;

// ------------------------------------------------------------------
// &T IS Copy (shared reference can always be duplicated)
// ------------------------------------------------------------------

#[test]
fn shared_ref_is_copy() {
    let ty: syn::Type = parse_quote!(&i32);
    assert!(is_copy_type(&ty), "&i32 should be Copy");
}

#[test]
fn shared_ref_to_vec_is_copy() {
    let ty: syn::Type = parse_quote!(&Vec<u8>);
    assert!(is_copy_type(&ty), "&Vec<u8> (the reference itself) should be Copy");
}

// ------------------------------------------------------------------
// &mut T is NOT Copy (exclusive borrow — BUG: currently returns true)
// ------------------------------------------------------------------

#[test]
fn mut_ref_is_not_copy() {
    let ty: syn::Type = parse_quote!(&mut i32);
    assert!(
        !is_copy_type(&ty),
        "&mut i32 must NOT be Copy — exclusive borrows cannot be duplicated"
    );
}

#[test]
fn mut_ref_to_vec_is_not_copy() {
    let ty: syn::Type = parse_quote!(&mut Vec<u8>);
    assert!(
        !is_copy_type(&ty),
        "&mut Vec<u8> must NOT be Copy"
    );
}

// ------------------------------------------------------------------
// Tuple edge cases
// ------------------------------------------------------------------

#[test]
fn unit_tuple_is_copy() {
    let ty: syn::Type = parse_quote!(());
    assert!(is_copy_type(&ty), "() should be Copy");
}

#[test]
fn tuple_of_copy_is_copy() {
    let ty: syn::Type = parse_quote!((i32, bool));
    assert!(is_copy_type(&ty), "(i32, bool) should be Copy");
}

#[test]
fn tuple_with_non_copy_is_not_copy() {
    let ty: syn::Type = parse_quote!((i32, String));
    assert!(!is_copy_type(&ty), "(i32, String) should NOT be Copy");
}

// ------------------------------------------------------------------
// Array edge cases
// ------------------------------------------------------------------

#[test]
fn array_of_copy_is_copy() {
    let ty: syn::Type = parse_quote!([u8; 4]);
    assert!(is_copy_type(&ty), "[u8; 4] should be Copy");
}

#[test]
fn array_of_non_copy_is_not_copy() {
    let ty: syn::Type = parse_quote!([String; 3]);
    assert!(!is_copy_type(&ty), "[String; 3] should NOT be Copy");
}

// ------------------------------------------------------------------
// Non-primitive types
// ------------------------------------------------------------------

#[test]
fn string_is_not_copy() {
    let ty: syn::Type = parse_quote!(String);
    assert!(!is_copy_type(&ty), "String should NOT be Copy");
}

#[test]
fn vec_is_not_copy() {
    let ty: syn::Type = parse_quote!(Vec<i32>);
    assert!(!is_copy_type(&ty), "Vec<i32> should NOT be Copy");
}

#[test]
fn option_is_not_copy() {
    let ty: syn::Type = parse_quote!(Option<String>);
    assert!(!is_copy_type(&ty), "Option<String> should NOT be Copy");
}

// ------------------------------------------------------------------
// Nested reference edge case (&&T — Copy, &&mut T — still Copy
// because the outer & is shared)
// ------------------------------------------------------------------

#[test]
fn double_shared_ref_is_copy() {
    let ty: syn::Type = parse_quote!(&&i32);
    assert!(is_copy_type(&ty), "&&i32 should be Copy (outer ref is shared)");
}

// ------------------------------------------------------------------
// BUG-22: PRIMITIVE_COPY_TYPES constant coverage
// ------------------------------------------------------------------

#[test]
fn primitive_copy_types_contains_all_expected() {
    use super::classify::PRIMITIVE_COPY_TYPES;
    let expected = [
        "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16",
        "u32", "u64", "u128", "usize",
    ];
    for prim in &expected {
        assert!(
            PRIMITIVE_COPY_TYPES.contains(prim),
            "PRIMITIVE_COPY_TYPES should contain {prim}"
        );
    }
    assert_eq!(
        PRIMITIVE_COPY_TYPES.len(),
        expected.len(),
        "PRIMITIVE_COPY_TYPES should have exactly {} entries",
        expected.len()
    );
}

#[test]
fn non_primitive_non_reference_is_not_copy() {
    let ty: syn::Type = parse_quote!(HashMap<String, Vec<i32>>);
    assert!(!is_copy_type(&ty), "HashMap should NOT be Copy");
}
