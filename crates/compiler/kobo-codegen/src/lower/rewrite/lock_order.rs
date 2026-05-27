/// Annotate lock acquisition order in inspect output.
///
/// When multiple `Arc<RwLock<T>>` bindings are accessed in the same scope,
/// annotate the lock acquisition order as a comment:
///
///   // kobo: lock order: state(1) → cache(2) → metrics(3)
///
/// Only appears in `kobo inspect` output, not in compiled binary.
use kobo_ir::KoboSpan;

/// Kind of lock access.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LockKind {
    Read,
    Write,
}

/// A detected lock acquisition site.
#[derive(Clone, Debug)]
pub(crate) struct LockSite {
    pub binding_name: String,
    pub lock_kind: LockKind,
    pub span: KoboSpan,
    pub order: u32,
}

/// Detect lock acquisition sites from source code and assign order numbers.
///
/// Looks for `.read().await` and `.write().await` patterns.
pub(crate) fn detect_lock_sites(source: &str) -> Vec<LockSite> {
    let mut sites = Vec::new();
    let mut order = 1u32;

    for (line_idx, line) in source.lines().enumerate() {
        let trimmed = line.trim();

        // Pattern: `let <name> = <expr>.read().await;` or `.write().await;`
        let lock_kind = if trimmed.contains(".read().await") {
            Some(LockKind::Read)
        } else if trimmed.contains(".write().await") {
            Some(LockKind::Write)
        } else {
            None
        };

        if let Some(kind) = lock_kind {
            // Extract the binding name from "let <name> =...".
            let binding_name = extract_lock_binding_name(trimmed);
            if let Some(name) = binding_name {
                sites.push(LockSite {
                    binding_name: name,
                    lock_kind: kind,
                    span: KoboSpan {
                        file_id: kobo_ir::FileId(0),
                        start: line_idx as u32,
                        end: line_idx as u32,
                    },
                    order,
                });
                order += 1;
            }
        }
    }

    sites
}

/// Generate lock order comment for a set of lock sites.
pub(crate) fn lock_order_comment(sites: &[LockSite]) -> Option<String> {
    if sites.len() < 2 {
        return None;
    }
    debug_assert!(sites.iter().all(|site| site.span.start <= site.span.end));

    let parts: Vec<String> = sites
        .iter()
        .map(|s| {
            let kind_str = match s.lock_kind {
                LockKind::Read => "r",
                LockKind::Write => "w",
            };
            format!("{}({},{})", s.binding_name, s.order, kind_str)
        })
        .collect();

    Some(format!("// kobo: lock order: {}", parts.join(" → ")))
}

fn extract_lock_binding_name(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if !trimmed.starts_with("let ") {
        return None;
    }
    let after_let = &trimmed[4..];
    // Skip `mut ` if present.
    let after_mut = if let Some(stripped) = after_let.strip_prefix("mut ") {
        stripped
    } else {
        after_let
    };
    // Extract identifier up to ` =` or `:`.
    let end = after_mut.find([' ', ':', '=']).unwrap_or(after_mut.len());
    let name = &after_mut[..end];
    if name.is_empty() {
        None
    } else {
        Some(name.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        detect_lock_sites, extract_lock_binding_name, lock_order_comment, LockKind, LockSite,
    };
    use kobo_ir::KoboSpan;

    #[test]
    fn detect_read_and_write_locks() {
        let source = r#"
let state = app_state.write().await;
let cache = app_cache.read().await;
"#;
        let sites = detect_lock_sites(source);
        assert_eq!(sites.len(), 2);
        assert_eq!(sites[0].binding_name, "state");
        assert_eq!(sites[0].lock_kind, LockKind::Write);
        assert_eq!(sites[0].order, 1);
        assert_eq!(sites[1].binding_name, "cache");
        assert_eq!(sites[1].lock_kind, LockKind::Read);
        assert_eq!(sites[1].order, 2);
    }

    #[test]
    fn lock_order_comment_with_two_locks() {
        let sites = vec![
            LockSite {
                binding_name: "state".to_owned(),
                lock_kind: LockKind::Write,
                span: KoboSpan {
                    file_id: kobo_ir::FileId(0),
                    start: 0,
                    end: 0,
                },
                order: 1,
            },
            LockSite {
                binding_name: "cache".to_owned(),
                lock_kind: LockKind::Read,
                span: KoboSpan {
                    file_id: kobo_ir::FileId(0),
                    start: 0,
                    end: 0,
                },
                order: 2,
            },
        ];
        let comment = lock_order_comment(&sites).unwrap();
        assert!(comment.contains("lock order"));
        assert!(comment.contains("state"));
        assert!(comment.contains("cache"));
    }

    #[test]
    fn single_lock_no_comment() {
        let sites = vec![LockSite {
            binding_name: "state".to_owned(),
            lock_kind: LockKind::Write,
            span: KoboSpan {
                file_id: kobo_ir::FileId(0),
                start: 0,
                end: 0,
            },
            order: 1,
        }];
        assert!(lock_order_comment(&sites).is_none());
    }

    #[test]
    fn no_locks_detected_in_non_lock_code() {
        let source = "let x = 1;\nlet y = x + 2;";
        let sites = detect_lock_sites(source);
        assert!(sites.is_empty());
    }

    #[test]
    fn extract_binding_with_mut() {
        let name = extract_lock_binding_name("let mut guard = state.write().await;");
        assert_eq!(name, Some("guard".to_owned()));
    }
}
