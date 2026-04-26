/// S-55: K0065 SelectBranchNotCancelSafe detection.
///
/// Static pattern-match on `select { }` branches against a known list of
/// non-cancel-safe methods. These methods lose partial progress when a
/// `tokio::select!` branch is cancelled, causing silent data loss.
///
/// Known non-cancel-safe methods (from tokio docs):
/// - `read_exact` — partial reads lost
/// - `read_line` — partial line lost
/// - `read_to_end` — partial buffer lost
/// - `read_to_string` — partial content lost
/// - `write_all` — partial writes lost
///
/// Emits a K0065 warning (never an error). Does NOT alter the lowered `tokio::select!`.

/// A detected cancel-safety violation in a select branch.
#[derive(Clone, Debug, PartialEq)]
pub struct CancelSafetyWarning {
    /// The method name that is not cancel-safe.
    pub method_name: String,
    /// Byte offset in the original source.
    pub source_offset: usize,
    /// Suggestion for the user.
    pub suggestion: String,
}

/// Methods known to be non-cancel-safe in tokio select branches.
const NON_CANCEL_SAFE_METHODS: &[&str] = &[
    "read_exact",
    "read_line",
    "read_to_end",
    "read_to_string",
    "write_all",
    "write_all_buf",
    "read_buf",
    "copy",
    "copy_buf",
];

/// Scan source text for `select { }` blocks and check branches for non-cancel-safe calls.
///
/// Works on the original source before any preprocessing, scanning for the
/// `select { arm from channel => { body } }` pattern.
pub fn scan_source_cancel_safety(source: &str) -> Vec<CancelSafetyWarning> {
    let mut warnings = Vec::new();
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut pos = 0;

    while pos + 6 < len {
        // Skip non-char-boundary bytes (multi-byte UTF-8 sequences)
        if !source.is_char_boundary(pos) || !source.is_char_boundary(pos + 6) {
            pos += 1;
            continue;
        }
        if &source[pos..pos + 6] == "select" {
            // Verify it's not part of a longer identifier
            if pos > 0 && is_ident_byte(bytes[pos - 1]) {
                pos += 1;
                continue;
            }
            let after = pos + 6;
            if after < len && is_ident_byte(bytes[after]) {
                pos += 1;
                continue;
            }

            // Skip whitespace to find `{`
            let mut scan = after;
            while scan < len && bytes[scan].is_ascii_whitespace() {
                scan += 1;
            }

            if scan < len && bytes[scan] == b'{' {
                if let Some(close) = find_matching_brace(source, scan) {
                    let block = &source[scan + 1..close];
                    let block_offset = scan + 1;
                    // Scan block content for non-cancel-safe methods
                    for &method in NON_CANCEL_SAFE_METHODS {
                        let pattern = format!(".{method}(");
                        let mut search_pos = 0;
                        while let Some(found) = block[search_pos..].find(&pattern) {
                            let abs_offset = block_offset + search_pos + found;
                            warnings.push(CancelSafetyWarning {
                                method_name: method.to_string(),
                                source_offset: abs_offset,
                                suggestion: format!(
                                    "`.{method}()` is not cancel-safe in select branches — \
                                     partial progress is lost on cancellation. \
                                     Consider using a cancellation-aware wrapper or \
                                     moving the operation outside the select."
                                ),
                            });
                            search_pos += found + pattern.len();
                        }
                    }
                    pos = close + 1;
                    continue;
                }
            }
        }
        pos += 1;
    }

    warnings
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn find_matching_brace(source: &str, open_pos: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 1u32;
    let mut i = open_pos + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_read_exact_in_select() {
        let source = r#"
select {
    data from reader => {
        reader.read_exact(&mut buf).await;
    }
}
"#;
        let warnings = scan_source_cancel_safety(source);
        assert!(!warnings.is_empty());
        assert_eq!(warnings[0].method_name, "read_exact");
    }

    #[test]
    fn detects_write_all_in_select() {
        let source = r#"
select {
    msg from rx => { handle(msg) }
    _ from timer => { writer.write_all(data).await }
}
"#;
        let warnings = scan_source_cancel_safety(source);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].method_name, "write_all");
    }

    #[test]
    fn no_warning_outside_select() {
        let source = r#"
reader.read_exact(&mut buf).await;
"#;
        let warnings = scan_source_cancel_safety(source);
        assert!(warnings.is_empty());
    }

    #[test]
    fn no_warning_for_safe_methods() {
        let source = r#"
select {
    msg from rx => { handle(msg.await) }
    _ from timer => { timer.tick().await }
}
"#;
        let warnings = scan_source_cancel_safety(source);
        assert!(warnings.is_empty());
    }

    #[test]
    fn detects_multiple_unsafe_methods() {
        let source = r#"
select {
    data from stream => {
        stream.read_line(&mut line).await;
        stream.read_to_end(&mut buf).await;
    }
}
"#;
        let warnings = scan_source_cancel_safety(source);
        assert!(warnings.len() >= 2);
        let names: Vec<&str> = warnings.iter().map(|w| w.method_name.as_str()).collect();
        assert!(names.contains(&"read_line"));
        assert!(names.contains(&"read_to_end"));
    }
}
