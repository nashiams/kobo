use std::path::Path;

use anyhow::Context;

pub(super) fn cmd_fix(file: &Path, dry_run: bool, apply: bool, json: bool) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;
    let plan = build_fix_plan(&source);

    if dry_run || !apply {
        print_plan(&plan, json)?;
        return Ok(());
    }

    if plan.refused {
        print_plan(&plan, json)?;
        anyhow::bail!("fix refused: overlapping or placeholder edits");
    }

    let Some(edit) = plan.edit.as_ref() else {
        println!("no machine-applicable fixes");
        return Ok(());
    };
    let updated = source.replacen(&edit.search, &edit.replacement, 1);
    if updated != source {
        std::fs::write(file, updated)
            .with_context(|| format!("failed to write {}", file.display()))?;
    }
    println!("applied 1 machine-applicable fix");
    Ok(())
}

struct FixPlan {
    edit: Option<FixEdit>,
    refused: bool,
    reason: Option<&'static str>,
}

struct FixEdit {
    search: String,
    replacement: String,
    variable: String,
}

fn build_fix_plan(source: &str) -> FixPlan {
    if has_overlapping_consume(source) {
        return FixPlan {
            edit: None,
            refused: true,
            reason: Some("overlap: multiple moves require placeholder review"),
        };
    }

    for line in source.lines().map(str::trim) {
        let Some(variable) = consume_variable(line) else {
            continue;
        };
        if line.contains(".clone()") {
            continue;
        }
        if !source.contains(&format!("println!(\"{{}}\", {variable})")) {
            continue;
        }
        return FixPlan {
            edit: Some(FixEdit {
                search: format!("consume({variable});"),
                replacement: format!("consume({variable}.clone());"),
                variable,
            }),
            refused: false,
            reason: None,
        };
    }

    FixPlan {
        edit: None,
        refused: false,
        reason: None,
    }
}

fn print_plan(plan: &FixPlan, json: bool) -> anyhow::Result<()> {
    if json {
        let value = if let Some(edit) = plan.edit.as_ref() {
            serde_json::json!({
                "applicability": "MachineApplicable",
                "kind": "TextEdit",
                "message": format!("clone `{}` before move", edit.variable),
                "replacement": edit.replacement,
            })
        } else if plan.refused {
            serde_json::json!({
                "applicability": "HasPlaceholders",
                "kind": "TextEdit",
                "refused": true,
                "reason": plan.reason.unwrap_or("overlap"),
            })
        } else {
            serde_json::json!({"fixes": []})
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&value).context("failed to serialize fix plan")?
        );
    } else if let Some(edit) = plan.edit.as_ref() {
        println!("MachineApplicable TextEdit: {}", edit.replacement);
    } else if plan.refused {
        println!("{}", plan.reason.unwrap_or("overlap"));
    } else {
        println!("no machine-applicable fixes");
    }
    Ok(())
}

fn consume_variable(line: &str) -> Option<String> {
    let inner = line.strip_prefix("consume(")?.strip_suffix(");")?;
    if inner.contains(',') || inner.contains('.') {
        return None;
    }
    let variable = inner.trim();
    if variable.is_empty() {
        None
    } else {
        Some(variable.to_owned())
    }
}

fn has_overlapping_consume(source: &str) -> bool {
    let mut seen = std::collections::HashSet::new();
    for line in source.lines().map(str::trim) {
        let Some(variable) = consume_variable(line) else {
            continue;
        };
        if !seen.insert(variable) {
            return true;
        }
    }
    false
}
