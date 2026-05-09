use kobo_errors::{DiagLabel, KDiagnostic, KErrorCode, Severity};
use kobo_ir::{FileId, KoboSpan, NodeIdGen};

use crate::ast::{ParseOutcome, ParseRecovery, RecoveryMode};
use crate::parse::{parse_file, ParseError};

const RECOVERY_LIMIT: usize = 32;

pub fn parse_file_recovering(
    source: &str,
    file_id: FileId,
    id_gen: &mut NodeIdGen,
    mode: RecoveryMode,
) -> ParseOutcome {
    match parse_file(source, file_id, id_gen) {
        Ok(file) => ParseOutcome {
            file: Some(file),
            diagnostics: Vec::new(),
            poisoned_spans: Vec::new(),
            recoveries: Vec::new(),
        },
        Err(error) if mode == RecoveryMode::FailFast => {
            let span = normalized_span(error.primary_span(), source.len(), file_id);
            ParseOutcome {
                file: None,
                diagnostics: vec![diagnostic_for_error(
                    KErrorCode::K0110,
                    span,
                    error.to_string(),
                )],
                poisoned_spans: vec![span],
                recoveries: vec![ParseRecovery {
                    span,
                    message: error.to_string(),
                }],
            }
        }
        Err(first_error) => recover_file(source, file_id, id_gen, first_error),
    }
}

fn recover_file(
    source: &str,
    file_id: FileId,
    id_gen: &mut NodeIdGen,
    first_error: ParseError,
) -> ParseOutcome {
    let mut diagnostics = Vec::new();
    let mut poisoned_spans = Vec::new();
    let mut recoveries = Vec::new();

    let regions = item_regions(source);
    if regions.is_empty() {
        let span = normalized_span(first_error.primary_span(), source.len(), file_id);
        diagnostics.push(diagnostic_for_error(
            KErrorCode::K0110,
            span,
            first_error.to_string(),
        ));
        poisoned_spans.push(span);
        recoveries.push(ParseRecovery {
            span,
            message: first_error.to_string(),
        });
    } else {
        for region in regions.iter().take(RECOVERY_LIMIT) {
            let region_source = &source[region.start..region.end];
            if syn::parse_file(region_source).is_ok() {
                continue;
            }

            let code = if region.unclosed_delimiter {
                KErrorCode::K0111
            } else {
                KErrorCode::K0110
            };
            let span = KoboSpan::new(region.start as u32, region.end as u32, file_id);
            let message = if region.unclosed_delimiter {
                "unclosed delimiter while recovering parser input".to_owned()
            } else {
                syntax_message_for_region(region_source)
            };
            diagnostics.push(diagnostic_for_error(code, span, message.clone()));
            poisoned_spans.push(span);
            recoveries.push(ParseRecovery { span, message });
        }

        if regions.len() > RECOVERY_LIMIT {
            let span = KoboSpan::new(0, source.len() as u32, file_id);
            diagnostics.push(diagnostic_for_error(
                KErrorCode::K0113,
                span,
                "parser recovery limit reached".to_owned(),
            ));
        }
    }

    let recovered_source = mask_poisoned_regions(source, &poisoned_spans);
    let file = parse_file(&recovered_source, file_id, id_gen).ok();

    ParseOutcome {
        file,
        diagnostics,
        poisoned_spans,
        recoveries,
    }
}

fn diagnostic_for_error(code: KErrorCode, span: KoboSpan, message: String) -> KDiagnostic {
    KDiagnostic::new(
        code,
        Severity::Error,
        DiagLabel::primary(span, message.clone()),
        message,
        "skip poisoned region and continue parsing",
    )
}

fn syntax_message_for_region(region_source: &str) -> String {
    syn::parse_file(region_source)
        .map(|_| "syntax error recovered".to_owned())
        .unwrap_or_else(|error| error.to_string())
}

fn normalized_span(span: KoboSpan, source_len: usize, file_id: FileId) -> KoboSpan {
    let start = span.start.min(source_len as u32);
    let mut end = span.end.min(source_len as u32);
    if end <= start {
        end = (start + 1).min(source_len as u32);
    }
    if end <= start {
        return KoboSpan::new(0, source_len as u32, file_id);
    }
    KoboSpan::new(start, end, file_id)
}

fn mask_poisoned_regions(source: &str, poisoned_spans: &[KoboSpan]) -> String {
    if poisoned_spans.is_empty() {
        return source.to_owned();
    }

    let mut bytes = source.as_bytes().to_vec();
    for span in poisoned_spans {
        let start = (span.start as usize).min(bytes.len());
        let end = (span.end as usize).min(bytes.len());
        for byte in &mut bytes[start..end] {
            if *byte != b'\n' && *byte != b'\r' {
                *byte = b' ';
            }
        }
    }
    String::from_utf8(bytes).expect("masking ASCII spaces preserves UTF-8")
}

#[derive(Copy, Clone, Debug)]
struct ItemRegion {
    start: usize,
    end: usize,
    unclosed_delimiter: bool,
}

fn item_regions(source: &str) -> Vec<ItemRegion> {
    let mut regions = Vec::new();
    let mut pos = 0usize;
    while pos < source.len() {
        let Some(start) = find_next_item_start(source, pos) else {
            break;
        };

        let Some(region_end) = item_region_end(source, start) else {
            regions.push(ItemRegion {
                start,
                end: source.len(),
                unclosed_delimiter: true,
            });
            break;
        };

        regions.push(ItemRegion {
            start,
            end: region_end.end,
            unclosed_delimiter: region_end.unclosed_delimiter,
        });
        pos = region_end.end.max(start + 1);
    }
    regions
}

fn find_next_item_start(source: &str, from: usize) -> Option<usize> {
    let mut pos = from;
    while pos < source.len() {
        if !source.is_char_boundary(pos) {
            pos += 1;
            continue;
        }
        if item_keyword_at(source, pos).is_some() {
            return Some(pos);
        }
        pos += source[pos..].chars().next()?.len_utf8();
    }
    None
}

#[derive(Copy, Clone, Debug)]
struct ItemRegionEnd {
    end: usize,
    unclosed_delimiter: bool,
}

fn item_region_end(source: &str, start: usize) -> Option<ItemRegionEnd> {
    let keyword = item_keyword_at(source, start)?;
    if matches!(keyword, "use" | "const" | "static" | "mod") {
        if let Some(semicolon) = find_statement_end(source, start) {
            return Some(ItemRegionEnd {
                end: semicolon + 1,
                unclosed_delimiter: false,
            });
        }
    }

    let open = find_next_top_level_byte(source, start, b'{')?;
    if let Some(close) = find_matching_brace(source, open) {
        return Some(ItemRegionEnd {
            end: close + 1,
            unclosed_delimiter: false,
        });
    }

    Some(ItemRegionEnd {
        end: find_next_item_start(source, open + 1).unwrap_or(source.len()),
        unclosed_delimiter: true,
    })
}

fn item_keyword_at(source: &str, pos: usize) -> Option<&'static str> {
    for keyword in [
        "fn", "impl", "struct", "enum", "use", "mod", "const", "static",
    ] {
        let end = pos + keyword.len();
        if end <= source.len()
            && source.is_char_boundary(end)
            && &source[pos..end] == keyword
            && is_word_boundary(source, pos, end)
        {
            return Some(keyword);
        }
    }
    None
}

fn is_word_boundary(source: &str, start: usize, end: usize) -> bool {
    let bytes = source.as_bytes();
    let before = start > 0 && is_ident_byte(bytes[start - 1]);
    let after = end < source.len() && is_ident_byte(bytes[end]);
    !before && !after
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn find_statement_end(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut pos = start;
    while pos < bytes.len() {
        match bytes[pos] {
            b';' => return Some(pos),
            b'{' => return find_matching_brace(source, pos),
            b'"' => pos = skip_string(source, pos),
            _ => pos += 1,
        }
    }
    None
}

fn find_next_top_level_byte(source: &str, start: usize, target: u8) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut pos = start;
    while pos < bytes.len() {
        match bytes[pos] {
            byte if byte == target => return Some(pos),
            b';' => return None,
            b'"' => pos = skip_string(source, pos),
            _ => pos += 1,
        }
    }
    None
}

fn find_matching_brace(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if bytes.get(open) != Some(&b'{') {
        return None;
    }
    let mut depth = 1u32;
    let mut pos = open + 1;
    while pos < bytes.len() {
        match bytes[pos] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(pos);
                }
            }
            b'"' => pos = skip_string(source, pos),
            _ => {}
        }
        pos += 1;
    }
    None
}

fn skip_string(source: &str, quote: usize) -> usize {
    let bytes = source.as_bytes();
    let mut pos = quote + 1;
    while pos < bytes.len() {
        if bytes[pos] == b'\\' {
            pos += 2;
            continue;
        }
        if bytes[pos] == b'"' {
            return pos + 1;
        }
        pos += 1;
    }
    bytes.len()
}
