/// Parse `#[kobo::engine]` attribute on structs.
///
/// Semantics:
/// - All bindings of this struct type → PlainOwned (no wrapping)
/// - All K-codes related to this struct → elevated to errors
/// - In kobo debt output, engine structs are highlighted
///
/// Engine annotation is stripped in --clean output.
use syn::visit::Visit;

/// Information about a struct annotated with `#[kobo::engine]`.
#[derive(Clone, Debug)]
pub struct EngineInfo {
    /// The struct name (e.g., "Physics").
    pub struct_name: String,
    /// Byte offset of the attribute in the original source.
    pub attr_offset: usize,
    /// Byte offset of the struct keyword.
    pub struct_offset: usize,
}

/// Scan a parsed syn file for `#[kobo::engine]` attributes.
pub fn collect_engine_structs(file: &syn::File) -> Vec<EngineInfo> {
    let mut visitor = EngineVisitor {
        results: Vec::new(),
    };
    visitor.visit_file(file);
    visitor.results
}

/// Check if an attribute is `#[kobo::engine]`.
fn is_engine_attribute(attr: &syn::Attribute) -> bool {
    if let syn::Meta::Path(path) = &attr.meta {
        let segments: Vec<_> = path.segments.iter().collect();
        if segments.len() == 2 {
            return segments[0].ident == "kobo" && segments[1].ident == "engine";
        }
    }
    false
}

/// Strip `#[kobo::engine]` attributes from source text for --clean output.
pub fn strip_engine_attributes(source: &str) -> String {
    // Simple line-based stripping for the attribute.
    source
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            trimmed != "#[kobo::engine]"
        })
        .collect::<Vec<_>>()
        .join("\n")
}

struct EngineVisitor {
    results: Vec<EngineInfo>,
}

impl<'ast> Visit<'ast> for EngineVisitor {
    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        for attr in &item.attrs {
            if is_engine_attribute(attr) {
                let attr_offset = attr.pound_token.span.byte_range().start;
                let struct_offset = item.struct_token.span.byte_range().start;

                self.results.push(EngineInfo {
                    struct_name: item.ident.to_string(),
                    attr_offset,
                    struct_offset,
                });
            }
        }

        // Continue visiting nested items.
        syn::visit::visit_item_struct(self, item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_engine_attribute_on_struct() {
        let source = r#"
#[kobo::engine]
struct Physics {
    bodies: Vec<u32>,
    gravity: f64,
}
"#;
        let file = syn::parse_file(source).unwrap();
        let engines = collect_engine_structs(&file);
        assert_eq!(engines.len(), 1);
        assert_eq!(engines[0].struct_name, "Physics");
    }

    #[test]
    fn no_engine_attribute_on_regular_struct() {
        let source = r#"
struct RegularStruct {
    data: Vec<u32>,
}
"#;
        let file = syn::parse_file(source).unwrap();
        let engines = collect_engine_structs(&file);
        assert_eq!(engines.len(), 0);
    }

    #[test]
    fn multiple_engine_structs() {
        let source = r#"
#[kobo::engine]
struct Physics {
    gravity: f64,
}

#[kobo::engine]
struct Renderer {
    width: u32,
}

struct Config {
    name: String,
}
"#;
        let file = syn::parse_file(source).unwrap();
        let engines = collect_engine_structs(&file);
        assert_eq!(engines.len(), 2);
        assert_eq!(engines[0].struct_name, "Physics");
        assert_eq!(engines[1].struct_name, "Renderer");
    }

    #[test]
    fn engine_attribute_not_on_fn() {
        // #[kobo::engine] on a function should be ignored by this pass.
        let source = r#"
#[kobo::engine]
fn not_a_struct() {}
"#;
        let file = syn::parse_file(source).unwrap();
        let engines = collect_engine_structs(&file);
        assert_eq!(engines.len(), 0);
    }

    #[test]
    fn strip_engine_attribute_from_source() {
        let source = "#[kobo::engine]\nstruct Physics {\n    gravity: f64,\n}";
        let stripped = strip_engine_attributes(source);
        assert!(!stripped.contains("#[kobo::engine]"));
        assert!(stripped.contains("struct Physics"));
    }

    #[test]
    fn is_engine_attr_check() {
        let source = r#"
#[kobo::engine]
struct X {}
"#;
        let file = syn::parse_file(source).unwrap();
        if let syn::Item::Struct(item) = &file.items[0] {
            assert!(is_engine_attribute(&item.attrs[0]));
        } else {
            panic!("expected struct");
        }
    }
}
