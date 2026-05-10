use std::path::{Path, PathBuf};

use anyhow::Context;
use serde_json::Value;

use crate::ErrorFormat;

use super::sim_model;

pub(super) fn cmd_replay(
    file: &Path,
    error_format: ErrorFormat,
    roundtrip_metadata: bool,
) -> anyhow::Result<()> {
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

    if witness["schema_version"].as_u64() == Some(1) {
        return replay_v1(&witness, file, error_format);
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
    println!(
        "metadata-only: legacy schema_version 0 witnesses do not carry v0.9 exact replay data"
    );
    Ok(())
}

struct VerifiedSource {
    path: PathBuf,
    hash: String,
    source: String,
}

fn replay_v1(
    witness: &Value,
    witness_path: &Path,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    let guarantee = witness["replay_guarantee"]
        .as_str()
        .unwrap_or("not_replayable");
    if guarantee != "exact" {
        let opaque = witness["opaque_boundaries"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let assumptions = witness["boundary_assumptions"].clone();
        replay_blocked(guarantee, &opaque, assumptions, error_format)?;
        anyhow::bail!("{guarantee} replay cannot claim exact replay");
    }

    let verified_source = verify_source_identity(witness, witness_path, error_format)?;
    let target = witness_target_scenario(witness)?;
    let document = sim_model::parse_document(verified_source.source.clone());
    let seed = witness["seed"].as_u64().unwrap_or(0);
    let profile = witness["backend_profile"]
        .as_str()
        .or_else(|| witness["guarantee_profile"].as_str())
        .unwrap_or("checked");
    let run = sim_model::run_quick_target(
        &document,
        sim_model::SimulationOptions {
            profile,
            seed,
            inject: None,
            event_budget: None,
        },
        Some(&target),
    );

    let source_display = witness["source"]["path"].as_str().unwrap_or("<unknown>");
    let expected = serde_json::json!({
        "backend": run.backend.as_str(),
        "backend_replay": sim_model::replay_token(&verified_source.hash, seed, &run),
        "failure": failure_json(source_display, &verified_source.source, &run),
        "events": events_json(&run.events),
    });
    let observed = serde_json::json!({
        "backend": witness["backend"].clone(),
        "backend_replay": witness["backend_replay"].clone(),
        "failure": witness["failure"].clone(),
        "events": witness["events"].clone(),
    });
    if expected != observed {
        return replay_divergence(expected, observed, error_format);
    }

    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "replay": "exact",
            "source": verified_source.path.display().to_string(),
            "backend": run.backend.as_str(),
            "failure": witness["failure"],
            "events": run.events.len(),
        }))?
    );
    Ok(())
}

fn verify_source_identity(
    witness: &Value,
    witness_path: &Path,
    error_format: ErrorFormat,
) -> anyhow::Result<VerifiedSource> {
    let Some(source_path) = witness["source"]["path"].as_str() else {
        return source_mismatch("missing source identity", error_format);
    };
    let Some(expected_hash) = witness["source"]["hash"].as_str() else {
        return source_mismatch("missing source hash", error_format);
    };
    let witness_dir = witness_path.parent().unwrap_or_else(|| Path::new("."));
    let source_path = sim_model::resolve_witness_source(source_path, witness_dir);
    let source = match std::fs::read_to_string(&source_path) {
        Ok(source) => source,
        Err(_) => {
            return source_mismatch(
                &format!("source mismatch: missing {}", source_path.display()),
                error_format,
            )
        }
    };
    let observed_hash = sim_model::source_hash(&source);
    if observed_hash != expected_hash {
        return source_mismatch("source mismatch: witness source hash changed", error_format);
    }
    Ok(VerifiedSource {
        path: source_path,
        hash: observed_hash,
        source,
    })
}

fn witness_target_scenario(witness: &Value) -> anyhow::Result<String> {
    let Some(target) = witness["target"].as_str() else {
        return witness_error("missing target");
    };
    let Some((_, scenario)) = target.rsplit_once(':') else {
        return witness_error("target must include source:path:scenario");
    };
    if scenario.trim().is_empty() {
        return witness_error("target scenario must not be empty");
    }
    Ok(scenario.to_owned())
}

fn failure_json(source_path: &str, source: &str, run: &sim_model::SimulationRun) -> Value {
    let Some(failure) = run.failure.as_ref() else {
        return Value::Null;
    };
    serde_json::json!({
        "code": failure.code.as_str(),
        "message": failure.message.clone(),
        "primary_span": format!(
            "{}:{}:1",
            source_path,
            one_based_line_for_offset(source, failure.primary_start),
        ),
        "related_spans": related_spans_json(source_path, source, run),
    })
}

fn related_spans_json(
    source_path: &str,
    source: &str,
    run: &sim_model::SimulationRun,
) -> Vec<Value> {
    run.obligations
        .iter()
        .filter(|obligation| !obligation.is_discharged)
        .map(|obligation| {
            serde_json::json!({
                "label": format!("obligation `{}` declared here", obligation.binding),
                "span": {
                    "path": source_path,
                    "line": one_based_line_for_offset(source, obligation.declaration_span.0),
                    "start": obligation.declaration_span.0,
                    "end": obligation.declaration_span.1.max(obligation.declaration_span.0 + 1),
                },
            })
        })
        .collect()
}

fn events_json(events: &[sim_model::SimEvent]) -> Vec<Value> {
    events
        .iter()
        .map(|event| {
            serde_json::json!({
                "kind": event.kind,
                "label": event.label,
                "value": event.value,
            })
        })
        .collect()
}

fn one_based_line_for_offset(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn replay_divergence(
    expected: Value,
    observed: Value,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    let payload = serde_json::json!({
        "code": "K0104",
        "message": "kwit replay diverged",
        "expected": expected,
        "observed": observed,
    });
    emit_replay_issue(&payload, error_format)?;
    anyhow::bail!("K0104 replay divergence")
}

fn source_mismatch<T>(message: &str, error_format: ErrorFormat) -> anyhow::Result<T> {
    let payload = serde_json::json!({
        "code": "K0104",
        "message": message,
    });
    emit_replay_issue(&payload, error_format)?;
    anyhow::bail!("{message}")
}

fn replay_blocked(
    guarantee: &str,
    opaque_boundaries: &str,
    assumptions: Value,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    let payload = serde_json::json!({
        "code": "K0104",
        "message": format!("{guarantee} replay witness cannot claim exact replay"),
        "replay_guarantee": guarantee,
        "opaque_boundaries": opaque_boundaries,
        "boundary_assumptions": assumptions,
    });
    emit_replay_issue(&payload, error_format)
}

fn emit_replay_issue(payload: &Value, error_format: ErrorFormat) -> anyhow::Result<()> {
    match error_format {
        ErrorFormat::Json => {
            println!("{}", serde_json::to_string(payload)?);
        }
        ErrorFormat::Human => {
            eprintln!(
                "error[{}]: {}",
                payload["code"].as_str().unwrap_or("K0104"),
                payload["message"].as_str().unwrap_or("replay failed"),
            );
            if let Some(guarantee) = payload["replay_guarantee"].as_str() {
                eprintln!("replay guarantee: {guarantee}");
            }
            if let Some(boundaries) = payload["opaque_boundaries"].as_str() {
                if !boundaries.is_empty() {
                    eprintln!("opaque boundaries: {boundaries}");
                }
            }
            if !payload["boundary_assumptions"].is_null() {
                eprintln!("boundary assumptions: {}", payload["boundary_assumptions"]);
            }
            if !payload["expected"].is_null() {
                eprintln!("expected: {}", payload["expected"]);
            }
            if !payload["observed"].is_null() {
                eprintln!("observed: {}", payload["observed"]);
            }
        }
    }
    Ok(())
}

fn validate_witness(witness: &Value) -> anyhow::Result<()> {
    let Some(version) = witness["schema_version"].as_u64() else {
        return witness_error("missing schema_version");
    };
    if version == 1 {
        let mut required = vec![
            &["kobo_version"][..],
            &["target"][..],
            &["guarantee_profile"][..],
            &["seed"][..],
            &["backend_profile"][..],
            &["backend"][..],
            &["backend_replay"][..],
            &["replay_guarantee"][..],
            &["expanded_policy", "ownership"][..],
            &["expanded_policy", "liveness"][..],
            &["expanded_policy", "replay"][..],
            &["expanded_policy", "boundaries"][..],
            &["expanded_policy", "errors"][..],
            &["modeled_boundaries"][..],
            &["opaque_boundaries"][..],
            &["boundary_assumptions"][..],
            &["obligations"][..],
            &["boundary_decisions"][..],
            &["failure", "code"][..],
            &["failure", "primary_span"][..],
            &["failure", "related_spans"][..],
            &["events"][..],
        ];
        if witness["replay_guarantee"].as_str() == Some("exact") {
            required.push(&["source", "path"][..]);
            required.push(&["source", "hash"][..]);
        }
        for path in required {
            if value_at(witness, path).is_none() {
                return witness_error(&format!("missing {}", path.join(".")));
            }
        }
        return Ok(());
    }
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

fn value_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

fn witness_error<T>(message: &str) -> anyhow::Result<T> {
    eprintln!("error[K0115]: {message}");
    eprintln!("help: .kwit schema_version 0 requires source.path and source.span");
    anyhow::bail!("invalid witness")
}
