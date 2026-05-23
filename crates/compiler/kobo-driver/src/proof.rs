use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use crate::config::EcosystemAdapterPolicy;
use kobo_ir::{
    lower_core_program, CoreBlock, CoreFunction, CoreStatement, CoreStatementKind,
    CoreTerminatorKind, KoboSpan, ScenarioLifecycleTemplateSource, ScenarioOpKind, ScenarioProgram,
};
use kobo_proof::{
    certificate_material_hash, core_material_hash, stable_hash, template_version_hash,
    AdapterConfidence, AdapterEvidence, AsyncModelEvidence, BoundaryAssumption, BoundaryPolicy,
    CancelEdgeEvidence, CandidateAdmissionEvidence, CandidateAdmissionFact, CoreCfgEdge,
    CoreCfgNode, CoreEvidence, CoverageLoss, FunctionSummary, FutureStateLocalEvidence,
    FutureStateObligationEvidence, HashEvidence, ObligationEvent, ObligationEventKind,
    ObligationState, ObligationStatus, OpaqueLedgerEntry, ProofCertificate, SelectPathEvidence,
    SourceEvidence, SourceSpan, SpawnedTaskObligationEvidence, SuspensionStateEvidence,
    TemplateVersionEvidence, TimeoutCancelEdgeEvidence,
};

pub use kobo_proof::{ArtifactKind, ReplayGrade};

pub struct ProofEmissionInput<'a> {
    pub source_path: &'a Path,
    pub source: &'a str,
    pub program: &'a ScenarioProgram,
    pub adapter_policies: &'a [EcosystemAdapterPolicy],
    pub replay_grade: ReplayGrade,
    pub artifact_kind: ArtifactKind,
}

#[derive(Debug, thiserror::Error)]
pub enum ProofEmissionError {
    #[error("failed to serialize proof material: {0}")]
    Serialize(#[from] serde_json::Error),
}

pub fn emit_proof_certificate(
    input: ProofEmissionInput<'_>,
) -> Result<ProofCertificate, ProofEmissionError> {
    let source_path = input.source_path.display().to_string();
    let core_program = lower_core_program(input.program);
    let cfg_nodes = core_cfg_nodes(&source_path, input.source, &core_program.functions);
    let cfg_edges = core_cfg_edges(&source_path, input.source, &core_program.functions);
    let (template_hashes, template_versions) =
        template_evidence(&source_path, input.source, input.program)?;
    let (boundary_assumption_hashes, boundary_assumptions, opaque_edge_ledger) =
        boundary_evidence(&source_path, input.source, input.program)?;
    let (entry_env, exit_env, obligation_events) =
        obligation_evidence(&source_path, input.source, &core_program.functions);
    let async_model = async_model_evidence(
        &source_path,
        input.source,
        input.program,
        &core_program.functions,
    );
    let core_hash = core_material_hash(
        core_program.core_version,
        &cfg_nodes,
        &cfg_edges,
        &async_model,
    )?;
    let mut adapter_confidence = adapter_evidence(
        input.program,
        input.adapter_policies,
        input.replay_grade.clone(),
    );
    let replay_grade = adapter_adjusted_replay_grade(input.replay_grade, &adapter_confidence);
    for adapter in &mut adapter_confidence {
        adapter.replay_grade = replay_grade.clone();
    }
    let candidate_admission = candidate_admission_evidence(
        input.source_path,
        input.source,
        replay_grade.clone(),
        &adapter_confidence,
    );
    let function_summaries = function_summaries(
        input.program,
        &entry_env,
        &exit_env,
        obligation_events.len(),
    );
    let coverage_loss = coverage_loss(input.program);

    let mut certificate = ProofCertificate {
        schema_version: 1,
        proof_target_version: "kobo-core-obligation-flow-1".to_owned(),
        semantic_schema: ".kproof".to_owned(),
        artifact_kind: input.artifact_kind,
        claim_scope: "modeled_core_obligation_flow_only".to_owned(),
        compiler_version: env!("CARGO_PKG_VERSION").to_owned(),
        source: SourceEvidence {
            path: source_path,
            hash: stable_hash(input.source),
        },
        core: CoreEvidence {
            hash: core_hash,
            version: core_program.core_version.to_owned(),
            cfg_nodes,
            cfg_edges,
            async_model,
        },
        replay_grade,
        template_hashes,
        template_versions,
        boundary_assumption_hashes,
        boundary_assumptions,
        adapter_confidence,
        obligation_events,
        entry_env,
        exit_env,
        function_summaries,
        coverage_loss,
        opaque_edge_ledger,
        candidate_admission,
        certificate_material_hash: String::new(),
    };
    certificate.certificate_material_hash = certificate_material_hash(&certificate)?;
    Ok(certificate)
}

fn core_cfg_nodes(source_path: &str, source: &str, functions: &[CoreFunction]) -> Vec<CoreCfgNode> {
    functions
        .iter()
        .flat_map(|function| {
            function.blocks.iter().map(move |block| CoreCfgNode {
                id: block.id.clone(),
                function: function.name.clone(),
                source_span: source_span_from_kobo(source_path, source, block_span(block)),
            })
        })
        .collect()
}

fn core_cfg_edges(source_path: &str, source: &str, functions: &[CoreFunction]) -> Vec<CoreCfgEdge> {
    functions
        .iter()
        .flat_map(|function| {
            function.blocks.iter().flat_map(move |block| {
                block.terminators.iter().flat_map(move |terminator| {
                    terminator
                        .edges
                        .iter()
                        .enumerate()
                        .map(move |(edge_index, edge)| CoreCfgEdge {
                            id: format!(
                                "{}:{}:{}:{edge_index}",
                                function.name, block.id, terminator.id
                            ),
                            function: function.name.clone(),
                            from: block.id.clone(),
                            to: edge
                                .strip_prefix("goto:")
                                .unwrap_or(edge.as_str())
                                .to_owned(),
                            kind: terminator.kind.as_str().to_owned(),
                            source_span: source_span_from_kobo(
                                source_path,
                                source,
                                terminator.source_span,
                            ),
                        })
                })
            })
        })
        .collect()
}

fn block_span(block: &CoreBlock) -> KoboSpan {
    block
        .statements
        .first()
        .map(|statement| statement.source_span)
        .or_else(|| {
            block
                .terminators
                .first()
                .map(|terminator| terminator.source_span)
        })
        .unwrap_or(KoboSpan {
            file_id: kobo_ir::FileId(0),
            start: 0,
            end: 0,
        })
}

fn template_evidence(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
) -> Result<(Vec<HashEvidence>, Vec<TemplateVersionEvidence>), serde_json::Error> {
    let mut hashes = Vec::new();
    let mut versions = Vec::new();
    for operation in &program.operations {
        let ScenarioOpKind::CreateObligation {
            template: Some(template),
            ..
        } = &operation.kind
        else {
            continue;
        };
        let template_source = match template.source {
            ScenarioLifecycleTemplateSource::Declaration => "declaration",
            ScenarioLifecycleTemplateSource::Inference => "inference",
        };
        let version = TemplateVersionEvidence {
            id: template.id.clone(),
            kind: template.kind.clone(),
            version: template.version.clone(),
            confidence: template.confidence.clone(),
            source: template_source.to_owned(),
            source_span: source_span_from_kobo(source_path, source, operation.span),
        };
        let hash = template_version_hash(&version)?;
        hashes.push(HashEvidence {
            id: template.id.clone(),
            hash,
        });
        versions.push(version);
    }
    Ok((hashes, versions))
}

fn boundary_evidence(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
) -> Result<
    (
        Vec<HashEvidence>,
        Vec<BoundaryAssumption>,
        Vec<OpaqueLedgerEntry>,
    ),
    serde_json::Error,
> {
    let mut hashes = Vec::new();
    let mut assumptions = Vec::new();
    let mut opaque_ledger = Vec::new();
    for (index, operation) in program.operations.iter().enumerate() {
        let ScenarioOpKind::ExternalBoundary {
            crate_name,
            policy,
            reason,
            ..
        } = &operation.kind
        else {
            continue;
        };
        let id = format!("boundary-{index}-{crate_name}");
        let assumption = BoundaryAssumption {
            id: id.clone(),
            boundary: crate_name.clone(),
            policy: boundary_policy(policy),
            reason: reason.clone(),
            source_span: source_span_from_kobo(source_path, source, operation.span),
        };
        let hash = serde_json::to_string(&assumption).map(|material| stable_hash(&material))?;
        hashes.push(HashEvidence {
            id: id.clone(),
            hash: hash.clone(),
        });
        if matches!(policy, kobo_ir::ScenarioBoundaryPolicy::Opaque) {
            opaque_ledger.push(OpaqueLedgerEntry {
                edge_id: id.clone(),
                boundary: crate_name.clone(),
                evidence_hash: hash,
            });
        }
        assumptions.push(assumption);
    }
    Ok((hashes, assumptions, opaque_ledger))
}

pub fn adapter_evidence(
    program: &ScenarioProgram,
    adapter_policies: &[EcosystemAdapterPolicy],
    requested_replay_grade: ReplayGrade,
) -> Vec<AdapterEvidence> {
    let mut evidence = Vec::new();
    for operation in &program.operations {
        let ScenarioOpKind::ExternalBoundary {
            crate_name, policy, ..
        } = &operation.kind
        else {
            continue;
        };
        let Some(adapter) = adapter_policies
            .iter()
            .find(|adapter| adapter.crate_name == *crate_name)
        else {
            continue;
        };
        let confidence = adapter_confidence(adapter, policy);
        let outcome = adapter_outcome(adapter, &confidence);
        evidence.push(AdapterEvidence {
            boundary: crate_name.clone(),
            adapter: adapter.package.clone(),
            version: adapter.version.clone(),
            confidence,
            replay_grade: requested_replay_grade.clone(),
            outcome,
            reason: adapter
                .reason
                .clone()
                .unwrap_or_else(|| "configured ecosystem adapter".to_owned()),
        });
    }
    evidence.sort_by(|left, right| {
        (&left.boundary, &left.adapter, &left.version).cmp(&(
            &right.boundary,
            &right.adapter,
            &right.version,
        ))
    });
    evidence.dedup_by(|left, right| {
        left.boundary == right.boundary
            && left.adapter == right.adapter
            && left.version == right.version
    });
    evidence
}

pub fn adapter_adjusted_replay_grade(
    requested_replay_grade: ReplayGrade,
    adapters: &[AdapterEvidence],
) -> ReplayGrade {
    if matches!(
        requested_replay_grade,
        ReplayGrade::Debt | ReplayGrade::NotReplayable
    ) {
        return requested_replay_grade;
    }
    if adapters.iter().any(adapter_is_stale) {
        return ReplayGrade::Debt;
    }
    if adapters.iter().any(|adapter| {
        matches!(
            adapter.confidence,
            AdapterConfidence::Sampled | AdapterConfidence::MetadataOnly
        )
    }) {
        return ReplayGrade::NotReplayable;
    }
    if requested_replay_grade == ReplayGrade::Exact
        && adapters
            .iter()
            .any(|adapter| adapter.confidence == AdapterConfidence::Modeled)
    {
        return ReplayGrade::Partial;
    }
    requested_replay_grade
}

fn adapter_confidence(
    adapter: &EcosystemAdapterPolicy,
    policy: &kobo_ir::ScenarioBoundaryPolicy,
) -> AdapterConfidence {
    match adapter.confidence.as_deref() {
        Some("exact") => AdapterConfidence::Exact,
        Some("modeled") => AdapterConfidence::Modeled,
        Some("sampled") => AdapterConfidence::Sampled,
        Some("metadata-only") | Some("metadata_only") => AdapterConfidence::MetadataOnly,
        _ if !adapter.validated => AdapterConfidence::MetadataOnly,
        _ if adapter.capture.as_deref() == Some("boundary-io")
            && matches!(policy, kobo_ir::ScenarioBoundaryPolicy::Record) =>
        {
            AdapterConfidence::Exact
        }
        _ if adapter.adapter_runtime.is_some() => AdapterConfidence::Modeled,
        _ => AdapterConfidence::MetadataOnly,
    }
}

fn adapter_outcome(adapter: &EcosystemAdapterPolicy, confidence: &AdapterConfidence) -> String {
    if adapter_version_is_stale(adapter.version.as_deref()) {
        return "debt".to_owned();
    }
    match confidence {
        AdapterConfidence::Exact | AdapterConfidence::Modeled => "proof".to_owned(),
        AdapterConfidence::Sampled => "probing_pass".to_owned(),
        AdapterConfidence::MetadataOnly => "metadata_only".to_owned(),
    }
}

fn adapter_is_stale(adapter: &AdapterEvidence) -> bool {
    adapter_version_is_stale(adapter.version.as_deref()) || adapter.outcome == "debt"
}

fn adapter_version_is_stale(version: Option<&str>) -> bool {
    let Some(version) = version else {
        return true;
    };
    version == "0.0.0" || version.contains("stale")
}

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

fn candidate_attrs(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Fn(item) => &item.attrs,
        syn::Item::Impl(item) => &item.attrs,
        syn::Item::Struct(item) => &item.attrs,
        syn::Item::Mod(item) => &item.attrs,
        syn::Item::Trait(item) => &item.attrs,
        _ => &[],
    }
}

fn candidate_attr_fields(attr: &syn::Attribute) -> Option<BTreeMap<String, String>> {
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

fn string_field(fields: &BTreeMap<String, String>, key: &str) -> Option<String> {
    fields.get(key).filter(|value| !value.is_empty()).cloned()
}

fn bool_field(fields: &BTreeMap<String, String>, key: &str) -> Option<bool> {
    match fields.get(key)?.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn candidate_evidence_facts(
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

fn derived_candidate_facts(
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

fn candidate_fact(key: &str, value: impl Into<String>) -> CandidateAdmissionFact {
    CandidateAdmissionFact {
        key: key.to_owned(),
        value: value.into(),
    }
}

fn read_project_config(source_path: &Path) -> Option<toml::Value> {
    for directory in source_path.parent().into_iter().flat_map(Path::ancestors) {
        let config_path = directory.join("Kobo.toml");
        let Ok(source) = std::fs::read_to_string(&config_path) else {
            continue;
        };
        if let Ok(config) = source.parse::<toml::Value>() {
            return Some(config);
        }
    }
    None
}

fn config_string(config: &Option<toml::Value>, path: &[&str]) -> Option<String> {
    let mut current = config.as_ref()?;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(str::to_owned)
}

fn config_bool(config: &Option<toml::Value>, path: &[&str]) -> Option<bool> {
    let mut current = config.as_ref()?;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_bool()
}

fn file_uses_nightly_features(file: &syn::File) -> bool {
    file.attrs
        .iter()
        .any(|attr| syn_path_ends_with(attr.path(), &["feature"]))
}

fn file_hidden_heap_site_count(file: &syn::File) -> usize {
    struct HiddenHeapVisitor {
        count: usize,
    }

    impl<'ast> syn::visit::Visit<'ast> for HiddenHeapVisitor {
        fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
            if let syn::Expr::Path(function) = call.func.as_ref() {
                if path_is_heap_constructor(&function.path) {
                    self.count += 1;
                }
            }
            syn::visit::visit_expr_call(self, call);
        }

        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            if matches!(
                call.method.to_string().as_str(),
                "with_capacity" | "try_with_capacity" | "collect" | "to_vec" | "to_string"
            ) {
                self.count += 1;
            }
            syn::visit::visit_expr_method_call(self, call);
        }

        fn visit_macro(&mut self, mac: &'ast syn::Macro) {
            if path_last_ident_is(&mac.path, "vec") || path_last_ident_is(&mac.path, "format") {
                self.count += 1;
            }
            syn::visit::visit_macro(self, mac);
        }
    }

    let mut visitor = HiddenHeapVisitor { count: 0 };
    syn::visit::visit_file(&mut visitor, file);
    visitor.count
}

fn file_cast_site_count(file: &syn::File) -> usize {
    struct CastVisitor {
        count: usize,
    }

    impl<'ast> syn::visit::Visit<'ast> for CastVisitor {
        fn visit_expr_cast(&mut self, cast: &'ast syn::ExprCast) {
            self.count += 1;
            syn::visit::visit_expr_cast(self, cast);
        }
    }

    let mut visitor = CastVisitor { count: 0 };
    syn::visit::visit_file(&mut visitor, file);
    visitor.count
}

fn file_has_hidden_impls(file: &syn::File) -> bool {
    file.items.iter().any(item_has_hidden_impl)
}

fn item_has_hidden_impl(item: &syn::Item) -> bool {
    match item {
        syn::Item::Impl(item) => {
            item.attrs
                .iter()
                .any(|attr| syn_path_ends_with(attr.path(), &["hidden_impl"]))
                || item.trait_.as_ref().is_some_and(|(_, trait_path, _)| {
                    path_has_segment(trait_path, "external")
                        || path_has_segment(trait_path, "ExternalTrait")
                })
                || type_has_segment(item.self_ty.as_ref(), "external")
                || type_has_segment(item.self_ty.as_ref(), "ExternalType")
        }
        syn::Item::Mod(item) => item
            .content
            .as_ref()
            .is_some_and(|(_, items)| items.iter().any(item_has_hidden_impl)),
        _ => false,
    }
}

fn file_has_hidden_globals(file: &syn::File) -> bool {
    struct HiddenGlobalVisitor {
        found: bool,
    }

    impl<'ast> syn::visit::Visit<'ast> for HiddenGlobalVisitor {
        fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
            if matches!(item.mutability, syn::StaticMutability::Mut(_)) {
                self.found = true;
            }
            syn::visit::visit_item_static(self, item);
        }

        fn visit_macro(&mut self, mac: &'ast syn::Macro) {
            if path_last_ident_is(&mac.path, "lazy_static")
                || path_last_ident_is(&mac.path, "thread_local")
            {
                self.found = true;
            }
            syn::visit::visit_macro(self, mac);
        }
    }

    let mut visitor = HiddenGlobalVisitor { found: false };
    syn::visit::visit_file(&mut visitor, file);
    visitor.found
}

fn file_has_context_binding(file: &syn::File) -> bool {
    struct ContextBindingVisitor {
        found: bool,
    }

    impl<'ast> syn::visit::Visit<'ast> for ContextBindingVisitor {
        fn visit_pat_ident(&mut self, pat: &'ast syn::PatIdent) {
            if pat.ident == "ctx" {
                self.found = true;
            }
            syn::visit::visit_pat_ident(self, pat);
        }
    }

    let mut visitor = ContextBindingVisitor { found: false };
    syn::visit::visit_file(&mut visitor, file);
    visitor.found
}

fn file_has_attr(file: &syn::File, suffix: &[&str]) -> bool {
    !file_attrs_with_path(file, suffix).is_empty()
}

fn file_attrs_with_path<'a>(file: &'a syn::File, suffix: &[&str]) -> Vec<&'a syn::Attribute> {
    let mut attrs = Vec::new();
    attrs.extend(
        file.attrs
            .iter()
            .filter(|attr| syn_path_ends_with(attr.path(), suffix)),
    );
    for item in &file.items {
        collect_item_attrs_with_path(item, suffix, &mut attrs);
    }
    attrs
}

fn collect_item_attrs_with_path<'a>(
    item: &'a syn::Item,
    suffix: &[&str],
    attrs: &mut Vec<&'a syn::Attribute>,
) {
    attrs.extend(
        item_attributes(item)
            .iter()
            .filter(|attr| syn_path_ends_with(attr.path(), suffix)),
    );
    match item {
        syn::Item::Impl(item) => {
            for impl_item in &item.items {
                attrs.extend(
                    impl_item_attributes(impl_item)
                        .iter()
                        .filter(|attr| syn_path_ends_with(attr.path(), suffix)),
                );
            }
        }
        syn::Item::Mod(item) => {
            if let Some((_, items)) = &item.content {
                for item in items {
                    collect_item_attrs_with_path(item, suffix, attrs);
                }
            }
        }
        _ => {}
    }
}

fn item_attributes(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Const(item) => &item.attrs,
        syn::Item::Enum(item) => &item.attrs,
        syn::Item::Fn(item) => &item.attrs,
        syn::Item::Impl(item) => &item.attrs,
        syn::Item::Mod(item) => &item.attrs,
        syn::Item::Static(item) => &item.attrs,
        syn::Item::Struct(item) => &item.attrs,
        syn::Item::Trait(item) => &item.attrs,
        syn::Item::Type(item) => &item.attrs,
        syn::Item::Union(item) => &item.attrs,
        _ => &[],
    }
}

fn impl_item_attributes(item: &syn::ImplItem) -> &[syn::Attribute] {
    match item {
        syn::ImplItem::Const(item) => &item.attrs,
        syn::ImplItem::Fn(item) => &item.attrs,
        syn::ImplItem::Type(item) => &item.attrs,
        _ => &[],
    }
}

fn attr_path_arguments(attr: &syn::Attribute) -> Vec<String> {
    let syn::Meta::List(list) = &attr.meta else {
        return Vec::new();
    };
    list.parse_args_with(syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)
        .map(|entries| {
            entries
                .into_iter()
                .filter_map(|entry| match entry {
                    syn::Meta::Path(path) => path
                        .segments
                        .last()
                        .map(|segment| segment.ident.to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn attr_has_path_argument(attr: &syn::Attribute, argument: &str) -> bool {
    attr_path_arguments(attr)
        .iter()
        .any(|candidate| candidate == argument)
}

fn attr_string_field(attr: &syn::Attribute, key: &str) -> Option<String> {
    let syn::Meta::List(list) = &attr.meta else {
        return None;
    };
    let entries = list
        .parse_args_with(
            syn::punctuated::Punctuated::<syn::MetaNameValue, syn::Token![,]>::parse_terminated,
        )
        .ok()?;
    for entry in entries {
        let field = entry
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())?;
        if field != key {
            continue;
        }
        if let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(value),
            ..
        }) = entry.value
        {
            return Some(value.value());
        }
    }
    None
}

fn path_is_heap_constructor(path: &syn::Path) -> bool {
    let last = path
        .segments
        .last()
        .map(|segment| segment.ident.to_string());
    let Some(last) = last else {
        return false;
    };
    matches!(
        last.as_str(),
        "new" | "from" | "with_capacity" | "try_with_capacity"
    ) && (path_has_segment(path, "Vec")
        || path_has_segment(path, "Box")
        || path_has_segment(path, "String")
        || path_has_segment(path, "alloc"))
}

fn path_last_ident_is(path: &syn::Path, expected: &str) -> bool {
    path.segments
        .last()
        .is_some_and(|segment| segment.ident == expected)
}

fn path_has_segment(path: &syn::Path, expected: &str) -> bool {
    path.segments
        .iter()
        .any(|segment| segment.ident == expected)
}

fn type_has_segment(ty: &syn::Type, expected: &str) -> bool {
    match ty {
        syn::Type::Path(ty) => path_has_segment(&ty.path, expected),
        syn::Type::Reference(ty) => type_has_segment(&ty.elem, expected),
        syn::Type::Group(ty) => type_has_segment(&ty.elem, expected),
        syn::Type::Paren(ty) => type_has_segment(&ty.elem, expected),
        _ => false,
    }
}

fn syn_path_ends_with(path: &syn::Path, suffix: &[&str]) -> bool {
    let segments = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    segments.len() >= suffix.len()
        && segments[segments.len() - suffix.len()..]
            .iter()
            .zip(suffix)
            .all(|(left, right)| left == right)
}

fn obligation_evidence(
    source_path: &str,
    source: &str,
    functions: &[CoreFunction],
) -> (
    Vec<ObligationState>,
    Vec<ObligationState>,
    Vec<ObligationEvent>,
) {
    let entry_env = env_states(&BTreeMap::new());
    let mut events = Vec::new();
    let mut terminal_envs = Vec::new();
    for function in functions {
        let replay = function_obligation_replay(function);
        for block in &function.blocks {
            let mut current_env = replay
                .block_entry_envs
                .get(&block.id)
                .cloned()
                .unwrap_or_default();
            for statement in &block.statements {
                let before = env_states(&current_env);
                apply_obligation_statement(statement, &mut current_env);
                let after = env_states(&current_env);
                if let Some(kind) = obligation_event_kind(&statement.kind) {
                    events.push(ObligationEvent {
                        id: statement.id.clone(),
                        kind,
                        binding: statement.binding.clone(),
                        action: statement.action.clone(),
                        source_span: source_span_from_kobo(
                            source_path,
                            source,
                            statement.source_span,
                        ),
                        state_before: before,
                        state_after: after,
                    });
                }
            }
        }
        terminal_envs.extend(replay.terminal_envs);
    }
    let exit_env = env_states(&merge_terminal_envs(&terminal_envs));
    (entry_env, exit_env, events)
}

struct FunctionObligationReplay {
    block_entry_envs: BTreeMap<String, BTreeMap<String, ObligationStatus>>,
    block_exit_envs: BTreeMap<String, BTreeMap<String, ObligationStatus>>,
    terminal_envs: Vec<BTreeMap<String, ObligationStatus>>,
}

fn function_obligation_replay(function: &CoreFunction) -> FunctionObligationReplay {
    let blocks = function
        .blocks
        .iter()
        .map(|block| (block.id.as_str(), block))
        .collect::<BTreeMap<_, _>>();
    let Some(entry_block) = function.blocks.first() else {
        return FunctionObligationReplay {
            block_entry_envs: BTreeMap::new(),
            block_exit_envs: BTreeMap::new(),
            terminal_envs: Vec::new(),
        };
    };

    let mut block_entry_envs = BTreeMap::<String, BTreeMap<String, ObligationStatus>>::new();
    let mut queued = VecDeque::new();
    block_entry_envs.insert(entry_block.id.clone(), BTreeMap::new());
    queued.push_back(entry_block.id.clone());

    while let Some(block_id) = queued.pop_front() {
        let Some(block) = blocks.get(block_id.as_str()).copied() else {
            continue;
        };
        let entry_env = block_entry_envs.get(&block.id).cloned().unwrap_or_default();
        let exit_env = apply_block_obligation_statements(block, entry_env);
        for target in block_successor_targets(block) {
            let Some(target_block) = blocks.get(target.as_str()) else {
                continue;
            };
            let changed = merge_block_entry_env(
                block_entry_envs
                    .entry(target_block.id.clone())
                    .or_insert_with(BTreeMap::new),
                &exit_env,
            );
            if changed {
                queued.push_back(target_block.id.clone());
            }
        }
    }

    let block_exit_envs = function
        .blocks
        .iter()
        .filter_map(|block| {
            block_entry_envs.get(&block.id).cloned().map(|entry_env| {
                (
                    block.id.clone(),
                    apply_block_obligation_statements(block, entry_env),
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    let terminal_envs = function
        .blocks
        .iter()
        .filter(|block| block_successor_targets(block).is_empty())
        .filter_map(|block| block_exit_envs.get(&block.id).cloned())
        .collect::<Vec<_>>();

    FunctionObligationReplay {
        block_entry_envs,
        block_exit_envs,
        terminal_envs,
    }
}

fn apply_block_obligation_statements(
    block: &CoreBlock,
    mut env: BTreeMap<String, ObligationStatus>,
) -> BTreeMap<String, ObligationStatus> {
    for statement in &block.statements {
        apply_obligation_statement(statement, &mut env);
    }
    env
}

fn block_successor_targets(block: &CoreBlock) -> Vec<String> {
    block
        .terminators
        .iter()
        .flat_map(|terminator| terminator.edges.iter())
        .filter_map(|edge| edge.strip_prefix("goto:").map(str::to_owned))
        .collect()
}

fn merge_block_entry_env(
    current: &mut BTreeMap<String, ObligationStatus>,
    incoming: &BTreeMap<String, ObligationStatus>,
) -> bool {
    let before = current.clone();
    for (binding, incoming_status) in incoming {
        match current.get(binding) {
            Some(current_status) if current_status == incoming_status => {}
            Some(_) => {
                current.insert(binding.clone(), ObligationStatus::BranchUnresolved);
            }
            None => {
                current.insert(binding.clone(), incoming_status.clone());
            }
        }
    }
    *current != before
}

fn merge_terminal_envs(
    terminal_envs: &[BTreeMap<String, ObligationStatus>],
) -> BTreeMap<String, ObligationStatus> {
    let mut merged = BTreeMap::new();
    for env in terminal_envs {
        merge_block_entry_env(&mut merged, env);
    }
    merged
}

fn async_model_evidence(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
    functions: &[CoreFunction],
) -> AsyncModelEvidence {
    let live_locals_by_await = parsed_live_locals_by_await(source, &program.target);
    let mut model = AsyncModelEvidence::default();

    for function in functions {
        let replay = function_obligation_replay(function);
        let mut await_index = 0usize;
        for block in &function.blocks {
            let block_exit_env = replay
                .block_exit_envs
                .get(&block.id)
                .cloned()
                .unwrap_or_default();
            for terminator in &block.terminators {
                match terminator.kind {
                    CoreTerminatorKind::Await => {
                        let suspension_id =
                            format!("{}:{}:{}", function.name, block.id, terminator.id);
                        let resume_edge = terminator
                            .edges
                            .iter()
                            .find(|edge| edge.as_str() == "await_resume")
                            .cloned()
                            .unwrap_or_else(|| "await_resume".to_owned());
                        let cancel_edge = terminator
                            .edges
                            .iter()
                            .find(|edge| edge.as_str() == "await_cancel")
                            .cloned()
                            .unwrap_or_else(|| "await_cancel".to_owned());
                        let source_span =
                            source_span_from_kobo(source_path, source, terminator.source_span);
                        model.suspension_states.push(SuspensionStateEvidence {
                            id: suspension_id.clone(),
                            function: function.name.clone(),
                            block: block.id.clone(),
                            terminator_kind: "await".to_owned(),
                            boundary: terminator.boundary.clone(),
                            resume_edge,
                            cancel_edge: cancel_edge.clone(),
                            source_span: source_span.clone(),
                        });
                        model.cancel_edges.push(CancelEdgeEvidence {
                            id: format!("{suspension_id}:future-drop"),
                            function: function.name.clone(),
                            from: block.id.clone(),
                            to: cancel_edge.clone(),
                            reason: "future_drop".to_owned(),
                            source_span: source_span.clone(),
                        });
                        let live_locals = live_locals_by_await
                            .get(await_index)
                            .cloned()
                            .unwrap_or_default();
                        await_index += 1;
                        for local in &live_locals {
                            model.future_state_locals.push(FutureStateLocalEvidence {
                                binding: local.clone(),
                                suspension_state: suspension_id.clone(),
                                source_span: source_span_for_binding(source_path, source, local),
                            });
                        }
                        for state in env_states(&block_exit_env)
                            .into_iter()
                            .filter(|state| state.state == ObligationStatus::Owned)
                        {
                            model
                                .future_state_obligations
                                .push(FutureStateObligationEvidence {
                                    binding: state.binding,
                                    state: state.state,
                                    suspension_state: suspension_id.clone(),
                                    source_span: source_span.clone(),
                                });
                        }
                        if terminator.boundary.as_deref() == Some("tokio::time::timeout") {
                            model.timeout_cancel_edges.push(TimeoutCancelEdgeEvidence {
                                id: format!("{suspension_id}:timeout"),
                                function: function.name.clone(),
                                suspension_state: suspension_id,
                                source: "tokio::time::timeout".to_owned(),
                                cancel_edge,
                                source_span,
                            });
                        }
                    }
                    CoreTerminatorKind::Branch => {
                        let source_span =
                            source_span_from_kobo(source_path, source, terminator.source_span);
                        let cancelled_obligations = env_states(&block_exit_env)
                            .into_iter()
                            .filter(|state| state.state == ObligationStatus::Owned)
                            .collect::<Vec<_>>();
                        for branch_target in terminator
                            .edges
                            .iter()
                            .filter_map(|edge| edge.strip_prefix("goto:"))
                        {
                            let obligation_results = replay
                                .block_exit_envs
                                .get(branch_target)
                                .map(env_states)
                                .unwrap_or_default();
                            for path_kind in ["winner", "loser_cancel"] {
                                let cancelled_obligations = (path_kind == "loser_cancel")
                                    .then(|| cancelled_obligations.clone())
                                    .unwrap_or_default();
                                model.select_paths.push(SelectPathEvidence {
                                    id: format!(
                                        "{}:{}:{}:{branch_target}:{path_kind}",
                                        function.name, block.id, terminator.id
                                    ),
                                    function: function.name.clone(),
                                    branch_block: block.id.clone(),
                                    branch_target: branch_target.to_owned(),
                                    path_kind: path_kind.to_owned(),
                                    obligation_results: obligation_results.clone(),
                                    cancelled_obligations: cancelled_obligations.clone(),
                                    obligation_result_hash: canonical_select_result_hash(
                                        branch_target,
                                        path_kind,
                                        &obligation_results,
                                        &cancelled_obligations,
                                    ),
                                    source_span: source_span.clone(),
                                });
                            }
                        }
                    }
                    CoreTerminatorKind::Goto
                    | CoreTerminatorKind::Return
                    | CoreTerminatorKind::ErrorExit
                    | CoreTerminatorKind::Panic
                    | CoreTerminatorKind::OpaqueBoundary => {}
                }
            }
        }
    }

    for operation in &program.operations {
        let ScenarioOpKind::CreateObligation {
            binding,
            type_name,
            actions,
            template,
        } = &operation.kind
        else {
            continue;
        };
        let is_spawned_task = type_name == "SpawnedTask"
            || template
                .as_ref()
                .is_some_and(|template| template.kind == "spawned_task");
        if is_spawned_task {
            model
                .spawned_task_obligations
                .push(SpawnedTaskObligationEvidence {
                    binding: binding.clone(),
                    required_resolution: actions.clone(),
                    source_span: source_span_from_kobo(source_path, source, operation.span),
                });
        }
    }

    model
}

fn apply_obligation_statement(
    statement: &CoreStatement,
    current_env: &mut BTreeMap<String, ObligationStatus>,
) {
    match statement.kind {
        CoreStatementKind::ObligationCreate => {
            if let Some(binding) = statement.binding.as_ref() {
                current_env.insert(binding.clone(), ObligationStatus::Owned);
            }
        }
        CoreStatementKind::ObligationDischarge => {
            if let Some(binding) = statement.binding.as_ref() {
                current_env.insert(binding.clone(), ObligationStatus::Resolved);
            }
        }
        CoreStatementKind::ObligationTransfer => {
            if let Some(binding) = statement.binding.as_ref() {
                current_env.insert(binding.clone(), ObligationStatus::Transferred);
            }
        }
        CoreStatementKind::ObligationMove => {
            if let Some(binding) = statement.binding.as_ref() {
                current_env.insert(binding.clone(), ObligationStatus::Moved);
            }
        }
        CoreStatementKind::ObligationBranchUnresolved => {
            if let Some(binding) = statement.binding.as_ref() {
                current_env.insert(binding.clone(), ObligationStatus::BranchUnresolved);
            }
        }
        CoreStatementKind::ObligationEscape => {
            if let Some(binding) = statement.binding.as_ref() {
                current_env.insert(binding.clone(), ObligationStatus::Escaped);
            }
        }
        CoreStatementKind::UnsupportedContainer | CoreStatementKind::Call => {}
    }
}

fn obligation_event_kind(kind: &CoreStatementKind) -> Option<ObligationEventKind> {
    match kind {
        CoreStatementKind::ObligationCreate => Some(ObligationEventKind::Create),
        CoreStatementKind::ObligationDischarge => Some(ObligationEventKind::Discharge),
        CoreStatementKind::ObligationTransfer => Some(ObligationEventKind::Transfer),
        CoreStatementKind::ObligationMove => Some(ObligationEventKind::Move),
        CoreStatementKind::ObligationBranchUnresolved => {
            Some(ObligationEventKind::BranchUnresolved)
        }
        CoreStatementKind::ObligationEscape => Some(ObligationEventKind::Escape),
        CoreStatementKind::UnsupportedContainer => Some(ObligationEventKind::UnsupportedContainer),
        CoreStatementKind::Call => Some(ObligationEventKind::Call),
    }
}

fn env_states(env: &BTreeMap<String, ObligationStatus>) -> Vec<ObligationState> {
    env.iter()
        .map(|(binding, state)| ObligationState {
            binding: binding.clone(),
            state: state.clone(),
        })
        .collect()
}

fn function_summaries(
    program: &ScenarioProgram,
    entry_env: &[ObligationState],
    exit_env: &[ObligationState],
    event_count: usize,
) -> Vec<FunctionSummary> {
    vec![FunctionSummary {
        function: program.target.clone(),
        event_count,
        entry_env: entry_env.to_vec(),
        exit_env: exit_env.to_vec(),
    }]
}

fn coverage_loss(program: &ScenarioProgram) -> Vec<CoverageLoss> {
    let unsupported = program
        .coverage
        .unsupported_constructs
        .iter()
        .map(|label| CoverageLoss {
            kind: "unsupported_construct".to_owned(),
            label: label.clone(),
            reason: "not modeled in current Core proof certificate".to_owned(),
        });
    let opaque = program
        .coverage
        .opaque_boundaries
        .iter()
        .map(|label| CoverageLoss {
            kind: "opaque_boundary".to_owned(),
            label: label.clone(),
            reason: "opaque boundary requires ledger evidence before exact replay".to_owned(),
        });
    unsupported.chain(opaque).collect()
}

fn canonical_select_result_hash(
    branch_target: &str,
    path_kind: &str,
    results: &[ObligationState],
    cancelled_obligations: &[ObligationState],
) -> String {
    let mut states = results
        .iter()
        .map(|state| format!("{}:{}", state.binding, state.state.as_str()))
        .collect::<Vec<_>>();
    states.sort_unstable();
    let mut cancelled = cancelled_obligations
        .iter()
        .map(|state| format!("{}:{}", state.binding, state.state.as_str()))
        .collect::<Vec<_>>();
    cancelled.sort_unstable();
    stable_hash(&format!(
        "select-result:{branch_target}:{path_kind}:{states:?}:cancelled:{cancelled:?}"
    ))
}

fn parsed_live_locals_by_await(source: &str, target: &str) -> Vec<Vec<String>> {
    let Ok(file) = syn::parse_file(source) else {
        return Vec::new();
    };
    let Some(function) = file.items.iter().find_map(|item| match item {
        syn::Item::Fn(function) if function.sig.ident == target => Some(function),
        _ => None,
    }) else {
        return Vec::new();
    };

    let mut locals_before_statement = Vec::<BTreeSet<String>>::new();
    let mut declared = BTreeSet::<String>::new();
    let mut await_statement_indexes = Vec::new();
    for (index, statement) in function.block.stmts.iter().enumerate() {
        locals_before_statement.push(declared.clone());
        for _ in 0..stmt_await_count(statement) {
            await_statement_indexes.push(index);
        }
        collect_pat_bindings_in_stmt(statement, &mut declared);
    }

    await_statement_indexes
        .into_iter()
        .map(|await_index| {
            let mut after_await_uses = BTreeSet::<String>::new();
            for statement in function.block.stmts.iter().skip(await_index + 1) {
                collect_ident_uses_in_stmt(statement, &mut after_await_uses);
            }
            locals_before_statement
                .get(await_index)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|binding| !binding.starts_with('_'))
                .filter(|binding| after_await_uses.contains(binding))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn stmt_await_count(statement: &syn::Stmt) -> usize {
    struct AwaitVisitor {
        count: usize,
    }

    impl<'ast> syn::visit::Visit<'ast> for AwaitVisitor {
        fn visit_expr_await(&mut self, expr: &'ast syn::ExprAwait) {
            self.count += 1;
            syn::visit::visit_expr_await(self, expr);
        }
    }

    let mut visitor = AwaitVisitor { count: 0 };
    syn::visit::visit_stmt(&mut visitor, statement);
    visitor.count
}

fn collect_pat_bindings_in_stmt(statement: &syn::Stmt, bindings: &mut BTreeSet<String>) {
    let syn::Stmt::Local(local) = statement else {
        return;
    };
    collect_pat_bindings(&local.pat, bindings);
}

fn collect_pat_bindings(pattern: &syn::Pat, bindings: &mut BTreeSet<String>) {
    match pattern {
        syn::Pat::Ident(ident) => {
            bindings.insert(ident.ident.to_string());
        }
        syn::Pat::Tuple(tuple) => {
            for element in &tuple.elems {
                collect_pat_bindings(element, bindings);
            }
        }
        syn::Pat::Struct(pattern) => {
            for field in &pattern.fields {
                collect_pat_bindings(&field.pat, bindings);
            }
        }
        syn::Pat::TupleStruct(pattern) => {
            for element in &pattern.elems {
                collect_pat_bindings(element, bindings);
            }
        }
        syn::Pat::Slice(pattern) => {
            for element in &pattern.elems {
                collect_pat_bindings(element, bindings);
            }
        }
        syn::Pat::Reference(pattern) => collect_pat_bindings(&pattern.pat, bindings),
        syn::Pat::Type(pattern) => collect_pat_bindings(&pattern.pat, bindings),
        syn::Pat::Or(pattern) => {
            for case in &pattern.cases {
                collect_pat_bindings(case, bindings);
            }
        }
        _ => {}
    }
}

fn collect_ident_uses_in_stmt(statement: &syn::Stmt, uses: &mut BTreeSet<String>) {
    struct UseVisitor<'a> {
        uses: &'a mut BTreeSet<String>,
    }

    impl<'a, 'ast> syn::visit::Visit<'ast> for UseVisitor<'a> {
        fn visit_expr_path(&mut self, expr: &'ast syn::ExprPath) {
            if expr.qself.is_none() && expr.path.segments.len() == 1 {
                if let Some(segment) = expr.path.segments.first() {
                    self.uses.insert(segment.ident.to_string());
                }
            }
            syn::visit::visit_expr_path(self, expr);
        }
    }

    let mut visitor = UseVisitor { uses };
    syn::visit::visit_stmt(&mut visitor, statement);
}

fn boundary_policy(policy: &kobo_ir::ScenarioBoundaryPolicy) -> BoundaryPolicy {
    match policy {
        kobo_ir::ScenarioBoundaryPolicy::Typed => BoundaryPolicy::Typed,
        kobo_ir::ScenarioBoundaryPolicy::Model => BoundaryPolicy::Model,
        kobo_ir::ScenarioBoundaryPolicy::Record => BoundaryPolicy::Record,
        kobo_ir::ScenarioBoundaryPolicy::Activity => BoundaryPolicy::Activity,
        kobo_ir::ScenarioBoundaryPolicy::Stub => BoundaryPolicy::Stub,
        kobo_ir::ScenarioBoundaryPolicy::Outside => BoundaryPolicy::Outside,
        kobo_ir::ScenarioBoundaryPolicy::Opaque => BoundaryPolicy::Opaque,
        kobo_ir::ScenarioBoundaryPolicy::Debt => BoundaryPolicy::Debt,
        kobo_ir::ScenarioBoundaryPolicy::Unselected => BoundaryPolicy::Unselected,
    }
}

fn source_span_from_kobo(source_path: &str, source: &str, span: KoboSpan) -> SourceSpan {
    source_span_from_range(source_path, source, span.start as usize, span.end as usize)
}

fn source_span_for_binding(source_path: &str, source: &str, binding: &str) -> SourceSpan {
    let let_binding = format!("let {binding}");
    let let_mut_binding = format!("let mut {binding}");
    let start = source
        .find(&let_binding)
        .or_else(|| source.find(&let_mut_binding))
        .unwrap_or(0);
    source_span_from_range(
        source_path,
        source,
        start,
        start.saturating_add(binding.len()),
    )
}

fn source_span_from_range(source_path: &str, source: &str, start: usize, end: usize) -> SourceSpan {
    let bounded_start = start.min(source.len());
    let bounded_end = end.max(bounded_start + 1).min(source.len());
    SourceSpan {
        path: source_path.to_owned(),
        line: one_based_line_for_offset(source, bounded_start),
        start: bounded_start,
        end: bounded_end,
        mapped: bounded_end > bounded_start,
        snippet: line_snippet(source, bounded_start),
    }
}

fn one_based_line_for_offset(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn line_snippet(source: &str, offset: usize) -> String {
    let bounded = offset.min(source.len());
    let line_start = source[..bounded]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let line_end = source[bounded..]
        .find('\n')
        .map(|index| bounded + index)
        .unwrap_or(source.len());
    source[line_start..line_end].trim().to_owned()
}
