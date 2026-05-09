//! S-57: `sync { }` / `async { }` bridge block preprocessing.
//!
//! Rewrites bridge blocks in Kobo source before syn parse:
//! - `sync { body }` inside an async fn → `tokio::task::spawn_blocking(move || { body }).await`
//! - `async { body }` inside a sync fn → `tokio::runtime::Handle::current().block_on(async { body })`
//!
//! These dissolve function-coloring friction by letting developers mix
//! sync and async code naturally. The preprocessor detects context (async/sync fn)
//! and emits the appropriate tokio bridge call.
use kobo_ir::{FileId, KoboSpan};

use super::source_map::{push_identity_segment, PreprocessSourceMap, PreprocessedSource};

/// Information about a bridge block rewrite.
#[derive(Clone, Debug, PartialEq)]
pub struct BridgeBlockInfo {
    /// Byte offset in the original source.
    pub offset: usize,
    /// Kind of bridge block.
    pub kind: BridgeKind,
    /// Byte range of the full bridge block in the ORIGINAL source.
    pub original_span: (usize, usize),
    /// Byte range of the copied body in the ORIGINAL source.
    pub original_body_span: (usize, usize),
    /// Byte range of the generated call prefix in the REWRITTEN source.
    pub generated_prefix_span: (usize, usize),
    /// Byte range of the copied body in the REWRITTEN source.
    pub generated_body_span: (usize, usize),
    /// Byte range of the generated call suffix in the REWRITTEN source.
    pub generated_suffix_span: (usize, usize),
}

/// Kind of bridge block detected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BridgeKind {
    /// `sync { }` inside async context → spawn_blocking
    SyncInAsync,
    /// `async { }` inside sync context → block_on
    AsyncInSync,
}

/// Rewrite `sync { ... }` and context-aware `async { ... }` bridge blocks.
///
/// Returns `(rewritten_source, bridge_infos)`.
///
/// Rules:
/// - `sync { body }` anywhere inside an `async fn` → `tokio::task::spawn_blocking(move || { body }).await`
/// - A bare `async { body }` inside a non-async `fn` (the "bridge" case) →
///   `tokio::runtime::Handle::current().block_on(async { body })`
///
/// We do NOT rewrite `async { }` inside async functions — that's a standard
/// Rust async block, not a bridge.
pub fn preprocess_bridge_blocks(source: &str) -> (String, Vec<BridgeBlockInfo>) {
    let mut result = String::with_capacity(source.len());
    let mut infos = Vec::new();
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut pos = 0;

    // Track whether we are inside an async fn via simple text scanning.
    // This is approximate: we look for `async fn` declarations and track
    // brace depth to know when we exit the function body.
    let async_fn_ranges = find_async_fn_ranges(source);
    let sync_fn_ranges = find_sync_fn_ranges(source);

    while pos < len {
        // Skip non-char-boundary bytes (inside multi-byte UTF-8 sequences)
        if !source.is_char_boundary(pos) {
            pos += 1;
            continue;
        }

        // Check for `sync` keyword followed by `{`
        if pos + 4 <= len
            && source.is_char_boundary(pos + 4)
            && &source[pos..pos + 4] == "sync"
            && (pos == 0 || !is_ident_byte(bytes[pos - 1]))
            && (pos + 4 >= len || !is_ident_byte(bytes[pos + 4]))
        {
            let mut scan = pos + 4;
            while scan < len && bytes[scan].is_ascii_whitespace() {
                scan += 1;
            }
            if scan < len && bytes[scan] == b'{' {
                if let Some(close) = find_matching_brace(source, scan) {
                    // Only rewrite if inside an async fn
                    if is_inside_ranges(pos, &async_fn_ranges) {
                        let body = &source[scan + 1..close];
                        let prefix = "tokio::task::spawn_blocking(move || {";
                        let suffix = "}).await";
                        let generated_start = result.len();
                        result.push_str(prefix);
                        let generated_body_start = result.len();
                        result.push_str(body);
                        let generated_body_end = result.len();
                        result.push_str(suffix);
                        infos.push(BridgeBlockInfo {
                            offset: pos,
                            kind: BridgeKind::SyncInAsync,
                            original_span: (pos, close + 1),
                            original_body_span: (scan + 1, close),
                            generated_prefix_span: (generated_start, generated_body_start),
                            generated_body_span: (generated_body_start, generated_body_end),
                            generated_suffix_span: (generated_body_end, result.len()),
                        });
                        pos = close + 1;
                        continue;
                    }
                }
            }
        }

        // Check for `async` keyword followed by `{` in a sync fn context
        // Be careful: `async fn`, `async move {`, `async {` inside async fn are NOT bridges
        if pos + 5 <= len
            && source.is_char_boundary(pos + 5)
            && &source[pos..pos + 5] == "async"
            && (pos == 0 || !is_ident_byte(bytes[pos - 1]))
            && (pos + 5 >= len || !is_ident_byte(bytes[pos + 5]))
        {
            let mut scan = pos + 5;
            while scan < len && bytes[scan].is_ascii_whitespace() {
                scan += 1;
            }

            // Skip `async fn` — that's a function definition, not a bridge
            if scan + 2 <= len && &source[scan..scan + 2] == "fn" {
                result.push(bytes[pos] as char);
                pos += 1;
                continue;
            }

            // Skip `async move` — leave as-is
            if scan + 4 <= len && &source[scan..scan + 4] == "move" {
                result.push(bytes[pos] as char);
                pos += 1;
                continue;
            }

            if scan < len && bytes[scan] == b'{' {
                if let Some(close) = find_matching_brace(source, scan) {
                    // Only rewrite if inside a sync fn (not inside an async fn)
                    if is_inside_ranges(pos, &sync_fn_ranges)
                        && !is_inside_ranges(pos, &async_fn_ranges)
                    {
                        let body = &source[scan + 1..close];
                        let prefix = "tokio::runtime::Handle::current().block_on(async {";
                        let suffix = "})";
                        let generated_start = result.len();
                        result.push_str(prefix);
                        let generated_body_start = result.len();
                        result.push_str(body);
                        let generated_body_end = result.len();
                        result.push_str(suffix);
                        infos.push(BridgeBlockInfo {
                            offset: pos,
                            kind: BridgeKind::AsyncInSync,
                            original_span: (pos, close + 1),
                            original_body_span: (scan + 1, close),
                            generated_prefix_span: (generated_start, generated_body_start),
                            generated_body_span: (generated_body_start, generated_body_end),
                            generated_suffix_span: (generated_body_end, result.len()),
                        });
                        pos = close + 1;
                        continue;
                    }
                }
            }
        }

        // Push the current character (may be multi-byte)
        let ch = &source[pos..];
        let c = ch.chars().next().expect("pos is at a char boundary");
        result.push(c);
        pos += c.len_utf8();
    }

    (result, infos)
}

pub fn preprocess_bridge_blocks_mapped(
    source: &str,
    file_id: FileId,
) -> PreprocessedSource<Vec<BridgeBlockInfo>> {
    let (rewritten, infos) = preprocess_bridge_blocks(source);
    if infos.is_empty() {
        return PreprocessedSource {
            rewritten,
            source_map: PreprocessSourceMap::identity_for(source, file_id),
            metadata: infos,
        };
    }

    let mut source_map = PreprocessSourceMap::default();
    let mut sorted: Vec<&BridgeBlockInfo> = infos.iter().collect();
    sorted.sort_by_key(|info| info.original_span.0);

    let mut rewritten_cursor = 0usize;
    let mut original_cursor = 0usize;
    for info in sorted {
        push_identity_segment(
            &mut source_map,
            file_id,
            rewritten_cursor,
            info.generated_prefix_span.0,
            original_cursor,
            info.original_span.0,
        );
        source_map.push_segment(
            KoboSpan::new(
                info.generated_prefix_span.0 as u32,
                info.generated_prefix_span.1 as u32,
                file_id,
            ),
            KoboSpan::new(
                info.original_span.0 as u32,
                (info.original_body_span.0 + 1) as u32,
                file_id,
            ),
        );
        source_map.push_segment(
            KoboSpan::new(
                info.generated_body_span.0 as u32,
                info.generated_body_span.1 as u32,
                file_id,
            ),
            KoboSpan::new(
                info.original_body_span.0 as u32,
                info.original_body_span.1 as u32,
                file_id,
            ),
        );
        source_map.push_segment(
            KoboSpan::new(
                info.generated_suffix_span.0 as u32,
                info.generated_suffix_span.1 as u32,
                file_id,
            ),
            KoboSpan::new(
                info.original_span.1.saturating_sub(1) as u32,
                info.original_span.1 as u32,
                file_id,
            ),
        );
        rewritten_cursor = info.generated_suffix_span.1;
        original_cursor = info.original_span.1;
    }

    push_identity_segment(
        &mut source_map,
        file_id,
        rewritten_cursor,
        rewritten.len(),
        original_cursor,
        source.len(),
    );

    PreprocessedSource {
        rewritten,
        source_map,
        metadata: infos,
    }
}

/// A range (start, end) of byte offsets for a function body.
type FnRange = (usize, usize);

/// Find byte ranges of async fn bodies.
fn find_async_fn_ranges(source: &str) -> Vec<FnRange> {
    find_fn_ranges(source, true)
}

/// Find byte ranges of non-async fn bodies.
fn find_sync_fn_ranges(source: &str) -> Vec<FnRange> {
    find_fn_ranges(source, false)
}

fn find_fn_ranges(source: &str, want_async: bool) -> Vec<FnRange> {
    let mut ranges = Vec::new();
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut pos = 0;

    while pos < len {
        // Skip non-char-boundary bytes
        if !source.is_char_boundary(pos) {
            pos += 1;
            continue;
        }

        // Look for "fn " keyword
        let fn_prefix = if want_async { "async fn " } else { "fn " };
        let end = pos + fn_prefix.len();
        if end <= len && source.is_char_boundary(end) && &source[pos..end] == fn_prefix {
            // Verify it's not `async fn` when we want sync
            if !want_async && pos >= 6 {
                let start = pos.saturating_sub(6);
                let start = if source.is_char_boundary(start) {
                    start
                } else {
                    (start..pos)
                        .find(|&i| source.is_char_boundary(i))
                        .unwrap_or(pos)
                };
                let before = &source[start..pos];
                if before.trim_end().ends_with("async") {
                    pos += 1;
                    continue;
                }
            }

            // Find the opening `{` of the function body
            let mut scan = pos + fn_prefix.len();
            while scan < len && bytes[scan] != b'{' {
                scan += 1;
            }
            if scan < len {
                if let Some(close) = find_matching_brace(source, scan) {
                    ranges.push((scan, close));
                    pos = close + 1;
                    continue;
                }
            }
        }
        pos += 1;
    }

    ranges
}

fn is_inside_ranges(offset: usize, ranges: &[FnRange]) -> bool {
    ranges
        .iter()
        .any(|&(start, end)| offset > start && offset < end)
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
    fn sync_in_async_fn_rewrites_to_spawn_blocking() {
        let source = r#"async fn process() {
    sync {
        heavy_compute();
    }
}"#;
        let (output, infos) = preprocess_bridge_blocks(source);
        assert!(output.contains("spawn_blocking"), "output: {output}");
        assert!(output.contains(".await"), "output: {output}");
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].kind, BridgeKind::SyncInAsync);
    }

    #[test]
    fn async_in_sync_fn_rewrites_to_block_on() {
        let source = r#"fn main() {
    async {
        fetch_data().await;
    }
}"#;
        let (output, infos) = preprocess_bridge_blocks(source);
        assert!(output.contains("block_on"), "output: {output}");
        assert!(output.contains("Handle::current()"), "output: {output}");
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].kind, BridgeKind::AsyncInSync);
    }

    #[test]
    fn async_in_async_fn_not_rewritten() {
        let source = r#"async fn handler() {
    let fut = async {
        do_thing().await;
    };
}"#;
        let (output, infos) = preprocess_bridge_blocks(source);
        // Should NOT rewrite — async block in async fn is normal Rust
        assert!(!output.contains("block_on"), "output: {output}");
        assert!(infos.is_empty());
    }

    #[test]
    fn sync_outside_async_fn_not_rewritten() {
        let source = r#"fn main() {
    sync {
        compute();
    }
}"#;
        let (output, infos) = preprocess_bridge_blocks(source);
        // sync in a sync fn has no meaningful bridge — leave as-is
        assert!(!output.contains("spawn_blocking"), "output: {output}");
        assert!(infos.is_empty());
    }

    #[test]
    fn preserves_async_fn_keyword() {
        let source = "async fn handler() { }";
        let (output, _) = preprocess_bridge_blocks(source);
        assert!(output.contains("async fn"), "output: {output}");
    }

    #[test]
    fn empty_source() {
        let (output, infos) = preprocess_bridge_blocks("");
        assert!(output.is_empty());
        assert!(infos.is_empty());
    }
}
