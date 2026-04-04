/// Kobo keyword pre-processor.
///
/// `syn::parse_file()` rejects `@strict` because `@` is not valid at
/// statement position in Rust. This module scans source text for Kobo
/// keywords, rewrites them to marker attributes (`#[__kobo_strict]`),
/// passes the rewritten source to `syn::parse_file()`, then
/// post-processes the AST to consume the marker attributes and set the
/// appropriate flags on AST nodes.
///
/// Design: table-driven so v0.6 can add new keywords without forking the
/// scanner. Per F-03 in the v0.5 design.
use syn::visit::Visit;
use syn::visit_mut::VisitMut;

use crate::ast::{KoboBlock, KoboItemFn};
use kobo_ir::{FileId, KoboSpan};

/// Allowed positions for a Kobo keyword.
#[derive(Debug, Clone, PartialEq)]
pub enum KeywordContext {
    /// `@strict { ... }` — block at statement position.
    Statement,
    /// `@strict fn ...` or `@strict async fn ...` — function declaration.
    FnDecl,
}

/// Configuration for one Kobo source keyword.
pub struct KoboKeywordConfig {
    /// The at-prefixed keyword in source (`"@strict"`).
    pub source_keyword: &'static str,
    /// The marker attribute name used after rewriting (`"__kobo_strict"`).
    pub marker_attribute: &'static str,
    /// Positions where this keyword is allowed.
    pub allowed_contexts: &'static [KeywordContext],
}

/// One occurrence of a Kobo keyword found during preprocessing.
#[derive(Debug, Clone)]
pub struct KeywordMarker {
    /// Index into the `KoboKeywordConfig` slice that matched.
    pub keyword_index: usize,
    /// Byte range `[start, end)` in the ORIGINAL source of the keyword token.
    pub original_span: (usize, usize),
}

/// Errors that can occur during preprocessing.
#[derive(Debug, thiserror::Error)]
pub enum PreprocessError {
    #[error("@strict is not allowed inside a closure body (use it at statement position)")]
    StrictInsideClosure { offset: usize },
    #[error(
        "@strict is not allowed as a sub-expression (use it at statement position only)"
    )]
    StrictSubExpression { offset: usize },
}

/// v0.5 keyword configuration table — single entry for `@strict`.
pub fn v05_keyword_configs() -> Vec<KoboKeywordConfig> {
    vec![KoboKeywordConfig {
        source_keyword: "@strict",
        marker_attribute: "__kobo_strict",
        allowed_contexts: &[KeywordContext::Statement, KeywordContext::FnDecl],
    }]
}

/// Scan source text, rewrite Kobo keywords to `#[marker_attribute]` syntax.
///
/// Returns `(rewritten_source, markers)`.  Markers record the original byte
/// offsets of each keyword so that `postprocess_strict_markers` can propagate
/// span information into the AST node.
///
/// # Rejection rules handled by a separate call
///
/// This function handles the SAFE cases only (no error):
/// - Skips `macro_rules!` bodies (Trap 21 / R-06).
///
/// Invalid cases (closures, sub-expressions) must be checked via
/// `preprocess_strict_reject_invalid` BEFORE calling this function.
pub fn preprocess_kobo_keywords(
    source: &str,
    configs: &[KoboKeywordConfig],
) -> (String, Vec<KeywordMarker>) {
    let mut result = String::with_capacity(source.len() + 128);
    let mut markers: Vec<KeywordMarker> = Vec::new();
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut pos = 0;

    // Track whether we are inside a macro_rules! body.
    let mut macro_rules_depth: u32 = 0;
    let mut brace_depth: u32 = 0;
    // When this is > 0, we are inside a macro_rules! brace at this depth.
    let mut macro_rules_brace_start: Option<u32> = None;

    while pos < len {
        // Detect `macro_rules!` to skip its body (Trap 21).
        if pos + 12 <= len && &source[pos..pos + 12] == "macro_rules!" {
            // Consume until the opening brace of the rule body.
            let start = pos;
            while pos < len && bytes[pos] != b'{' {
                pos += 1;
            }
            result.push_str(&source[start..pos]);
            if pos < len && bytes[pos] == b'{' {
                brace_depth += 1;
                macro_rules_brace_start = Some(brace_depth);
                macro_rules_depth += 1;
                result.push('{');
                pos += 1;
            }
            continue;
        }

        // Track brace depth to know when macro_rules! body ends.
        if bytes[pos] == b'{' {
            brace_depth += 1;
            result.push('{');
            pos += 1;
            continue;
        }
        if bytes[pos] == b'}' {
            if let Some(start_depth) = macro_rules_brace_start {
                if brace_depth == start_depth {
                    macro_rules_depth = macro_rules_depth.saturating_sub(1);
                    if macro_rules_depth == 0 {
                        macro_rules_brace_start = None;
                    }
                }
            }
            brace_depth = brace_depth.saturating_sub(1);
            result.push('}');
            pos += 1;
            continue;
        }

        // Skip string literals to avoid false positives inside strings.
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

        // Inside macro_rules! body — copy verbatim, no rewriting.
        if macro_rules_depth > 0 {
            result.push(bytes[pos] as char);
            pos += 1;
            continue;
        }

        // Try each keyword config.
        let mut matched = false;
        for (config_index, config) in configs.iter().enumerate() {
            let kw = config.source_keyword;
            if pos + kw.len() <= len && &source[pos..pos + kw.len()] == kw {
                // Ensure the character after the keyword is whitespace (not
                // e.g. @strict_foo).
                let after = pos + kw.len();
                let followed_by_space = after >= len
                    || bytes[after].is_ascii_whitespace()
                    || bytes[after] == b'\n'
                    || bytes[after] == b'\r';
                if !followed_by_space {
                    break;
                }

                let keyword_end = pos + kw.len();
                markers.push(KeywordMarker {
                    keyword_index: config_index,
                    original_span: (pos, keyword_end),
                });

                // Check what follows: `fn`, `async`, or `{`.
                let after_ws = skip_whitespace(source, keyword_end);
                let rest = &source[after_ws..];

                if rest.starts_with("async ") || rest.starts_with("async\t") || rest.starts_with("async\n") {
                    // @strict async fn → #[__kobo_strict] async fn
                    result.push_str("#[__kobo_strict]\n");
                } else if rest.starts_with("fn ") || rest.starts_with("fn\t") || rest.starts_with("fn\n") {
                    // @strict fn → #[__kobo_strict] fn
                    result.push_str("#[__kobo_strict]\n");
                } else {
                    // @strict { ... } → #[__kobo_strict] { ... }
                    // Attribute on a block expression is valid Rust; syn will
                    // parse it as ExprBlock { attrs: [#[__kobo_strict]], .. }.
                    result.push_str("#[__kobo_strict] ");
                    pos = keyword_end;
                    matched = true;
                    break;
                }

                pos = keyword_end;
                matched = true;
                break;
            }
        }

        if !matched {
            result.push(bytes[pos] as char);
            pos += 1;
        }
    }

    (result, markers)
}

fn skip_whitespace(source: &str, mut pos: usize) -> usize {
    let bytes = source.as_bytes();
    while pos < source.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
        pos += 1;
    }
    pos
}

/// Validate that `@strict` does not appear in forbidden positions.
///
/// Returns `Err` if `@strict` is found:
/// - inside a closure body (`|| { @strict ... }`), or
/// - as a sub-expression (`let x = @strict { ... }`).
///
/// Must be called BEFORE `preprocess_kobo_keywords`.
pub fn preprocess_strict_reject_invalid(
    source: &str,
    configs: &[KoboKeywordConfig],
) -> Result<(), PreprocessError> {
    // Find all @strict occurrences.
    for config in configs {
        let kw = config.source_keyword;
        let mut search_pos = 0;
        while let Some(rel) = source[search_pos..].find(kw) {
            let abs = search_pos + rel;
            // Check if inside a macro_rules body — skip those.
            if is_inside_macro_rules(source, abs) {
                search_pos = abs + kw.len();
                continue;
            }
            // Check if followed by valid whitespace/char.
            let after = abs + kw.len();
            if after < source.len() {
                let b = source.as_bytes()[after];
                if b.is_ascii_alphanumeric() || b == b'_' {
                    search_pos = abs + kw.len();
                    continue;
                }
            }
            // Check: is this @strict preceded by `= ` or `, ` or `return ` (sub-expression position)?
            if is_subexpression_position(source, abs) {
                return Err(PreprocessError::StrictSubExpression { offset: abs });
            }
            // Check: is this @strict inside a closure body?
            if is_inside_closure(source, abs) {
                return Err(PreprocessError::StrictInsideClosure { offset: abs });
            }
            search_pos = abs + kw.len();
        }
    }
    Ok(())
}

/// Heuristic: is `@strict` at `pos` preceded by assignment-like contexts?
fn is_subexpression_position(source: &str, pos: usize) -> bool {
    // Look backwards for `=` or `(` or `,` skipping whitespace.
    let before = source[..pos].trim_end();
    before.ends_with('=') || before.ends_with('(') || before.ends_with(',')
}

/// Heuristic: is `@strict` at `pos` inside a closure body?
///
/// A closure body is `|| { ... }` or `|args| { ... }`. We detect this by
/// checking if any unmatched `||` or `|args|` appears before `pos`
/// within the innermost brace pair.
fn is_inside_closure(source: &str, pos: usize) -> bool {
    let before = &source[..pos];
    // Find the start of the innermost `{` that contains pos.
    // Walk backwards, tracking brace depth.
    let bytes = before.as_bytes();
    let mut depth: i32 = 0;
    let mut block_start: Option<usize> = None;

    for i in (0..bytes.len()).rev() {
        match bytes[i] {
            b'}' => depth += 1,
            b'{' => {
                if depth == 0 {
                    block_start = Some(i);
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }

    let Some(bs) = block_start else {
        return false;
    };

    // Check if the text just before `{` looks like a closure.
    let prefix = before[..bs].trim_end();
    prefix.ends_with("||")
        || prefix.ends_with('|')
        || (prefix.ends_with(')') && {
            // Could be a multi-arg closure like `|a, b|`
            // Rough heuristic: search for `|` before the `|...|` group.
            prefix.rfind('|').is_some()
        })
}

/// Heuristic: is `pos` inside the body of a `macro_rules!` definition?
fn is_inside_macro_rules(source: &str, pos: usize) -> bool {
    let before = &source[..pos];
    // Find the last `macro_rules!` before pos.
    let Some(mr_pos) = before.rfind("macro_rules!") else {
        return false;
    };
    // Count braces between mr_pos and pos.
    let bytes = before.as_bytes();
    let mut depth: i32 = 0;
    for &b in &bytes[mr_pos..] {
        match b {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
    }
    // If we have unclosed braces at pos, we're inside the macro_rules! body.
    depth > 0
}

/// Walk the AST after `syn::parse_file()`, remove `#[__kobo_strict]` attributes,
/// and record which nodes were marked. Contract C06: the marker never survives
/// into the final AST.
pub fn postprocess_strict_markers(
    file: &mut syn::File,
    _markers: &[KeywordMarker],
) -> Result<(), PreprocessError> {
    let mut remover = StrictMarkerRemover;
    remover.visit_file_mut(file);
    Ok(())
}

struct StrictMarkerRemover;

impl VisitMut for StrictMarkerRemover {
    fn visit_item_fn_mut(&mut self, node: &mut syn::ItemFn) {
        // Remove #[__kobo_strict] from fn-level attributes.
        node.attrs.retain(|attr| !is_kobo_strict_attr(attr));
        syn::visit_mut::visit_item_fn_mut(self, node);
    }

    fn visit_expr_block_mut(&mut self, node: &mut syn::ExprBlock) {
        // Remove #[__kobo_strict] from block-expression attributes.
        node.attrs.retain(|attr| !is_kobo_strict_attr(attr));
        syn::visit_mut::visit_expr_block_mut(self, node);
    }
}

fn is_kobo_strict_attr(attr: &syn::Attribute) -> bool {
    attr.path()
        .get_ident()
        .map(|id| id == "__kobo_strict")
        .unwrap_or(false)
}

/// Walk a `syn::File` (BEFORE stripping markers) and collect every
/// `@strict` block and `@strict fn` item as `KoboBlock` / `KoboItemFn`.
///
/// Must be called on the syn file BEFORE `postprocess_strict_markers`;
/// after stripping the `#[__kobo_strict]` attributes are gone and this
/// function would return empty vectors.
pub fn collect_strict_items_from_syn(
    file: &syn::File,
    source: &str,
    file_id: FileId,
) -> (Vec<KoboBlock>, Vec<KoboItemFn>) {
    let mut collector = StrictItemCollector::new(source, file_id);
    collector.visit_file(file);
    collector.finish()
}

struct StrictItemCollector<'src> {
    file_id: FileId,
    strict_blocks: Vec<KoboBlock>,
    strict_fns: Vec<KoboItemFn>,
    line_starts: Vec<usize>,
    source_len: usize,
    // Phantom to keep lifetime:
    _source: std::marker::PhantomData<&'src str>,
}

impl<'src> StrictItemCollector<'src> {
    fn new(source: &'src str, file_id: FileId) -> Self {
        let mut line_starts = vec![0usize];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self {
            file_id,
            strict_blocks: Vec::new(),
            strict_fns: Vec::new(),
            line_starts,
            source_len: source.len(),
            _source: std::marker::PhantomData,
        }
    }

    fn byte_offset(&self, lc: proc_macro2::LineColumn) -> u32 {
        let line_idx = lc.line.saturating_sub(1);
        let line_start = self.line_starts.get(line_idx).copied().unwrap_or(self.source_len);
        ((line_start + lc.column).min(self.source_len)) as u32
    }

    fn kobo_span_from(&self, span: proc_macro2::Span) -> KoboSpan {
        KoboSpan::new(
            self.byte_offset(span.start()),
            self.byte_offset(span.end()),
            self.file_id,
        )
    }

    fn finish(self) -> (Vec<KoboBlock>, Vec<KoboItemFn>) {
        (self.strict_blocks, self.strict_fns)
    }
}

impl<'ast, 'src> Visit<'ast> for StrictItemCollector<'src> {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        use syn::spanned::Spanned;
        if node.attrs.iter().any(|a| is_kobo_strict_attr(a)) {
            self.strict_fns.push(KoboItemFn {
                inner: node.clone(),
                span: self.kobo_span_from(node.span()),
                is_strict: true,
                strict_keyword_span: None,
                is_async: node.sig.asyncness.is_some(),
            });
        }
        syn::visit::visit_item_fn(self, node);
    }

    fn visit_expr_block(&mut self, node: &'ast syn::ExprBlock) {
        use syn::spanned::Spanned;
        if node.attrs.iter().any(|a| is_kobo_strict_attr(a)) {
            // Use node.block.span() (the inner { ... }), not node.span() which
            // includes the #[__kobo_strict] attribute. This ensures the span is
            // stable across postprocess_strict_markers which strips the attrs.
            self.strict_blocks.push(KoboBlock {
                body: node.block.clone(),
                span: self.kobo_span_from(node.block.span()),
                is_strict: true,
                strict_keyword_span: None,
            });
        }
        syn::visit::visit_expr_block(self, node);
    }
}

