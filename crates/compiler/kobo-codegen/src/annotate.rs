use crate::lower::LoweringSite;
use crate::sourcemap::SourceMapEntry;

pub fn annotate(formatted: &str, sites: &[LoweringSite], entries: &mut [SourceMapEntry]) -> String {
    debug_assert_eq!(sites.len(), entries.len());

    let had_trailing_newline = formatted.ends_with('\n');
    let mut lines: Vec<String> = formatted.lines().map(str::to_owned).collect();
    let mut inserted_before = 0usize;

    for (site, entry) in sites.iter().zip(entries.iter_mut()) {
        let insertion_index = entry
            .rs_span
            .line
            .saturating_sub(1)
            .saturating_add(inserted_before)
            .min(lines.len());
        lines.insert(
            insertion_index,
            format!(
                "// kobo: {} @ line {} -> {} ({})",
                site.binding_name, site.kobo_line, entry.ownership_tier, site.reason
            ),
        );
        entry.rs_span.line += inserted_before + 1;
        inserted_before += 1;
    }

    let mut annotated = lines.join("\n");
    if had_trailing_newline || !annotated.is_empty() {
        annotated.push('\n');
    }

    annotated
}
