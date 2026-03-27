use kobo_ir::Kir;

/// Inserts `// kobo: ...` comments into the generated `syn::File`.
///
/// Stub at v0.1: no-op. Full comment insertion is added in v0.2.
pub fn annotate(_file: &mut syn::File, _kir: &Kir) {}
