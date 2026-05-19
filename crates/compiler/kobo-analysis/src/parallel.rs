#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParallelWarning {
    pub kind: ParallelWarningKind,
    pub source_offset: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParallelWarningKind {
    NonSendCapture {
        binding_name: String,
        type_name: String,
    },
    SharedMutation {
        binding_name: String,
    },
    MissingBoundaryPolicy,
}

#[derive(Clone, Debug)]
struct BindingFact {
    name: String,
    type_name: Option<String>,
    is_mutable: bool,
}

#[derive(Clone, Debug)]
struct LineInfo<'a> {
    text: &'a str,
    offset: usize,
}

pub fn scan_source_parallel_warnings(source: &str) -> Vec<ParallelWarning> {
    let lines = line_infos(source);
    let mut warnings = Vec::new();
    let mut bindings = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.text.trim();
        if let Some(binding) = parse_binding(trimmed) {
            bindings.push(binding);
        }
        if !trimmed.contains("#[kobo::parallel") {
            continue;
        }
        if parallel_attr_has_key(trimmed, "order") {
            continue;
        }
        let has_policy = parallel_attr_has_key(trimmed, "policy");
        let Some(loop_extent) = loop_extent(&lines, index + 1) else {
            continue;
        };
        warnings.extend(loop_warnings(source, &bindings, has_policy, loop_extent));
    }

    warnings
}

fn loop_warnings(
    source: &str,
    bindings: &[BindingFact],
    has_policy: bool,
    extent: (usize, usize),
) -> Vec<ParallelWarning> {
    let loop_source = &source[extent.0..extent.1.min(source.len())];
    let mut warnings = Vec::new();
    if !has_policy && loop_source.contains("ward.") {
        let ward_offset = extent.0 + loop_source.find("ward.").unwrap_or(0);
        warnings.push(ParallelWarning {
            kind: ParallelWarningKind::MissingBoundaryPolicy,
            source_offset: ward_offset,
        });
    }

    for binding in bindings {
        if binding
            .type_name
            .as_deref()
            .is_some_and(|type_name| type_name == "Rc")
            && contains_ident(loop_source, &binding.name)
        {
            warnings.push(ParallelWarning {
                kind: ParallelWarningKind::NonSendCapture {
                    binding_name: binding.name.clone(),
                    type_name: "Rc".to_owned(),
                },
                source_offset: extent.0 + ident_offset(loop_source, &binding.name).unwrap_or(0),
            });
        }
        if binding.is_mutable {
            if let Some(offset) = mutation_offset(loop_source, &binding.name) {
                warnings.push(ParallelWarning {
                    kind: ParallelWarningKind::SharedMutation {
                        binding_name: binding.name.clone(),
                    },
                    source_offset: extent.0 + offset,
                });
            }
        }
    }
    warnings
}

fn line_infos(source: &str) -> Vec<LineInfo<'_>> {
    let mut offset = 0usize;
    source
        .split_inclusive('\n')
        .map(|line| {
            let info = LineInfo { text: line, offset };
            offset += line.len();
            info
        })
        .collect()
}

fn parse_binding(line: &str) -> Option<BindingFact> {
    let rest = line.strip_prefix("let ")?;
    let (is_mutable, rest) = match rest.strip_prefix("mut ") {
        Some(rest) => (true, rest),
        None => (false, rest),
    };
    let name = rest
        .split([':', '=', ' '])
        .next()
        .filter(|name| !name.is_empty())?
        .to_owned();
    let type_name = if rest.contains("Rc::new") {
        Some("Rc".to_owned())
    } else if rest.contains("Arc::new") {
        Some("Arc".to_owned())
    } else {
        None
    };
    Some(BindingFact {
        name,
        type_name,
        is_mutable,
    })
}

fn parallel_attr_has_key(line: &str, key: &str) -> bool {
    line.contains(&format!("{key} =")) || line.contains(&format!("{key}="))
}

fn loop_extent(lines: &[LineInfo<'_>], start_index: usize) -> Option<(usize, usize)> {
    let mut start = None;
    let mut depth = 0isize;
    for line in &lines[start_index..] {
        if start.is_none() && line.text.contains("for ") {
            start = Some(line.offset);
        }
        if start.is_none() {
            continue;
        }
        depth += line.text.matches('{').count() as isize;
        depth -= line.text.matches('}').count() as isize;
        if depth <= 0 && line.text.contains('}') {
            return Some((start?, line.offset + line.text.len()));
        }
    }
    start.map(|start| {
        (
            start,
            lines
                .last()
                .map(|line| line.offset + line.text.len())
                .unwrap_or(start),
        )
    })
}

fn mutation_offset(source: &str, binding: &str) -> Option<usize> {
    [".push(", ".insert(", ".extend(", ".remove("]
        .iter()
        .filter_map(|method| source.find(&format!("{binding}{method}")))
        .min()
}

fn contains_ident(source: &str, ident: &str) -> bool {
    ident_offset(source, ident).is_some()
}

fn ident_offset(source: &str, ident: &str) -> Option<usize> {
    source.match_indices(ident).find_map(|(index, _)| {
        let before = source[..index].chars().next_back();
        let after = source[index + ident.len()..].chars().next();
        (!before.is_some_and(is_ident_char) && !after.is_some_and(is_ident_char)).then_some(index)
    })
}

fn is_ident_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}
