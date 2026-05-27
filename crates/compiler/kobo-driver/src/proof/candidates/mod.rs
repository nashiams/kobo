use std::collections::BTreeMap;
use std::path::Path;

use kobo_proof::{
    AdapterEvidence, CandidateAdmissionEvidence, CandidateAdmissionFact, ReplayGrade,
};

mod inspection;

use inspection::{
    attr_has_path_argument, attr_path_arguments, attr_string_field, config_bool, config_string,
    file_attrs_with_path, file_cast_site_count, file_has_attr, file_has_context_binding,
    file_has_hidden_globals, file_has_hidden_impls, file_hidden_heap_site_count,
    file_uses_nightly_features, read_project_config,
};

pub(super) use inspection::syn_path_ends_with;

pub fn candidate_admission_evidence(
    source_path: &Path,
    source: &str,
    replay_grade: ReplayGrade,
    adapter_confidence: &[AdapterEvidence],
) -> Vec<CandidateAdmissionEvidence> {
    let Ok(file) = syn::parse_file(source) else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    for item in &file.items {
        for attr in candidate_attrs(item) {
            let Some(fields) = candidate_attr_fields(attr) else {
                continue;
            };
            let replay_related = bool_field(&fields, "replay_related").unwrap_or(false);
            candidates.push(CandidateAdmissionEvidence {
                id: string_field(&fields, "id").unwrap_or_else(|| "unknown".to_owned()),
                track: string_field(&fields, "track").unwrap_or_else(|| "unknown".to_owned()),
                status: string_field(&fields, "status").unwrap_or_else(|| "research".to_owned()),
                evidence: candidate_evidence_facts(source_path, &file, &fields),
                inspect_visibility: string_field(&fields, "inspect"),
                manual_rust_equivalent: string_field(&fields, "manual_rust"),
                strict_compatible: bool_field(&fields, "strict").unwrap_or(false),
                whole_ecosystem_modeling_required: bool_field(&fields, "whole_ecosystem")
                    .unwrap_or(true),
                diagnostic_snapshots: string_field(&fields, "diagnostic_snapshot")
                    .into_iter()
                    .collect(),
                replay_related,
                replay_grade: replay_related.then_some(replay_grade.clone()),
                adapter_confidence: replay_related
                    .then(|| adapter_confidence.to_vec())
                    .unwrap_or_default(),
            });
        }
    }
    candidates
}

pub(super) fn candidate_attrs(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Fn(item) => &item.attrs,
        syn::Item::Impl(item) => &item.attrs,
        syn::Item::Struct(item) => &item.attrs,
        syn::Item::Mod(item) => &item.attrs,
        syn::Item::Trait(item) => &item.attrs,
        _ => &[],
    }
}

pub(super) fn candidate_attr_fields(attr: &syn::Attribute) -> Option<BTreeMap<String, String>> {
    if !syn_path_ends_with(attr.path(), &["kobo", "candidate_track"]) {
        return None;
    }
    let syn::Meta::List(list) = &attr.meta else {
        return None;
    };
    let entries = list
        .parse_args_with(
            syn::punctuated::Punctuated::<syn::MetaNameValue, syn::Token![,]>::parse_terminated,
        )
        .ok()?;
    let mut fields = BTreeMap::new();
    for entry in entries {
        let key = entry
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())?;
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(value),
            ..
        }) = entry.value
        else {
            continue;
        };
        fields.insert(key, value.value());
    }
    Some(fields)
}

pub(super) fn string_field(fields: &BTreeMap<String, String>, key: &str) -> Option<String> {
    fields.get(key).filter(|value| !value.is_empty()).cloned()
}

pub(super) fn bool_field(fields: &BTreeMap<String, String>, key: &str) -> Option<bool> {
    match fields.get(key)?.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

pub(super) fn candidate_evidence_facts(
    source_path: &Path,
    file: &syn::File,
    fields: &BTreeMap<String, String>,
) -> Vec<CandidateAdmissionFact> {
    const RESERVED: &[&str] = &[
        "id",
        "track",
        "status",
        "inspect",
        "manual_rust",
        "strict",
        "whole_ecosystem",
        "diagnostic_snapshot",
        "replay_related",
    ];
    const DERIVED: &[&str] = &[
        "target_rust",
        "avoids_nightly",
        "no_hidden_heap",
        "allocation_report",
        "memory_budget",
        "hidden_heap_sites",
        "cast_policy",
        "debt_casts",
        "strict_casts",
        "transform_set",
        "desugaring",
        "coherence",
        "hidden_impls",
        "context_threading",
        "hidden_globals",
        "stable_semantics",
        "temporal_extension",
        "adapter_scope",
        "adapter_treadmill",
        "minimization_proof",
        "backend_user_theory",
        "backend_assumptions",
    ];
    let mut facts = fields
        .iter()
        .filter(|(key, _)| !RESERVED.contains(&key.as_str()))
        .filter(|(key, _)| !DERIVED.contains(&key.as_str()))
        .map(|(key, value)| CandidateAdmissionFact {
            key: key.clone(),
            value: value.clone(),
        })
        .collect::<Vec<_>>();
    facts.extend(derived_candidate_facts(source_path, file, fields));
    facts.sort_by(|left, right| left.key.cmp(&right.key));
    facts.dedup_by(|left, right| left.key == right.key);
    facts
}

pub(super) fn derived_candidate_facts(
    source_path: &Path,
    file: &syn::File,
    fields: &BTreeMap<String, String>,
) -> Vec<CandidateAdmissionFact> {
    let mut facts = Vec::new();
    let candidate_id = fields.get("id").map(String::as_str).unwrap_or("");
    let config = read_project_config(source_path);
    match candidate_id {
        "S-32" => {
            if let Some(target_rust) = config_string(&config, &["output", "target_rust"]) {
                facts.push(candidate_fact("target_rust", target_rust));
                facts.push(candidate_fact(
                    "avoids_nightly",
                    (!file_uses_nightly_features(file)).to_string(),
                ));
            }
        }
        "S-49" => {
            if config_bool(&config, &["output", "no_std"]).unwrap_or(false) {
                let hidden_heap_sites = file_hidden_heap_site_count(file);
                facts.push(candidate_fact(
                    "no_hidden_heap",
                    (hidden_heap_sites == 0).to_string(),
                ));
                facts.push(candidate_fact("allocation_report", "structural"));
                facts.push(candidate_fact(
                    "hidden_heap_sites",
                    hidden_heap_sites.to_string(),
                ));
                if let Some(memory_budget) = config_string(&config, &["output", "memory_budget"]) {
                    facts.push(candidate_fact("memory_budget", memory_budget));
                }
            }
        }
        "S-37+" => {
            if let Some(policy) = config_string(&config, &["casts", "policy"]) {
                let cast_count = file_cast_site_count(file);
                facts.push(candidate_fact("cast_policy", policy));
                if cast_count > 0 {
                    facts.push(candidate_fact("debt_casts", "source_spans"));
                    facts.push(candidate_fact("strict_casts", "raw-casts-present"));
                } else {
                    facts.push(candidate_fact("debt_casts", "none"));
                    facts.push(candidate_fact("strict_casts", "explicit"));
                }
            }
        }
        "S-46" => {
            let transform_set = file_attrs_with_path(file, &["kobo", "sugar"])
                .into_iter()
                .flat_map(attr_path_arguments)
                .filter(|argument| {
                    matches!(
                        argument.as_str(),
                        "builder" | "visitor" | "state_machine" | "event_enum"
                    )
                })
                .collect::<Vec<_>>();
            if !transform_set.is_empty() {
                let transform_set = ["builder", "visitor", "state_machine", "event_enum"]
                    .into_iter()
                    .filter(|token| transform_set.iter().any(|argument| argument == token))
                    .collect::<Vec<_>>()
                    .join("|");
                facts.push(candidate_fact("transform_set", transform_set));
                facts.push(candidate_fact("desugaring", "inspectable"));
            }
        }
        "S-47" => {
            if !file_attrs_with_path(file, &["kobo", "newtype_scaffold"]).is_empty() {
                facts.push(candidate_fact("coherence", "newtype_forwarding"));
                facts.push(candidate_fact(
                    "hidden_impls",
                    file_has_hidden_impls(file).to_string(),
                ));
            }
        }
        "S-48" => {
            let hidden_globals = file_has_hidden_globals(file);
            if file_has_context_binding(file) || hidden_globals {
                facts.push(candidate_fact("context_threading", "explicit"));
                facts.push(candidate_fact("hidden_globals", hidden_globals.to_string()));
            }
        }
        "research-smt-temporal" => {
            if file_has_attr(file, &["kobo", "ward"]) && file_has_attr(file, &["kobo", "invariant"])
            {
                facts.push(candidate_fact("stable_semantics", "ward|invariant"));
            }
            if file_attrs_with_path(file, &["kobo", "temporal"])
                .into_iter()
                .any(|attr| {
                    attr_string_field(attr, "query").as_deref()
                        == Some("beyond_always_eventually_never")
                })
            {
                facts.push(candidate_fact(
                    "temporal_extension",
                    "beyond_always_eventually_never",
                ));
            }
        }
        "research-broad-adapters" => {
            if let Some(scope) = config_string(&config, &["adapters", "scope"]) {
                facts.push(candidate_fact("adapter_scope", scope));
            }
            if let Some(treadmill) = config_string(&config, &["adapters", "treadmill"]) {
                facts.push(candidate_fact("adapter_treadmill", treadmill));
            }
        }
        "research-model-checking" => {
            if file_attrs_with_path(file, &["kobo", "witness_minimization"])
                .into_iter()
                .any(|attr| attr_has_path_argument(attr, "labeled_trace"))
            {
                facts.push(candidate_fact("minimization_proof", "labeled_trace"));
            }
            for attr in file_attrs_with_path(file, &["kobo", "model_check"]) {
                if let Some(theory) = attr_string_field(attr, "backend_user_theory") {
                    if theory == "kobo_core_loop" || theory == "z3_smt" {
                        facts.push(candidate_fact("backend_user_theory", theory));
                    }
                }
                if attr_string_field(attr, "backend_assumptions").as_deref() == Some("ledger") {
                    facts.push(candidate_fact("backend_assumptions", "ledger"));
                }
            }
        }
        _ => {}
    }
    facts
}

pub(super) fn candidate_fact(key: &str, value: impl Into<String>) -> CandidateAdmissionFact {
    CandidateAdmissionFact {
        key: key.to_owned(),
        value: value.into(),
    }
}
