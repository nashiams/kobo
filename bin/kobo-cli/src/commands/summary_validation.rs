use anyhow::Context;

pub(super) struct ValidSummary {
    pub(super) value: serde_json::Value,
    pub(super) schema_version: u64,
    pub(super) hash: String,
}

pub(super) fn load_valid_summary(
    summary: &kobo_driver::EcosystemSummaryPolicy,
) -> anyhow::Result<ValidSummary> {
    let source = std::fs::read_to_string(&summary.path)
        .with_context(|| format!("failed to read .kobo-summary {}", summary.path.display()))?;
    let parsed: serde_json::Value = serde_json::from_str(&source)
        .with_context(|| format!("failed to parse .kobo-summary {}", summary.path.display()))?;
    let schema_version = parsed
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    if schema_version != 1 {
        anyhow::bail!(
            "K0126: .kobo-summary version mismatch for {}: expected 1, found {}",
            summary.crate_name,
            schema_version
        );
    }
    let embedded_hash = parsed
        .get("summary_hash")
        .and_then(serde_json::Value::as_str)
        .with_context(|| {
            format!(
                "K0126: .kobo-summary missing summary_hash for {}",
                summary.crate_name
            )
        })?;
    let computed_hash = computed_summary_hash(&parsed)?;
    if embedded_hash != computed_hash {
        anyhow::bail!(
            "K0126: .kobo-summary hash mismatch for {}: embedded {}, computed {}",
            summary.crate_name,
            embedded_hash,
            computed_hash
        );
    }
    if summary.hash != computed_hash {
        anyhow::bail!(
            "K0126: .kobo-summary hash mismatch for {}: expected {}, found {}",
            summary.crate_name,
            summary.hash,
            computed_hash
        );
    }
    Ok(ValidSummary {
        value: parsed,
        schema_version,
        hash: computed_hash,
    })
}

fn computed_summary_hash(parsed: &serde_json::Value) -> anyhow::Result<String> {
    let mut body = parsed.clone();
    let Some(object) = body.as_object_mut() else {
        anyhow::bail!("K0126: .kobo-summary root must be a JSON object");
    };
    object.remove("summary_hash");
    Ok(stable_hash(&body.to_string()))
}

fn stable_hash(source: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in source.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
