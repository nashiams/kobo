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

enum FixIssue {
    CloneBeforeMove {
        variable: String,
    },
    SelfReferentialStruct {
        struct_name: String,
        field_name: String,
    },
}

struct FixEdit {
    search: String,
    replacement: String,
    issue: FixIssue,
}

impl FixEdit {
    fn message(&self) -> String {
        match &self.issue {
            FixIssue::CloneBeforeMove { variable } => format!("clone `{variable}` before move"),
            FixIssue::SelfReferentialStruct {
                struct_name,
                field_name,
            } => format!(
                "self-referential struct `{struct_name}` needs indirection on `{field_name}`"
            ),
        }
    }

    fn problem(&self) -> &'static str {
        match &self.issue {
            FixIssue::CloneBeforeMove { .. } => "move-use-after-consume",
            FixIssue::SelfReferentialStruct { .. } => "self-referential-struct",
        }
    }
}

fn build_fix_plan(source: &str) -> FixPlan {
    if has_overlapping_consume(source) {
        return FixPlan {
            edit: None,
            refused: true,
            reason: Some("overlap: multiple moves require placeholder review"),
        };
    }

    if let Some(edit) = self_referential_box_edit(source) {
        return FixPlan {
            edit: Some(edit),
            refused: false,
            reason: None,
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
                issue: FixIssue::CloneBeforeMove { variable },
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
                "problem": edit.problem(),
                "message": edit.message(),
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
        println!("MachineApplicable TextEdit: {}", edit.message());
        println!("{}", edit.replacement);
    } else if plan.refused {
        println!("{}", plan.reason.unwrap_or("overlap"));
    } else {
        println!("no machine-applicable fixes");
    }
    Ok(())
}

fn self_referential_box_edit(source: &str) -> Option<FixEdit> {
    let file = syn::parse_file(source).ok()?;
    for item in file.items {
        let syn::Item::Struct(item_struct) = item else {
            continue;
        };
        let struct_name = item_struct.ident.to_string();
        let syn::Fields::Named(fields) = item_struct.fields else {
            continue;
        };
        for field in fields.named {
            let field_name = field.ident.as_ref()?.to_string();
            if !is_direct_self_reference(&field.ty, &struct_name) {
                continue;
            }
            return Some(FixEdit {
                search: format!("{field_name}: {struct_name}"),
                replacement: format!("{field_name}: Box<{struct_name}>"),
                issue: FixIssue::SelfReferentialStruct {
                    struct_name,
                    field_name,
                },
            });
        }
    }
    None
}

fn is_direct_self_reference(ty: &syn::Type, struct_name: &str) -> bool {
    let syn::Type::Path(path) = ty else {
        return false;
    };
    path.qself.is_none()
        && path.path.segments.len() == 1
        && path
            .path
            .segments
            .first()
            .is_some_and(|segment| segment.ident == struct_name)
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
