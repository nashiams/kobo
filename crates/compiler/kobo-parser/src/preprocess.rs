pub(crate) mod bridge;
pub use bridge::{collect_bridge_decisions, BridgedKeyword, PreprocessBridge};
pub(crate) mod bridge_blocks;
pub(crate) mod channel;
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
mod collect;
mod engine;
mod handler;
mod postprocess;
mod rewrite;
pub(crate) mod select;
pub mod source_map;
pub(crate) mod spawn;
mod validate;

pub use bridge_blocks::{
    preprocess_bridge_blocks, preprocess_bridge_blocks_mapped, BridgeBlockInfo, BridgeKind,
};
pub use channel::{
    preprocess_chan_type, validate_channel_dependencies, ChannelInfo, ChannelWarning,
};
pub use collect::collect_strict_items_from_syn;
pub use engine::{collect_engine_structs, strip_engine_attributes, EngineInfo};
pub use handler::validate_handler_attributes;
pub use postprocess::postprocess_strict_markers;
pub use rewrite::{preprocess_kobo_keywords, preprocess_kobo_keywords_mapped};
pub use select::{preprocess_select_blocks, SelectError, SelectInfo, SelectWarning};
pub use source_map::{PreprocessMapSegment, PreprocessSourceMap, PreprocessedSource};
pub use spawn::{
    preprocess_spawn_blocks, preprocess_spawn_blocks_mapped, validate_spawn_context,
    SpawnBlockInfo, SpawnContextError,
};
pub use validate::preprocess_strict_reject_invalid;

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
    /// Byte range `[start, end)` in the REWRITTEN source of the generated marker.
    pub generated_span: (usize, usize),
}

/// Errors that can occur during preprocessing.
#[derive(Debug, thiserror::Error)]
pub enum PreprocessError {
    #[error("@strict is not allowed inside a closure body (use it at statement position)")]
    StrictInsideClosure { offset: usize },
    #[error("@strict is not allowed as a sub-expression (use it at statement position only)")]
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

fn is_kobo_strict_attr(attr: &syn::Attribute) -> bool {
    attr.path()
        .get_ident()
        .map(|id| id == "__kobo_strict")
        .unwrap_or(false)
}
