/// Formats a `syn::File` into a `rustfmt`-compliant `.rs` string.
///
/// Uses `prettyplease::unparse` — output is always formatted, never raw tokens.
pub fn emit_file(file: &syn::File) -> String {
    prettyplease::unparse(file)
}
