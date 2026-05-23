#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskLocalWarning {
    pub kind: TaskLocalWarningKind,
    pub source_offset: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskLocalCapture {
    pub binding_name: String,
    pub type_name: String,
    pub source_offset: usize,
    pub is_explicit_local: bool,
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
    for capture in scan_source_task_local_captures(source) {
        if !capture.is_explicit_local {
            warnings.push(TaskLocalWarning {
                kind: TaskLocalWarningKind::NormalSpawnNonSendCapture {
                    binding_name: capture.binding_name,
                    type_name: capture.type_name,
                },
                source_offset: capture.source_offset,
            });
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

pub fn scan_source_task_local_captures(source: &str) -> Vec<TaskLocalCapture> {
    let mut captures = Vec::new();
    let non_send_bindings = non_send_bindings(source);
    for binding in &non_send_bindings {
        let mut search_offset = 0usize;
        while let Some(found) = source[search_offset..].find("spawn") {
            let spawn_offset = search_offset + found;
            let Some(block) = spawn_block(source, spawn_offset) else {
                search_offset = spawn_offset + "spawn".len();
                continue;
            };
            let body = &source[block.body_start..block.body_end];
            if contains_ident(body, &binding.name) {
                captures.push(TaskLocalCapture {
                    binding_name: binding.name.clone(),
                    type_name: binding.type_name.clone(),
                    source_offset: spawn_offset,
                    is_explicit_local: block.is_local,
                });
            }
            search_offset = block.end;
        }
    }
    captures
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
    let non_send_type_names = non_send_type_names(source);
    let aliases = non_send_aliases(source);
    let mut bindings = source
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
            if contains_non_send_alias(rest, &aliases, "Rc") || rest.contains("std::rc::Rc") {
                return Some(NonSendBinding {
                    name: name.to_owned(),
                    type_name: matching_non_send_alias(rest, &aliases, "Rc")
                        .unwrap_or_else(|| "Rc".to_owned()),
                });
            }
            if contains_non_send_alias(rest, &aliases, "RefCell") {
                return Some(NonSendBinding {
                    name: name.to_owned(),
                    type_name: matching_non_send_alias(rest, &aliases, "RefCell")
                        .unwrap_or_else(|| "RefCell".to_owned()),
                });
            }
            for type_name in &non_send_type_names {
                if rest.contains(&format!("{type_name}::"))
                    || rest.contains(&format!("{type_name} {{"))
                    || rest.contains(&format!(": {type_name}"))
                {
                    return Some(NonSendBinding {
                        name: name.to_owned(),
                        type_name: type_name.clone(),
                    });
                }
            }
            None
        })
        .collect::<Vec<_>>();
    bindings.extend(non_send_fn_params(source, &aliases));
    bindings.sort_by(|left, right| left.name.cmp(&right.name));
    bindings.dedup_by(|left, right| left.name == right.name && left.type_name == right.type_name);
    bindings
}

fn non_send_aliases(source: &str) -> Vec<(String, String)> {
    let mut aliases = vec![
        ("Rc".to_owned(), "Rc".to_owned()),
        ("RefCell".to_owned(), "RefCell".to_owned()),
        ("Cell".to_owned(), "Cell".to_owned()),
    ];
    for line in source.lines().map(str::trim) {
        if let Some(alias) = use_alias(line, "std::rc::Rc") {
            aliases.push((alias, "Rc".to_owned()));
        }
        if let Some(alias) = use_alias(line, "std::cell::RefCell") {
            aliases.push((alias, "RefCell".to_owned()));
        }
        if let Some(alias) = use_alias(line, "std::cell::Cell") {
            aliases.push((alias, "Cell".to_owned()));
        }
    }
    aliases
}

fn use_alias(line: &str, path: &str) -> Option<String> {
    let rest = line.strip_prefix("use ")?.strip_suffix(';')?.trim();
    let alias = rest.strip_prefix(path)?.trim();
    alias
        .strip_prefix("as ")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn contains_non_send_alias(source: &str, aliases: &[(String, String)], family: &str) -> bool {
    matching_non_send_alias(source, aliases, family).is_some()
}

fn matching_non_send_alias(
    source: &str,
    aliases: &[(String, String)],
    family: &str,
) -> Option<String> {
    aliases.iter().find_map(|(alias, alias_family)| {
        if alias_family != family {
            return None;
        }
        (contains_type_alias(source, alias) || contains_ctor_alias(source, alias))
            .then(|| alias.clone())
    })
}

fn contains_type_alias(source: &str, alias: &str) -> bool {
    source.match_indices(alias).any(|(index, _)| {
        let before = source[..index].chars().next_back();
        let after = source[index + alias.len()..].chars().next();
        !before.is_some_and(is_ident_char)
            && matches!(after, Some('<') | Some('>') | Some(',') | Some(')') | None)
    })
}

fn contains_ctor_alias(source: &str, alias: &str) -> bool {
    source.match_indices(alias).any(|(index, _)| {
        let before = source[..index].chars().next_back();
        let tail = &source[index + alias.len()..];
        !before.is_some_and(is_ident_char) && tail.trim_start().starts_with("::new")
    })
}

fn non_send_fn_params(source: &str, aliases: &[(String, String)]) -> Vec<NonSendBinding> {
    let mut bindings = Vec::new();
    for signature in function_signatures(source) {
        let Some(params) = signature
            .split_once('(')
            .and_then(|(_, rest)| rest.rsplit_once(')').map(|(params, _)| params))
        else {
            continue;
        };
        for param in params.split(',') {
            let Some((name, ty)) = param.split_once(':') else {
                continue;
            };
            let name = name.trim().trim_start_matches("mut ").trim();
            if name.is_empty() || name == "self" {
                continue;
            }
            if let Some(type_name) = matching_non_send_alias(ty, aliases, "Rc")
                .or_else(|| matching_non_send_alias(ty, aliases, "RefCell"))
                .or_else(|| matching_non_send_alias(ty, aliases, "Cell"))
            {
                bindings.push(NonSendBinding {
                    name: name.to_owned(),
                    type_name,
                });
            }
        }
    }
    bindings
}

fn function_signatures(source: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    let mut lines = source.lines().peekable();
    while let Some(line) = lines.next() {
        if !line.contains("fn ") {
            continue;
        }
        let mut signature = line.trim().to_owned();
        while !signature.contains('{') && !signature.ends_with(';') {
            let Some(next) = lines.peek() else {
                break;
            };
            signature.push(' ');
            signature.push_str(next.trim());
            lines.next();
        }
        signatures.push(signature);
    }
    signatures
}

fn non_send_type_names(source: &str) -> Vec<String> {
    let mut type_names = Vec::new();
    let mut search_offset = 0usize;
    while let Some(found) = source[search_offset..].find("struct ") {
        let struct_offset = search_offset + found;
        let name_start = struct_offset + "struct ".len();
        let Some(name_end) = source[name_start..]
            .find(|ch: char| !(ch == '_' || ch.is_ascii_alphanumeric()))
            .map(|relative| name_start + relative)
        else {
            break;
        };
        let type_name = source[name_start..name_end].trim();
        let Some(open_brace) = source[name_end..]
            .find('{')
            .map(|relative| name_end + relative)
        else {
            search_offset = name_end;
            continue;
        };
        let Some(close_brace) = find_matching_delimiter(source, open_brace, '{', '}') else {
            search_offset = open_brace + 1;
            continue;
        };
        let body = &source[open_brace + 1..close_brace];
        if contains_non_send_type(body) {
            type_names.push(type_name.to_owned());
        }
        search_offset = close_brace + 1;
    }
    type_names
}

fn contains_non_send_type(source: &str) -> bool {
    source.contains("Rc<")
        || source.contains("Rc ::")
        || source.contains("std::rc::Rc")
        || source.contains("RefCell<")
        || source.contains("Cell<")
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
        format!("Some({binding}"),
        format!("Ok({binding}"),
        format!("vec![{binding}"),
        format!("{binding},"),
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
    fn detects_normal_spawn_user_defined_non_send_capture() {
        let warnings = scan_source_task_local_warnings(
            r#"
use std::rc::Rc;

struct LocalState {
    inner: Rc<String>,
}

impl LocalState {
    fn new() -> Self {
        Self { inner: Rc::new(String::from("local")) }
    }
}

async fn f() {
    let state = LocalState::new();
    spawn { println!("{}", state.inner); };
}
"#,
        );
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warnings[0].kind,
            TaskLocalWarningKind::NormalSpawnNonSendCapture {
                binding_name: "state".to_owned(),
                type_name: "LocalState".to_owned(),
            }
        );
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
    fn captures_are_scoped_to_the_spawn_body() {
        let captures = scan_source_task_local_captures(
            r#"
use std::rc::Rc;
async fn f() {
    let local_state = Rc::new(1);
    spawn local { println!("{}", local_state); };
    let cross_state = Rc::new(2);
    spawn { println!("{}", cross_state); };
}
"#,
        );
        assert_eq!(captures.len(), 2);
        assert!(captures
            .iter()
            .any(|capture| { capture.binding_name == "local_state" && capture.is_explicit_local }));
        assert!(captures.iter().any(|capture| {
            capture.binding_name == "cross_state" && !capture.is_explicit_local
        }));
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
