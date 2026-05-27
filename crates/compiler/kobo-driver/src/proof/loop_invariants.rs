use super::{
    attr_name_value_fields, env_states_for_bindings, function_obligation_replay,
    invariant_confidence, invariant_template_source, lifecycle_template_version,
    source_span_from_range, syn_path_ends_with, template_by_binding, template_hash,
    type_by_binding, BTreeMap, CoreFunction, CoreLoopBackEdgeFact, HashEvidence,
    InvariantBindingTemplateEvidence, InvariantPreservation, InvariantTemplateEvidence,
    InvariantTier, LoopInvariantEvidence, ObligationEvent, ObligationEventKind, ObligationStatus,
    ScenarioProgram, SourceSpan, UserInvariantFactEvidence, UserInvariantPredicate,
};

#[derive(Clone, Debug)]
pub(super) struct UserLoopInvariantDirective {
    expression: String,
    obligation_kind: Option<String>,
    loop_label: Option<String>,
    source_span: SourceSpan,
}

pub(super) fn loop_invariant_evidence(
    program: &ScenarioProgram,
    functions: &[CoreFunction],
    loop_facts: &[CoreLoopBackEdgeFact],
    obligation_events: &[ObligationEvent],
    template_hashes: &[HashEvidence],
    user_loop_invariant: Option<&UserLoopInvariantDirective>,
) -> Vec<LoopInvariantEvidence> {
    if loop_facts.is_empty() {
        return Vec::new();
    }
    let template_by_binding = template_by_binding(program);
    let type_by_binding = type_by_binding(program);
    let replay_by_function = functions
        .iter()
        .map(|function| (function.name.as_str(), function_obligation_replay(function)))
        .collect::<BTreeMap<_, _>>();
    loop_facts
        .iter()
        .filter_map(|fact| {
            let user_invariant = user_invariant_for_fact(user_loop_invariant, fact, loop_facts);
            let scoped_bindings = created_bindings_for_loop(fact, obligation_events);
            let relevant_bindings = scoped_bindings
                .iter()
                .filter(|binding| {
                    user_invariant
                        .and_then(|directive| directive.obligation_kind.as_deref())
                        .map(|kind| {
                            type_by_binding
                                .get(binding.as_str())
                                .is_some_and(|type_name| type_name == kind)
                        })
                        .unwrap_or_else(|| template_by_binding.contains_key(binding.as_str()))
                })
                .cloned()
                .collect::<Vec<_>>();
            let user_domain_bindings =
                user_invariant_domain_bindings(user_invariant, &type_by_binding);
            let checked_bindings = user_domain_bindings
                .as_ref()
                .unwrap_or(&relevant_bindings)
                .clone();
            let first_binding = relevant_bindings.first();
            if user_invariant.is_none() && first_binding.is_none() {
                return None;
            }
            let template_binding = first_binding.or_else(|| checked_bindings.first());
            let template = template_binding
                .and_then(|binding| template_by_binding.get(binding.as_str()).copied());
            let binding_templates = invariant_binding_templates(
                &relevant_bindings,
                &template_by_binding,
                &type_by_binding,
                template_hashes,
            );
            let obligation_kind = type_by_binding
                .get(template_binding.map(String::as_str).unwrap_or_default())
                .cloned()
                .or_else(|| user_invariant.and_then(|directive| directive.obligation_kind.clone()))
                .unwrap_or_else(|| "obligation".to_owned());
            let replay = replay_by_function.get(fact.function.as_str())?;
            let entry_states = replay
                .block_entry_envs
                .get(&fact.entry_block)
                .map(|env| env_states_for_bindings(env, &checked_bindings))
                .unwrap_or_default();
            let back_edge_states = replay
                .block_exit_envs
                .get(&fact.back_edge_source)
                .map(|env| env_states_for_bindings(env, &checked_bindings))
                .unwrap_or_default();
            let entry_leak = entry_states.iter().any(|state| {
                checked_bindings.contains(&state.binding)
                    && matches!(
                        state.state,
                        ObligationStatus::Owned
                            | ObligationStatus::Moved
                            | ObligationStatus::BranchUnresolved
                    )
            });
            let back_edge_leak = back_edge_states.iter().any(|state| {
                checked_bindings.contains(&state.binding)
                    && matches!(
                        state.state,
                        ObligationStatus::Owned
                            | ObligationStatus::Moved
                            | ObligationStatus::BranchUnresolved
                    )
            });
            let malformed_user_invariant =
                user_invariant.is_some_and(|directive| directive.obligation_kind.is_none());
            let unknown_user_kind = malformed_user_invariant
                || (user_invariant.is_some()
                    && user_domain_bindings.as_ref().map_or(true, Vec::is_empty));
            let preservation = if entry_leak || back_edge_leak || unknown_user_kind {
                InvariantPreservation::Failed
            } else {
                InvariantPreservation::Preserved
            };
            let user_fact = user_invariant.and_then(|directive| {
                user_domain_bindings.as_ref().and_then(|domain_bindings| {
                    user_invariant_fact(&fact.loop_id, directive, domain_bindings, template)
                })
            });
            Some(LoopInvariantEvidence {
                id: fact.id.clone(),
                function: fact.function.clone(),
                loop_id: fact.loop_id.clone(),
                loop_label: fact.loop_label.clone(),
                entry_block: fact.entry_block.clone(),
                back_edge_source: fact.back_edge_source.clone(),
                back_edge_target: fact.back_edge_target.clone(),
                tier: user_invariant
                    .map(|_| InvariantTier::User)
                    .unwrap_or(InvariantTier::Inferred),
                expression: user_invariant
                    .map(|directive| directive.expression.clone())
                    .unwrap_or_else(|| format!("no_pending({obligation_kind})")),
                source_span: user_invariant
                    .map(|directive| directive.source_span.clone())
                    .unwrap_or_else(|| fact.source_span.clone()),
                obligations_created: relevant_bindings,
                entry_states,
                back_edge_states,
                preservation,
                template: template.map(|template| InvariantTemplateEvidence {
                    id: template.id.clone(),
                    version: lifecycle_template_version(template),
                    schema_hash: template_hash(template, template_hashes),
                    source: invariant_template_source(&template.source),
                    confidence: invariant_confidence(&template.confidence),
                    obligation_kind,
                    lifecycle_owner: template.lifecycle_owner.clone(),
                }),
                binding_templates,
                user_fact,
                downgrade_reason: user_invariant_downgrade_reason(
                    user_invariant,
                    malformed_user_invariant,
                    unknown_user_kind,
                    entry_leak,
                    back_edge_leak,
                ),
            })
        })
        .collect()
}

pub(super) fn user_invariant_for_fact<'a>(
    directive: Option<&'a UserLoopInvariantDirective>,
    fact: &CoreLoopBackEdgeFact,
    loop_facts: &[CoreLoopBackEdgeFact],
) -> Option<&'a UserLoopInvariantDirective> {
    let directive = directive?;
    if let Some(loop_label) = directive.loop_label.as_deref() {
        return fact
            .loop_label
            .as_deref()
            .is_some_and(|label| label == loop_label)
            .then_some(directive);
    }
    let function_loop_count = loop_facts
        .iter()
        .filter(|loop_fact| loop_fact.function == fact.function)
        .count();
    (function_loop_count == 1).then_some(directive)
}

pub(super) fn user_invariant_domain_bindings(
    user_invariant: Option<&UserLoopInvariantDirective>,
    type_by_binding: &BTreeMap<&str, String>,
) -> Option<Vec<String>> {
    let obligation_kind = user_invariant?.obligation_kind.as_deref()?;
    Some(
        type_by_binding
            .iter()
            .filter(|(_, type_name)| type_name.as_str() == obligation_kind)
            .map(|(binding, _)| (*binding).to_owned())
            .collect(),
    )
}

pub(super) fn user_invariant_fact(
    loop_id: &str,
    directive: &UserLoopInvariantDirective,
    domain_bindings: &[String],
    template: Option<&kobo_ir::ScenarioLifecycleTemplate>,
) -> Option<UserInvariantFactEvidence> {
    let obligation_kind = directive.obligation_kind.clone()?;
    Some(UserInvariantFactEvidence {
        loop_id: loop_id.to_owned(),
        predicate: UserInvariantPredicate::NoPending,
        obligation_kind,
        lifecycle_owner: template.map(|template| template.lifecycle_owner.clone()),
        template_id: template.map(|template| template.id.clone()),
        template_version: template.map(|template| lifecycle_template_version(template)),
        domain_bindings: domain_bindings.to_vec(),
    })
}

pub(super) fn invariant_binding_templates(
    bindings: &[String],
    template_by_binding: &BTreeMap<&str, &kobo_ir::ScenarioLifecycleTemplate>,
    type_by_binding: &BTreeMap<&str, String>,
    template_hashes: &[HashEvidence],
) -> Vec<InvariantBindingTemplateEvidence> {
    bindings
        .iter()
        .filter_map(|binding| {
            let template = template_by_binding.get(binding.as_str()).copied()?;
            let obligation_kind = type_by_binding
                .get(binding.as_str())
                .cloned()
                .unwrap_or_else(|| "obligation".to_owned());
            Some(InvariantBindingTemplateEvidence {
                binding: binding.clone(),
                id: template.id.clone(),
                version: lifecycle_template_version(template),
                schema_hash: template_hash(template, template_hashes),
                source: invariant_template_source(&template.source),
                confidence: invariant_confidence(&template.confidence),
                obligation_kind,
                lifecycle_owner: template.lifecycle_owner.clone(),
            })
        })
        .collect()
}

pub(super) fn created_bindings_for_loop(
    fact: &CoreLoopBackEdgeFact,
    obligation_events: &[ObligationEvent],
) -> Vec<String> {
    obligation_events
        .iter()
        .filter(|event| event.kind == ObligationEventKind::Create)
        .filter(|event| {
            event
                .loop_regions
                .iter()
                .any(|loop_id| loop_id == &fact.loop_id)
        })
        .filter_map(|event| event.binding.clone())
        .collect()
}

pub(super) fn user_invariant_downgrade_reason(
    user_loop_invariant: Option<&UserLoopInvariantDirective>,
    malformed_user_invariant: bool,
    unknown_user_kind: bool,
    entry_leak: bool,
    back_edge_leak: bool,
) -> Option<String> {
    if user_loop_invariant.is_some() && malformed_user_invariant {
        return Some("malformed user invariant expression".to_owned());
    }
    if user_loop_invariant.is_some() && unknown_user_kind {
        return Some("unknown obligation kind in user invariant".to_owned());
    }
    if entry_leak {
        return Some("user invariant is not true at loop entry".to_owned());
    }
    back_edge_leak.then(|| "unresolved obligation reaches loop back-edge".to_owned())
}

pub(super) fn user_loop_invariant_directive(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
) -> Option<UserLoopInvariantDirective> {
    let file = syn::parse_file(source).ok()?;
    let function = file.items.iter().find_map(|item| match item {
        syn::Item::Fn(function) if function.sig.ident == program.target => Some(function),
        _ => None,
    })?;
    let attr = function
        .attrs
        .iter()
        .find(|attr| syn_path_ends_with(attr.path(), &["kobo", "invariant"]))?;
    let fields = attr_name_value_fields(attr)?;
    let expression = fields.get("expression")?.clone();
    let obligation_kind = no_pending_obligation_kind(&expression);
    let loop_label = fields
        .get("loop_label")
        .or_else(|| fields.get("loop"))
        .cloned();
    let source_span = source_span_for_user_invariant(source_path, source, &expression);
    Some(UserLoopInvariantDirective {
        expression,
        obligation_kind,
        loop_label,
        source_span,
    })
}

pub(super) fn no_pending_obligation_kind(expression: &str) -> Option<String> {
    let obligation_kind = expression
        .strip_prefix("no_pending(")?
        .strip_suffix(')')?
        .trim();
    (!obligation_kind.is_empty()).then(|| obligation_kind.to_owned())
}

pub(super) fn source_span_for_user_invariant(
    source_path: &str,
    source: &str,
    expression: &str,
) -> SourceSpan {
    let start = source.find(expression).unwrap_or(0);
    source_span_from_range(
        source_path,
        source,
        start,
        start.saturating_add(expression.len()),
    )
}
