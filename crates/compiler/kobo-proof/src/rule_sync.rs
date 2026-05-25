use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

const OBLIGATION_RULE_CATALOG: &str = include_str!("../rules/obligation_rules.toml");
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
    pub rust_test: String,
    pub lean_rule: String,
    pub lean_theorem: String,
    pub examples: Vec<String>,
    pub negative_examples: Vec<String>,
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
}

#[derive(Clone, Debug)]
struct RuleExpectation {
    id: &'static str,
    input_states: &'static [&'static str],
    output_states: &'static [&'static str],
    allowed_modeled_exits: &'static [&'static str],
    rust_module: &'static str,
    rust_verifier: &'static str,
    lean_rule: &'static str,
    lean_theorem: &'static str,
    rust_event_kinds: &'static [&'static str],
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

pub fn validate_obligation_rule_catalog(
    catalog: &ObligationRuleCatalog,
) -> Result<(), RuleSyncError> {
    let rules_by_id = rules_by_id(catalog)?;
    verify_required_rules(&rules_by_id)?;
    verify_rule_fields(catalog)?;
    verify_rule_expectations(&rules_by_id)
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
        verify_non_empty_list(
            &rule.id,
            "required_certificate_fields",
            &rule.required_certificate_fields,
        )?;
        verify_non_empty_list(&rule.id, "examples", &rule.examples)?;
        verify_non_empty_list(&rule.id, "negative_examples", &rule.negative_examples)?;
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

fn verify_rule_expectations(
    rules_by_id: &BTreeMap<&str, &ObligationRule>,
) -> Result<(), RuleSyncError> {
    for expectation in rule_expectations() {
        let rule =
            rules_by_id
                .get(expectation.id)
                .ok_or_else(|| RuleSyncError::MissingRequiredRule {
                    id: expectation.id.to_owned(),
                })?;
        verify_list_field(
            rule,
            "input_states",
            expectation.input_states,
            &rule.input_states,
        )?;
        verify_list_field(
            rule,
            "output_states",
            expectation.output_states,
            &rule.output_states,
        )?;
        verify_list_field(
            rule,
            "allowed_modeled_exits",
            expectation.allowed_modeled_exits,
            &rule.allowed_modeled_exits,
        )?;
        verify_list_field(
            rule,
            "rust_event_kinds",
            expectation.rust_event_kinds,
            &rule.rust_event_kinds,
        )?;
        verify_text_field(
            rule,
            "rust_module",
            expectation.rust_module,
            &rule.rust_module,
        )?;
        verify_text_field(
            rule,
            "rust_verifier",
            expectation.rust_verifier,
            &rule.rust_verifier,
        )?;
        verify_text_field(rule, "lean_rule", expectation.lean_rule, &rule.lean_rule)?;
        verify_text_field(
            rule,
            "lean_theorem",
            expectation.lean_theorem,
            &rule.lean_theorem,
        )?;
    }
    Ok(())
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

fn verify_list_field(
    rule: &ObligationRule,
    field: &str,
    expected: &[&str],
    observed: &[String],
) -> Result<(), RuleSyncError> {
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
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

fn drift(rule: &ObligationRule, field: &str, expected: String, observed: String) -> RuleSyncError {
    RuleSyncError::Drift {
        id: rule.id.clone(),
        field: field.to_owned(),
        expected,
        observed,
    }
}

fn rule_expectations() -> Vec<RuleExpectation> {
    vec![
        RuleExpectation {
            id: "create",
            input_states: &[],
            output_states: &["owned"],
            allowed_modeled_exits: &[],
            rust_module: "crates/compiler/kobo-proof/src/obligation.rs",
            rust_verifier: "apply_obligation_event::Create",
            lean_rule: "step_create",
            lean_theorem: "preservation_create",
            rust_event_kinds: &["create"],
        },
        RuleExpectation {
            id: "transfer",
            input_states: &["owned"],
            output_states: &["transferred"],
            allowed_modeled_exits: &[],
            rust_module: "crates/compiler/kobo-proof/src/obligation.rs",
            rust_verifier: "apply_obligation_event::Transfer",
            lean_rule: "step_transfer",
            lean_theorem: "preservation_transfer",
            rust_event_kinds: &["transfer", "move"],
        },
        RuleExpectation {
            id: "split",
            input_states: &["owned"],
            output_states: &["owned", "transferred"],
            allowed_modeled_exits: &[],
            rust_module: "crates/compiler/kobo-proof/src/obligation.rs",
            rust_verifier: "cfg_branch_join_split",
            lean_rule: "step_split",
            lean_theorem: "preservation_split",
            rust_event_kinds: &["branch_unresolved"],
        },
        RuleExpectation {
            id: "discharge",
            input_states: &["owned"],
            output_states: &["resolved"],
            allowed_modeled_exits: &[],
            rust_module: "crates/compiler/kobo-proof/src/obligation.rs",
            rust_verifier: "apply_obligation_event::Discharge",
            lean_rule: "step_discharge",
            lean_theorem: "preservation_discharge",
            rust_event_kinds: &["discharge"],
        },
        RuleExpectation {
            id: "return",
            input_states: &[],
            output_states: &[],
            allowed_modeled_exits: &["return"],
            rust_module: "crates/compiler/kobo-proof/src/obligation.rs",
            rust_verifier: "reject_unresolved_exit::return",
            lean_rule: "step_return",
            lean_theorem: "preservation_return",
            rust_event_kinds: &[],
        },
        RuleExpectation {
            id: "cancel",
            input_states: &[],
            output_states: &[],
            allowed_modeled_exits: &["cancel"],
            rust_module: "crates/compiler/kobo-proof/src/async_model.rs",
            rust_verifier: "verify_cancel_edges",
            lean_rule: "step_cancel",
            lean_theorem: "preservation_cancel",
            rust_event_kinds: &[],
        },
        RuleExpectation {
            id: "panic",
            input_states: &[],
            output_states: &[],
            allowed_modeled_exits: &["panic"],
            rust_module: "crates/compiler/kobo-proof/src/obligation.rs",
            rust_verifier: "reject_unresolved_exit::panic",
            lean_rule: "step_panic",
            lean_theorem: "preservation_panic",
            rust_event_kinds: &[],
        },
        RuleExpectation {
            id: "opaque",
            input_states: &["owned"],
            output_states: &["escaped"],
            allowed_modeled_exits: &["opaque_boundary"],
            rust_module: "crates/compiler/kobo-proof/src/boundary.rs",
            rust_verifier: "verify_boundary_policies::Opaque",
            lean_rule: "step_opaque",
            lean_theorem: "preservation_opaque",
            rust_event_kinds: &["escape"],
        },
    ]
}
