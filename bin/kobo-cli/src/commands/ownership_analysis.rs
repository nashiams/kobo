use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub(super) struct OwnershipAnalysis {
    pub audit_records: Vec<AuditRecord>,
    pub perf: PerfSummary,
}

#[derive(Clone, Debug)]
pub(super) struct AuditRecord {
    pub tier: u8,
    pub kind: &'static str,
    pub line: usize,
    pub column: usize,
    pub evidence: String,
}

#[derive(Clone, Debug)]
pub(super) struct PerfSummary {
    pub borrow_count: u64,
    pub mut_borrow_count: u64,
    pub contention: u64,
    pub hot_paths: Vec<HotPath>,
}

#[derive(Clone, Debug)]
pub(super) struct HotPath {
    pub line: usize,
    pub kind: &'static str,
    pub binding: Option<String>,
    pub field: Option<String>,
}

#[derive(Clone, Debug)]
struct BorrowSite {
    root: String,
    line: usize,
}

#[derive(Clone, Debug)]
struct MutationSite {
    root: Option<String>,
    field: Option<String>,
}

pub(super) fn analyze_source(source: &str) -> OwnershipAnalysis {
    let local_symbols = local_symbols(source);
    let mut audit_records = Vec::new();
    let mut hot_paths = Vec::new();
    let mut borrow_count = 0_u64;
    let mut mut_borrow_count = 0_u64;
    let mut contention = 0_u64;
    let mut immutable_borrows = Vec::new();

    for (line_index, line) in source.lines().enumerate() {
        let line_number = line_index + 1;
        let trimmed = line.trim();
        let column = line.find(trimmed).unwrap_or(0) + 1;

        if let Some(column) = clone_column(line) {
            audit_records.push(AuditRecord {
                tier: 1,
                kind: "mechanical",
                line: line_number,
                column,
                evidence: "clone can be reviewed as mechanical ownership debt".to_owned(),
            });
        }

        if let Some(site) = borrow_site(trimmed, line_number) {
            borrow_count += 1;
            if trimmed.contains("&mut") {
                mut_borrow_count += 1;
                hot_paths.push(HotPath {
                    line: line_number,
                    kind: "mutable-borrow",
                    binding: Some(site.root.clone()),
                    field: None,
                });
            } else {
                immutable_borrows.push(site);
            }
        }

        if let Some(site) = mutation_site(trimmed) {
            contention += 1;
            hot_paths.push(HotPath {
                line: line_number,
                kind: "mutation",
                binding: site.root.clone(),
                field: site.field.clone(),
            });
            if let Some(root) = site.root.as_deref() {
                if let Some(borrow) = immutable_borrows.iter().find(|borrow| borrow.root == root) {
                    audit_records.push(AuditRecord {
                        tier: 2,
                        kind: "structural",
                        line: line_number,
                        column,
                        evidence: format!(
                            "mutation of `{root}` conflicts with immutable borrow from line {}",
                            borrow.line
                        ),
                    });
                }
            }
        }

        if let Some(external) = external_boundary(trimmed, &local_symbols) {
            audit_records.push(AuditRecord {
                tier: 3,
                kind: "unknown",
                line: line_number,
                column,
                evidence: format!(
                    "external runtime boundary `{external}` requires unknown debt classification"
                ),
            });
        }
    }

    OwnershipAnalysis {
        audit_records,
        perf: PerfSummary {
            borrow_count,
            mut_borrow_count,
            contention,
            hot_paths,
        },
    }
}

fn clone_column(line: &str) -> Option<usize> {
    line.find(".clone(")
        .or_else(|| line.find(".clone()"))
        .map(|index| index + 1)
}

fn borrow_site(line: &str, line_number: usize) -> Option<BorrowSite> {
    let ampersand = line.find('&')?;
    let after_ampersand = line[ampersand + 1..]
        .strip_prefix("mut ")
        .unwrap_or(&line[ampersand + 1..]);
    let root = ident_prefix(after_ampersand.trim_start())?;
    Some(BorrowSite {
        root,
        line: line_number,
    })
}

fn mutation_site(line: &str) -> Option<MutationSite> {
    let operator = ["+=", "-=", "*=", "/=", ".push(", ".insert("]
        .into_iter()
        .find(|operator| line.contains(operator))?;
    let before = line.split(operator).next().unwrap_or(line).trim();
    let target = before;
    let mut parts = target
        .split('.')
        .map(str::trim)
        .filter(|part| !part.is_empty());
    let root = parts.next().and_then(ident_prefix);
    let field = parts.next_back().and_then(ident_prefix);
    Some(MutationSite { root, field })
}

fn external_boundary(line: &str, local_symbols: &BTreeSet<String>) -> Option<String> {
    if let Some(rest) = line.strip_prefix("extern crate ") {
        return ident_prefix(rest);
    }
    for (index, _) in line.match_indices("::") {
        let prefix = ident_suffix(&line[..index])?;
        if !is_known_path_prefix(&prefix) && !local_symbols.contains(&prefix) {
            return Some(prefix);
        }
    }
    None
}

fn local_symbols(source: &str) -> BTreeSet<String> {
    let mut symbols = BTreeSet::new();
    for line in source.lines().map(str::trim) {
        for prefix in [
            "struct ",
            "enum ",
            "fn ",
            "async fn ",
            "pub fn ",
            "pub async fn ",
        ] {
            if let Some(rest) = line.strip_prefix(prefix) {
                if let Some(name) = ident_prefix(rest) {
                    symbols.insert(name);
                }
            }
        }
    }
    symbols
}

fn is_known_path_prefix(prefix: &str) -> bool {
    matches!(
        prefix,
        "std"
            | "core"
            | "alloc"
            | "crate"
            | "self"
            | "super"
            | "String"
            | "Vec"
            | "Option"
            | "Result"
            | "Some"
            | "None"
            | "Ok"
            | "Err"
            | "Rc"
            | "RefCell"
            | "Arc"
            | "Mutex"
            | "RwLock"
            | "HashMap"
            | "HashSet"
            | "BTreeMap"
            | "BTreeSet"
            | "kobo"
    )
}

fn ident_prefix(input: &str) -> Option<String> {
    let ident = input
        .trim_start()
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect::<String>();
    (!ident.is_empty()).then_some(ident)
}

fn ident_suffix(input: &str) -> Option<String> {
    input
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .filter(|part| !part.is_empty())
        .next_back()
        .map(str::to_owned)
}
