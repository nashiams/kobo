use std::path::Path;

use kobo_errors::{
    DiagLabel, DiagnosticRelatedInfo, DiagnosticSuggestion, KDiagnostic, KErrorCode, Severity,
    SuggestionApplicability, TextEdit,
};
use kobo_ir::{FieldCapabilityField, FieldCapabilityView, FileId, Kir, KoboSpan};
use kobo_parser::{
    collect_strict_items_from_syn,
    mode_parse::{parse_legacy_mode_directive, LegacyModeDirective},
    parse_file_recovering, postprocess_strict_markers, preprocess_bridge_blocks_mapped,
    preprocess_concurrent_sugar_mapped, preprocess_kobo_keywords_mapped,
    preprocess_spawn_blocks_mapped, preprocess_strict_reject_invalid, v05_keyword_configs,
    KoboFile, PreprocessSourceMap, PreprocessedSource, RecoveryMode,
};
use kobo_transform::{build_kir, TransformOptions};

use crate::filesystem::read_kobo_file;
use crate::session::CompileSession;

pub fn run_kir_phase(session: &mut CompileSession, input: &Path) -> Result<(KoboFile, Kir), ()> {
    let source = match read_kobo_file(input) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("kobo: io error: {error}");
            return Err(());
        }
    };
    let file_id = session.register_source_file(input.to_path_buf(), source.clone());
    let field_capability_views = collect_field_capability_views(&source, file_id);

    match parse_legacy_mode_directive(&source) {
        Ok(Some(directive)) => {
            push_legacy_mode_directive_diagnostic(session, &source, file_id, &directive);
            if !session.cli_profile_override {
                session.config.guarantee_policy =
                    kobo_ir::GuaranteePolicy::for_profile(directive.profile);
            }
        }
        Ok(None) => {}
        Err(e) => {
            eprintln!("kobo: {e}");
            return Err(());
        }
    }

    // v0.5 preprocessing: rewrite @strict → marker attributes before syn parse.
    // v0.10 preprocessing: rewrite concurrent-state sugar to Kobo attributes.
    let ward_mapped = preprocess_ward_syntax_mapped(&source, file_id);
    let mut rewritten = ward_mapped.rewritten;
    let mut preprocess_source_map = ward_mapped.source_map;

    let concurrent_mapped = preprocess_concurrent_sugar_mapped(&rewritten, file_id);
    rewritten = concurrent_mapped.rewritten;
    preprocess_source_map = concurrent_mapped
        .source_map
        .compose_with(&preprocess_source_map);

    let configs = v05_keyword_configs();
    if let Err(e) = preprocess_strict_reject_invalid(&rewritten, &configs) {
        eprintln!("kobo: preprocess error: {e}");
        return Err(());
    }
    let strict_mapped = preprocess_kobo_keywords_mapped(&rewritten, file_id, &configs);
    rewritten = strict_mapped.rewritten;
    let markers = strict_mapped.metadata;
    preprocess_source_map = strict_mapped
        .source_map
        .compose_with(&preprocess_source_map);

    // v0.8: rewrite spawn { ... } → __kobo_spawn_block!({ ... }) before syn parse.
    let spawn_mapped = preprocess_spawn_blocks_mapped(&rewritten, file_id);
    rewritten = spawn_mapped.rewritten;
    preprocess_source_map = spawn_mapped.source_map.compose_with(&preprocess_source_map);

    // S-57: rewrite sync { } / async { } bridge blocks before syn parse.
    let bridge_mapped = preprocess_bridge_blocks_mapped(&rewritten, file_id);
    rewritten = bridge_mapped.rewritten;
    preprocess_source_map = bridge_mapped
        .source_map
        .compose_with(&preprocess_source_map);
    rewritten = mask_field_capability_views(&rewritten);

    let recovery_mode = if session.config.enable_parse_recovery {
        RecoveryMode::Recover
    } else {
        RecoveryMode::FailFast
    };
    let outcome = parse_file_recovering(&rewritten, file_id, &mut session.id_gen, recovery_mode);
    let parse_diagnostics = outcome
        .diagnostics
        .into_iter()
        .map(|diagnostic| remap_diagnostic_to_original_source(diagnostic, &preprocess_source_map))
        .collect::<Vec<_>>();
    let poisoned_spans = outcome
        .poisoned_spans
        .into_iter()
        .map(|span| remap_span_to_original(span, &preprocess_source_map))
        .collect::<Vec<_>>();
    session.diagnostics.extend(parse_diagnostics);
    session.poisoned_spans.extend(poisoned_spans);

    let Some(mut kobo_file) = outcome.file else {
        return Err(());
    };

    // Validate #[kobo::handler] usage (must be async fn).
    if let Err(e) = kobo_parser::validate_handler_attributes(kobo_file.syn_file()) {
        eprintln!("kobo: {e}");
        return Err(());
    }

    // S-3: Collect #[kobo::engine] structs for downstream constraint ceilings.
    let engine_structs = kobo_parser::collect_engine_structs(kobo_file.syn_file());
    if !engine_structs.is_empty() {
        session.engine_struct_names = engine_structs
            .iter()
            .map(|e| e.struct_name.clone())
            .collect();
    }

    // Collect @strict blocks/fns before stripping marker attributes.
    let (strict_blocks, strict_fns) =
        collect_strict_items_from_syn(kobo_file.syn_file(), &rewritten, file_id);
    if let Err(e) = postprocess_strict_markers(kobo_file.syn_file_mut(), &markers) {
        eprintln!("kobo: postprocess error: {e}");
        return Err(());
    }
    kobo_file.set_strict_items(strict_blocks, strict_fns);

    let mut kir = build_kir(
        &kobo_file,
        &mut session.id_gen,
        TransformOptions {
            small_struct_clone_threshold_bytes: session.config.small_struct_clone_threshold_bytes,
            copy_types: session.config.copy_types.clone(),
            mutating_methods: session.config.mutating_methods.clone(),
        },
    );
    kir.set_field_capability_views(field_capability_views);

    // G5: copy relaxed fn ranges into session so the rendering path can filter warnings.
    session.relaxed_fn_ranges = kir.relaxed_fn_ranges().to_vec();

    // S-3: Mark KIR nodes whose binding type matches an engine struct.
    if !session.engine_struct_names.is_empty() {
        let mut ceiling_nodes = std::collections::HashSet::new();
        let engine_ast_ids = kobo_file.engine_typed_binding_ids(&session.engine_struct_names);
        for ast_id in engine_ast_ids {
            if let Some(kir_id) = kir.kir_for_ast(ast_id) {
                ceiling_nodes.insert(kir_id);
            }
        }
        kir.set_engine_ceiling_nodes(ceiling_nodes);
    }

    Ok((kobo_file, kir))
}

fn push_legacy_mode_directive_diagnostic(
    session: &mut CompileSession,
    source: &str,
    file_id: FileId,
    directive: &LegacyModeDirective,
) {
    let profile = directive.profile_name();
    let span = legacy_mode_directive_span(source, file_id, directive);
    let replacement = format!("//! kobo:profile = \"{profile}\"");
    session.diagnostics.push(
        KDiagnostic::new(
            KErrorCode::K0096,
            Severity::Warning,
            DiagLabel::primary(span, "`kobo:mode` is a legacy profile alias"),
            format!(
                "`//! kobo:mode = {}` still applies as compatibility, but Kobo is one language; gradualness now lives in guarantee policy.",
                directive.value
            ),
            format!("use `--profile {profile}` or project guarantee policy instead"),
        )
        .with_help(format!(
            "legacy `kobo:mode = {}` is equivalent to the `{profile}` guarantee profile",
            directive.value
        ))
        .with_run(format!("kobo check --profile {profile} <file>"))
        .with_suggestion(DiagnosticSuggestion::new(
            format!("replace legacy directive with `{replacement}`"),
            SuggestionApplicability::MachineApplicable,
            vec![TextEdit::replace(span, replacement)],
        )),
    );
}

fn legacy_mode_directive_span(
    source: &str,
    file_id: FileId,
    directive: &LegacyModeDirective,
) -> KoboSpan {
    let mut offset = 0usize;
    for (index, line) in source.lines().enumerate() {
        if index + 1 == directive.line {
            let start_in_line = line.find("kobo:mode").unwrap_or(0);
            let end_in_line = line
                .find(&directive.value)
                .map(|value_start| value_start + directive.value.len())
                .unwrap_or(line.len());
            return KoboSpan::new(
                (offset + start_in_line) as u32,
                (offset + end_in_line) as u32,
                file_id,
            );
        }
        offset += line.len() + 1;
    }

    KoboSpan::new(0, 0, file_id)
}

fn mask_field_capability_views(source: &str) -> String {
    let mut output = source.to_owned();
    let mut search_start = 0usize;
    while let Some(relative) = output[search_start..].find(" using {") {
        let start = search_start + relative;
        let Some(close_relative) = output[start..].find('}') else {
            break;
        };
        let end = start + close_relative + 1;
        output.replace_range(start..end, &" ".repeat(end - start));
        search_start = end;
    }
    output
}

fn preprocess_ward_syntax_mapped(source: &str, file_id: FileId) -> PreprocessedSource<()> {
    let cleaned = scrub_comments_and_strings(source);
    if find_ward_keyword(&cleaned, 0).is_none() {
        return PreprocessedSource {
            rewritten: source.to_owned(),
            source_map: PreprocessSourceMap::identity_for(source, file_id),
            metadata: (),
        };
    }

    let mut rewritten = String::with_capacity(source.len() + 256);
    let mut source_map = PreprocessSourceMap::default();
    let mut cursor = 0;
    while let Some(ward_start) = find_ward_keyword(&cleaned, cursor) {
        let Some(ward) = parse_ward_block(source, &cleaned, ward_start) else {
            break;
        };
        push_rewrite_segment(
            &mut rewritten,
            &mut source_map,
            file_id,
            source,
            cursor,
            ward_start,
        );
        let generated_start = rewritten.len();
        rewritten.push_str(&ward_to_attribute_source(&ward));
        source_map.push_segment(
            KoboSpan::new(generated_start as u32, rewritten.len() as u32, file_id),
            KoboSpan::new(ward.start as u32, ward.end as u32, file_id),
        );
        cursor = ward.end;
    }
    push_rewrite_segment(
        &mut rewritten,
        &mut source_map,
        file_id,
        source,
        cursor,
        source.len(),
    );
    PreprocessedSource {
        rewritten,
        source_map,
        metadata: (),
    }
}

fn push_rewrite_segment(
    rewritten: &mut String,
    source_map: &mut PreprocessSourceMap,
    file_id: FileId,
    source: &str,
    start: usize,
    end: usize,
) {
    if start >= end {
        return;
    }
    let rewritten_start = rewritten.len();
    rewritten.push_str(&source[start..end]);
    source_map.push_segment(
        KoboSpan::new(rewritten_start as u32, rewritten.len() as u32, file_id),
        KoboSpan::new(start as u32, end as u32, file_id),
    );
}

struct WardBlock {
    name: String,
    start: usize,
    end: usize,
    body: String,
}

struct WardFacts {
    states: Vec<(String, String)>,
    obligations: Vec<(String, Vec<String>)>,
    scenarios: Vec<(String, String)>,
    metadata_comments: Vec<String>,
}

fn find_ward_keyword(source: &[u8], from: usize) -> Option<usize> {
    let mut cursor = from;
    while cursor + "ward".len() <= source.len() {
        let Some(relative) = find_bytes(&source[cursor..], b"ward") else {
            return None;
        };
        let start = cursor + relative;
        let end = start + "ward".len();
        let before = source.get(start.saturating_sub(1));
        let after = source.get(end);
        if !before.is_some_and(is_ident_byte)
            && after.is_some_and(|byte| byte.is_ascii_whitespace())
        {
            return Some(start);
        }
        cursor = end;
    }
    None
}

fn parse_ward_block(source: &str, cleaned: &[u8], start: usize) -> Option<WardBlock> {
    let mut name_start = start + "ward".len();
    while cleaned
        .get(name_start)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        name_start += 1;
    }
    let mut name_end = name_start;
    while cleaned.get(name_end).is_some_and(is_ident_byte) {
        name_end += 1;
    }
    let name = source[name_start..name_end].to_owned();
    let brace_start = find_bytes(&cleaned[name_end..], b"{")? + name_end;
    let brace_end = matching_brace_in_bytes(cleaned, brace_start)?;
    Some(WardBlock {
        name,
        start,
        end: brace_end + 1,
        body: source[brace_start + 1..brace_end].to_owned(),
    })
}

fn ward_to_attribute_source(ward: &WardBlock) -> String {
    let facts = parse_ward_facts(&ward.body);
    let mut output = String::new();
    output.push_str("#[kobo::ward]\n");
    output.push_str(&format!("struct {} {{\n", ward.name));
    for (name, ty) in &facts.states {
        output.push_str(&format!("    {name}: {ty},\n"));
    }
    output.push_str("}\n\n");
    for (type_name, actions) in &facts.obligations {
        output.push_str(&format!(
            "#[kobo::must_call({})]\nstruct {type_name} {{}}\n\n",
            actions.join(" | ")
        ));
        output.push_str(&format!("impl {type_name} {{\n"));
        for action in actions {
            output.push_str(&format!("    fn {action}(self) {{}}\n"));
        }
        output.push_str("}\n\n");
    }
    for comment in &facts.metadata_comments {
        output.push_str(comment);
        output.push('\n');
    }
    for (name, body) in &facts.scenarios {
        output.push_str("#[kobo::scenario(profile = \"sync\")]\n");
        output.push_str(&format!("fn {name}() {{\n{body}\n}}\n\n"));
    }
    output
}

fn parse_ward_facts(body: &str) -> WardFacts {
    let mut facts = WardFacts {
        states: Vec::new(),
        obligations: Vec::new(),
        scenarios: Vec::new(),
        metadata_comments: Vec::new(),
    };
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("state ") {
            if let Some((name, ty)) = rest.split_once(':') {
                facts
                    .states
                    .push((name.trim().to_owned(), ty.trim().to_owned()));
            }
        } else if let Some(rest) = trimmed.strip_prefix("obligation ") {
            if let Some((type_name, actions)) = rest.split_once(" must ") {
                facts.obligations.push((
                    type_name.trim().to_owned(),
                    actions
                        .split('|')
                        .map(str::trim)
                        .filter(|action| !action.is_empty())
                        .map(str::to_owned)
                        .collect(),
                ));
            }
        } else if let Some(rest) = trimmed.strip_prefix("invariant ") {
            facts
                .metadata_comments
                .push(format!("// kobo: invariant {}", rest.trim()));
        } else if let Some(rest) = trimmed.strip_prefix("temporal ") {
            facts
                .metadata_comments
                .push(format!("// kobo: temporal {}", rest.trim()));
        } else if let Some(rest) = trimmed.strip_prefix("port ") {
            facts
                .metadata_comments
                .push(format!("// kobo: port {}", rest.trim()));
        } else if let Some(rest) = trimmed.strip_prefix("recording ") {
            facts
                .metadata_comments
                .push(format!("// kobo: recording {}", rest.trim()));
        } else if let Some(rest) = trimmed.strip_prefix("debt ") {
            facts
                .metadata_comments
                .push(format!("// kobo: debt {}", rest.trim()));
        }
    }
    let cleaned = scrub_comments_and_strings(body);
    let mut search = 0;
    while let Some(scenario_start) = find_keyword(&cleaned, search, "scenario") {
        let name_start = scenario_start + "scenario ".len();
        let mut name_end = name_start;
        while cleaned.get(name_end).is_some_and(is_ident_byte) {
            name_end += 1;
        }
        let name = body[name_start..name_end].trim().to_owned();
        let Some(brace_start) = find_bytes(&cleaned[name_end..], b"{").map(|relative| name_end + relative) else {
            search = name_end;
            continue;
        };
        let Some(brace_end) = matching_brace_in_bytes(&cleaned, brace_start) else {
            break;
        };
        facts.scenarios.push((
            name,
            body[brace_start + 1..brace_end]
                .trim_matches('\n')
                .to_owned(),
        ));
        search = brace_end + 1;
    }
    facts
}

fn find_keyword(source: &[u8], from: usize, keyword: &str) -> Option<usize> {
    let needle = keyword.as_bytes();
    let mut cursor = from;
    while cursor + needle.len() <= source.len() {
        let Some(relative) = find_bytes(&source[cursor..], needle) else {
            return None;
        };
        let start = cursor + relative;
        let end = start + needle.len();
        let before = source.get(start.saturating_sub(1));
        let after = source.get(end);
        if !before.is_some_and(is_ident_byte)
            && after.is_some_and(|byte| byte.is_ascii_whitespace())
        {
            return Some(start);
        }
        cursor = end;
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

#[derive(Clone)]
struct CapabilityParam {
    name: String,
    ty: Option<String>,
}

struct ParsedCapabilityField {
    owner: Option<String>,
    name: String,
    mutable: bool,
    span: KoboSpan,
}

fn collect_field_capability_views(source: &str, file_id: FileId) -> Vec<FieldCapabilityView> {
    let mut views: Vec<FieldCapabilityView> = Vec::new();
    let mut search_start = 0usize;
    while let Some(relative) = source[search_start..].find(" using {") {
        let using_start = search_start + relative;
        let list_start = using_start + " using {".len();
        let Some(close_relative) = source[list_start..].find('}') else {
            break;
        };
        let list_end = list_start + close_relative;
        let using_end = list_end + 1;
        let Some((function, default_owner, params)) = capability_context(source, using_start)
        else {
            search_start = using_end;
            continue;
        };
        let fields = capability_fields(source, list_start, list_end, file_id);
        for field in fields {
            let owner = field
                .owner
                .clone()
                .or_else(|| default_owner.clone())
                .unwrap_or_else(|| "value".to_owned());
            let owner_type = params
                .iter()
                .find(|param| param.name == owner)
                .and_then(|param| param.ty.clone());
            let capability_field = FieldCapabilityField {
                name: field.name,
                mutable: field.mutable,
                span: field.span,
            };

            if let Some(index) = views
                .iter()
                .position(|view| view.function == function && view.owner == owner)
            {
                views[index].fields.push(capability_field);
                continue;
            }

            views.push(FieldCapabilityView {
                function: function.clone(),
                owner,
                owner_type,
                using_span: KoboSpan::new(using_start as u32, using_end as u32, file_id),
                fields: vec![capability_field],
            });
        }
        search_start = using_end;
    }
    views
}

fn capability_context(
    source: &str,
    using_start: usize,
) -> Option<(String, Option<String>, Vec<CapabilityParam>)> {
    let prefix = &source[..using_start];
    let fn_start = prefix.rfind("fn ")?;
    let function = ident_prefix(&source[fn_start + "fn ".len()..])?;
    let args_start = source[fn_start..using_start].find('(')? + fn_start + 1;
    let args_end = prefix.rfind(')').unwrap_or(using_start);
    let args = &source[args_start..args_end.min(using_start)];
    let params = capability_params(args);
    let default_owner = params.last().map(|param| param.name.clone());
    Some((function, default_owner, params))
}

fn capability_params(args: &str) -> Vec<CapabilityParam> {
    args.split(',')
        .filter_map(|arg| {
            let (name, ty) = arg.split_once(':')?;
            let name = ident_suffix(name)?;
            let ty = ident_prefix(normalize_type_prefix(ty));
            Some(CapabilityParam { name, ty })
        })
        .collect()
}

fn capability_fields(
    source: &str,
    list_start: usize,
    list_end: usize,
    file_id: FileId,
) -> Vec<ParsedCapabilityField> {
    let mut fields = Vec::new();
    let mut part_start = list_start;
    for raw in source[list_start..list_end].split(',') {
        let raw_end = part_start + raw.len();
        if let Some(field) = capability_field(source, part_start, raw_end, file_id) {
            fields.push(field);
        }
        part_start = raw_end + 1;
    }
    fields
}

fn capability_field(
    source: &str,
    part_start: usize,
    raw_end: usize,
    file_id: FileId,
) -> Option<ParsedCapabilityField> {
    let raw = &source[part_start..raw_end];
    let trimmed = raw.trim();
    let (mutable, expr) = if let Some(rest) = trimmed.strip_prefix("mut") {
        if rest.chars().next().is_some_and(char::is_whitespace) {
            (true, rest.trim_start())
        } else {
            (false, trimmed)
        }
    } else {
        (false, trimmed)
    };
    let (owner, field_text) = expr
        .rsplit_once('.')
        .map(|(owner, field)| (ident_suffix(owner), field))
        .unwrap_or((None, expr));
    let name = ident_prefix(field_text)?;
    let name_len = name.len();
    let name_start = source[part_start..raw_end]
        .find(&name)
        .map(|relative| part_start + relative)
        .unwrap_or(part_start);
    Some(ParsedCapabilityField {
        owner,
        name,
        mutable,
        span: KoboSpan::new(name_start as u32, (name_start + name_len) as u32, file_id),
    })
}

fn normalize_type_prefix(ty: &str) -> &str {
    let ty = ty.trim().trim_start_matches('&').trim_start();
    ty.strip_prefix("mut")
        .filter(|rest| rest.chars().next().is_some_and(char::is_whitespace))
        .map(str::trim_start)
        .unwrap_or(ty)
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

fn remap_diagnostic_to_original_source(
    mut diagnostic: KDiagnostic,
    source_map: &PreprocessSourceMap,
) -> KDiagnostic {
    diagnostic.primary = remap_label(diagnostic.primary, source_map);
    diagnostic.secondary = diagnostic
        .secondary
        .into_iter()
        .map(|label| remap_label(label, source_map))
        .collect();
    diagnostic.related = diagnostic
        .related
        .into_iter()
        .map(|related| DiagnosticRelatedInfo {
            span: remap_span_to_original(related.span, source_map),
            message: related.message,
        })
        .collect();
    diagnostic.suggestions = diagnostic
        .suggestions
        .into_iter()
        .map(|mut suggestion| {
            suggestion.edits = suggestion
                .edits
                .into_iter()
                .map(|edit| TextEdit {
                    span: remap_span_to_original(edit.span, source_map),
                    replacement: edit.replacement,
                })
                .collect();
            suggestion
        })
        .collect();
    diagnostic.suppressed_by = diagnostic
        .suppressed_by
        .map(|span| remap_span_to_original(span, source_map));
    diagnostic
}

fn remap_label(mut label: DiagLabel, source_map: &PreprocessSourceMap) -> DiagLabel {
    label.span = remap_span_to_original(label.span, source_map);
    label
}

fn remap_span_to_original(span: KoboSpan, source_map: &PreprocessSourceMap) -> KoboSpan {
    source_map.rewritten_span_to_original(span).unwrap_or(span)
}
