// Source map precision: LINE_LEVEL
// Column fields in RsSpan are line-boundary approximations (0..line_len).
// Reason: prettyplease::unparse returns a String with no token positions.
// Token-level precision requires post-format scanning; not implemented in v0.2.
// Upgrade path: replace line-level binding lookup with token scanning when it lands.

use std::path::Path;

use kobo_ir::{KoboSpan, OwnershipTier};
use serde::{Deserialize, Serialize};

use crate::lower::LoweringSite;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RsSpan {
    pub line: usize,
    pub column_start: usize,
    pub column_end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceMapEntry {
    pub rs_span: RsSpan,
    pub kobo_span: KoboSpan,
    pub ownership_tier: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KoboSourceMap {
    pub version: u32,
    pub file: String,
    pub sources: Vec<String>,
    #[serde(rename = "x_kobo_mappings")]
    pub x_kobo_mappings: Vec<SourceMapEntry>,
}

pub fn build_source_map_entries(formatted: &str, sites: &[LoweringSite]) -> Vec<SourceMapEntry> {
    let lines: Vec<&str> = formatted.lines().collect();
    let mut search_start = 0usize;
    let mut entries = Vec::with_capacity(sites.len());

    for site in sites {
        let line_index = find_binding_line(&lines, &site.binding_name, search_start)
            .or_else(|| find_binding_line(&lines, &site.binding_name, 0))
            .unwrap_or(0);
        let line_text = lines.get(line_index).copied().unwrap_or("");
        search_start = line_index.saturating_add(1);

        entries.push(SourceMapEntry {
            rs_span: RsSpan {
                line: line_index + 1,
                column_start: 0,
                column_end: line_text.len(),
            },
            kobo_span: site.kobo_span,
            ownership_tier: ownership_tier_label(site.ownership_tier).to_owned(),
        });
    }

    entries
}

pub fn wrap_source_map(
    kobo_path: &Path,
    rs_path: &Path,
    entries: Vec<SourceMapEntry>,
) -> KoboSourceMap {
    KoboSourceMap {
        version: 3,
        file: rs_path.display().to_string(),
        sources: vec![kobo_path.display().to_string()],
        x_kobo_mappings: entries,
    }
}

impl KoboSourceMap {
    pub fn generated_file(&self) -> &str {
        &self.file
    }

    pub fn kobo_path(&self) -> &str {
        self.sources
            .first()
            .map(String::as_str)
            .unwrap_or(self.file.as_str())
    }

    pub fn lookup_kobo_span(&self, rs_span: RsSpan) -> Option<KoboSpan> {
        self.x_kobo_mappings
            .iter()
            .find(|entry| {
                entry.rs_span.line == rs_span.line
                    && spans_overlap(&entry.rs_span, &rs_span)
            })
            .map(|entry| entry.kobo_span)
            .or_else(|| {
                self.x_kobo_mappings
                    .iter()
                    .find(|entry| entry.rs_span.line == rs_span.line)
                    .map(|entry| entry.kobo_span)
            })
    }

    pub fn lookup_rs_spans(&self, kobo_span: KoboSpan) -> Vec<RsSpan> {
        self.x_kobo_mappings
            .iter()
            .filter(|entry| entry.kobo_span.overlaps(kobo_span) || entry.kobo_span == kobo_span)
            .map(|entry| entry.rs_span.clone())
            .collect()
    }

    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

fn find_binding_line(lines: &[&str], binding_name: &str, start: usize) -> Option<usize> {
    lines
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(index, line)| contains_token(line, binding_name).then_some(index))
}

fn contains_token(line: &str, binding_name: &str) -> bool {
    let Some(start) = line.find(binding_name) else {
        return false;
    };
    let end = start + binding_name.len();
    let before = line[..start].chars().next_back();
    let after = line[end..].chars().next();

    !before.is_some_and(is_ident_char) && !after.is_some_and(is_ident_char)
}

fn is_ident_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn spans_overlap(left: &RsSpan, right: &RsSpan) -> bool {
    left.column_start <= right.column_end && right.column_start <= left.column_end
}

fn ownership_tier_label(tier: OwnershipTier) -> &'static str {
    match tier {
        OwnershipTier::PlainOwned => "plain",
        OwnershipTier::BoxOwned => "box",
        OwnershipTier::RcShared => "rc",
        OwnershipTier::ArcShared => "arc",
        OwnershipTier::RcMutShared => "rc_refcell",
        OwnershipTier::ArcMutShared => "arc_mutex",
        OwnershipTier::Scoped => "scoped_handle",
        OwnershipTier::Undecided => "plain",
    }
}

#[cfg(test)]
mod tests {
    use kobo_ir::{KoboSpan, OwnershipTier};

    use crate::lower::LoweringSite;

    use super::{build_source_map_entries, wrap_source_map, RsSpan};

    #[test]
    fn source_map_supports_forward_and_reverse_lookup() {
        let formatted = "fn main() {\n    let names = Rc::new(RefCell::new(vec![\"a\"]));\n}\n";
        let entries = build_source_map_entries(
            formatted,
            &[LoweringSite {
                binding_name: "names".to_owned(),
                ownership_tier: OwnershipTier::RcMutShared,
                kobo_span: KoboSpan::new(4, 9, kobo_ir::FileId(0)),
                kobo_line: 2,
                reason: "non-Copy shared binding".to_owned(),
            }],
        );
        let source_map = wrap_source_map("src/main.kobo".as_ref(), "src/main.rs".as_ref(), entries);
        assert_eq!(source_map.generated_file(), "src/main.rs");
        assert_eq!(source_map.kobo_path(), "src/main.kobo");

        let kobo_span = source_map
            .lookup_kobo_span(RsSpan {
                line: 2,
                column_start: 0,
                column_end: 47,
            })
            .expect("mapped line should resolve");
        assert_eq!(kobo_span, KoboSpan::new(4, 9, kobo_ir::FileId(0)));

        let rs_spans = source_map.lookup_rs_spans(KoboSpan::new(4, 9, kobo_ir::FileId(0)));
        assert_eq!(rs_spans.len(), 1);
        assert_eq!(rs_spans[0].line, 2);
    }
}
