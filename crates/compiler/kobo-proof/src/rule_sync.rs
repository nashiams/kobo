use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

const OBLIGATION_RULE_CATALOG: &str = include_str!("../rules/obligation_rules.toml");
const LEAN_CORE_SOURCE: &str = include_str!("../../../../proof/lean/KoboCore.lean");
const LEAN_OBLIGATION_RULES_SOURCE: &str =
    include_str!("../../../../proof/lean/ObligationRules.lean");
const LEAN_PRESERVATION_SOURCE: &str = include_str!("../../../../proof/lean/Preservation.lean");
const LEAN_NO_SILENT_LOSS_SOURCE: &str = include_str!("../../../../proof/lean/NoSilentLoss.lean");
const REQUIRED_RULE_IDS: [&str; 8] = [
    "create",
    "transfer",
    "split",
    "discharge",
    "return",
    "cancel",
    "panic",
    "opaque",
];

const TEMPLATE_ASSUMPTION_FIELDS: &[(&str, &str)] = &[
    ("templateId", "template_id"),
    ("templateVersion", "template_version"),
    ("obligationKind", "obligation_kind"),
    ("statement", "statement"),
    ("source", "source"),
    ("confidence", "confidence"),
    ("rustCertificateFieldPath", "rust_certificate_field_path"),
    ("leanAssumptionName", "lean_assumption_name"),
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ObligationRuleCatalog {
    pub rules: Vec<ObligationRule>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ObligationRule {
    pub id: String,
    pub name: String,
    pub input_states: Vec<String>,
    pub output_states: Vec<String>,
    pub allowed_modeled_exits: Vec<String>,
    pub rust_event_kinds: Vec<String>,
    pub required_certificate_fields: Vec<String>,
    pub rust_module: String,
    pub rust_verifier: String,
    pub rust_verifiers: Vec<String>,
    pub rust_test: String,
    pub lean_rule: String,
    pub lean_constructor_arity: usize,
    pub lean_required_premises: Vec<String>,
    pub lean_output_states: Vec<String>,
    pub lean_theorem: String,
    pub template_assumption_fields: Vec<String>,
    pub examples: Vec<String>,
    pub negative_examples: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeanRuleManifest {
    pub rules: Vec<LeanRuleShape>,
    pub template_assumption_fields: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeanRuleShape {
    pub id: String,
    pub constructor: String,
    pub constructor_arity: usize,
    pub required_premises: Vec<String>,
    pub input_states: Vec<String>,
    pub output_states: Vec<String>,
    pub allowed_modeled_exits: Vec<String>,
    pub theorem: String,
}

impl LeanRuleManifest {
    pub fn rule(&self, id: &str) -> Option<&LeanRuleShape> {
        self.rules.iter().find(|rule| rule.id == id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RuleSyncError {
    #[error("obligation rule catalog parse error: {message}")]
    Parse { message: String },
    #[error("missing required obligation rule `{id}`")]
    MissingRequiredRule { id: String },
    #[error("duplicate obligation rule `{id}`")]
    DuplicateRule { id: String },
    #[error("obligation rule `{id}` has empty field `{field}`")]
    EmptyField { id: String, field: String },
    #[error(
        "obligation rule `{id}` drift in `{field}`: expected `{expected}`, observed `{observed}`"
    )]
    Drift {
        id: String,
        field: String,
        expected: String,
        observed: String,
    },
    #[error("Lean rule manifest is missing `{item}`")]
    LeanManifestMissing { item: String },
}

pub fn required_obligation_rule_ids() -> &'static [&'static str] {
    &REQUIRED_RULE_IDS
}

pub fn load_obligation_rule_catalog() -> Result<ObligationRuleCatalog, RuleSyncError> {
    parse_obligation_rule_catalog(OBLIGATION_RULE_CATALOG)
}

pub fn parse_obligation_rule_catalog(source: &str) -> Result<ObligationRuleCatalog, RuleSyncError> {
    toml::from_str::<ObligationRuleCatalog>(source).map_err(|error| RuleSyncError::Parse {
        message: error.to_string(),
    })
}

pub fn load_lean_rule_manifest() -> Result<LeanRuleManifest, RuleSyncError> {
    parse_lean_rule_manifest_sources(
        LEAN_CORE_SOURCE,
        LEAN_OBLIGATION_RULES_SOURCE,
        &[LEAN_PRESERVATION_SOURCE, LEAN_NO_SILENT_LOSS_SOURCE],
    )
}

pub fn parse_lean_rule_manifest_sources(
    core_source: &str,
    obligation_rules_source: &str,
    theorem_sources: &[&str],
) -> Result<LeanRuleManifest, RuleSyncError> {
    parse_lean_rule_manifest(core_source, obligation_rules_source, theorem_sources)
}

pub fn validate_obligation_rule_catalog(
    catalog: &ObligationRuleCatalog,
) -> Result<(), RuleSyncError> {
    let lean_manifest = load_lean_rule_manifest()?;
    validate_obligation_rule_catalog_against_lean_manifest(catalog, &lean_manifest)
}

pub fn validate_obligation_rule_catalog_against_lean_manifest(
    catalog: &ObligationRuleCatalog,
    lean_manifest: &LeanRuleManifest,
) -> Result<(), RuleSyncError> {
    let rules_by_id = rules_by_id(catalog)?;
    verify_required_rules(&rules_by_id)?;
    verify_rule_fields(catalog)?;
    verify_lean_manifest_with_rules(&rules_by_id, lean_manifest)
}

fn rules_by_id<'a>(
    catalog: &'a ObligationRuleCatalog,
) -> Result<BTreeMap<&'a str, &'a ObligationRule>, RuleSyncError> {
    let mut rules = BTreeMap::new();
    for rule in &catalog.rules {
        if rules.insert(rule.id.as_str(), rule).is_some() {
            return Err(RuleSyncError::DuplicateRule {
                id: rule.id.clone(),
            });
        }
    }
    Ok(rules)
}

fn verify_required_rules(
    rules_by_id: &BTreeMap<&str, &ObligationRule>,
) -> Result<(), RuleSyncError> {
    for id in REQUIRED_RULE_IDS {
        if rules_by_id.contains_key(id) {
            continue;
        }
        return Err(RuleSyncError::MissingRequiredRule { id: id.to_owned() });
    }
    Ok(())
}

fn verify_rule_fields(catalog: &ObligationRuleCatalog) -> Result<(), RuleSyncError> {
    for rule in &catalog.rules {
        verify_non_empty(&rule.id, "name", &rule.name)?;
        verify_non_empty(&rule.id, "rust_module", &rule.rust_module)?;
        verify_non_empty(&rule.id, "rust_verifier", &rule.rust_verifier)?;
        verify_non_empty(&rule.id, "rust_test", &rule.rust_test)?;
        verify_non_empty(&rule.id, "lean_rule", &rule.lean_rule)?;
        verify_non_empty(&rule.id, "lean_theorem", &rule.lean_theorem)?;
        verify_non_empty_list(&rule.id, "rust_verifiers", &rule.rust_verifiers)?;
        verify_non_empty_list(
            &rule.id,
            "required_certificate_fields",
            &rule.required_certificate_fields,
        )?;
        verify_list_values(
            &rule.id,
            "lean_required_premises",
            &rule.lean_required_premises,
        )?;
        verify_list_values(&rule.id, "lean_output_states", &rule.lean_output_states)?;
        verify_list_values(
            &rule.id,
            "template_assumption_fields",
            &rule.template_assumption_fields,
        )?;
        verify_non_empty_list(&rule.id, "examples", &rule.examples)?;
        verify_non_empty_list(&rule.id, "negative_examples", &rule.negative_examples)?;
        verify_primary_verifier(rule)?;
        verify_field_backed_verifiers(rule)?;
    }
    Ok(())
}

fn verify_non_empty(id: &str, field: &str, value: &str) -> Result<(), RuleSyncError> {
    if !value.trim().is_empty() {
        return Ok(());
    }
    Err(RuleSyncError::EmptyField {
        id: id.to_owned(),
        field: field.to_owned(),
    })
}

fn verify_non_empty_list(id: &str, field: &str, values: &[String]) -> Result<(), RuleSyncError> {
    if values.iter().any(|value| !value.trim().is_empty()) {
        return Ok(());
    }
    Err(RuleSyncError::EmptyField {
        id: id.to_owned(),
        field: field.to_owned(),
    })
}

fn verify_list_values(id: &str, field: &str, values: &[String]) -> Result<(), RuleSyncError> {
    if values.iter().all(|value| !value.trim().is_empty()) {
        return Ok(());
    }
    Err(RuleSyncError::EmptyField {
        id: id.to_owned(),
        field: field.to_owned(),
    })
}

fn verify_primary_verifier(rule: &ObligationRule) -> Result<(), RuleSyncError> {
    if rule
        .rust_verifiers
        .iter()
        .any(|verifier| verifier == &rule.rust_verifier)
    {
        return Ok(());
    }
    Err(drift(
        rule,
        "rust_verifiers",
        rule.rust_verifier.clone(),
        rule.rust_verifiers.join(","),
    ))
}

fn verify_field_backed_verifiers(rule: &ObligationRule) -> Result<(), RuleSyncError> {
    for field in &rule.required_certificate_fields {
        let Some(required_verifier) = verifier_for_certificate_field(field) else {
            continue;
        };
        if rule
            .rust_verifiers
            .iter()
            .any(|verifier| verifier == required_verifier)
        {
            continue;
        }
        return Err(drift(
            rule,
            "rust_verifiers",
            required_verifier.to_owned(),
            rule.rust_verifiers.join(","),
        ));
    }
    Ok(())
}

fn verifier_for_certificate_field(field: &str) -> Option<&'static str> {
    match field {
        "core.async_model.cancel_edges" => Some("verify_cancel_edges"),
        "core.async_model.future_state_obligations" => Some("verify_future_state_obligations"),
        _ => None,
    }
}

fn verify_lean_manifest_with_rules(
    rules_by_id: &BTreeMap<&str, &ObligationRule>,
    lean_manifest: &LeanRuleManifest,
) -> Result<(), RuleSyncError> {
    verify_catalog_rules_have_lean_counterparts(rules_by_id, lean_manifest)?;
    verify_lean_rules_have_catalog_entries(rules_by_id, lean_manifest)?;
    for (id, rule) in rules_by_id {
        let rule_id = *id;
        let rule = *rule;
        let lean_rule =
            lean_manifest
                .rule(rule_id)
                .ok_or_else(|| RuleSyncError::LeanManifestMissing {
                    item: format!("Lean counterpart for catalog rule `{rule_id}`"),
                })?;
        verify_string_list_field(
            rule,
            "input_states",
            &lean_rule.input_states,
            &rule.input_states,
        )?;
        verify_string_list_field(
            rule,
            "output_states",
            &lean_rule.output_states,
            &rule.output_states,
        )?;
        verify_string_list_field(
            rule,
            "allowed_modeled_exits",
            &lean_rule.allowed_modeled_exits,
            &rule.allowed_modeled_exits,
        )?;
        verify_text_field(rule, "lean_rule", &lean_rule.constructor, &rule.lean_rule)?;
        verify_usize_field(
            rule,
            "lean_constructor_arity",
            lean_rule.constructor_arity,
            rule.lean_constructor_arity,
        )?;
        verify_string_list_field(
            rule,
            "lean_required_premises",
            &lean_rule.required_premises,
            &rule.lean_required_premises,
        )?;
        verify_string_list_field(
            rule,
            "lean_output_states",
            &lean_rule.output_states,
            &rule.lean_output_states,
        )?;
        verify_text_field(rule, "lean_theorem", &lean_rule.theorem, &rule.lean_theorem)?;
        if rule_id == "opaque" {
            verify_string_list_field(
                rule,
                "template_assumption_fields",
                &lean_manifest.template_assumption_fields,
                &rule.template_assumption_fields,
            )?;
        }
    }
    Ok(())
}

fn verify_catalog_rules_have_lean_counterparts(
    rules_by_id: &BTreeMap<&str, &ObligationRule>,
    lean_manifest: &LeanRuleManifest,
) -> Result<(), RuleSyncError> {
    for id in rules_by_id.keys() {
        if lean_manifest.rule(id).is_some() {
            continue;
        }
        return Err(RuleSyncError::LeanManifestMissing {
            item: format!("Lean counterpart for catalog rule `{id}`"),
        });
    }
    Ok(())
}

fn verify_lean_rules_have_catalog_entries(
    rules_by_id: &BTreeMap<&str, &ObligationRule>,
    lean_manifest: &LeanRuleManifest,
) -> Result<(), RuleSyncError> {
    for lean_rule in &lean_manifest.rules {
        let Some(rule) = rules_by_id.get(lean_rule.id.as_str()) else {
            return Err(RuleSyncError::LeanManifestMissing {
                item: format!(
                    "catalog entry for Lean constructor `{}`",
                    lean_rule.constructor
                ),
            });
        };
        if rule.lean_rule == lean_rule.constructor {
            continue;
        }
        return Err(RuleSyncError::LeanManifestMissing {
            item: format!(
                "catalog entry for Lean constructor `{}`",
                lean_rule.constructor
            ),
        });
    }
    Ok(())
}

fn parse_lean_rule_manifest(
    core_source: &str,
    obligation_rules_source: &str,
    theorem_sources: &[&str],
) -> Result<LeanRuleManifest, RuleSyncError> {
    let constructors = parse_lean_constructors(obligation_rules_source);
    let constructor_signatures = parse_lean_constructor_signatures(obligation_rules_source);
    let input_states = parse_rule_state_relation(core_source, "RuleInputState");
    let output_states = parse_rule_state_relation(core_source, "RuleOutputState");
    let modeled_exits = parse_modeled_exits(obligation_rules_source);
    let theorem_names = parse_theorem_names(theorem_sources);
    let template_assumption_fields = parse_template_assumption_fields(core_source)?;
    let mut rules = Vec::new();
    for (constructor, signature) in constructor_signatures {
        let Some(id) = rule_id_for_constructor_signature(&constructor, &signature) else {
            continue;
        };
        let constructor_shape = constructors.get(constructor.as_str()).ok_or_else(|| {
            RuleSyncError::LeanManifestMissing {
                item: format!("constructor `{constructor}`"),
            }
        })?;
        let theorem = format!("preservation_{id}");
        if !theorem_names.contains(theorem.as_str()) {
            return Err(RuleSyncError::LeanManifestMissing {
                item: format!("theorem `{theorem}`"),
            });
        }
        rules.push(LeanRuleShape {
            id: id.clone(),
            constructor,
            constructor_arity: constructor_shape.arity,
            required_premises: constructor_shape.required_premises.clone(),
            input_states: input_states.get(id.as_str()).cloned().unwrap_or_default(),
            output_states: output_states.get(id.as_str()).cloned().unwrap_or_default(),
            allowed_modeled_exits: modeled_exits.get(id.as_str()).cloned().unwrap_or_default(),
            theorem,
        });
    }
    Ok(LeanRuleManifest {
        rules,
        template_assumption_fields,
    })
}

#[derive(Clone, Debug)]
struct LeanConstructorShape {
    arity: usize,
    required_premises: Vec<String>,
}

fn parse_lean_constructors(source: &str) -> BTreeMap<String, LeanConstructorShape> {
    let mut constructors = BTreeMap::new();
    for (name, signature) in parse_lean_constructor_signatures(source) {
        let parameter_groups = constructor_parameter_groups(&signature);
        constructors.insert(
            name,
            LeanConstructorShape {
                arity: parameter_groups.len(),
                required_premises: constructor_required_premises(&parameter_groups),
            },
        );
    }
    constructors
}

fn constructor_parameter_groups(signature: &str) -> Vec<String> {
    let end = signature
        .find(" Step ")
        .or_else(|| signature.find(" ModeledExitStep "))
        .unwrap_or(signature.len());
    let mut groups = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    for (index, character) in signature[..end].char_indices() {
        match character {
            '(' => {
                if depth == 0 {
                    start = Some(index + 1);
                }
                depth += 1;
            }
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    if let Some(group_start) = start.take() {
                        groups.push(signature[group_start..index].trim().to_owned());
                    }
                }
            }
            _ => {}
        }
    }
    groups
}

fn constructor_required_premises(parameter_groups: &[String]) -> Vec<String> {
    let mut premises = Vec::new();
    for group in parameter_groups {
        if let Some(premise) = premise_name_for_parameter(group) {
            premises.push(premise.to_owned());
        }
    }
    premises
}

fn premise_name_for_parameter(group: &str) -> Option<&'static str> {
    if group.starts_with("nonempty ") {
        return Some("nonempty_id");
    }
    if group.starts_with("projection ") {
        return Some("projection");
    }
    if group.starts_with("owned ") && group.contains("requiresState") {
        return Some("requires_owned");
    }
    if group.starts_with("precondition ") && group.contains("dischargePrecondition") {
        return Some("owned_or_transferred");
    }
    if group.starts_with("safe ") && group.contains("noUnresolvedLocal") {
        return Some("no_unresolved_local");
    }
    if group.starts_with("cancelEvidence ") {
        return Some("cancel_edge_evidence");
    }
    if group.starts_with("futureObligation ") {
        return Some("future_state_obligation_evidence");
    }
    if group.starts_with("recorded ") && group.contains("cancellationEvidenceRecorded") {
        return Some("cancellation_evidence_recorded");
    }
    if group.starts_with("precondition ") && group.contains("opaqueBoundaryPrecondition") {
        return Some("opaque_boundary_precondition");
    }
    if group.starts_with("current ") && group.contains("templateAssumptionIsCurrent") {
        return Some("template_assumption_current");
    }
    if group.starts_with("matched ") && group.contains("templateAssumptionMatches") {
        return Some("template_assumption_matches_requirement");
    }
    if group.starts_with("recorded ") && group.contains("opaqueLedgerRecorded") {
        return Some("opaque_ledger_recorded");
    }
    None
}

fn parse_rule_state_relation(source: &str, relation: &str) -> BTreeMap<String, Vec<String>> {
    let normalized = source.split_whitespace().collect::<Vec<_>>().join(" ");
    let marker = format!("{relation} RuleId.");
    let mut states_by_rule = BTreeMap::<String, BTreeSet<String>>::new();
    for segment in normalized.split(marker.as_str()).skip(1) {
        let Some((rule, state_segment)) = segment.split_once(" ObligationState.") else {
            continue;
        };
        let state = state_segment
            .chars()
            .take_while(|character| character.is_ascii_alphanumeric())
            .collect::<String>();
        if state.is_empty() {
            continue;
        }
        states_by_rule
            .entry(rule_name(rule).to_owned())
            .or_default()
            .insert(state_name(&state));
    }
    states_by_rule
        .into_iter()
        .map(|(rule, states)| (rule, states.into_iter().collect()))
        .collect()
}

fn parse_modeled_exits(source: &str) -> BTreeMap<String, Vec<String>> {
    let constructors = parse_lean_constructor_signatures(source);
    let mut exits = BTreeMap::new();
    for signature in constructors.values() {
        let Some((rule, exit)) = modeled_exit_rule_for_signature(signature) else {
            continue;
        };
        exits.insert(rule, vec![modeled_exit_name(&exit)]);
    }
    exits
}

fn rule_id_for_constructor_signature(constructor: &str, signature: &str) -> Option<String> {
    if signature.contains(" Step ") {
        return rule_id_from_step_signature(signature)
            .or_else(|| rule_id_from_constructor(constructor));
    }
    if signature.contains(" ModeledExitStep ") {
        if let Some((rule, _)) = modeled_exit_rule_for_signature(signature) {
            return Some(rule);
        }
        if is_ignored_modeled_exit_signature(signature) {
            return None;
        }
        return rule_id_from_constructor(constructor);
    }
    None
}

fn rule_id_from_step_signature(signature: &str) -> Option<String> {
    lean_name_after_marker(signature, "RuleId.")
}

fn modeled_exit_rule_for_signature(signature: &str) -> Option<(String, String)> {
    if !signature.contains(" ModeledExitStep ") {
        return None;
    }
    let exit = lean_name_after_marker(signature, "ModeledExit.")?;
    modeled_exit_rule_id(&exit).map(|rule| (rule, exit))
}

fn is_ignored_modeled_exit_signature(signature: &str) -> bool {
    matches!(
        lean_name_after_marker(signature, "ModeledExit.").as_deref(),
        Some("errorExit" | "breakExit")
    )
}

fn rule_id_from_constructor(constructor: &str) -> Option<String> {
    constructor
        .strip_prefix("step_")
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

fn lean_name_after_marker(source: &str, marker: &str) -> Option<String> {
    source
        .split(marker)
        .nth(1)
        .map(|segment| {
            segment
                .chars()
                .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                .collect::<String>()
        })
        .filter(|name| !name.is_empty())
}

fn modeled_exit_rule_id(exit: &str) -> Option<String> {
    match exit {
        "errorExit" | "breakExit" => None,
        "opaqueBoundary" => Some("opaque".to_owned()),
        other => Some(modeled_exit_name(other)),
    }
}

fn parse_lean_constructor_signatures(source: &str) -> BTreeMap<String, String> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut signatures = BTreeMap::new();
    let mut index = 0;
    let mut in_rule_inductive = false;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if is_rule_constructor_inductive(trimmed) {
            in_rule_inductive = true;
            index += 1;
            continue;
        }
        if in_rule_inductive && is_top_level_lean_declaration(trimmed) {
            in_rule_inductive = false;
            index += 1;
            continue;
        }
        if !in_rule_inductive {
            index += 1;
            continue;
        }
        if !trimmed.starts_with("| step_") {
            index += 1;
            continue;
        }
        let mut signature = trimmed.to_owned();
        index += 1;
        while index < lines.len() && !has_constructor_result_type(&signature) {
            signature.push(' ');
            signature.push_str(lines[index].trim());
            index += 1;
        }
        if !has_constructor_result_type(&signature) {
            continue;
        }
        let name = signature
            .strip_prefix("| ")
            .and_then(|rest| rest.split_whitespace().next())
            .unwrap_or_default()
            .to_owned();
        signatures.insert(name, signature);
    }
    signatures
}

fn is_rule_constructor_inductive(trimmed: &str) -> bool {
    trimmed.starts_with("inductive Step ") || trimmed.starts_with("inductive ModeledExitStep ")
}

fn is_top_level_lean_declaration(trimmed: &str) -> bool {
    trimmed.starts_with("abbrev ")
        || trimmed.starts_with("def ")
        || trimmed.starts_with("end ")
        || trimmed.starts_with("inductive ")
        || trimmed.starts_with("namespace ")
        || trimmed.starts_with("structure ")
        || trimmed.starts_with("theorem ")
}

fn has_constructor_result_type(signature: &str) -> bool {
    signature.contains(" Step ") || signature.contains(" ModeledExitStep ")
}

fn parse_theorem_names(sources: &[&str]) -> BTreeSet<String> {
    sources
        .iter()
        .flat_map(|source| source.lines())
        .filter_map(|line| {
            let trimmed = line.trim();
            trimmed
                .strip_prefix("theorem ")
                .and_then(|rest| rest.split_whitespace().next())
                .map(str::to_owned)
        })
        .collect()
}

fn parse_template_assumption_fields(source: &str) -> Result<Vec<String>, RuleSyncError> {
    let mut fields = Vec::new();
    let mut in_template_assumption = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed == "structure TemplateAssumption where" {
            in_template_assumption = true;
            continue;
        }
        if !in_template_assumption {
            continue;
        }
        if trimmed.starts_with("deriving ") {
            break;
        }
        let Some((field, _)) = trimmed.split_once(" : ") else {
            continue;
        };
        let mapped = TEMPLATE_ASSUMPTION_FIELDS
            .iter()
            .find_map(|(lean_field, catalog_field)| {
                (*lean_field == field).then_some((*catalog_field).to_owned())
            })
            .ok_or_else(|| RuleSyncError::LeanManifestMissing {
                item: format!("template field `{field}`"),
            })?;
        fields.push(mapped);
    }
    if fields.is_empty() {
        return Err(RuleSyncError::LeanManifestMissing {
            item: "TemplateAssumption fields".to_owned(),
        });
    }
    Ok(fields)
}

fn rule_name(name: &str) -> &str {
    name.trim_matches(|character: char| !character.is_ascii_alphanumeric() && character != '_')
}

fn state_name(name: &str) -> String {
    match name {
        "branchUnresolved" => "branch_unresolved".to_owned(),
        other => other.to_ascii_lowercase(),
    }
}

fn modeled_exit_name(name: &str) -> String {
    match name {
        "errorExit" => "error_exit".to_owned(),
        "breakExit" => "break_exit".to_owned(),
        "opaqueBoundary" => "opaque_boundary".to_owned(),
        other => other.to_ascii_lowercase(),
    }
}

fn verify_text_field(
    rule: &ObligationRule,
    field: &str,
    expected: &str,
    observed: &str,
) -> Result<(), RuleSyncError> {
    if expected == observed {
        return Ok(());
    }
    Err(drift(rule, field, expected.to_owned(), observed.to_owned()))
}

fn verify_string_list_field(
    rule: &ObligationRule,
    field: &str,
    expected: &[String],
    observed: &[String],
) -> Result<(), RuleSyncError> {
    let expected = expected.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let observed = observed.iter().map(String::as_str).collect::<BTreeSet<_>>();
    if expected == observed {
        return Ok(());
    }
    Err(drift(
        rule,
        field,
        expected.into_iter().collect::<Vec<_>>().join(","),
        observed.into_iter().collect::<Vec<_>>().join(","),
    ))
}

fn verify_usize_field(
    rule: &ObligationRule,
    field: &str,
    expected: usize,
    observed: usize,
) -> Result<(), RuleSyncError> {
    if expected == observed {
        return Ok(());
    }
    Err(drift(
        rule,
        field,
        expected.to_string(),
        observed.to_string(),
    ))
}

fn drift(rule: &ObligationRule, field: &str, expected: String, observed: String) -> RuleSyncError {
    RuleSyncError::Drift {
        id: rule.id.clone(),
        field: field.to_owned(),
        expected,
        observed,
    }
}
