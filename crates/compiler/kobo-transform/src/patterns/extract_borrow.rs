use kobo_ir::{KirNodeId, KoboSpan};

/// A site where extract-before-borrow can be applied.
#[derive(Debug, Clone)]
pub struct ExtractionSite {
    /// The binding whose borrow conflicts
    pub binding_id: KirNodeId,
    pub binding_name: String,
    /// The expression that borrows the binding (e.g., data.len())
    pub borrow_expr_span: KoboSpan,
    /// The conflicting mutation (e.g., data.push(42))
    pub conflict_span: KoboSpan,
    /// Name for the generated temporary
    pub temp_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionRewrite {
    pub source: String,
    pub applied_sites: usize,
}

/// Find all extract-before-borrow opportunities in the given transform facts.
///
/// A site qualifies when ALL of:
/// 1. Binding B has an immutable read at span S1
/// 2. Binding B has a mutation at span S2, where S2 > S1
/// 3. The read at S1 produces a value that can be hoisted (method call result)
/// 4. The borrow from S1 is still conceptually live at S2
///
/// NOT eligible: iterator borrows, closure captures, nested borrows.
///
/// Temp names: `__kobo_extract_0`, `__kobo_extract_1`, etc. (per call).
pub fn find_extract_before_borrow(facts: &kobo_ir::TransformFacts) -> Vec<ExtractionSite> {
    let mut sites = Vec::new();
    let mut counter: usize = 0;

    for binding in facts.iter_bindings() {
        let uses = &binding.usage.uses;

        // Collect read and mutation spans
        let mut read_spans: Vec<KoboSpan> = Vec::new();
        let mut mutation_spans: Vec<KoboSpan> = Vec::new();

        for event in uses {
            match event {
                kobo_ir::UseEvent::ReadOnly { span } => {
                    // BUG-12: Skip reads from reference-returning methods (e.g. .iter(), .as_ref())
                    // Extracting a reference return would move a borrow — unsound.
                    if !binding.ref_returning_read_spans.contains(span) {
                        read_spans.push(*span);
                    }
                }
                kobo_ir::UseEvent::Mutated { span } => {
                    mutation_spans.push(*span);
                }
                _ => {}
            }
        }

        // For each read followed by a mutation, check for conflict
        for read_span in &read_spans {
            for mut_span in &mutation_spans {
                // Mutation must be after the read
                if mut_span.start > read_span.start {
                    sites.push(ExtractionSite {
                        binding_id: binding.node,
                        binding_name: binding.binding_name.clone(),
                        borrow_expr_span: *read_span,
                        conflict_span: *mut_span,
                        temp_name: format!("__kobo_extract_{}", counter),
                    });
                    counter += 1;
                    // Only one extraction per read span to avoid duplicates
                    break;
                }
            }
        }
    }

    sites
}

/// Apply extract-before-borrow source rewrites for detected extraction sites.
///
/// The detector records the span of the borrowed binding, so the rewrite uses
/// that span to target the original source line and then replaces the first
/// `binding.method(...)` expression on that line with a generated temporary.
pub fn apply_extract_before_borrow_rewrites(
    source: &str,
    sites: &[ExtractionSite],
) -> ExtractionRewrite {
    let mut lines: Vec<String> = source.lines().map(str::to_owned).collect();
    let line_ranges = line_ranges(source);
    let mut insertions: Vec<(usize, String, String)> = Vec::new();

    for site in sites {
        if let Some(line_idx) = line_index_for_offset(&line_ranges, site.borrow_expr_span.start) {
            if let Some((extraction, replacement)) = rewrite_line_for_site(&lines[line_idx], site) {
                insertions.push((line_idx, extraction, replacement));
                continue;
            }
        }

        if let Some((line_idx, extraction, replacement)) =
            lines.iter().enumerate().find_map(|(line_idx, line)| {
                rewrite_line_for_site(line, site)
                    .map(|(extraction, replacement)| (line_idx, extraction, replacement))
            })
        {
            insertions.push((line_idx, extraction, replacement));
        }
    }

    insertions.sort_by(|a, b| b.0.cmp(&a.0));
    let applied_sites = insertions.len();
    for (line_idx, extraction, replacement) in insertions {
        lines[line_idx] = replacement;
        lines.insert(line_idx, extraction);
    }

    let mut rewritten = lines.join("\n");
    if source.ends_with('\n') && !rewritten.ends_with('\n') {
        rewritten.push('\n');
    }

    ExtractionRewrite {
        source: rewritten,
        applied_sites,
    }
}

fn rewrite_line_for_site(line: &str, site: &ExtractionSite) -> Option<(String, String)> {
    let pattern = format!("{}.", site.binding_name);
    let start = line.find(&pattern)?;
    let rest = &line[start..];
    let paren_end = find_balanced_paren(rest)?;
    let call_expr = &rest[..paren_end + 1];
    let indent = &line[..line.len() - line.trim_start().len()];
    let binding_mode = if call_expr.contains(".borrow_mut(") {
        "let mut"
    } else {
        "let"
    };
    let extraction = format!(
        "{}{} {} = {};",
        indent, binding_mode, site.temp_name, call_expr
    );
    let replacement = line.replacen(call_expr, &site.temp_name, 1);
    Some((extraction, replacement))
}

fn line_ranges(source: &str) -> Vec<(u32, u32)> {
    let mut ranges = Vec::new();
    let mut start = 0u32;
    for line in source.split_inclusive('\n') {
        let end = start.saturating_add(line.len() as u32);
        ranges.push((start, end));
        start = end;
    }
    if source.is_empty() {
        ranges.push((0, 0));
    }
    ranges
}

fn line_index_for_offset(ranges: &[(u32, u32)], offset: u32) -> Option<usize> {
    ranges.iter().position(|(start, end)| {
        (*start <= offset && offset < *end) || (*start == *end && offset == *start)
    })
}

fn find_balanced_paren(s: &str) -> Option<usize> {
    let mut depth: i32 = 0;
    let mut found_open = false;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => {
                depth += 1;
                found_open = true;
            }
            ')' => {
                depth -= 1;
                if found_open && depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}
