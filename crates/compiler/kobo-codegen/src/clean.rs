/// Strip all Kobo-specific wrappers from generated Rust code.
///
/// Removals:
/// 1. `DiagOwner<T>` → `T`
/// 2. `ScopedHandle<T>` → `T`
/// 3. `Owned<T>` → `T`
/// 4. `kobo::` attribute annotations → removed
/// 5. `__kobo_` prefixed markers → removed
/// 6. `kobo_diag::` imports → removed
///
/// Additions (via `generate_use_stmts`):
/// 1. `use` statements for standard library types (Arc, Rc, RefCell, etc.)
/// 2. `use` statements for external crate types from dependencies
///
/// String-level manipulation is chosen for simplicity. Kobo generates wrappers
/// in predictable formats so direct string replacement handles all known patterns.

/// Strip Kobo-specific wrappers from generated Rust source.
pub fn strip_kobo_wrappers(source: &str) -> String {
    let mut result = source.to_owned();

    // Remove kobo_diag imports.
    result = remove_kobo_imports(&result);

    // DiagOwner<T> → T (in type positions and constructor calls).
    result = strip_wrapper_type(&result, "DiagOwner");

    // ScopedHandle<T> → T.
    result = strip_wrapper_type(&result, "ScopedHandle");

    // Owned<T> → T.
    result = strip_wrapper_type(&result, "Owned");

    // Remove __kobo_ prefixed macro invocations (standalone statement form).
    result = remove_kobo_markers(&result);

    // Remove #[kobo::...] attributes.
    result = remove_kobo_attributes(&result);

    result
}

/// Analyze stripped source and return required `use` statements.
pub fn generate_use_stmts(source: &str, _dependencies: &[String]) -> Vec<String> {
    let mut uses = Vec::new();

    // Detect async context: if source contains `async fn` we use tokio::sync::Mutex,
    // otherwise std::sync::Mutex.
    let is_async = source.contains("async fn") || source.contains("tokio::spawn");

    let mutex_import = if is_async {
        "use tokio::sync::Mutex;"
    } else {
        "use std::sync::Mutex;"
    };

    let std_types: &[(&str, &str)] = &[
        ("Arc", "use std::sync::Arc;"),
        ("RwLock", "use tokio::sync::RwLock;"),
        ("Rc", "use std::rc::Rc;"),
        ("RefCell", "use std::cell::RefCell;"),
        ("HashMap", "use std::collections::HashMap;"),
        ("HashSet", "use std::collections::HashSet;"),
        ("BTreeMap", "use std::collections::BTreeMap;"),
        ("BTreeSet", "use std::collections::BTreeSet;"),
        ("PathBuf", "use std::path::PathBuf;"),
        ("Path", "use std::path::Path;"),
    ];

    for (type_name, use_stmt) in std_types {
        if contains_word(source, type_name) {
            uses.push(use_stmt.to_string());
        }
    }

    // Mutex handled separately for context-aware import
    if contains_word(source, "Mutex") {
        uses.push(mutex_import.to_string());
    }

    uses
}

// --- Internal helpers ---

/// Check if `source` contains `word` as a whole word (not part of another identifier).
fn contains_word(source: &str, word: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = source[start..].find(word) {
        let abs = start + pos;
        let before_ok = abs == 0 || !is_ident_char(source.as_bytes()[abs - 1]);
        let after_pos = abs + word.len();
        let after_ok = after_pos >= source.len() || !is_ident_char(source.as_bytes()[after_pos]);
        if before_ok && after_ok {
            return true;
        }
        start = abs + word.len();
    }
    false
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn remove_kobo_imports(source: &str) -> String {
    source
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !(trimmed.starts_with("use kobo_") || trimmed.starts_with("use kobo::"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_wrapper_type(source: &str, wrapper: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let wrapper_len = wrapper.len();
    let new_pattern = format!("{wrapper}::new(");
    let new_len = new_pattern.len();
    let mut i = 0;

    while i < bytes.len() {
        // Check for WrapperName::new(expr) → expr
        if source[i..].starts_with(&new_pattern) {
            // Ensure it's not part of a longer identifier.
            let before_ok = i == 0 || !is_ident_char(bytes[i - 1]);
            if before_ok {
                i += new_len;
                let inner = extract_balanced(source, i, b'(', b')');
                result.push_str(&inner.content);
                i = inner.end_pos;
                continue;
            }
        }
        // Check for WrapperName<T> → T
        if source[i..].starts_with(wrapper) && bytes.get(i + wrapper_len) == Some(&b'<') {
            let before_ok = i == 0 || !is_ident_char(bytes[i - 1]);
            if before_ok {
                i += wrapper_len + 1; // skip wrapper name + '<'
                let inner = extract_balanced(source, i, b'<', b'>');
                result.push_str(&inner.content);
                i = inner.end_pos;
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }

    result
}

struct BalancedExtract {
    content: String,
    end_pos: usize,
}

/// Extract content between balanced delimiters starting at `start` (after opening delimiter).
fn extract_balanced(source: &str, start: usize, open: u8, close: u8) -> BalancedExtract {
    let bytes = source.as_bytes();
    let mut depth = 1i32;
    let mut inner = String::new();
    let mut pos = start;

    while pos < bytes.len() {
        let b = bytes[pos];
        if b == open {
            depth += 1;
            inner.push(b as char);
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                pos += 1;
                break;
            }
            inner.push(b as char);
        } else {
            inner.push(b as char);
        }
        pos += 1;
    }

    BalancedExtract {
        content: inner,
        end_pos: pos,
    }
}

fn remove_kobo_markers(source: &str) -> String {
    source
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.starts_with("__kobo_")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn remove_kobo_attributes(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if source[i..].starts_with("#[kobo::") {
            // Skip to the matching ']'.
            while i < bytes.len() && bytes[i] != b']' {
                i += 1;
            }
            if i < bytes.len() {
                i += 1; // skip ']'
            }
            // Skip trailing whitespace/newline.
            while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'\n' {
                i += 1;
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_diag_owner_type() {
        let input = "let x: DiagOwner<Vec<i32>> = DiagOwner::new(vec![1, 2, 3]);";
        let output = strip_kobo_wrappers(input);
        assert!(!output.contains("DiagOwner"));
        assert!(output.contains("let x: Vec<i32>"));
        assert!(output.contains("vec![1, 2, 3]"));
    }

    #[test]
    fn strip_scoped_handle_type() {
        let input = "let h: ScopedHandle<File> = ScopedHandle::new(file);";
        let output = strip_kobo_wrappers(input);
        assert!(!output.contains("ScopedHandle"));
        assert!(output.contains("let h: File"));
        assert!(output.contains("file"));
    }

    #[test]
    fn strip_kobo_diag_import() {
        let input = "use kobo_diag::DiagOwner;\nlet x = 1;";
        let output = strip_kobo_wrappers(input);
        assert!(!output.contains("kobo_diag"));
        assert!(output.contains("let x = 1"));
    }

    #[test]
    fn strip_kobo_attribute() {
        let input = "#[kobo::handler]\nasync fn handle() {}";
        let output = strip_kobo_wrappers(input);
        assert!(!output.contains("#[kobo::handler]"));
        assert!(output.contains("async fn handle"));
    }

    #[test]
    fn strip_kobo_marker_lines() {
        let input = "__kobo_spawn_block!({ body });\nlet x = 1;";
        let output = strip_kobo_wrappers(input);
        assert!(!output.contains("__kobo_"));
        assert!(output.contains("let x = 1"));
    }

    #[test]
    fn generate_use_stmts_for_arc() {
        let source = "let x: Arc<RwLock<HashMap<String, i32>>> = Arc::new(RwLock::new(HashMap::new()));";
        let uses = generate_use_stmts(source, &[]);
        assert!(uses.contains(&"use std::sync::Arc;".to_string()));
        assert!(uses.contains(&"use std::collections::HashMap;".to_string()));
    }

    #[test]
    fn generate_use_stmts_no_false_positives() {
        let source = "let archive = \"test\";"; // "Arc" substring inside "archive"
        let uses = generate_use_stmts(source, &[]);
        assert!(!uses.contains(&"use std::sync::Arc;".to_string()));
    }

    #[test]
    fn strip_nested_wrapper() {
        let input = "let x: DiagOwner<Arc<RwLock<Vec<i32>>>> = DiagOwner::new(Arc::new(RwLock::new(vec![])));";
        let output = strip_kobo_wrappers(input);
        assert!(!output.contains("DiagOwner"));
        assert!(output.contains("Arc<RwLock<Vec<i32>>>"));
    }

    #[test]
    fn contains_word_boundary() {
        assert!(contains_word("let x: Arc<T>", "Arc"));
        assert!(!contains_word("let archive = 1", "Arc"));
        assert!(contains_word("Arc::new(1)", "Arc"));
        assert!(!contains_word("MyArc<T>", "Arc"));
    }

    // ─── BUG-08 tests: remove_kobo_imports should catch ALL kobo_* prefixes ───

    #[test]
    fn strip_kobo_ir_import() {
        let input = "use kobo_ir::Kir;\nlet x = 1;";
        let output = strip_kobo_wrappers(input);
        assert!(!output.contains("kobo_ir"), "kobo_ir import not stripped");
        assert!(output.contains("let x = 1"));
    }

    #[test]
    fn strip_kobo_debt_import() {
        let input = "use kobo_debt::DebtTracker;\nlet x = 1;";
        let output = strip_kobo_wrappers(input);
        assert!(!output.contains("kobo_debt"), "kobo_debt import not stripped");
    }

    #[test]
    fn strip_kobo_analysis_import() {
        let input = "use kobo_analysis::analyze;\nfn main() {}";
        let output = strip_kobo_wrappers(input);
        assert!(
            !output.contains("kobo_analysis"),
            "kobo_analysis import not stripped"
        );
    }

    #[test]
    fn strip_kobo_transform_import() {
        let input = "use kobo_transform::tiered;\nfn main() {}";
        let output = strip_kobo_wrappers(input);
        assert!(
            !output.contains("kobo_transform"),
            "kobo_transform import not stripped"
        );
    }

    #[test]
    fn strip_kobo_imports_preserves_non_kobo() {
        let input = "use std::sync::Arc;\nuse kobo_ir::Kir;\nuse serde::Serialize;";
        let output = strip_kobo_wrappers(input);
        assert!(output.contains("use std::sync::Arc;"));
        assert!(output.contains("use serde::Serialize;"));
        assert!(!output.contains("kobo_ir"));
    }

    // ─── BUG-11 tests: Mutex import is async-context-aware ───

    #[test]
    fn mutex_uses_tokio_in_async_context() {
        let source = "async fn handle() { let m = Mutex::new(0); }";
        let uses = generate_use_stmts(source, &[]);
        assert!(
            uses.contains(&"use tokio::sync::Mutex;".to_string()),
            "async context should use tokio::sync::Mutex, got: {uses:?}"
        );
        assert!(
            !uses.contains(&"use std::sync::Mutex;".to_string()),
            "async context should NOT use std::sync::Mutex"
        );
    }

    #[test]
    fn mutex_uses_std_in_sync_context() {
        let source = "fn handle() { let m = Mutex::new(0); }";
        let uses = generate_use_stmts(source, &[]);
        assert!(
            uses.contains(&"use std::sync::Mutex;".to_string()),
            "sync context should use std::sync::Mutex, got: {uses:?}"
        );
        assert!(
            !uses.contains(&"use tokio::sync::Mutex;".to_string()),
            "sync context should NOT use tokio::sync::Mutex"
        );
    }

    #[test]
    fn mutex_uses_tokio_when_tokio_spawn_present() {
        let source = "fn main() { tokio::spawn(async { let m = Mutex::new(0); }); }";
        let uses = generate_use_stmts(source, &[]);
        assert!(
            uses.contains(&"use tokio::sync::Mutex;".to_string()),
            "tokio::spawn context should use tokio::sync::Mutex"
        );
    }

    // ─── v0.8 edge-case tests ───

    /// Trap 10: ALL kobo_ crate imports must be stripped.
    #[test]
    fn strips_all_kobo_crate_prefixes() {
        let input = "use kobo_diag::DiagOwner;\nuse kobo_ir::Kir;\nuse kobo_transform::tiered;\nuse kobo_analysis::analyze;\nuse kobo_debt::Debt;\nuse kobo_errors::Severity;\nuse kobo::runtime::Ctx;\nuse std::sync::Arc;\n";
        let output = remove_kobo_imports(input);
        assert!(!output.contains("kobo_diag"), "kobo_diag leaked");
        assert!(!output.contains("kobo_ir"), "kobo_ir leaked");
        assert!(!output.contains("kobo_transform"), "kobo_transform leaked");
        assert!(!output.contains("kobo_analysis"), "kobo_analysis leaked");
        assert!(!output.contains("kobo_debt"), "kobo_debt leaked");
        assert!(!output.contains("kobo_errors"), "kobo_errors leaked");
        assert!(!output.contains("use kobo::"), "kobo:: leaked");
        assert!(output.contains("use std::sync::Arc;"), "std import must survive");
    }

    /// Trap 10: __kobo_ macro markers stripped.
    #[test]
    fn strips_kobo_markers() {
        let input = "__kobo_spawn_block!({ body });\n__kobo_select_arm!(rx);\nlet x = 1;\n";
        let output = strip_kobo_wrappers(input);
        assert!(!output.contains("__kobo_"), "marker leaked: {output}");
        assert!(output.contains("let x = 1"));
    }

    /// Trap 10: #[kobo::*] attributes stripped.
    #[test]
    fn strips_kobo_attributes() {
        let input = "#[kobo::handler]\n#[kobo::tick(100ms)]\nasync fn handle() {}\n";
        let output = strip_kobo_wrappers(input);
        assert!(!output.contains("#[kobo::"), "attribute leaked: {output}");
        assert!(output.contains("async fn handle"));
    }

    /// "archive" contains "Arc" as substring but must NOT trigger Arc import.
    #[test]
    fn word_boundary_prevents_archive_matching_arc() {
        let source = "let archive = Vec::new();";
        let uses = generate_use_stmts(source, &[]);
        assert!(
            !uses.iter().any(|u| u.contains("Arc")),
            "'archive' must not trigger Arc import"
        );
    }

    /// "Arcs" should not match "Arc" (suffix check).
    #[test]
    fn word_boundary_prevents_suffix_match() {
        let source = "let Arcs = 42;";
        let uses = generate_use_stmts(source, &[]);
        assert!(
            !uses.iter().any(|u| u.contains("std::sync::Arc")),
            "'Arcs' must not trigger Arc import"
        );
    }

    /// DiagOwner<T> wrapper fully stripped.
    #[test]
    fn diag_owner_stripped() {
        let input = "let x: DiagOwner<Vec<i32>> = DiagOwner::new(vec![1, 2]);";
        let output = strip_kobo_wrappers(input);
        assert!(!output.contains("DiagOwner"), "DiagOwner leaked: {output}");
        assert!(output.contains("Vec<i32>"));
    }

    /// ScopedHandle<T> wrapper fully stripped.
    #[test]
    fn scoped_handle_stripped() {
        let input = "let f: ScopedHandle<File> = ScopedHandle::new(open());";
        let output = strip_kobo_wrappers(input);
        assert!(!output.contains("ScopedHandle"), "ScopedHandle leaked: {output}");
        assert!(output.contains("File"));
    }

    /// Owned<T> wrapper fully stripped.
    #[test]
    fn owned_wrapper_stripped() {
        let input = "let v: Owned<Vec<String>> = Owned::new(vec![]);";
        let output = strip_kobo_wrappers(input);
        assert!(!output.contains("Owned<"), "Owned leaked: {output}");
        assert!(output.contains("Vec<String>"));
    }

    /// Nested wrapper: DiagOwner<Arc<RwLock<T>>> → Arc<RwLock<T>>.
    #[test]
    fn nested_wrapper_preserves_inner() {
        let input = "let x: DiagOwner<Arc<RwLock<HashMap<String, Vec<i32>>>>> = DiagOwner::new(val);";
        let output = strip_kobo_wrappers(input);
        assert!(!output.contains("DiagOwner"));
        assert!(output.contains("Arc<RwLock<HashMap<String, Vec<i32>>>>"));
    }

    /// Empty input → empty output.
    #[test]
    fn empty_input_no_crash() {
        let output = strip_kobo_wrappers("");
        assert_eq!(output, "");
        let uses = generate_use_stmts("", &[]);
        assert!(uses.is_empty());
    }

    /// Source with no recognized types → no use stmts.
    #[test]
    fn no_types_no_use_stmts() {
        let source = "fn main() { let x = 42; }";
        let uses = generate_use_stmts(source, &[]);
        assert!(uses.is_empty());
    }
}
