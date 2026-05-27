/// Validation: reject `@strict` in forbidden positions.
///
/// Must be called BEFORE `preprocess_kobo_keywords`.
use super::{KoboKeywordConfig, PreprocessError};

/// Validate that `@strict` does not appear in forbidden positions.
///
/// Returns `Err` if `@strict` is found:
/// - inside a closure body (`|| { @strict... }`), or
/// - as a sub-expression (`let x = @strict {... }`).
pub fn preprocess_strict_reject_invalid(
    source: &str,
    configs: &[KoboKeywordConfig],
) -> Result<(), PreprocessError> {
    for config in configs {
        let kw = config.source_keyword;
        let mut search_pos = 0;
        while let Some(rel) = source[search_pos..].find(kw) {
            let abs = search_pos + rel;
            if is_inside_macro_rules(source, abs) {
                search_pos = abs + kw.len();
                continue;
            }
            let after = abs + kw.len();
            if after < source.len() {
                let b = source.as_bytes()[after];
                if b.is_ascii_alphanumeric() || b == b'_' {
                    search_pos = abs + kw.len();
                    continue;
                }
            }
            if is_subexpression_position(source, abs) {
                return Err(PreprocessError::StrictSubExpression { offset: abs });
            }
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
    let before = source[..pos].trim_end();
    before.ends_with('=') || before.ends_with('(') || before.ends_with(',')
}

/// Heuristic: is `@strict` at `pos` inside a closure body?
fn is_inside_closure(source: &str, pos: usize) -> bool {
    let before = &source[..pos];
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

    let prefix = before[..bs].trim_end();
    prefix.ends_with("||")
        || prefix.ends_with('|')
        || (prefix.ends_with(')') && { prefix.rfind('|').is_some() })
}

/// Heuristic: is `pos` inside the body of a `macro_rules!` definition?
fn is_inside_macro_rules(source: &str, pos: usize) -> bool {
    let before = &source[..pos];
    let Some(mr_pos) = before.rfind("macro_rules!") else {
        return false;
    };
    let bytes = before.as_bytes();
    let mut depth: i32 = 0;
    for &b in &bytes[mr_pos..] {
        match b {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
    }
    depth > 0
}
