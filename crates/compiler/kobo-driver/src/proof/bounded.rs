use super::{
    classify_bounded_completeness, normalized_bound_hash, proof_bounded_wording,
    syn_path_ends_with, BTreeMap, BoundDeclaration, BoundDimension, BoundSource,
    BoundedClassificationInput, BoundedCompleteness, BoundedHistoryEvidence, BoundedProofEvidence,
    CoreLoopBackEdgeFact, CoreLoopExitFact, PrunedHistoryEvidence, ScenarioProgram,
};

#[derive(Debug)]
pub(super) struct BoundedHistoryExploration {
    histories: Vec<kobo_sim_core::BoundedHistory>,
    enumerated_history_count: u64,
    expected_complete_history_count: u64,
    pruned_histories: Vec<PrunedHistoryEvidence>,
    is_complete: bool,
}

#[derive(Clone, Debug, Default)]
pub(super) struct RequiredBoundDimensions {
    queue_capacity: Option<u64>,
    message_count: Option<u64>,
    retry_attempts: Option<u64>,
    timeout_paths: Option<u64>,
    external_boundary_recordings: Option<u64>,
}

impl RequiredBoundDimensions {
    fn from_fields(fields: &BTreeMap<String, String>) -> Self {
        Self {
            queue_capacity: first_numeric_field(fields, &["queue_capacity"]),
            message_count: first_numeric_field(fields, &["message_count", "messages"]),
            retry_attempts: first_numeric_field(fields, &["retry_attempts", "retries"]),
            timeout_paths: first_numeric_field(fields, &["timeout_paths"]),
            external_boundary_recordings: first_numeric_field(
                fields,
                &[
                    "external_boundary_recordings",
                    "external_boundary_models",
                    "external_boundaries",
                ],
            ),
        }
    }

    fn has_all_required_dimensions(&self) -> bool {
        self.queue_capacity.is_some()
            && self.message_count.is_some()
            && self.retry_attempts.is_some()
            && self.timeout_paths.is_some()
            && self.external_boundary_recordings.is_some()
    }

    fn push_declarations(&self, bounds: &mut Vec<BoundDeclaration>) {
        push_bound_if_declared(bounds, BoundDimension::QueueCapacity, self.queue_capacity);
        push_bound_if_declared(bounds, BoundDimension::MessageCount, self.message_count);
        push_bound_if_declared(bounds, BoundDimension::RetryAttempts, self.retry_attempts);
        push_bound_if_declared(bounds, BoundDimension::TimeoutPaths, self.timeout_paths);
        push_bound_if_declared(
            bounds,
            BoundDimension::ExternalBoundaryRecordings,
            self.external_boundary_recordings,
        );
    }

    fn state_dimensions(&self, loop_iteration_bound: u64) -> kobo_sim_core::BoundedStateDimensions {
        kobo_sim_core::BoundedStateDimensions::from_bounds(
            Some(loop_iteration_bound),
            self.queue_capacity,
            self.message_count,
            self.retry_attempts,
            self.timeout_paths,
            self.external_boundary_recordings,
        )
    }
}

pub(super) fn bounded_evidence(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
    loop_facts: &[CoreLoopBackEdgeFact],
    loop_exit_facts: &[CoreLoopExitFact],
) -> Vec<BoundedProofEvidence> {
    let Ok(file) = syn::parse_file(source) else {
        return Vec::new();
    };
    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Fn(function) if function.sig.ident == program.target => {
                bounded_attr(function).map(|fields| {
                    bounded_evidence_from_fields(
                        source_path,
                        source,
                        program,
                        loop_facts,
                        loop_exit_facts,
                        &fields,
                    )
                })
            }
            _ => None,
        })
        .collect()
}

pub(super) fn bounded_attr(function: &syn::ItemFn) -> Option<BTreeMap<String, String>> {
    function
        .attrs
        .iter()
        .find(|attr| syn_path_ends_with(attr.path(), &["kobo", "bounded"]))
        .and_then(attr_name_value_fields)
}

pub(super) fn attr_name_value_fields(attr: &syn::Attribute) -> Option<BTreeMap<String, String>> {
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

pub(super) fn bounded_evidence_from_fields(
    _source_path: &str,
    _source: &str,
    program: &ScenarioProgram,
    loop_facts: &[CoreLoopBackEdgeFact],
    loop_exit_facts: &[CoreLoopExitFact],
    fields: &BTreeMap<String, String>,
) -> BoundedProofEvidence {
    let history_bound = numeric_field(fields, "histories").unwrap_or_default();
    let loop_iteration_bound = numeric_field(fields, "loop_iterations").unwrap_or(1);
    let scheduler_dimensions = dimension_values(fields, "scheduler");
    let fault_dimensions = dimension_values(fields, "fault");
    let cancellation_points = dimension_values(fields, "cancellation");
    let required_dimensions = RequiredBoundDimensions::from_fields(fields);
    let exploration = bounded_history_exploration(
        &program.target,
        history_bound,
        numeric_field(fields, "unique_histories"),
        &scheduler_dimensions,
        &fault_dimensions,
        &cancellation_points,
        loop_iteration_bound,
        &required_dimensions,
    );
    let expected_complete_history_count = Some(exploration.expected_complete_history_count);
    let declared_expected = numeric_field(fields, "expected");
    let declared_completeness =
        bounded_completeness(fields.get("completeness").map(String::as_str));
    let completeness = classify_bounded_completeness(&BoundedClassificationInput {
        declared: declared_completeness,
        declared_expected,
        is_complete: exploration.is_complete,
        expected_complete_history_count: exploration.expected_complete_history_count,
        enumerated_history_count: exploration.enumerated_history_count,
        has_scheduler_dimensions: !scheduler_dimensions.is_empty(),
        has_fault_dimensions: !fault_dimensions.is_empty(),
        has_cancellation_points: !cancellation_points.is_empty(),
        has_required_dimensions: required_dimensions.has_all_required_dimensions(),
    });
    let wording = proof_bounded_wording(
        &completeness,
        exploration.enumerated_history_count,
        expected_complete_history_count,
    );
    let canonical_histories = canonical_histories(exploration.histories);
    let bounds = bound_declarations(
        exploration.expected_complete_history_count,
        loop_iteration_bound,
        &fault_dimensions,
        &cancellation_points,
        &required_dimensions,
    );
    let mut evidence = BoundedProofEvidence {
        id: format!("bounded-{}", program.target),
        function: program.target.clone(),
        loop_ids: loop_facts
            .iter()
            .filter(|fact| fact.function == program.target)
            .map(|fact| fact.id.clone())
            .chain(
                loop_exit_facts
                    .iter()
                    .filter(|fact| fact.function == program.target)
                    .map(|fact| fact.id.clone()),
            )
            .collect(),
        normalized_bound_hash: String::new(),
        bounds,
        enumerated_history_count: exploration.enumerated_history_count,
        expected_complete_history_count,
        scheduler_dimensions,
        fault_dimensions,
        cancellation_points,
        canonical_histories,
        pruned_histories: exploration.pruned_histories,
        completeness,
        wording,
    };
    evidence.normalized_bound_hash = normalized_bound_hash(&evidence);
    evidence
}

pub(super) fn bounded_history_exploration(
    function: &str,
    history_bound: u64,
    unique_history_count: Option<u64>,
    scheduler_dimensions: &[String],
    fault_dimensions: &[String],
    cancellation_points: &[String],
    loop_iteration_bound: u64,
    required_dimensions: &RequiredBoundDimensions,
) -> BoundedHistoryExploration {
    let sim_exploration = kobo_sim_core::explore_bounded_histories(
        function,
        history_bound,
        scheduler_dimensions,
        fault_dimensions,
        cancellation_points,
        &required_dimensions.state_dimensions(loop_iteration_bound),
    );
    let mut histories = sim_exploration.histories;
    let original_history_count = histories.len() as u64;
    let mut pruned_histories = Vec::new();
    if let Some(unique_history_count) = unique_history_count {
        if unique_history_count < original_history_count {
            histories.truncate(unique_history_count as usize);
            pruned_histories.push(PrunedHistoryEvidence {
                id: format!("pruned-{function}"),
                reason: format!(
                    "{} duplicate histories collapsed before completeness evaluation",
                    original_history_count - unique_history_count
                ),
            });
        } else if unique_history_count > original_history_count {
            pruned_histories.push(PrunedHistoryEvidence {
                id: format!("pruned-{function}"),
                reason: format!(
                    "declared unique history count {unique_history_count} exceeds {original_history_count} explored histories"
                ),
            });
        }
    }
    BoundedHistoryExploration {
        enumerated_history_count: histories.len() as u64,
        expected_complete_history_count: sim_exploration.expected_history_count,
        histories,
        is_complete: sim_exploration.is_complete && pruned_histories.is_empty(),
        pruned_histories,
    }
}

pub(super) fn canonical_histories(
    histories: Vec<kobo_sim_core::BoundedHistory>,
) -> Vec<BoundedHistoryEvidence> {
    histories
        .into_iter()
        .map(|history| BoundedHistoryEvidence {
            id: history.id,
            scheduler: history.scheduler,
            fault: history.fault,
            cancellation: history.cancellation,
            loop_iteration: history.loop_iteration,
            queue_capacity: history.queue_capacity,
            message_count: history.message_count,
            retry_attempts: history.retry_attempts,
            timeout_path: history.timeout_path,
            external_boundary_recording: history.external_boundary_recording,
            history_hash: history.history_hash,
        })
        .collect()
}

pub(super) fn bound_declarations(
    history_bound: u64,
    loop_iteration_bound: u64,
    fault_dimensions: &[String],
    cancellation_points: &[String],
    required_dimensions: &RequiredBoundDimensions,
) -> Vec<BoundDeclaration> {
    let mut bounds = vec![BoundDeclaration {
        dimension: BoundDimension::SchedulerHistories,
        value: history_bound,
        source: BoundSource::Ward,
        proof_relevant: true,
    }];
    bounds.push(BoundDeclaration {
        dimension: BoundDimension::LoopIterations,
        value: loop_iteration_bound,
        source: BoundSource::Ward,
        proof_relevant: true,
    });
    if !fault_dimensions.is_empty() {
        bounds.push(BoundDeclaration {
            dimension: BoundDimension::FaultInjectionChoices,
            value: fault_dimensions.len() as u64,
            source: BoundSource::Ward,
            proof_relevant: true,
        });
    }
    if !cancellation_points.is_empty() {
        bounds.push(BoundDeclaration {
            dimension: BoundDimension::CancellationPoints,
            value: cancellation_points.len() as u64,
            source: BoundSource::Ward,
            proof_relevant: true,
        });
    }
    required_dimensions.push_declarations(&mut bounds);
    bounds
}

pub(super) fn numeric_field(fields: &BTreeMap<String, String>, key: &str) -> Option<u64> {
    fields.get(key)?.parse().ok()
}

pub(super) fn first_numeric_field(fields: &BTreeMap<String, String>, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| numeric_field(fields, key))
}

pub(super) fn push_bound_if_declared(
    bounds: &mut Vec<BoundDeclaration>,
    dimension: BoundDimension,
    value: Option<u64>,
) {
    let Some(value) = value else {
        return;
    };
    bounds.push(BoundDeclaration {
        dimension,
        value,
        source: BoundSource::Ward,
        proof_relevant: true,
    });
}

pub(super) fn dimension_values(fields: &BTreeMap<String, String>, key: &str) -> Vec<String> {
    fields
        .get(key)
        .into_iter()
        .flat_map(|value| value.split(['|', ',']))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) fn bounded_completeness(value: Option<&str>) -> BoundedCompleteness {
    match value {
        Some("complete") => BoundedCompleteness::Complete,
        Some("timeout") => BoundedCompleteness::Timeout,
        Some("incomplete") => BoundedCompleteness::Incomplete,
        Some("sampled") | None => BoundedCompleteness::Sampled,
        Some(_) => BoundedCompleteness::Incomplete,
    }
}
