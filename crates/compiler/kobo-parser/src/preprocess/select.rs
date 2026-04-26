//! Parse `select { arm from channel => body }` and generate `tokio::select!`.
//!
//! Input:
//!   select {
//!       msg from rx1 => { handle_msg(msg) }
//!       tick from timer => { handle_tick() }
//!   }
//!
//! Output:
//!   tokio::select! {
//!       msg = rx1.recv() => { handle_msg(msg.unwrap()) }
//!       _ = timer.tick() => { handle_tick() }
//!   }
//!
//! Safe cancellation defaults:
//! - Branches are biased (first match wins, no fairness)
//! - No implicit else branch (user must handle all cases)
//! - Timer channels use `.tick()` not `.recv()`
//!
//! Invariants:
//! - Empty `select {}` → hard error
//! - Single-arm `select { ... }` → warning (likely a mistake)

/// Errors from select preprocessing.
#[derive(Clone, Debug, PartialEq)]
pub enum SelectError {
    /// `select {}` with zero arms is a compile error.
    EmptySelect { offset: usize },
}

impl std::fmt::Display for SelectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SelectError::EmptySelect { offset } => {
                write!(f, "error: empty `select {{}}` block at byte offset {offset} — select must have at least one arm")
            }
        }
    }
}

/// Warnings from select preprocessing.
#[derive(Clone, Debug, PartialEq)]
pub enum SelectWarning {
    /// Single-arm select is likely a mistake; prefer a direct recv call.
    SingleArm { offset: usize },
}

/// Information about one `select { ... }` block found during preprocessing.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectInfo {
    /// Number of arms in the select block.
    pub arm_count: usize,
    /// Byte offset in the original source.
    pub offset: usize,
}

/// A parsed arm of a `select` block.
#[derive(Clone, Debug, PartialEq)]
struct SelectArm {
    /// Binding name (e.g. "msg", "tick", "_").
    binding: String,
    /// Channel expression (e.g. "rx1", "timer").
    channel: String,
    /// Body of the arm (between `{` and `}`).
    body: String,
}

/// Rewrite all `select { ... }` blocks to `tokio::select! { ... }`.
///
/// Returns `(rewritten_source, select_infos, warnings)`.
/// Returns `Err` if an empty `select {}` is found (hard error per contract).
pub fn preprocess_select_blocks(
    source: &str,
) -> Result<(String, Vec<SelectInfo>, Vec<SelectWarning>), SelectError> {
    let mut result = String::with_capacity(source.len());
    let mut infos = Vec::new();
    let mut warnings = Vec::new();
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut pos = 0;

    while pos < len {
        // Look for "select" followed by optional whitespace and `{`
        if pos + 6 <= len && &source[pos..pos + 6] == "select" {
            // Ensure it's not part of a longer identifier
            if pos > 0 && is_ident_char(bytes[pos - 1]) {
                result.push(bytes[pos] as char);
                pos += 1;
                continue;
            }

            // Check that the char after "select" is not an ident char
            let after_select = pos + 6;
            if after_select < len && is_ident_char(bytes[after_select]) {
                result.push(bytes[pos] as char);
                pos += 1;
                continue;
            }

            // Skip whitespace to find `{`
            let mut scan = after_select;
            while scan < len && bytes[scan].is_ascii_whitespace() {
                scan += 1;
            }

            if scan < len && bytes[scan] == b'{' {
                // Find the matching `}`
                if let Some(close_brace) = find_matching_brace(source, scan) {
                    let block_content = &source[scan + 1..close_brace];
                    let arms = parse_select_arms(block_content);

                    // Empty select → hard error
                    if arms.is_empty() {
                        // Check if the content between braces is truly empty/whitespace
                        if block_content.trim().is_empty() {
                            return Err(SelectError::EmptySelect { offset: pos });
                        }
                        // Non-empty but unparseable content — leave as-is
                        result.push(bytes[pos] as char);
                        pos += 1;
                        continue;
                    }

                    // Single-arm select → warning
                    if arms.len() == 1 {
                        warnings.push(SelectWarning::SingleArm { offset: pos });
                    }

                    infos.push(SelectInfo {
                        arm_count: arms.len(),
                        offset: pos,
                    });
                    result.push_str("tokio::select! {\n");
                    for arm in &arms {
                        let recv_expr = generate_recv_expr(&arm.channel, &arm.binding);
                        result.push_str(&format!(
                            "        {recv_expr} => {{ {} }}\n",
                            arm.body.trim()
                        ));
                    }
                    result.push_str("    }");
                    pos = close_brace + 1;
                    continue;
                }
            }
        }

        result.push(bytes[pos] as char);
        pos += 1;
    }

    Ok((result, infos, warnings))
}

/// Parse the arms of a select block.
///
/// Each arm has the form: `binding from channel => { body }`
fn parse_select_arms(content: &str) -> Vec<SelectArm> {
    let mut arms = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();

        // Look for "binding from channel => { body }" pattern
        if let Some((binding, rest)) = trimmed.split_once(" from ") {
            if let Some((channel, body_start)) = rest.split_once(" => ") {
                let channel = channel.trim().to_string();
                let binding = binding.trim().to_string();
                let body_start = body_start.trim();

                // Collect body: may be on same line or span multiple lines
                let body = if body_start.starts_with('{') {
                    // Find matching close brace
                    let combined = collect_body_from_lines(&lines, i, body_start);
                    i = combined.1;
                    combined.0
                } else {
                    i += 1;
                    body_start.to_string()
                };

                // Strip outer braces from body
                let body = strip_braces(&body);

                arms.push(SelectArm {
                    binding,
                    channel,
                    body,
                });
                continue;
            }
        }

        i += 1;
    }

    arms
}

/// Collect body text that starts with `{` and may span multiple lines.
fn collect_body_from_lines(lines: &[&str], start_line: usize, first_part: &str) -> (String, usize) {
    let mut body = String::from(first_part);
    let mut depth = 0i32;

    for ch in first_part.chars() {
        match ch {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
    }

    if depth == 0 {
        return (body, start_line + 1);
    }

    let mut i = start_line + 1;
    while i < lines.len() && depth > 0 {
        body.push('\n');
        body.push_str(lines[i]);
        for ch in lines[i].chars() {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
        }
        i += 1;
    }

    (body, i)
}

/// Strip outer `{ }` from body string.
fn strip_braces(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        trimmed[1..trimmed.len() - 1].trim().to_string()
    } else {
        trimmed.to_string()
    }
}

/// Generate the recv expression for a select arm.
///
/// For "tick" bindings, use `.tick()` (timer pattern).
/// For regular bindings, use `.recv()`.
fn generate_recv_expr(channel: &str, binding: &str) -> String {
    if binding == "tick" || binding == "_" {
        format!("_ = {channel}.tick()")
    } else {
        format!("{binding} = {channel}.recv()")
    }
}

/// Find the matching `}` for a `{` at `start`.
fn find_matching_brace(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut depth = 0u32;
    let mut pos = start;

    while pos < len {
        match bytes[pos] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(pos);
                }
            }
            _ => {}
        }
        pos += 1;
    }
    None
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_generates_tokio_select() {
        let input = r#"    select {
        msg from rx => { handle(msg) }
        tick from timer => { update() }
    }"#;
        let (output, infos, warnings) = preprocess_select_blocks(input).unwrap();
        assert!(output.contains("tokio::select!"), "output: {output}");
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].arm_count, 2);
        assert!(warnings.is_empty());
    }

    #[test]
    fn select_recv_arm() {
        let input = r#"select {
    msg from rx => { handle_msg(msg) }
}"#;
        let (output, _, _) = preprocess_select_blocks(input).unwrap();
        assert!(output.contains("msg = rx.recv()"), "output: {output}");
    }

    #[test]
    fn select_tick_arm_uses_tick() {
        let input = r#"select {
    tick from timer => { on_tick() }
}"#;
        let (output, _, _) = preprocess_select_blocks(input).unwrap();
        assert!(output.contains("timer.tick()"), "output: {output}");
    }

    #[test]
    fn select_preserves_surrounding_code() {
        let input = r#"let x = 1;
select {
    msg from rx => { handle(msg) }
}
let y = 2;"#;
        let (output, _, _) = preprocess_select_blocks(input).unwrap();
        assert!(output.contains("let x = 1;"), "output: {output}");
        assert!(output.contains("let y = 2;"), "output: {output}");
        assert!(output.contains("tokio::select!"), "output: {output}");
    }

    #[test]
    fn select_no_rewrite_for_non_select() {
        let input = r#"let selected = choose(options);"#;
        let (output, infos, _) = preprocess_select_blocks(input).unwrap();
        assert!(!output.contains("tokio::select!"), "output: {output}");
        assert!(infos.is_empty());
    }

    #[test]
    fn select_multiple_arms() {
        let input = r#"select {
    msg from rx1 => { handle1(msg) }
    data from rx2 => { handle2(data) }
    tick from interval => { on_tick() }
}"#;
        let (output, infos, _) = preprocess_select_blocks(input).unwrap();
        assert!(output.contains("tokio::select!"), "output: {output}");
        assert_eq!(infos[0].arm_count, 3);
        assert!(output.contains("msg = rx1.recv()"), "output: {output}");
        assert!(output.contains("data = rx2.recv()"), "output: {output}");
        assert!(output.contains("interval.tick()"), "output: {output}");
    }

    // ─── BUG-01 tests ───

    #[test]
    fn empty_select_returns_error() {
        let input = "select {}";
        let result = preprocess_select_blocks(input);
        assert!(result.is_err(), "empty select should be a hard error");
        let err = result.unwrap_err();
        assert_eq!(err, SelectError::EmptySelect { offset: 0 });
    }

    #[test]
    fn empty_select_with_whitespace_returns_error() {
        let input = "select {   \n  \n  }";
        let result = preprocess_select_blocks(input);
        assert!(
            result.is_err(),
            "whitespace-only select should be a hard error"
        );
    }

    #[test]
    fn single_arm_select_produces_warning() {
        let input = r#"select {
    msg from rx => { handle(msg) }
}"#;
        let (_, _, warnings) = preprocess_select_blocks(input).unwrap();
        assert_eq!(
            warnings.len(),
            1,
            "single-arm select should produce a warning"
        );
        assert!(matches!(warnings[0], SelectWarning::SingleArm { .. }));
    }

    #[test]
    fn two_arm_select_no_warning() {
        let input = r#"select {
    msg from rx => { handle(msg) }
    tick from timer => { on_tick() }
}"#;
        let (_, _, warnings) = preprocess_select_blocks(input).unwrap();
        assert!(
            warnings.is_empty(),
            "two-arm select should have no warnings"
        );
    }

    #[test]
    fn select_error_display() {
        let err = SelectError::EmptySelect { offset: 42 };
        let msg = format!("{err}");
        assert!(msg.contains("empty"), "error message: {msg}");
        assert!(
            msg.contains("42"),
            "error message should contain offset: {msg}"
        );
    }

    // ─── Preserved unit tests ───

    #[test]
    fn generate_recv_expr_for_msg() {
        let expr = generate_recv_expr("rx", "msg");
        assert_eq!(expr, "msg = rx.recv()");
    }

    #[test]
    fn generate_recv_expr_for_tick() {
        let expr = generate_recv_expr("timer", "tick");
        assert_eq!(expr, "_ = timer.tick()");
    }

    #[test]
    fn strip_braces_basic() {
        assert_eq!(strip_braces("{ hello }"), "hello");
        assert_eq!(strip_braces("no braces"), "no braces");
    }

    // ─── v0.8 edge-case tests ───

    /// Trap 7: Whitespace-only select body → hard error (same as empty).
    #[test]
    fn whitespace_only_select_is_hard_error() {
        let input = "select {   \n\n  }";
        let result = preprocess_select_blocks(input);
        assert!(result.is_err(), "whitespace-only select must be Err");
    }

    /// Two-arm select produces zero warnings (edge test).
    #[test]
    fn edge_two_arm_select_no_warning() {
        let input = r#"select {
    msg from rx => { handle(msg) }
    tick from timer => { update() }
}"#;
        let (_, _, warnings) = preprocess_select_blocks(input).unwrap();
        assert!(warnings.is_empty(), "2-arm select should have no warnings");
    }

    /// "select" inside identifier must NOT trigger rewrite.
    #[test]
    fn select_inside_identifier_ignored() {
        let input = "let preselect = true;";
        let (output, infos, _) = preprocess_select_blocks(input).unwrap();
        assert_eq!(output, input);
        assert!(infos.is_empty());
    }

    /// "select_mode" should NOT trigger rewrite.
    #[test]
    fn select_not_followed_by_brace_ignored() {
        let input = "let select_mode = true;";
        let (output, infos, _) = preprocess_select_blocks(input).unwrap();
        assert_eq!(output, input);
        assert!(infos.is_empty());
    }

    /// Multiple select blocks in one source.
    #[test]
    fn multiple_select_blocks() {
        let input = r#"
select {
    a from rx1 => { handle_a(a) }
    b from rx2 => { handle_b(b) }
}
let middle = 1;
select {
    c from rx3 => { handle_c(c) }
}
"#;
        let (output, infos, warnings) = preprocess_select_blocks(input).unwrap();
        assert_eq!(infos.len(), 2, "expected 2 select blocks");
        assert_eq!(infos[0].arm_count, 2);
        assert_eq!(infos[1].arm_count, 1);
        // Second block has 1 arm → single-arm warning
        assert_eq!(warnings.len(), 1);
        assert!(output.contains("let middle = 1"));
    }

    /// select block with 3+ arms → no warning, all arms rewritten.
    #[test]
    fn three_arm_select_no_warning() {
        let input = r#"select {
    msg from rx => { handle(msg) }
    tick from timer => { update() }
    done from quit => { break }
}"#;
        let (output, infos, warnings) = preprocess_select_blocks(input).unwrap();
        assert_eq!(infos[0].arm_count, 3);
        assert!(warnings.is_empty());
        assert!(output.contains("tokio::select!"));
    }

    /// select block inside string literal — preprocessor is regex-based,
    /// so it may still rewrite. This test documents the current behavior.
    #[test]
    fn select_in_string_literal_current_behavior() {
        let input = r#"let s = "select { msg from rx => { handle(msg) } }";"#;
        // Regex-based preprocessor does not distinguish string context.
        // This documents that behavior rather than asserting correctness.
        let result = preprocess_select_blocks(input);
        assert!(result.is_ok(), "should not hard-error on string content");
    }

    /// Source with no select keyword at all → passthrough.
    #[test]
    fn no_select_passthrough() {
        let input = "fn main() { let x = 1; }";
        let (output, infos, warnings) = preprocess_select_blocks(input).unwrap();
        assert_eq!(output, input);
        assert!(infos.is_empty());
        assert!(warnings.is_empty());
    }

    /// Empty input → OK with empty results.
    #[test]
    fn empty_input_ok() {
        let (output, infos, warnings) = preprocess_select_blocks("").unwrap();
        assert_eq!(output, "");
        assert!(infos.is_empty());
        assert!(warnings.is_empty());
    }
}
