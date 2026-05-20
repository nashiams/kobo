use kobo_ir::{FileId, KoboSpan};

use super::source_map::{push_identity_segment, PreprocessSourceMap, PreprocessedSource};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WardSyntaxModel {
    pub wards: Vec<WardBlock>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WardBlock {
    pub name: String,
    pub span: KoboSpan,
    pub name_span: KoboSpan,
    pub items: Vec<WardItem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WardItem {
    State(WardState),
    Obligation(WardObligation),
    Invariant(WardNamedBlock),
    Temporal(WardNamedBlock),
    Port(WardLineFact),
    Recording(WardLineFact),
    Debt(WardLineFact),
    Scenario(WardScenario),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WardState {
    pub name: String,
    pub ty: String,
    pub span: KoboSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WardObligation {
    pub type_name: String,
    pub actions: Vec<String>,
    pub span: KoboSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WardNamedBlock {
    pub name: String,
    pub body: String,
    pub span: KoboSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WardLineFact {
    pub text: String,
    pub span: KoboSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WardScenario {
    pub name: String,
    pub profile: WardScenarioProfile,
    pub body: String,
    pub span: KoboSpan,
    pub body_span: KoboSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WardScenarioProfile {
    Sync,
    Async,
    Network,
    Distributed,
    Custom(String),
}

impl WardScenarioProfile {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Sync => "sync",
            Self::Async => "async",
            Self::Network => "network",
            Self::Distributed => "distributed",
            Self::Custom(profile) => profile.as_str(),
        }
    }

    fn from_str(profile: &str) -> Self {
        match profile {
            "sync" => Self::Sync,
            "async" => Self::Async,
            "network" => Self::Network,
            "distributed" => Self::Distributed,
            other => Self::Custom(other.to_owned()),
        }
    }
}

pub fn preprocess_ward_syntax_mapped(
    source: &str,
    file_id: FileId,
) -> PreprocessedSource<WardSyntaxModel> {
    let model = parse_ward_syntax(source, file_id);
    if model.wards.is_empty() {
        return PreprocessedSource {
            rewritten: source.to_owned(),
            source_map: PreprocessSourceMap::identity_for(source, file_id),
            metadata: model,
        };
    }

    let mut rewritten = String::with_capacity(source.len() + 256);
    let mut source_map = PreprocessSourceMap::default();
    let mut cursor = 0usize;
    for ward in &model.wards {
        let ward_start = ward.span.start as usize;
        push_identity_segment(
            &mut source_map,
            file_id,
            rewritten.len(),
            rewritten.len() + ward_start.saturating_sub(cursor),
            cursor,
            ward_start,
        );
        rewritten.push_str(&source[cursor..ward_start]);
        push_ward_attribute_source(&mut rewritten, &mut source_map, file_id, ward);
        cursor = ward.span.end as usize;
    }
    push_identity_segment(
        &mut source_map,
        file_id,
        rewritten.len(),
        rewritten.len() + source.len().saturating_sub(cursor),
        cursor,
        source.len(),
    );
    rewritten.push_str(&source[cursor..]);

    PreprocessedSource {
        rewritten,
        source_map,
        metadata: model,
    }
}

pub fn parse_ward_syntax(source: &str, file_id: FileId) -> WardSyntaxModel {
    let cleaned = scrub_comments_and_strings(source);
    let mut wards = Vec::new();
    let mut cursor = 0usize;
    while let Some(ward_start) = find_word(&cleaned, cursor, "ward") {
        let Some(ward) = parse_ward_block(source, &cleaned, file_id, ward_start) else {
            cursor = ward_start + "ward".len();
            continue;
        };
        cursor = ward.span.end as usize;
        wards.push(ward);
    }
    WardSyntaxModel { wards }
}

fn parse_ward_block(
    source: &str,
    cleaned: &[u8],
    file_id: FileId,
    start: usize,
) -> Option<WardBlock> {
    let name_start = skip_ascii_whitespace(cleaned, start + "ward".len(), cleaned.len());
    let name_end = ident_end(cleaned, name_start);
    if name_start == name_end {
        return None;
    }
    let brace_start = find_bytes(&cleaned[name_end..], b"{")? + name_end;
    let brace_end = matching_brace_in_bytes(cleaned, brace_start)?;
    let items = parse_ward_items(source, cleaned, file_id, brace_start + 1, brace_end);
    Some(WardBlock {
        name: source[name_start..name_end].to_owned(),
        span: KoboSpan::new(start as u32, (brace_end + 1) as u32, file_id),
        name_span: KoboSpan::new(name_start as u32, name_end as u32, file_id),
        items,
    })
}

fn parse_ward_items(
    source: &str,
    cleaned: &[u8],
    file_id: FileId,
    body_start: usize,
    body_end: usize,
) -> Vec<WardItem> {
    let mut items = Vec::new();
    let mut cursor = body_start;
    while cursor < body_end {
        cursor = skip_ascii_whitespace(cleaned, cursor, body_end);
        if cursor >= body_end {
            break;
        }

        if starts_with_word(cleaned, cursor, "state") {
            if let Some((state, next)) = parse_state(source, cleaned, file_id, cursor, body_end) {
                items.push(WardItem::State(state));
                cursor = next;
                continue;
            }
        } else if starts_with_word(cleaned, cursor, "obligation") {
            if let Some((obligation, next)) =
                parse_obligation(source, cleaned, file_id, cursor, body_end)
            {
                items.push(WardItem::Obligation(obligation));
                cursor = next;
                continue;
            }
        } else if starts_with_word(cleaned, cursor, "invariant") {
            if let Some((block, next)) =
                parse_named_block(source, cleaned, file_id, cursor, body_end, "invariant")
            {
                items.push(WardItem::Invariant(block));
                cursor = next;
                continue;
            }
        } else if starts_with_word(cleaned, cursor, "temporal") {
            if let Some((block, next)) =
                parse_named_block(source, cleaned, file_id, cursor, body_end, "temporal")
            {
                items.push(WardItem::Temporal(block));
                cursor = next;
                continue;
            }
            if let Some((block, next)) =
                parse_temporal_line(source, cleaned, file_id, cursor, body_end)
            {
                items.push(WardItem::Temporal(block));
                cursor = next;
                continue;
            }
        } else if starts_with_word(cleaned, cursor, "port") {
            if let Some((fact, next)) =
                parse_line_fact(source, cleaned, file_id, cursor, body_end, "port")
            {
                items.push(WardItem::Port(fact));
                cursor = next;
                continue;
            }
        } else if starts_with_word(cleaned, cursor, "recording") {
            if let Some((fact, next)) =
                parse_line_fact(source, cleaned, file_id, cursor, body_end, "recording")
            {
                items.push(WardItem::Recording(fact));
                cursor = next;
                continue;
            }
        } else if starts_with_word(cleaned, cursor, "debt") {
            if let Some((fact, next)) =
                parse_line_fact(source, cleaned, file_id, cursor, body_end, "debt")
            {
                items.push(WardItem::Debt(fact));
                cursor = next;
                continue;
            }
        } else if starts_with_word(cleaned, cursor, "scenario") {
            if let Some((scenario, next)) =
                parse_scenario(source, cleaned, file_id, cursor, body_end)
            {
                items.push(WardItem::Scenario(scenario));
                cursor = next;
                continue;
            }
        }

        cursor = next_statement_start(cleaned, cursor, body_end);
    }
    items
}

fn parse_state(
    source: &str,
    cleaned: &[u8],
    file_id: FileId,
    start: usize,
    body_end: usize,
) -> Option<(WardState, usize)> {
    let (text, span, next) = line_fact_text(source, cleaned, file_id, start, body_end, "state")?;
    let (name, ty) = text.split_once(':')?;
    let name = name.trim();
    let ty = ty.trim();
    if name.is_empty() || ty.is_empty() {
        return None;
    }
    Some((
        WardState {
            name: name.to_owned(),
            ty: ty.to_owned(),
            span,
        },
        next,
    ))
}

fn parse_obligation(
    source: &str,
    cleaned: &[u8],
    file_id: FileId,
    start: usize,
    body_end: usize,
) -> Option<(WardObligation, usize)> {
    let (text, span, next) =
        line_fact_text(source, cleaned, file_id, start, body_end, "obligation")?;
    let (type_name, actions) = text.split_once(" must ")?;
    let actions = actions
        .split('|')
        .map(str::trim)
        .filter(|action| !action.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if type_name.trim().is_empty() || actions.is_empty() {
        return None;
    }
    Some((
        WardObligation {
            type_name: type_name.trim().to_owned(),
            actions,
            span,
        },
        next,
    ))
}

fn parse_line_fact(
    source: &str,
    cleaned: &[u8],
    file_id: FileId,
    start: usize,
    body_end: usize,
    keyword: &str,
) -> Option<(WardLineFact, usize)> {
    let (text, span, next) = line_fact_text(source, cleaned, file_id, start, body_end, keyword)?;
    (!text.is_empty()).then_some((WardLineFact { text, span }, next))
}

fn line_fact_text(
    source: &str,
    cleaned: &[u8],
    file_id: FileId,
    start: usize,
    body_end: usize,
    keyword: &str,
) -> Option<(String, KoboSpan, usize)> {
    let text_start = skip_ascii_whitespace(cleaned, start + keyword.len(), body_end);
    let text_end = statement_end(cleaned, text_start, body_end);
    let span_end = text_end.max(text_start);
    let text = source[text_start..span_end]
        .trim()
        .trim_end_matches(';')
        .trim();
    let next = (span_end + 1).min(body_end);
    Some((
        text.to_owned(),
        KoboSpan::new(start as u32, span_end as u32, file_id),
        next,
    ))
}

fn parse_named_block(
    source: &str,
    cleaned: &[u8],
    file_id: FileId,
    start: usize,
    body_end: usize,
    keyword: &str,
) -> Option<(WardNamedBlock, usize)> {
    let header_start = skip_ascii_whitespace(cleaned, start + keyword.len(), body_end);
    let header_end = statement_end(cleaned, header_start, body_end);
    let brace_start = find_bytes(&cleaned[header_start..header_end], b"{")? + header_start;
    let brace_end = matching_brace_in_bytes(cleaned, brace_start)?;
    let header = source[header_start..brace_start].trim();
    let name = ident_prefix(header)?;
    Some((
        WardNamedBlock {
            name,
            body: source[brace_start + 1..brace_end].trim().to_owned(),
            span: KoboSpan::new(start as u32, (brace_end + 1) as u32, file_id),
        },
        brace_end + 1,
    ))
}

fn parse_temporal_line(
    source: &str,
    cleaned: &[u8],
    file_id: FileId,
    start: usize,
    body_end: usize,
) -> Option<(WardNamedBlock, usize)> {
    let (text, span, next) = line_fact_text(source, cleaned, file_id, start, body_end, "temporal")?;
    (!text.is_empty()).then_some((
        WardNamedBlock {
            name: String::new(),
            body: text,
            span,
        },
        next,
    ))
}

fn parse_scenario(
    source: &str,
    cleaned: &[u8],
    file_id: FileId,
    start: usize,
    body_end: usize,
) -> Option<(WardScenario, usize)> {
    let header_start = skip_ascii_whitespace(cleaned, start + "scenario".len(), body_end);
    let brace_start = find_bytes(&cleaned[header_start..body_end], b"{")? + header_start;
    let brace_end = matching_brace_in_bytes(cleaned, brace_start)?;
    let header = source[header_start..brace_start].trim();
    let (name, profile) = scenario_header(header)?;
    Some((
        WardScenario {
            name,
            profile,
            body: source[brace_start + 1..brace_end]
                .trim_matches('\n')
                .to_owned(),
            span: KoboSpan::new(start as u32, (brace_end + 1) as u32, file_id),
            body_span: KoboSpan::new((brace_start + 1) as u32, brace_end as u32, file_id),
        },
        brace_end + 1,
    ))
}

fn scenario_header(header: &str) -> Option<(String, WardScenarioProfile)> {
    let name = ident_prefix(header)?;
    let rest = header[name.len()..].trim();
    let profile = scenario_profile_from_header(rest).unwrap_or(WardScenarioProfile::Sync);
    Some((name, profile))
}

fn scenario_profile_from_header(rest: &str) -> Option<WardScenarioProfile> {
    if let Some(profile) = rest.strip_prefix("profile") {
        return normalized_profile(profile).map(WardScenarioProfile::from_str);
    }
    if rest.starts_with('(') {
        let profile_start = rest.find("profile")?;
        return normalized_profile(&rest[profile_start + "profile".len()..])
            .map(WardScenarioProfile::from_str);
    }
    None
}

fn normalized_profile(value: &str) -> Option<&str> {
    let value = value
        .trim()
        .trim_start_matches('=')
        .trim()
        .trim_matches(|ch| ch == '"' || ch == '\'' || ch == ')' || ch == ',');
    (!value.is_empty()).then_some(value)
}

fn push_ward_attribute_source(
    output: &mut String,
    source_map: &mut PreprocessSourceMap,
    file_id: FileId,
    ward: &WardBlock,
) {
    push_generated(output, source_map, file_id, "#[kobo::ward]\n", ward.span);
    push_generated(
        output,
        source_map,
        file_id,
        &format!("struct {} {{\n", ward.name),
        ward.name_span,
    );
    for item in &ward.items {
        if let WardItem::State(state) = item {
            push_generated(
                output,
                source_map,
                file_id,
                &format!("    {}: {},\n", state.name, state.ty),
                state.span,
            );
        }
    }
    push_generated(output, source_map, file_id, "}\n\n", ward.span);
    for item in &ward.items {
        push_ward_item_attribute_source(output, source_map, file_id, item);
    }
}

fn push_ward_item_attribute_source(
    output: &mut String,
    source_map: &mut PreprocessSourceMap,
    file_id: FileId,
    item: &WardItem,
) {
    match item {
        WardItem::State(_) => {}
        WardItem::Obligation(obligation) => {
            push_obligation_attribute_source(output, source_map, file_id, obligation)
        }
        WardItem::Invariant(block) => push_generated(
            output,
            source_map,
            file_id,
            &format!("// kobo: invariant {} {{ {} }}\n", block.name, block.body),
            block.span,
        ),
        WardItem::Temporal(block) => push_generated(
            output,
            source_map,
            file_id,
            &temporal_metadata_comment(block),
            block.span,
        ),
        WardItem::Port(fact) => push_generated(
            output,
            source_map,
            file_id,
            &format!("// kobo: port {}\n", fact.text),
            fact.span,
        ),
        WardItem::Recording(fact) => push_generated(
            output,
            source_map,
            file_id,
            &format!("// kobo: recording {}\n", fact.text),
            fact.span,
        ),
        WardItem::Debt(fact) => push_generated(
            output,
            source_map,
            file_id,
            &format!("// kobo: debt {}\n", fact.text),
            fact.span,
        ),
        WardItem::Scenario(scenario) => {
            push_scenario_attribute_source(output, source_map, file_id, scenario)
        }
    }
}

fn temporal_metadata_comment(block: &WardNamedBlock) -> String {
    if block.name.is_empty() {
        format!("// kobo: temporal {}\n", block.body)
    } else {
        format!("// kobo: temporal {} {{ {} }}\n", block.name, block.body)
    }
}

fn push_obligation_attribute_source(
    output: &mut String,
    source_map: &mut PreprocessSourceMap,
    file_id: FileId,
    obligation: &WardObligation,
) {
    push_generated(
        output,
        source_map,
        file_id,
        &format!(
            "#[kobo::must_call({})]\nstruct {} {{}}\n\n",
            obligation.actions.join(" | "),
            obligation.type_name
        ),
        obligation.span,
    );
    push_generated(
        output,
        source_map,
        file_id,
        &format!("impl {} {{\n", obligation.type_name),
        obligation.span,
    );
    for action in &obligation.actions {
        push_generated(
            output,
            source_map,
            file_id,
            &format!("    fn {action}(self) {{}}\n"),
            obligation.span,
        );
    }
    push_generated(output, source_map, file_id, "}\n\n", obligation.span);
}

fn push_scenario_attribute_source(
    output: &mut String,
    source_map: &mut PreprocessSourceMap,
    file_id: FileId,
    scenario: &WardScenario,
) {
    push_generated(
        output,
        source_map,
        file_id,
        &format!(
            "#[kobo::scenario(profile = \"{}\")]\n",
            scenario.profile.as_str()
        ),
        scenario.span,
    );
    push_generated(
        output,
        source_map,
        file_id,
        &format!("fn {}() {{\n{}\n}}\n\n", scenario.name, scenario.body),
        scenario.span,
    );
}

fn push_generated(
    output: &mut String,
    source_map: &mut PreprocessSourceMap,
    file_id: FileId,
    text: &str,
    original: KoboSpan,
) {
    let generated_start = output.len();
    output.push_str(text);
    source_map.push_segment(
        KoboSpan::new(generated_start as u32, output.len() as u32, file_id),
        original,
    );
}

fn next_statement_start(bytes: &[u8], start: usize, limit: usize) -> usize {
    let mut cursor = start + 1;
    while cursor < limit && bytes[cursor] != b'\n' && bytes[cursor] != b'}' {
        cursor += 1;
    }
    (cursor + 1).min(limit)
}

fn statement_end(bytes: &[u8], start: usize, limit: usize) -> usize {
    let mut cursor = start;
    while cursor < limit && bytes[cursor] != b'\n' && bytes[cursor] != b';' {
        cursor += 1;
    }
    cursor
}

fn skip_ascii_whitespace(bytes: &[u8], mut cursor: usize, limit: usize) -> usize {
    while cursor < limit && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    cursor
}

fn ident_prefix(input: &str) -> Option<String> {
    let ident = input
        .trim_start()
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect::<String>();
    (!ident.is_empty()).then_some(ident)
}

fn ident_end(bytes: &[u8], mut cursor: usize) -> usize {
    while bytes.get(cursor).is_some_and(is_ident_byte) {
        cursor += 1;
    }
    cursor
}

fn starts_with_word(bytes: &[u8], start: usize, word: &str) -> bool {
    let word_bytes = word.as_bytes();
    if bytes.get(start..start + word_bytes.len()) != Some(word_bytes) {
        return false;
    }
    let before = start.checked_sub(1).and_then(|index| bytes.get(index));
    let after = bytes.get(start + word_bytes.len());
    !before.is_some_and(is_ident_byte) && !after.is_some_and(is_ident_byte)
}

fn find_word(bytes: &[u8], from: usize, word: &str) -> Option<usize> {
    let mut cursor = from;
    while cursor + word.len() <= bytes.len() {
        let Some(relative) = find_bytes(&bytes[cursor..], word.as_bytes()) else {
            return None;
        };
        let start = cursor + relative;
        if starts_with_word(bytes, start, word) {
            return Some(start);
        }
        cursor = start + word.len();
    }
    None
}

fn matching_brace_in_bytes(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0_usize;
    for (index, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn scrub_comments_and_strings(source: &str) -> Vec<u8> {
    let bytes = source.as_bytes();
    let mut cleaned = bytes.to_vec();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                let start = index;
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
                blank_non_newlines(&mut cleaned, start, index);
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                let start = index;
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
                blank_non_newlines(&mut cleaned, start, index);
            }
            b'"' => {
                let start = index;
                index += 1;
                while index < bytes.len() {
                    if bytes[index] == b'\\' {
                        index = (index + 2).min(bytes.len());
                    } else if bytes[index] == b'"' {
                        index += 1;
                        break;
                    } else {
                        index += 1;
                    }
                }
                blank_non_newlines(&mut cleaned, start, index);
            }
            _ => index += 1,
        }
    }
    cleaned
}

fn blank_non_newlines(bytes: &mut [u8], start: usize, end: usize) {
    let bounded_end = end.min(bytes.len());
    for byte in &mut bytes[start..bounded_end] {
        if *byte != b'\n' {
            *byte = b' ';
        }
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len())
        .find(|start| &haystack[*start..*start + needle.len()] == needle)
}

fn is_ident_byte(byte: &u8) -> bool {
    byte.is_ascii_alphanumeric() || *byte == b'_'
}

#[cfg(test)]
mod tests {
    use kobo_ir::FileId;

    use super::{parse_ward_syntax, WardItem, WardScenarioProfile};

    #[test]
    fn parser_preserves_scenario_profile() {
        let model = parse_ward_syntax(
            r#"
ward Gateway {
    scenario handle profile async {
        let token = Token {};
    }
}
"#,
            FileId(0),
        );
        let scenario = model.wards[0]
            .items
            .iter()
            .find_map(|item| match item {
                WardItem::Scenario(scenario) => Some(scenario),
                _ => None,
            })
            .expect("scenario should parse");
        assert_eq!(scenario.profile, WardScenarioProfile::Async);
    }

    #[test]
    fn parser_accepts_ward_at_file_start() {
        let model = parse_ward_syntax(
            "ward DurableQueue {\n    port storage: durable_log\n}\n",
            FileId(0),
        );
        assert_eq!(model.wards.len(), 1);
        assert_eq!(model.wards[0].name, "DurableQueue");
    }

    #[test]
    fn parser_preserves_line_temporal_checks() {
        let model = parse_ward_syntax(
            "ward TraceWard {\n    temporal always deterministic-task\n\n    scenario trace_case {\n        ward.task();\n    }\n}\n",
            FileId(0),
        );
        let temporal = model.wards[0]
            .items
            .iter()
            .find_map(|item| match item {
                WardItem::Temporal(block) => Some(block),
                _ => None,
            })
            .expect("temporal check should parse");
        assert_eq!(temporal.body, "always deterministic-task");
        assert!(model.wards[0].items.iter().any(
            |item| matches!(item, WardItem::Scenario(scenario) if scenario.name == "trace_case")
        ));
    }
}
