use std::path::Path;

use anyhow::Context;
use serde_json::Value;

pub(super) fn cmd_replay(file: &Path, roundtrip_metadata: bool) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;
    let witness: Value = serde_json::from_str(&source)
        .with_context(|| format!("failed to parse witness {}", file.display()))?;
    validate_witness(&witness)?;

    if roundtrip_metadata {
        println!(
            "{}",
            serde_json::to_string_pretty(&witness)
                .context("failed to serialize witness metadata")?
        );
        return Ok(());
    }

    let source_path = witness["source"]["path"].as_str().unwrap_or("<unknown>");
    let span_start = witness["source"]["span"]["start"].as_u64().unwrap_or(0);
    let span_end = witness["source"]["span"]["end"].as_u64().unwrap_or(0);
    let scenario = witness["scenario"].as_str().unwrap_or("<unknown>");
    let event_count = witness["events"].as_array().map(Vec::len).unwrap_or(0);

    println!("Replay witness metadata-only");
    println!("schema_version: 0");
    println!("source: {source_path} span {span_start}..{span_end}");
    println!("scenario: {scenario}");
    println!("events: {event_count}");
    println!("metadata-only: full deterministic replay is not available in v0.8.5");
    Ok(())
}

fn validate_witness(witness: &Value) -> anyhow::Result<()> {
    let Some(version) = witness["schema_version"].as_u64() else {
        return witness_error("missing schema_version");
    };
    if version != 0 {
        return witness_error("unsupported schema_version");
    }

    if witness["source"]["path"].as_str().is_none() {
        return witness_error("missing source.path");
    }
    if witness["source"]["span"]["start"].as_u64().is_none()
        || witness["source"]["span"]["end"].as_u64().is_none()
    {
        return witness_error("missing source.span");
    }

    Ok(())
}

fn witness_error(message: &str) -> anyhow::Result<()> {
    eprintln!("error[K0104]: {message}");
    eprintln!("help: .kwit schema_version 0 requires source.path and source.span");
    anyhow::bail!("invalid witness")
}
