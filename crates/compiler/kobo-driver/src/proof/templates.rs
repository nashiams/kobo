use super::*;

pub(super) fn template_by_binding(
    program: &ScenarioProgram,
) -> BTreeMap<&str, &kobo_ir::ScenarioLifecycleTemplate> {
    program
        .operations
        .iter()
        .filter_map(|operation| match &operation.kind {
            ScenarioOpKind::CreateObligation {
                binding,
                template: Some(template),
                ..
            } => Some((binding.as_str(), template)),
            _ => None,
        })
        .collect()
}

pub(super) fn type_by_binding(program: &ScenarioProgram) -> BTreeMap<&str, String> {
    program
        .operations
        .iter()
        .filter_map(|operation| match &operation.kind {
            ScenarioOpKind::CreateObligation {
                binding, type_name, ..
            } => Some((binding.as_str(), type_name.clone())),
            _ => None,
        })
        .collect()
}

pub(super) fn lifecycle_template_version(template: &kobo_ir::ScenarioLifecycleTemplate) -> String {
    if matches!(template.source, ScenarioLifecycleTemplateSource::Inference) {
        return "0.1".to_owned();
    }
    template.schema_version.to_string()
}

pub(super) fn template_hash(
    template: &kobo_ir::ScenarioLifecycleTemplate,
    template_hashes: &[HashEvidence],
) -> String {
    template_hashes
        .iter()
        .find(|hash| hash.id == template.id)
        .map(|hash| hash.hash.clone())
        .unwrap_or_else(|| stable_hash(&template.id))
}

pub(super) fn invariant_template_source(
    source: &ScenarioLifecycleTemplateSource,
) -> InvariantTemplateSource {
    match source {
        ScenarioLifecycleTemplateSource::Declaration => InvariantTemplateSource::Declaration,
        ScenarioLifecycleTemplateSource::Inference => InvariantTemplateSource::BuiltIn,
    }
}

pub(super) fn invariant_confidence(confidence: &str) -> InvariantConfidence {
    match confidence {
        "exact_template" | "exact" => InvariantConfidence::Exact,
        "sampled" => InvariantConfidence::Sampled,
        "metadata-only" | "metadata_only" => InvariantConfidence::MetadataOnly,
        _ => InvariantConfidence::Modeled,
    }
}

pub(super) fn template_evidence(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
) -> Result<(Vec<HashEvidence>, Vec<TemplateSchemaEvidence>), serde_json::Error> {
    let mut hashes = Vec::new();
    let mut schemas = Vec::new();
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
            ScenarioLifecycleTemplateSource::Inference => "built_in",
        };
        let schema = TemplateSchemaEvidence {
            id: template.id.clone(),
            kind: template.kind.clone(),
            template_schema: template.template_schema.clone(),
            schema_version: template.schema_version,
            confidence: template.confidence.clone(),
            source: template_source.to_owned(),
            lifecycle_owner: template.lifecycle_owner.clone(),
            cancel_policy: template.cancel_policy.clone(),
            registry_source: template.registry_source.clone(),
            source_span: source_span_from_kobo(source_path, source, operation.span),
        };
        let hash = template_schema_hash(&schema)?;
        hashes.push(HashEvidence {
            id: template.id.clone(),
            hash,
        });
        schemas.push(schema);
    }
    Ok((hashes, schemas))
}
