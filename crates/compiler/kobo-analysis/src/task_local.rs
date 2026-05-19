#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskLocalWarning {
    pub kind: TaskLocalWarningKind,
    pub source_offset: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TaskLocalWarningKind {
    NormalSpawnNonSendCapture {
        binding_name: String,
        type_name: String,
    },
    LocalFutureEscape {
        binding_name: String,
    },
}

pub fn scan_source_task_local_warnings(source: &str) -> Vec<TaskLocalWarning> {
    let mut warnings = Vec::new();
    let non_send_bindings = non_send_bindings(source);
    for binding in &non_send_bindings {
        let mut search_offset = 0usize;
        while let Some(found) = source[search_offset..].find("spawn") {
            let spawn_offset = search_offset + found;
            let Some(block) = spawn_block(source, spawn_offset) else {
                search_offset = spawn_offset + "spawn".len();
                continue;
            };
            if block.is_local {
                search_offset = block.end;
                continue;
            }
            let body = &source[block.body_start..block.body_end];
            if contains_ident(body, &binding.name) {
                warnings.push(TaskLocalWarning {
                    kind: TaskLocalWarningKind::NormalSpawnNonSendCapture {
                        binding_name: binding.name.clone(),
                        type_name: binding.type_name.clone(),
                    },
                    source_offset: spawn_offset,
                });
            }
            search_offset = block.end;
        }
    }

    let mut search_offset = 0usize;
    while let Some(found) = source[search_offset..].find("let ") {
        let let_offset = search_offset + found;
        let Some((binding, init_offset)) = local_binding_initializer(source, let_offset) else {
            search_offset = let_offset + "let ".len();
            continue;
        };
        let Some(block) = spawn_block(source, init_offset) else {
            search_offset = init_offset;
            continue;
        };
        if !block.is_local {
            search_offset = block.end;
            continue;
        }
        if let Some(escape_offset) = local_handle_escape(source, block.end, &binding) {
            warnings.push(TaskLocalWarning {
                kind: TaskLocalWarningKind::LocalFutureEscape {
                    binding_name: binding,
                },
                source_offset: escape_offset,
            });
        }
        search_offset = block.end;
    }

    warnings
}

struct NonSendBinding {
    name: String,
    type_name: String,
}

struct SpawnBlock {
    is_local: bool,
    body_start: usize,
    body_end: usize,
    end: usize,
}

fn non_send_bindings(source: &str) -> Vec<NonSendBinding> {
    source
        .lines()
        .scan(0usize, |offset, line| {
            let current = *offset;
            *offset += line.len() + 1;
            Some((current, line))
        })
        .filter_map(|(_, line)| {
            let trimmed = line.trim_start();
            let rest = trimmed.strip_prefix("let ")?;
            let name = rest
                .split([':', '=', ' '])
                .find(|part| !part.is_empty() && *part != "mut")?;
            if rest.contains("Rc::new") || rest.contains("Rc<") || rest.contains("std::rc::Rc") {
                return Some(NonSendBinding {
                    name: name.to_owned(),
                    type_name: "Rc".to_owned(),
                });
            }
            if rest.contains("RefCell::new") || rest.contains("RefCell<") {
                return Some(NonSendBinding {
                    name: name.to_owned(),
                    type_name: "RefCell".to_owned(),
                });
            }
            None
        })
        .collect()
}

fn spawn_block(source: &str, offset: usize) -> Option<SpawnBlock> {
    let bytes = source.as_bytes();
    if offset + "spawn".len() > bytes.len() || &bytes[offset..offset + "spawn".len()] != b"spawn" {
        return None;
    }
    let preceded_by_ident = offset > 0 && is_ident_byte(bytes[offset - 1]);
    let after_spawn = offset + "spawn".len();
    let followed_by_ident = after_spawn < bytes.len() && is_ident_byte(bytes[after_spawn]);
    if preceded_by_ident || followed_by_ident {
        return None;
    }

    let mut cursor = skip_ws(bytes, after_spawn);
    let mut is_local = false;
    if cursor + "local".len() <= bytes.len()
        && &bytes[cursor..cursor + "local".len()] == b"local"
        && (cursor + "local".len() == bytes.len() || !is_ident_byte(bytes[cursor + "local".len()]))
    {
        is_local = true;
        cursor = skip_ws(bytes, cursor + "local".len());
    }
    if cursor >= bytes.len() || bytes[cursor] != b'{' {
        return None;
    }
    let close = find_matching_delimiter(source, cursor, '{', '}')?;
    Some(SpawnBlock {
        is_local,
        body_start: cursor + 1,
        body_end: close,
        end: close + 1,
    })
}

fn local_binding_initializer(source: &str, let_offset: usize) -> Option<(String, usize)> {
    let after_let = let_offset + "let ".len();
    let semicolon = source[after_let..].find(';')? + after_let;
    let statement = &source[after_let..semicolon];
    let (name_part, init_part) = statement.split_once('=')?;
    let binding = name_part
        .trim()
        .trim_start_matches("mut ")
        .split(':')
        .next()?
        .trim();
    if binding.is_empty() {
        return None;
    }
    let init_offset = after_let + name_part.len() + 1 + init_part.find("spawn")?;
    Some((binding.to_owned(), init_offset))
}

fn local_handle_escape(source: &str, after_offset: usize, binding: &str) -> Option<usize> {
    let tail = &source[after_offset..];
    for pattern in [
        format!("return {binding}"),
        format!("{binding}.await"),
        format!("{binding};"),
    ] {
        if let Some(found) = tail.find(&pattern) {
            return Some(after_offset + found);
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

fn is_ident_byte(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

fn skip_ws(bytes: &[u8], mut cursor: usize) -> usize {
    while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    cursor
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
    fn detects_normal_spawn_rc_capture() {
        let warnings = scan_source_task_local_warnings(
            r#"
use std::rc::Rc;
async fn f() {
    let state = Rc::new(1);
    spawn { println!("{}", state); };
}
"#,
        );
        assert_eq!(warnings.len(), 1);
        assert!(matches!(
            warnings[0].kind,
            TaskLocalWarningKind::NormalSpawnNonSendCapture { .. }
        ));
    }

    #[test]
    fn ignores_spawn_local_rc_capture() {
        let warnings = scan_source_task_local_warnings(
            r#"
use std::rc::Rc;
async fn f() {
    let state = Rc::new(1);
    spawn local { println!("{}", state); };
}
"#,
        );
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
    }

    #[test]
    fn detects_local_handle_escape() {
        let warnings = scan_source_task_local_warnings(
            r#"
async fn f() {
    let task = spawn local { println!("local"); };
    return task;
}
"#,
        );
        assert_eq!(warnings.len(), 1);
        assert!(matches!(
            warnings[0].kind,
            TaskLocalWarningKind::LocalFutureEscape { .. }
        ));
    }
}
