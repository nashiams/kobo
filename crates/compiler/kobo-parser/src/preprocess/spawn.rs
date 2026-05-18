/// Preprocess `spawn { ... }` blocks into `__kobo_spawn_block!({ ... })`.
///
/// `spawn` is a Kobo expression (not a keyword). At the text level:
/// 1. Scan for `spawn` followed by whitespace and `{`
/// 2. Find matching closing brace (handle nesting)
/// 3. Replace with `__kobo_spawn_block!({ ... })`
///
/// After syn parsing, the macro invocation is recognized in codegen
/// and lowered to `tokio::spawn(async move { ... })`.
use kobo_ir::{FileId, KoboSpan};

use super::source_map::{push_identity_segment, PreprocessSourceMap, PreprocessedSource};

/// Information about one `spawn { ... }` block found during preprocessing.
#[derive(Clone, Debug)]
pub struct SpawnBlockInfo {
    /// Span of the entire `spawn { ... }` expression in the ORIGINAL source.
    pub span: KoboSpan,
    /// Span of the body (between braces) in the ORIGINAL source.
    pub body_span: KoboSpan,
    /// Span of the generated macro name in the REWRITTEN source.
    pub generated_macro_span: KoboSpan,
    /// Span of the copied body in the REWRITTEN source.
    pub generated_body_span: KoboSpan,
    /// Span of the generated macro close in the REWRITTEN source.
    pub generated_close_span: KoboSpan,
    /// Whether the original source used `spawn local { ... }`.
    pub is_local: bool,
}

/// Error when spawn is used outside an async function.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnContextError {
    /// Byte offset of the `spawn` keyword.
    pub offset: usize,
    /// Name of the enclosing function (if any).
    pub function_name: Option<String>,
}

impl std::fmt::Display for SpawnContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.function_name {
            Some(name) => write!(
                f,
                "error: `spawn` block at byte offset {} is inside non-async function `{name}` — \
                 spawn requires an async context",
                self.offset
            ),
            None => write!(
                f,
                "error: `spawn` block at byte offset {} is outside any function — \
                 spawn requires an async context",
                self.offset
            ),
        }
    }
}

/// Validate that all spawn blocks appear inside async function context.
///
/// Returns a list of errors for any spawn blocks found in non-async functions.
/// This is a best-effort text-level check: it scans for `async fn` and `fn` declarations
/// and determines which function scope each spawn block falls into.
pub fn validate_spawn_context(source: &str) -> Vec<SpawnContextError> {
    let mut errors = Vec::new();
    let bytes = source.as_bytes();
    let len = bytes.len();

    // Collect function ranges: (start_offset, is_async, name, brace_start, brace_end)
    let mut functions: Vec<(usize, bool, String, usize, usize)> = Vec::new();
    let mut pos = 0;

    while pos < len {
        // Skip string literals
        if bytes[pos] == b'"' {
            pos += 1;
            while pos < len {
                if bytes[pos] == b'\\' {
                    pos += 2;
                    continue;
                }
                if bytes[pos] == b'"' {
                    pos += 1;
                    break;
                }
                pos += 1;
            }
            continue;
        }

        // Check for "async fn" or "fn"
        let is_async_fn = pos + 8 <= len && &source[pos..pos + 8] == "async fn";
        let is_plain_fn = !is_async_fn && pos + 2 <= len && &source[pos..pos + 2] == "fn";

        if is_async_fn || is_plain_fn {
            let fn_start = pos;
            let preceded = fn_start > 0
                && (bytes[fn_start - 1].is_ascii_alphanumeric() || bytes[fn_start - 1] == b'_');
            let keyword_end = if is_async_fn { pos + 8 } else { pos + 2 };
            let followed = keyword_end < len
                && (bytes[keyword_end].is_ascii_alphanumeric() || bytes[keyword_end] == b'_');

            // Skip whitespace after `fn` to get function name
            if !preceded && !followed {
                let mut name_start = keyword_end;
                while name_start < len && bytes[name_start].is_ascii_whitespace() {
                    name_start += 1;
                }
                let mut name_end = name_start;
                while name_end < len
                    && (bytes[name_end].is_ascii_alphanumeric() || bytes[name_end] == b'_')
                {
                    name_end += 1;
                }
                let fn_name = source[name_start..name_end].to_string();

                // Find the opening brace of the function body
                let mut scan = name_end;
                let mut paren_depth = 0i32;
                while scan < len {
                    match bytes[scan] {
                        b'(' => paren_depth += 1,
                        b')' => paren_depth -= 1,
                        b'{' if paren_depth == 0 => break,
                        _ => {}
                    }
                    scan += 1;
                }

                if scan < len && bytes[scan] == b'{' {
                    if let Some(brace_end) = find_matching_brace(source, scan) {
                        functions.push((fn_start, is_async_fn, fn_name, scan, brace_end));
                        pos = scan + 1; // continue scanning inside the function
                        continue;
                    }
                }
            }
        }
        pos += 1;
    }

    // Now find all spawn block offsets
    let spawn_offsets = find_spawn_offsets(source);

    // For each spawn, determine if it's inside an async fn
    for spawn_offset in spawn_offsets {
        // Find the innermost function containing this spawn
        let mut enclosing: Option<&(usize, bool, String, usize, usize)> = None;
        for func in &functions {
            let (_, _, _, brace_start, brace_end) = func;
            if spawn_offset > *brace_start && spawn_offset < *brace_end {
                // This function contains the spawn; pick the innermost
                match enclosing {
                    None => enclosing = Some(func),
                    Some(prev) => {
                        if func.3 > prev.3 {
                            enclosing = Some(func);
                        }
                    }
                }
            }
        }

        match enclosing {
            Some((_, true, _, _, _)) => {} // async fn — OK
            Some((_, false, name, _, _)) => {
                errors.push(SpawnContextError {
                    offset: spawn_offset,
                    function_name: Some(name.clone()),
                });
            }
            None => {
                errors.push(SpawnContextError {
                    offset: spawn_offset,
                    function_name: None,
                });
            }
        }
    }

    errors
}

/// Find byte offsets of all `spawn {` occurrences in source (excluding strings/comments).
fn find_spawn_offsets(source: &str) -> Vec<usize> {
    let mut offsets = Vec::new();
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut pos = 0;

    while pos < len {
        // Skip strings
        if bytes[pos] == b'"' {
            pos += 1;
            while pos < len {
                if bytes[pos] == b'\\' {
                    pos += 2;
                    continue;
                }
                if bytes[pos] == b'"' {
                    pos += 1;
                    break;
                }
                pos += 1;
            }
            continue;
        }
        // Skip line comments
        if pos + 1 < len && bytes[pos] == b'/' && bytes[pos + 1] == b'/' {
            while pos < len && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }

        if let Some((brace_pos, _)) = spawn_block_at(source, pos) {
            offsets.push(pos);
            pos = brace_pos + 1;
            continue;
        }
        pos += 1;
    }
    offsets
}

/// Rewrite `spawn { ... }` to `__kobo_spawn_block!({ ... })`.
///
/// Returns `(rewritten_source, spawn_infos)`.
///
/// Uses a depth-tracking approach so nested `spawn {}` blocks are each
/// rewritten independently (innermost bodies are scanned, not copied verbatim).
pub fn preprocess_spawn_blocks(source: &str, file_id: FileId) -> (String, Vec<SpawnBlockInfo>) {
    let mut result = String::with_capacity(source.len() + 128);
    let mut infos: Vec<SpawnBlockInfo> = Vec::new();
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut pos = 0;

    // Track brace depth so we know when a spawn block's closing `}` is reached.
    let mut brace_depth: u32 = 0;
    // Stack of brace depths at which a spawn block opened — when `}` brings
    // us back to this depth we append `)` to close `__kobo_spawn_block!(...)`.
    let mut spawn_close_depths: Vec<(u32, usize)> = Vec::new(); // (depth_before_open, info_index)

    while pos < len {
        // Skip string literals.
        if bytes[pos] == b'"' {
            let start = pos;
            pos += 1;
            while pos < len {
                if bytes[pos] == b'\\' {
                    pos += 2;
                    continue;
                }
                if bytes[pos] == b'"' {
                    pos += 1;
                    break;
                }
                pos += 1;
            }
            result.push_str(&source[start..pos]);
            continue;
        }

        // Skip line comments.
        if pos + 1 < len && bytes[pos] == b'/' && bytes[pos + 1] == b'/' {
            let start = pos;
            while pos < len && bytes[pos] != b'\n' {
                pos += 1;
            }
            result.push_str(&source[start..pos]);
            continue;
        }

        // Skip block comments.
        if pos + 1 < len && bytes[pos] == b'/' && bytes[pos + 1] == b'*' {
            let start = pos;
            pos += 2;
            let mut depth = 1u32;
            while pos + 1 < len && depth > 0 {
                if bytes[pos] == b'/' && bytes[pos + 1] == b'*' {
                    depth += 1;
                    pos += 2;
                } else if bytes[pos] == b'*' && bytes[pos + 1] == b'/' {
                    depth -= 1;
                    pos += 2;
                } else {
                    pos += 1;
                }
            }
            result.push_str(&source[start..pos]);
            continue;
        }

        // Look for `spawn` keyword.
        if let Some((brace_pos, is_local)) = spawn_block_at(source, pos) {
                    let spawn_start = pos;
                    let body_start = brace_pos + 1;
                    let macro_name = if is_local {
                        "__kobo_spawn_local_block"
                    } else {
                        "__kobo_spawn_block"
                    };
                    let macro_open = format!("{macro_name}!({{");

                    // Find closing brace to record the full span info.
                    if let Some(brace_end) = find_matching_brace(source, brace_pos) {
                        let generated_macro_start = result.len();
                        let generated_body_start = generated_macro_start + macro_open.len();
                        infos.push(SpawnBlockInfo {
                            span: KoboSpan::new(
                                spawn_start as u32,
                                (brace_end + 1) as u32,
                                file_id,
                            ),
                            body_span: KoboSpan::new(body_start as u32, brace_end as u32, file_id),
                            generated_macro_span: KoboSpan::new(
                                generated_macro_start as u32,
                                (generated_macro_start + macro_name.len() + "!".len()) as u32,
                                file_id,
                            ),
                            generated_body_span: KoboSpan::new(
                                generated_body_start as u32,
                                generated_body_start as u32,
                                file_id,
                            ),
                            generated_close_span: KoboSpan::new(0, 0, file_id),
                            is_local,
                        });
                    }

                    // Emit macro open and the `{`.
                    result.push_str(&macro_open);
                    if !infos.is_empty() {
                        spawn_close_depths.push((brace_depth, infos.len() - 1));
                    }
                    brace_depth += 1;
                    pos = body_start;
                    continue;
        }

        // Track brace depth for non-spawn braces.
        if bytes[pos] == b'{' {
            brace_depth += 1;
            result.push('{');
            pos += 1;
            continue;
        }
        if bytes[pos] == b'}' {
            brace_depth = brace_depth.saturating_sub(1);
            // Check if this closes a spawn block.
            if let Some(&(depth, info_index)) = spawn_close_depths.last() {
                if brace_depth == depth {
                    spawn_close_depths.pop();
                    let generated_close_start = result.len();
                    if let Some(info) = infos.get_mut(info_index) {
                        info.generated_body_span.end = generated_close_start as u32;
                        info.generated_close_span = KoboSpan::new(
                            generated_close_start as u32,
                            (generated_close_start + "})".len()) as u32,
                            file_id,
                        );
                    }
                    result.push_str("})");
                    pos += 1;
                    continue;
                }
            }
            result.push('}');
            pos += 1;
            continue;
        }

        // Default: copy character.
        let b = bytes[pos];
        if b.is_ascii() {
            result.push(b as char);
            pos += 1;
        } else {
            let start = pos;
            pos += 1;
            while pos < len && !source.is_char_boundary(pos) {
                pos += 1;
            }
            result.push_str(&source[start..pos]);
        }
    }

    (result, infos)
}

fn spawn_block_at(source: &str, pos: usize) -> Option<(usize, bool)> {
    let bytes = source.as_bytes();
    let len = bytes.len();
    if pos + 5 > len || &bytes[pos..pos + 5] != b"spawn" {
        return None;
    }
    let preceded_by_ident =
        pos > 0 && (bytes[pos - 1].is_ascii_alphanumeric() || bytes[pos - 1] == b'_');
    let followed_by_ident =
        pos + 5 < len && (bytes[pos + 5].is_ascii_alphanumeric() || bytes[pos + 5] == b'_');
    if preceded_by_ident || followed_by_ident {
        return None;
    }

    let mut cursor = pos + 5;
    while cursor < len && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }

    let mut is_local = false;
    if cursor + 5 <= len && &bytes[cursor..cursor + 5] == b"local" {
        let local_followed_by_ident = cursor + 5 < len
            && (bytes[cursor + 5].is_ascii_alphanumeric() || bytes[cursor + 5] == b'_');
        if !local_followed_by_ident {
            is_local = true;
            cursor += 5;
            while cursor < len && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
        }
    }

    (cursor < len && bytes[cursor] == b'{').then_some((cursor, is_local))
}

pub fn preprocess_spawn_blocks_mapped(
    source: &str,
    file_id: FileId,
) -> PreprocessedSource<Vec<SpawnBlockInfo>> {
    let (rewritten, infos) = preprocess_spawn_blocks(source, file_id);
    let mut source_map = PreprocessSourceMap::default();

    let mut top_level: Vec<&SpawnBlockInfo> = infos
        .iter()
        .filter(|candidate| {
            !infos.iter().any(|other| {
                other.span != candidate.span
                    && other.span.start <= candidate.span.start
                    && candidate.span.end <= other.span.end
            })
        })
        .collect();
    top_level.sort_by_key(|info| info.span.start);

    let mut rewritten_cursor = 0usize;
    let mut original_cursor = 0usize;
    for info in top_level {
        push_identity_segment(
            &mut source_map,
            file_id,
            rewritten_cursor,
            info.generated_macro_span.start as usize,
            original_cursor,
            info.span.start as usize,
        );
        push_spawn_segments(&mut source_map, file_id, info);
        rewritten_cursor = info.generated_close_span.end as usize;
        original_cursor = info.span.end as usize;
    }
    push_identity_segment(
        &mut source_map,
        file_id,
        rewritten_cursor,
        rewritten.len(),
        original_cursor,
        source.len(),
    );

    for info in &infos {
        push_spawn_segments(&mut source_map, file_id, info);
    }

    PreprocessedSource {
        rewritten,
        source_map,
        metadata: infos,
    }
}

fn push_spawn_segments(
    source_map: &mut PreprocessSourceMap,
    file_id: FileId,
    info: &SpawnBlockInfo,
) {
    source_map.push_segment(
        info.generated_macro_span,
        KoboSpan::new(
            info.span.start,
            info.span.start + "spawn".len() as u32,
            file_id,
        ),
    );
    source_map.push_segment(info.generated_body_span, info.body_span);
    source_map.push_segment(
        info.generated_close_span,
        KoboSpan::new(info.span.end.saturating_sub(1), info.span.end, file_id),
    );
}

/// Find the matching closing brace for an opening brace at `start`.
/// Returns the index of the closing `}`.
fn find_matching_brace(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let len = bytes.len();

    if start >= len || bytes[start] != b'{' {
        return None;
    }

    let mut depth = 1u32;
    let mut pos = start + 1;

    while pos < len && depth > 0 {
        match bytes[pos] {
            b'"' => {
                pos += 1;
                while pos < len {
                    if bytes[pos] == b'\\' {
                        pos += 2;
                        continue;
                    }
                    if bytes[pos] == b'"' {
                        pos += 1;
                        break;
                    }
                    pos += 1;
                }
                continue;
            }
            b'\'' => {
                // Character literal or lifetime — skip carefully.
                pos += 1;
                if pos < len && bytes[pos] == b'\\' {
                    pos += 2;
                }
                if pos < len {
                    pos += 1;
                }
                if pos < len && bytes[pos] == b'\'' {
                    pos += 1;
                }
                continue;
            }
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        if depth > 0 {
            pos += 1;
        }
    }

    if depth == 0 {
        Some(pos)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_spawn_block() {
        let code = r#"
async fn main() {
    let x = 42;
    spawn {
        println!("{}", x);
    };
}
"#;
        let (rewritten, infos) = preprocess_spawn_blocks(code, FileId(0));
        assert_eq!(infos.len(), 1);
        assert!(rewritten.contains("__kobo_spawn_block!"));
        assert!(!rewritten.contains("spawn {"));
    }

    #[test]
    fn test_nested_spawn_blocks() {
        let code = r#"
async fn main() {
    spawn {
        spawn {
            println!("inner");
        };
    };
}
"#;
        let (rewritten, infos) = preprocess_spawn_blocks(code, FileId(0));
        assert_eq!(infos.len(), 2);
        assert!(rewritten.contains("__kobo_spawn_block!"));
    }

    #[test]
    fn test_no_spawn_no_rewrite() {
        let code = "fn main() { println!(\"hello\"); }";
        let (rewritten, infos) = preprocess_spawn_blocks(code, FileId(0));
        assert_eq!(infos.len(), 0);
        assert_eq!(rewritten, code);
    }

    #[test]
    fn test_spawn_in_string_not_rewritten() {
        let code = r#"fn main() { let s = "spawn { nope }"; }"#;
        let (rewritten, infos) = preprocess_spawn_blocks(code, FileId(0));
        assert_eq!(infos.len(), 0);
        assert_eq!(rewritten, code);
    }

    #[test]
    fn test_respawn_not_rewritten() {
        let code = "fn main() { respawn { }; }";
        let (rewritten, infos) = preprocess_spawn_blocks(code, FileId(0));
        assert_eq!(infos.len(), 0);
        assert_eq!(rewritten, code);
    }

    #[test]
    fn test_spawn_suffix_not_rewritten() {
        let code = "fn main() { spawn_task { }; }";
        let (rewritten, infos) = preprocess_spawn_blocks(code, FileId(0));
        assert_eq!(infos.len(), 0);
        assert_eq!(rewritten, code);
    }

    #[test]
    fn test_spawn_preserves_body_content() {
        let code = "spawn { do_work(data) }";
        let (rewritten, infos) = preprocess_spawn_blocks(code, FileId(0));
        assert_eq!(infos.len(), 1);
        assert!(rewritten.contains("__kobo_spawn_block!({ do_work(data) })"));
    }

    // ─── BUG-03 tests: spawn in non-async context ───

    #[test]
    fn spawn_in_async_fn_ok() {
        let code = r#"
async fn work() {
    spawn { do_stuff() };
}
"#;
        let errors = validate_spawn_context(code);
        assert!(
            errors.is_empty(),
            "spawn in async fn should be OK, got: {errors:?}"
        );
    }

    #[test]
    fn spawn_in_non_async_fn_error() {
        let code = r#"
fn work() {
    spawn { do_stuff() };
}
"#;
        let errors = validate_spawn_context(code);
        assert_eq!(errors.len(), 1, "spawn in non-async fn should be an error");
        assert_eq!(errors[0].function_name.as_deref(), Some("work"));
    }

    #[test]
    fn spawn_outside_any_fn_error() {
        let code = r#"spawn { do_stuff() };"#;
        let errors = validate_spawn_context(code);
        assert_eq!(errors.len(), 1, "spawn outside fn should be an error");
        assert!(errors[0].function_name.is_none());
    }

    #[test]
    fn spawn_context_error_display() {
        let err = SpawnContextError {
            offset: 10,
            function_name: Some("main".to_string()),
        };
        let msg = format!("{err}");
        assert!(msg.contains("non-async"), "error message: {msg}");
        assert!(msg.contains("main"), "error message: {msg}");
    }

    #[test]
    fn nested_spawn_in_async_ok() {
        let code = r#"
async fn work() {
    spawn {
        spawn { inner() };
    };
}
"#;
        let errors = validate_spawn_context(code);
        // Both spawns are inside async fn work — both OK
        assert!(
            errors.is_empty(),
            "nested spawns in async fn should be OK, got: {errors:?}"
        );
    }

    #[test]
    fn spawn_in_string_not_flagged() {
        let code = r#"
fn work() {
    let s = "spawn { nope }";
}
"#;
        let errors = validate_spawn_context(code);
        assert!(
            errors.is_empty(),
            "spawn in string literal should not be flagged"
        );
    }

    // ─── v0.8 edge-case tests ───

    /// "respawn" must NOT be recognized as spawn.
    #[test]
    fn respawn_prefix_not_matched() {
        let code = r#"
fn main() {
    let respawn = true;
}
"#;
        let errors = validate_spawn_context(code);
        assert!(errors.is_empty(), "'respawn' must not match 'spawn'");
    }

    /// "spawn_worker()" must NOT be recognized as spawn block.
    #[test]
    fn spawn_suffixed_not_matched() {
        let code = r#"
fn main() {
    spawn_worker();
}
"#;
        let errors = validate_spawn_context(code);
        assert!(errors.is_empty(), "'spawn_worker' must not match 'spawn'");
    }

    /// Multiple spawn blocks: 2 in sync fns + 1 in async fn → 2 errors.
    #[test]
    fn mixed_async_sync_spawn_errors() {
        let code = r#"
fn sync_fn() {
    spawn { bad1(); }
}
async fn async_fn() {
    spawn { good(); }
}
fn another_sync() {
    spawn { bad2(); }
}
"#;
        let errors = validate_spawn_context(code);
        assert_eq!(
            errors.len(),
            2,
            "expected 2 errors for sync fns, got: {errors:?}"
        );
        let names: Vec<Option<String>> = errors.iter().map(|e| e.function_name.clone()).collect();
        assert!(names.contains(&Some("sync_fn".to_owned())));
        assert!(names.contains(&Some("another_sync".to_owned())));
    }

    /// spawn at top level (no enclosing fn) → error with function_name=None.
    #[test]
    fn spawn_at_top_level_error_with_no_fn_name() {
        let code = r#"
spawn {
    orphan();
}
"#;
        let errors = validate_spawn_context(code);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].function_name.is_none());
    }

    /// spawn in comment should NOT be rewritten.
    #[test]
    fn spawn_in_line_comment_not_rewritten() {
        let code = r#"
async fn handler() {
    // spawn { commented }
    let x = 1;
}
"#;
        let (output, infos) = preprocess_spawn_blocks(code, FileId(0));
        assert!(infos.is_empty(), "commented spawn should not be rewritten");
        assert!(output.contains("// spawn { commented }"));
    }

    /// Empty async fn with no spawn → no errors, no infos.
    #[test]
    fn empty_async_fn_no_errors() {
        let code = "async fn noop() {}";
        let errors = validate_spawn_context(code);
        assert!(errors.is_empty());
        let (_, infos) = preprocess_spawn_blocks(code, FileId(0));
        assert!(infos.is_empty());
    }

    /// Source with no "spawn" at all → empty results.
    #[test]
    fn no_spawn_keyword_passthrough() {
        let code = "fn main() { let x = 42; }";
        let errors = validate_spawn_context(code);
        assert!(errors.is_empty());
        let (output, infos) = preprocess_spawn_blocks(code, FileId(0));
        assert_eq!(output, code);
        assert!(infos.is_empty());
    }

    /// Display format for sync fn error.
    #[test]
    fn display_sync_fn_error_format() {
        let err = SpawnContextError {
            offset: 42,
            function_name: Some("do_stuff".to_owned()),
        };
        let msg = format!("{err}");
        assert!(msg.contains("42"));
        assert!(msg.contains("do_stuff"));
        assert!(msg.contains("non-async"));
    }

    /// Display format for top-level error.
    #[test]
    fn display_top_level_error_format() {
        let err = SpawnContextError {
            offset: 0,
            function_name: None,
        };
        let msg = format!("{err}");
        assert!(msg.contains("outside any function"));
    }
}
