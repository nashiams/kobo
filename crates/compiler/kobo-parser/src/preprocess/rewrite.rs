/// Source text rewriting: scan for Kobo keywords, replace with marker attributes.
///
/// Handles `macro_rules!` body skipping (Trap 21 / R-06) and string literal skipping.
use super::{KeywordMarker, KoboKeywordConfig};

/// Scan source text, rewrite Kobo keywords to `#[marker_attribute]` syntax.
///
/// Returns `(rewritten_source, markers)`.
pub fn preprocess_kobo_keywords(
    source: &str,
    configs: &[KoboKeywordConfig],
) -> (String, Vec<KeywordMarker>) {
    let mut result = String::with_capacity(source.len() + 128);
    let mut markers: Vec<KeywordMarker> = Vec::new();
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut pos = 0;

    let mut macro_rules_depth: u32 = 0;
    let mut brace_depth: u32 = 0;
    let mut macro_rules_brace_start: Option<u32> = None;

    while pos < len {
        if pos + 12 <= len && bytes[pos..pos + 12] == *b"macro_rules!" {
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

        if macro_rules_depth > 0 {
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
            continue;
        }

        let mut matched = false;
        for (config_index, config) in configs.iter().enumerate() {
            let kw = config.source_keyword;
            if pos + kw.len() <= len && bytes[pos..pos + kw.len()] == *kw.as_bytes() {
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

                let after_ws = skip_whitespace(source, keyword_end);
                let rest = &source[after_ws..];

                if rest.starts_with("async ")
                    || rest.starts_with("async\t")
                    || rest.starts_with("async\n")
                    || rest.starts_with("fn ")
                    || rest.starts_with("fn\t")
                    || rest.starts_with("fn\n")
                {
                    result.push_str("#[__kobo_strict]\n");
                } else {
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
            // Handle multi-byte UTF-8 characters correctly.
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
