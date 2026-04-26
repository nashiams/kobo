#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HandlerLeakWarning {
    pub fn_name: String,
    pub binding_name: String,
    pub source_offset: usize,
}

pub fn scan_source_handler_leaks(source: &str) -> Vec<HandlerLeakWarning> {
    let mut warnings = Vec::new();
    for (attr_offset, _) in source.match_indices("#[kobo::handler]") {
        let after_attr = &source[attr_offset..];
        let Some(async_rel) = after_attr.find("async fn ") else {
            continue;
        };
        let sig_offset = attr_offset + async_rel;
        let sig = &source[sig_offset..];
        let Some(open_paren_rel) = sig.find('(') else {
            continue;
        };
        let name = sig["async fn ".len()..open_paren_rel].trim();
        let open_paren = sig_offset + open_paren_rel;
        let Some(close_paren) = find_matching_delimiter(source, open_paren, '(', ')') else {
            continue;
        };
        let params = extract_param_names(&source[open_paren + 1..close_paren]);
        if params.is_empty() {
            continue;
        }
        let Some(body_open_rel) = source[close_paren..].find('{') else {
            continue;
        };
        let body_open = close_paren + body_open_rel;
        let Some(body_close) = find_matching_delimiter(source, body_open, '{', '}') else {
            continue;
        };
        let body = &source[body_open + 1..body_close];
        for param in &params {
            if let Some(leak_rel) = find_spawn_capture(body, param) {
                warnings.push(HandlerLeakWarning {
                    fn_name: name.to_owned(),
                    binding_name: param.clone(),
                    source_offset: body_open + 1 + leak_rel,
                });
            }
        }
    }
    warnings
}

fn extract_param_names(params: &str) -> Vec<String> {
    params
        .split(',')
        .filter_map(|param| {
            let name = param.split(':').next()?.trim();
            let name = name
                .trim_start_matches("mut ")
                .trim_start_matches('&')
                .trim_start_matches("mut ")
                .trim();
            if name.is_empty() || name == "self" {
                None
            } else {
                Some(name.to_owned())
            }
        })
        .collect()
}

fn find_spawn_capture(body: &str, binding: &str) -> Option<usize> {
    for marker in ["spawn {", "tokio::spawn", "spawn_local"] {
        let Some(spawn_pos) = body.find(marker) else {
            continue;
        };
        let spawned_body = &body[spawn_pos..];
        if contains_ident(spawned_body, binding) {
            return Some(spawn_pos);
        }
    }
    None
}

fn contains_ident(source: &str, ident: &str) -> bool {
    source.match_indices(ident).any(|(idx, _)| {
        let before = source[..idx].chars().next_back();
        let after = source[idx + ident.len()..].chars().next();
        !before.is_some_and(is_ident_char) && !after.is_some_and(is_ident_char)
    })
}

fn is_ident_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn find_matching_delimiter(
    source: &str,
    open_offset: usize,
    open: char,
    close: char,
) -> Option<usize> {
    let mut depth = 0usize;
    for (rel, ch) in source[open_offset..].char_indices() {
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(open_offset + rel);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_request_capture_in_spawn() {
        let source = r#"
#[kobo::handler]
async fn handle(req: Request) {
    spawn {
        println!("{}", req.path);
    }
}
"#;

        let warnings = scan_source_handler_leaks(source);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].fn_name, "handle");
        assert_eq!(warnings[0].binding_name, "req");
    }

    #[test]
    fn ignores_handler_without_spawn_capture() {
        let source = r#"
#[kobo::handler]
async fn handle(req: Request) {
    println!("{}", req.path);
    spawn {
        println!("background");
    }
}
"#;

        let warnings = scan_source_handler_leaks(source);

        assert!(warnings.is_empty(), "warnings: {warnings:?}");
    }
}
