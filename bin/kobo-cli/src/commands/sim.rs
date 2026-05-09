use std::path::Path;

use anyhow::Context;
use serde_json::json;

pub(super) fn cmd_sim_scout(
    file: &Path,
    json_output: bool,
    backend_recommendations: bool,
) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;

    if backend_recommendations {
        let recommendations = backend_recommendations_for(&source);
        print_value(
            json!({
                "backend_fit": recommendations,
                "executed": false,
                "note": "metadata only; no backend run",
            }),
            json_output,
        )?;
        return Ok(());
    }

    let scout = scout_source(&source, file);
    print_value(scout, json_output)
}

pub(super) fn cmd_sim_backends(json_output: bool) -> anyhow::Result<()> {
    let backends = json!([
        {
            "name": "Loom",
            "backend_fit": ["thread interleavings", "sync concurrency"],
            "executes_in_v085": false
        },
        {
            "name": "Shuttle",
            "backend_fit": ["async schedules", "spawn/select boundaries"],
            "executes_in_v085": false
        },
        {
            "name": "Turmoil",
            "backend_fit": ["network islands", "virtual time"],
            "executes_in_v085": false
        },
        {
            "name": "Madsim",
            "backend_fit": ["distributed simulation", "virtual time"],
            "executes_in_v085": false
        },
        {
            "name": "proptest",
            "backend_fit": ["stateful input", "parser/property checks"],
            "executes_in_v085": false
        },
        {
            "name": "failpoints",
            "backend_fit": ["failure injection", "retry paths"],
            "executes_in_v085": false
        }
    ]);
    print_value(
        json!({ "backends": backends, "executed": false }),
        json_output,
    )
}

fn scout_source(source: &str, file: &Path) -> serde_json::Value {
    let target = scenario_or_function_name(source);
    let mut reasons = scout_reasons(source);

    if reasons.is_empty() {
        return json!({
            "top_target": null,
            "summary": "no high-value simulation target",
            "reasons": [],
            "next_command": null,
        });
    }

    reasons.sort();
    reasons.dedup();
    json!({
        "top_target": target.unwrap_or_else(|| "module".to_owned()),
        "score": reasons.len(),
        "reasons": reasons,
        "expected_first_value": "one liveness, nondeterminism, or boundary finding",
        "next_command": format!("kobo sim scout --json {}", file.display()),
    })
}

fn scout_reasons(source: &str) -> Vec<&'static str> {
    let mut reasons = Vec::new();
    if source.contains("kobo::scenario") {
        reasons.push("scenario metadata");
    }
    if source.contains("tokio::spawn") || source.contains("spawn(") {
        reasons.push("async spawn boundary");
    }
    if source.contains("select!") {
        reasons.push("select cancellation boundary");
    }
    if source.contains("must_call") || contains_obligation_word(source) {
        reasons.push("liveness obligation vocabulary");
    }
    if contains_raw_nondeterminism(source) {
        reasons.push("raw nondeterminism");
    }
    reasons
}

fn backend_recommendations_for(source: &str) -> serde_json::Value {
    if source.contains("tokio::spawn")
        || source.contains("select!")
        || source.contains("async fn")
        || source.contains("async move")
    {
        return json!([
            {"name": "Loom", "backend_fit": "sync concurrency interleavings"},
            {"name": "Shuttle", "backend_fit": "async spawn/select schedule exploration"}
        ]);
    }

    if source.contains("assert(")
        || source.contains("parse(")
        || source.to_ascii_lowercase().contains("property")
    {
        return json!([
            {"name": "proptest", "backend_fit": "input and property exploration"}
        ]);
    }

    json!([
        {"name": "failpoints", "backend_fit": "manual failure injection points"}
    ])
}

fn scenario_or_function_name(source: &str) -> Option<String> {
    if let Some(name) = extract_scenario_name(source) {
        return Some(name);
    }

    source
        .lines()
        .map(str::trim)
        .find(|line| {
            line.starts_with("fn ")
                || line.starts_with("async fn ")
                || line.starts_with("pub fn ")
                || line.starts_with("pub async fn ")
        })
        .map(super::debt::extract_fn_name_from_line)
}

fn extract_scenario_name(source: &str) -> Option<String> {
    let marker = "scenario";
    let name_marker = "name";
    for line in source.lines().map(str::trim) {
        if !line.contains(marker) || !line.contains(name_marker) {
            continue;
        }
        let name_start = line.find(name_marker)?;
        let after_name = &line[name_start + name_marker.len()..];
        let quote_start = after_name.find('"')?;
        let rest = &after_name[quote_start + 1..];
        let quote_end = rest.find('"')?;
        return Some(rest[..quote_end].to_owned());
    }
    None
}

fn contains_raw_nondeterminism(source: &str) -> bool {
    source.contains("SystemTime::now")
        || source.contains("Instant::now")
        || source.contains("rand::")
        || source.contains("thread_rng")
        || source.contains("std::fs::")
        || source.contains("std::process::")
}

fn contains_obligation_word(source: &str) -> bool {
    [
        "commit", "rollback", "ack", "nack", "reply", "cancel", "finish", "abort",
    ]
    .iter()
    .any(|word| source.contains(word))
}

fn print_value(value: serde_json::Value, json_output: bool) -> anyhow::Result<()> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&value).context("failed to serialize scout output")?
        );
    } else {
        println!("{value}");
    }
    Ok(())
}
