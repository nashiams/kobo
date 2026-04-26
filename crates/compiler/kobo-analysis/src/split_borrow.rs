//! S-20: Auto split-borrow detection for `self.field` patterns in impl blocks.
//!
//! Detects methods where multiple `self.field` accesses could benefit from
//! split-borrow destructuring. Pure syntactic analysis — no type resolution.

use kobo_ir::{FileId, KoboSpan};

// --- Types ---

/// A method in an impl block where split-borrow destructuring could help.
#[derive(Clone, Debug)]
pub struct SplitBorrowSite {
    pub method_span: KoboSpan,
    pub struct_name: String,
    pub method_name: String,
    pub fields_accessed: Vec<FieldAccess>,
}

/// A single `self.field` access found in a method body.
#[derive(Clone, Debug)]
pub struct FieldAccess {
    pub field_name: String,
    pub access_kind: FieldAccessKind,
    pub span: KoboSpan,
}

/// How a field is accessed through `self`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldAccessKind {
    /// `self.field` (no mutation)
    Read,
    /// `self.field = ...`
    Write,
    /// `self.field.method()` where method takes `&mut self`
    BorrowMut,
}

// --- Span helpers ---

/// Convert a `proc_macro2::Span` to a `KoboSpan` using source text line offsets.
///
/// `proc_macro2` with `span-locations` feature exposes 1-based line and
/// 0-based column. We pre-compute a line-start-offset table from the source.
fn span_to_kobo_span(span: proc_macro2::Span, line_offsets: &[u32], file_id: FileId) -> KoboSpan {
    let start_lc = span.start();
    let end_lc = span.end();

    let start = line_col_to_offset(start_lc.line, start_lc.column, line_offsets);
    let end = line_col_to_offset(end_lc.line, end_lc.column, line_offsets);

    KoboSpan::new(start, end, file_id)
}

/// Build a table mapping 1-based line number → byte offset of the start of that line.
fn build_line_offsets(source: &str) -> Vec<u32> {
    let mut offsets = vec![0u32]; // line 1 starts at byte 0
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            offsets.push((i + 1) as u32);
        }
    }
    offsets
}

fn line_col_to_offset(line: usize, column: usize, line_offsets: &[u32]) -> u32 {
    if line == 0 || line > line_offsets.len() {
        return 0;
    }
    line_offsets[line - 1] + column as u32
}

// --- Detection ---

/// Scan syn items for impl methods that would benefit from split-borrow.
///
/// Detection criteria:
/// 1. Method has `&mut self` receiver
/// 2. Method body contains 2+ distinct `self.field` accesses
/// 3. At least one field access is Write or BorrowMut
/// 4. At least one OTHER field is accessed in the same scope
pub fn detect_split_borrow_sites(items: &[syn::Item]) -> Vec<SplitBorrowSite> {
    detect_split_borrow_sites_with_source(items, "", FileId(0))
}

/// Like `detect_split_borrow_sites` but uses `source` to compute real spans.
pub fn detect_split_borrow_sites_with_source(
    items: &[syn::Item],
    source: &str,
    file_id: FileId,
) -> Vec<SplitBorrowSite> {
    let line_offsets = build_line_offsets(source);
    let mut sites = Vec::new();

    for item in items {
        let syn::Item::Impl(impl_block) = item else {
            continue;
        };

        // Extract struct name from the impl target type.
        let struct_name = match impl_block.self_ty.as_ref() {
            syn::Type::Path(tp) => tp
                .path
                .segments
                .last()
                .map(|seg| seg.ident.to_string())
                .unwrap_or_default(),
            _ => continue,
        };

        for impl_item in &impl_block.items {
            let syn::ImplItem::Fn(method) = impl_item else {
                continue;
            };

            // Only &mut self methods benefit from split-borrow.
            if !has_mut_self_receiver(method) {
                continue;
            }

            let method_name = method.sig.ident.to_string();
            let fields = collect_self_field_accesses(&method.block, &line_offsets, file_id);

            // Need 2+ distinct fields with at least one mutable access.
            let distinct_fields: std::collections::HashSet<&str> =
                fields.iter().map(|f| f.field_name.as_str()).collect();
            let has_mutable = fields.iter().any(|f| {
                matches!(
                    f.access_kind,
                    FieldAccessKind::Write | FieldAccessKind::BorrowMut
                )
            });

            if distinct_fields.len() >= 2 && has_mutable {
                let method_span =
                    span_to_kobo_span(method.sig.ident.span(), &line_offsets, file_id);
                sites.push(SplitBorrowSite {
                    method_span,
                    struct_name: struct_name.clone(),
                    method_name,
                    fields_accessed: fields,
                });
            }
        }
    }

    sites
}

/// Returns true if the method has `&mut self` as its first argument.
fn has_mut_self_receiver(method: &syn::ImplItemFn) -> bool {
    method
        .sig
        .inputs
        .first()
        .map(|arg| matches!(arg, syn::FnArg::Receiver(r) if r.mutability.is_some()))
        .unwrap_or(false)
}

/// Walk the block and collect all `self.field` accesses, classifying each as
/// Read, Write, or BorrowMut.
fn collect_self_field_accesses(
    block: &syn::Block,
    line_offsets: &[u32],
    file_id: FileId,
) -> Vec<FieldAccess> {
    use syn::visit::Visit;

    struct FieldCollector<'a> {
        fields: Vec<FieldAccess>,
        line_offsets: &'a [u32],
        file_id: FileId,
    }

    impl<'a> FieldCollector<'a> {
        fn make_span(&self, span: proc_macro2::Span) -> KoboSpan {
            span_to_kobo_span(span, self.line_offsets, self.file_id)
        }
    }

    impl<'a, 'ast> Visit<'ast> for FieldCollector<'a> {
        fn visit_expr_assign(&mut self, assign: &'ast syn::ExprAssign) {
            // Check if left side is self.field
            if let Some((name, span)) = extract_self_field_with_span(&assign.left) {
                self.fields.push(FieldAccess {
                    field_name: name,
                    access_kind: FieldAccessKind::Write,
                    span: self.make_span(span),
                });
            }
            // Visit right side for reads
            syn::visit::visit_expr(self, &assign.right);
        }

        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            // self.field.method() → the field is borrowed (mutably if method takes &mut self,
            // but we can't know that syntactically). Treat as BorrowMut conservatively.
            if let Some((name, span)) = extract_self_field_with_span(&call.receiver) {
                self.fields.push(FieldAccess {
                    field_name: name,
                    access_kind: FieldAccessKind::BorrowMut,
                    span: self.make_span(span),
                });
                // Visit args but NOT the receiver (already handled).
                for arg in &call.args {
                    syn::visit::visit_expr(self, arg);
                }
                return;
            }
            // Default traversal
            syn::visit::visit_expr_method_call(self, call);
        }

        fn visit_expr_field(&mut self, field: &'ast syn::ExprField) {
            // self.field as a standalone read (not inside method call or assignment LHS).
            if let Some((name, span)) = extract_self_field_from_expr_field_with_span(field) {
                self.fields.push(FieldAccess {
                    field_name: name,
                    access_kind: FieldAccessKind::Read,
                    span: self.make_span(span),
                });
            }
            syn::visit::visit_expr_field(self, field);
        }
    }

    let mut collector = FieldCollector {
        fields: Vec::new(),
        line_offsets,
        file_id,
    };
    syn::visit::visit_block(&mut collector, block);
    collector.fields
}

/// If `expr` is `self.field`, return the field name and its span.
fn extract_self_field_with_span(expr: &syn::Expr) -> Option<(String, proc_macro2::Span)> {
    let syn::Expr::Field(field_expr) = expr else {
        return None;
    };
    extract_self_field_from_expr_field_with_span(field_expr)
}

/// If a field expression is `self.<ident>`, return the ident name and its span.
fn extract_self_field_from_expr_field_with_span(
    field: &syn::ExprField,
) -> Option<(String, proc_macro2::Span)> {
    // Check base is `self`
    let syn::Expr::Path(path) = field.base.as_ref() else {
        return None;
    };
    if !path.path.is_ident("self") {
        return None;
    }
    // Extract field name
    match &field.member {
        syn::Member::Named(ident) => Some((ident.to_string(), ident.span())),
        syn::Member::Unnamed(_) => None, // Tuple fields not supported
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_split_borrow_two_fields() {
        let code = r#"
impl GameState {
    fn update(&mut self) {
        self.physics.step();
        self.renderer.draw(&self.world);
    }
}
"#;
        let items = syn::parse_file(code).unwrap().items;
        let sites = detect_split_borrow_sites(&items);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].method_name, "update");
        assert_eq!(sites[0].fields_accessed.len(), 3); // physics, renderer, world
    }

    #[test]
    fn test_no_split_borrow_single_field() {
        let code = r#"
impl Counter {
    fn increment(&mut self) {
        self.count += 1;
    }
}
"#;
        let items = syn::parse_file(code).unwrap().items;
        let sites = detect_split_borrow_sites(&items);
        assert!(sites.is_empty()); // single field — no split needed
    }

    #[test]
    fn test_no_split_borrow_all_reads() {
        let code = r#"
impl State {
    fn display(&self) {
        println!("{} {}", self.name, self.count);
    }
}
"#;
        let items = syn::parse_file(code).unwrap().items;
        let sites = detect_split_borrow_sites(&items);
        assert!(sites.is_empty()); // &self — no split needed
    }

    #[test]
    fn test_split_borrow_write_and_read() {
        let code = r#"
impl State {
    fn update(&mut self) {
        self.counter += 1;
        self.logger.log(self.counter);
    }
}
"#;
        let items = syn::parse_file(code).unwrap().items;
        let sites = detect_split_borrow_sites(&items);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].method_name, "update");
    }

    #[test]
    fn test_multiple_methods_detected() {
        let code = r#"
impl Game {
    fn tick(&mut self) {
        self.physics.update();
        self.score += 1;
    }
    fn render(&mut self) {
        self.renderer.draw(&self.world);
    }
}
"#;
        let items = syn::parse_file(code).unwrap().items;
        let sites = detect_split_borrow_sites(&items);
        assert_eq!(sites.len(), 2);
    }

    #[test]
    fn test_ignores_non_impl_items() {
        let code = r#"
fn standalone() {
    let x = 1;
}
struct Foo { x: i32 }
"#;
        let items = syn::parse_file(code).unwrap().items;
        let sites = detect_split_borrow_sites(&items);
        assert!(sites.is_empty());
    }

    // ─── BUG-09 tests: real spans from source text ───

    #[test]
    fn spans_are_non_zero_with_source() {
        let code = r#"impl GameState {
    fn update(&mut self) {
        self.physics.step();
        self.renderer.draw(&self.world);
    }
}"#;
        let items = syn::parse_file(code).unwrap().items;
        let sites = detect_split_borrow_sites_with_source(&items, code, kobo_ir::FileId(0));
        assert_eq!(sites.len(), 1);
        let site = &sites[0];
        // method_span should cover "update" — NOT (0, 0)
        assert!(
            site.method_span.start > 0 || site.method_span.end > 0,
            "method_span should not be a dummy zero span: {:?}",
            site.method_span
        );
        // field access spans should also be non-zero
        for fa in &site.fields_accessed {
            assert!(
                fa.span.start > 0 || fa.span.end > 0,
                "field '{}' span should not be dummy zero: {:?}",
                fa.field_name,
                fa.span
            );
        }
    }

    #[test]
    fn method_span_points_to_ident() {
        let code = "impl S {\n    fn do_work(&mut self) {\n        self.a = 1;\n        self.b.run();\n    }\n}";
        let items = syn::parse_file(code).unwrap().items;
        let sites = detect_split_borrow_sites_with_source(&items, code, kobo_ir::FileId(0));
        assert_eq!(sites.len(), 1);
        let span = sites[0].method_span;
        let slice = &code[span.start as usize..span.end as usize];
        assert_eq!(slice, "do_work", "method_span should point at the ident");
    }

    // ─── v0.8 edge-case tests ───

    /// Single field write → no split borrow needed.
    #[test]
    fn single_field_write_no_split() {
        let code = r#"
impl State {
    fn reset(&mut self) {
        self.counter = 0;
    }
}
"#;
        let items = syn::parse_file(code).unwrap().items;
        let sites = detect_split_borrow_sites(&items);
        assert!(
            sites.is_empty(),
            "single-field write must not trigger split"
        );
    }

    /// &self method → never triggers (not &mut self).
    #[test]
    fn immutable_self_never_triggers() {
        let code = r#"
impl State {
    fn show(&self) {
        println!("{} {}", self.name, self.age);
    }
}
"#;
        let items = syn::parse_file(code).unwrap().items;
        let sites = detect_split_borrow_sites(&items);
        assert!(sites.is_empty());
    }

    /// Three fields (one write, two reads via method call) → triggers.
    #[test]
    fn three_fields_one_write_triggers() {
        let code = r#"
impl Game {
    fn tick(&mut self) {
        self.score = 1;
        self.width.step();
        self.height.run();
    }
}
"#;
        let items = syn::parse_file(code).unwrap().items;
        let sites = detect_split_borrow_sites(&items);
        assert_eq!(sites.len(), 1);
        assert!(sites[0].fields_accessed.len() >= 3);
    }

    /// Nested self.field.inner → only first-level field counts.
    #[test]
    fn nested_field_counts_first_level() {
        let code = r#"
impl App {
    fn update(&mut self) {
        self.renderer.clear();
        self.state.counter += 1;
    }
}
"#;
        let items = syn::parse_file(code).unwrap().items;
        let sites = detect_split_borrow_sites(&items);
        assert_eq!(sites.len(), 1);
        let names: Vec<&str> = sites[0]
            .fields_accessed
            .iter()
            .map(|f| f.field_name.as_str())
            .collect();
        assert!(names.contains(&"renderer"));
        assert!(names.contains(&"state"));
    }

    /// Empty impl block → no sites.
    #[test]
    fn empty_impl_block_no_sites() {
        let code = "impl Foo {}";
        let items = syn::parse_file(code).unwrap().items;
        let sites = detect_split_borrow_sites(&items);
        assert!(sites.is_empty());
    }

    /// Method with no body statements → no sites.
    #[test]
    fn method_with_empty_body() {
        let code = r#"
impl Foo {
    fn noop(&mut self) {}
}
"#;
        let items = syn::parse_file(code).unwrap().items;
        let sites = detect_split_borrow_sites(&items);
        assert!(sites.is_empty());
    }

    /// with_source produces different file_id than without_source.
    #[test]
    fn with_source_has_correct_file_id() {
        let code = r#"impl S {
    fn work(&mut self) {
        self.a.step();
        self.b = 1;
    }
}"#;
        let items = syn::parse_file(code).unwrap().items;
        let sites = detect_split_borrow_sites_with_source(&items, code, kobo_ir::FileId(42));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].method_span.file_id, kobo_ir::FileId(42));
    }

    /// Multiple impl blocks in one file.
    #[test]
    fn multiple_impl_blocks() {
        let code = r#"
impl A {
    fn do_a(&mut self) {
        self.x = 1;
        self.y.run();
    }
}
impl B {
    fn do_b(&mut self) {
        self.p.step();
        self.q = 2;
    }
}
"#;
        let items = syn::parse_file(code).unwrap().items;
        let sites = detect_split_borrow_sites(&items);
        assert_eq!(sites.len(), 2, "expected 2 split borrow sites for 2 impls");
    }

    /// Source with no impl at all → empty.
    #[test]
    fn no_impl_no_sites() {
        let code = "fn main() { let x = 1; }";
        let items = syn::parse_file(code).unwrap().items;
        let sites = detect_split_borrow_sites(&items);
        assert!(sites.is_empty());
    }
}
