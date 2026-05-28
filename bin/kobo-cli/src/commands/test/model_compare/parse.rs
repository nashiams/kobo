use super::super::trace_checks::{
    find_word, matching_brace, scrub_comments_and_strings, ward_blocks, WardBlock,
};
use super::{
    ModelBoundaryCall, ModelComparisonSpec, ModelEventExpectation, ModelObligationExpectation,
    WardModelStep, WardModelStepKind,
};

pub(crate) fn parse_model_comparison_spec(source: &str) -> ModelComparisonSpec {
    let mut spec = ModelComparisonSpec {
        steps: Vec::new(),
        events: Vec::new(),
        obligations: Vec::new(),
        source: "not_requested",
    };
    parse_ward_model_specs(source, &mut spec);
    parse_legacy_model_directives(source, &mut spec);
    spec
}

fn parse_ward_model_specs(source: &str, spec: &mut ModelComparisonSpec) {
    for block in ward_blocks(source) {
        parse_structured_model_blocks(&block, spec);
        let mut offset = block.body_start;
        for raw_line in block.body.split_inclusive('\n') {
            let line = raw_line.trim_end_matches(['\r', '\n']);
            let trimmed = line.trim();
            let leading = line.find(trimmed).unwrap_or(0);
            let span = (offset + leading, offset + line.len());
            if let Some(rest) = trimmed.strip_prefix("model ") {
                spec.source = "ward_model";
                parse_model_directive(rest, span, spec);
            }
            offset += raw_line.len();
        }
    }
}

fn parse_structured_model_blocks(block: &WardBlock<'_>, spec: &mut ModelComparisonSpec) {
    let cleaned = scrub_comments_and_strings(block.body);
    let mut cursor = 0;
    while let Some(model_start) = find_word(&cleaned, cursor, "model") {
        let after_model = model_start + "model".len();
        let Some(open) = model_block_open(&cleaned, after_model) else {
            cursor = after_model;
            continue;
        };
        let Some(close) = matching_brace(&cleaned, open) else {
            break;
        };
        spec.source = "ward_model";
        let model_body = &block.body[open + 1..close];
        parse_structured_model_statements(model_body, block.body_start + open + 1, spec);
        cursor = close + 1;
    }
}

fn model_block_open(bytes: &[u8], from: usize) -> Option<usize> {
    let mut cursor = from;
    while let Some(byte) = bytes.get(cursor) {
        if *byte == b'{' {
            return Some(cursor);
        }
        if !byte.is_ascii_whitespace() {
            return None;
        }
        cursor += 1;
    }
    None
}

fn parse_structured_model_statements(
    model_body: &str,
    body_start: usize,
    spec: &mut ModelComparisonSpec,
) {
    let mut statement_start = 0usize;
    for statement in model_body.split_terminator(';') {
        let leading = statement
            .find(|ch: char| !ch.is_ascii_whitespace())
            .unwrap_or(0);
        let trimmed = statement.trim();
        let span = (
            body_start + statement_start + leading,
            body_start + statement_start + statement.len(),
        );
        parse_model_directive(trimmed, span, spec);
        statement_start += statement.len() + 1;
    }
}

fn parse_legacy_model_directives(source: &str, spec: &mut ModelComparisonSpec) {
    let mut offset = 0;
    for raw_line in source.split_inclusive('\n') {
        let line = raw_line.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim();
        let leading = line.find(trimmed).unwrap_or(0);
        let span = (offset + leading, offset + line.len());
        if let Some(rest) = model_directive(trimmed) {
            if spec.source == "not_requested" {
                spec.source = "legacy_directive";
            }
            parse_model_directive(rest, span, spec);
        }
        offset += raw_line.len();
    }
}

fn model_directive(line: &str) -> Option<&str> {
    line.strip_prefix("// kobo:model ")
}

fn parse_model_directive(rest: &str, span: (usize, usize), spec: &mut ModelComparisonSpec) {
    if let Some(event) = rest.strip_prefix("event ") {
        let event = event.trim().trim_matches([';', '}']);
        if !event.is_empty() {
            spec.steps.push(WardModelStep {
                kind: WardModelStepKind::EmitEvent(event.to_owned()),
                span,
            });
            spec.events.push(ModelEventExpectation { span });
        }
    } else if let Some(obligation) = rest.strip_prefix("obligation ") {
        let mut parts = obligation.split_whitespace();
        let Some(binding) = parts.next() else {
            return;
        };
        let Some(state) = parts.next() else {
            return;
        };
        if matches!(state, "discharged" | "leaked") {
            spec.steps.push(WardModelStep {
                kind: WardModelStepKind::SetObligation {
                    binding: binding.to_owned(),
                    state: state.to_owned(),
                },
                span,
            });
            spec.obligations.push(ModelObligationExpectation {
                binding: binding.to_owned(),
                span,
            });
        }
    } else if let Some((name, value)) = parse_model_state(rest) {
        spec.steps.push(WardModelStep {
            kind: WardModelStepKind::SetState { name, value },
            span,
        });
    } else if let Some(preset) = parse_model_scheduler(rest) {
        spec.steps.push(WardModelStep {
            kind: WardModelStepKind::SchedulerAssumption { preset },
            span,
        });
    } else if let Some((name, boundary)) = parse_model_transition(rest) {
        spec.steps.push(WardModelStep {
            kind: WardModelStepKind::Transition { name, boundary },
            span,
        });
    } else if let Some(boundary) = parse_model_boundary_call(rest) {
        spec.steps.push(WardModelStep {
            kind: WardModelStepKind::BoundaryCall(boundary),
            span,
        });
    }
}

fn parse_model_state(rest: &str) -> Option<(String, String)> {
    let state = rest.strip_prefix("state ")?;
    let state = state.trim().trim_matches([';', '}']).trim();
    if let Some((name, value)) = state.split_once('=') {
        let name = name.trim();
        let value = value.trim().trim_matches('"');
        if !name.is_empty() && !value.is_empty() {
            return Some((name.to_owned(), value.to_owned()));
        }
    }
    let mut parts = state.split_whitespace();
    let name = parts.next()?;
    let value = parts.next()?;
    Some((name.to_owned(), value.to_owned()))
}

fn parse_model_scheduler(rest: &str) -> Option<String> {
    let preset = rest
        .strip_prefix("scheduler ")?
        .trim()
        .trim_matches([';', '}'])
        .trim();
    (!preset.is_empty()).then(|| preset.to_owned())
}

fn parse_model_transition(rest: &str) -> Option<(String, ModelBoundaryCall)> {
    let transition = rest.strip_prefix("transition ")?;
    let transition = transition.trim().trim_matches([';', '}']).trim();
    let (name, target) = transition.split_once("->")?;
    let name = name.trim();
    let boundary = parse_model_boundary_call(target.trim())?;
    (!name.is_empty()).then(|| (name.to_owned(), boundary))
}

fn parse_model_boundary_call(rest: &str) -> Option<ModelBoundaryCall> {
    let mut normalized = rest.trim().trim_matches([';', '}']).trim();
    normalized = normalized.strip_suffix("()").unwrap_or(normalized).trim();
    match normalized {
        "ward.time" => Some(ModelBoundaryCall::WardTime),
        "ward.random" => Some(ModelBoundaryCall::WardRandom),
        "ward.task" => Some(ModelBoundaryCall::WardTask),
        "ward.task.local" => Some(ModelBoundaryCall::WardTaskLocal),
        "ward.storage" => Some(ModelBoundaryCall::WardStorage),
        "ward.network" => Some(ModelBoundaryCall::WardNetwork),
        _ => None,
    }
}
