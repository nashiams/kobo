mod ast;
pub mod mode_parse;
mod parse;
pub mod preprocess;
mod recovery;

#[cfg(test)]
mod tests;

pub use ast::{
    KoboAstNode, KoboBinding, KoboBindingKind, KoboBlock, KoboFile, KoboItemFn, ParseOutcome,
    ParseRecovery, RecoveryMode,
};
pub use parse::{parse_file, ParseError};
pub use preprocess::{
    collect_engine_structs, collect_strict_items_from_syn, postprocess_strict_markers,
    preprocess_bridge_blocks, preprocess_bridge_blocks_mapped, preprocess_concurrent_sugar,
    preprocess_concurrent_sugar_mapped, preprocess_kobo_keywords, preprocess_kobo_keywords_mapped,
    preprocess_select_blocks, preprocess_spawn_blocks, preprocess_spawn_blocks_mapped,
    preprocess_strict_reject_invalid, v05_keyword_configs, validate_handler_attributes,
    BridgeBlockInfo, BridgeKind, BridgedKeyword, ConcurrentSugarKind, ConcurrentSugarOccurrence,
    EngineInfo, KeywordMarker, KoboKeywordConfig, PreprocessBridge, PreprocessError,
    PreprocessMapSegment, PreprocessSourceMap, PreprocessedSource, SpawnBlockInfo,
};
pub use recovery::parse_file_recovering;
