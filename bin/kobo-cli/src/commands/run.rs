use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;
use kobo_driver::{
    load_config, run_and_compile, run_and_compile_with_lifetime_erasure, run_codegen_pipeline,
    run_kir_phase, CodegenArtifacts,
};
use kobo_ir::{GuaranteePolicy, MustCallObligation};

use super::{
    ownership_analysis, policy,
    session::{build_session, line_number_for_offset, render_diagnostics},
};
use crate::{ErrorFormat, GuaranteeProfileArg};

pub(super) fn cmd_run(
    file: &Path,
    cli_policy: Option<GuaranteePolicy>,
    guarantee_profile: Option<GuaranteeProfileArg>,
    erase_lifetimes: bool,
) -> anyhow::Result<()> {
    let guarantee_policy = if let Some(profile) = guarantee_profile {
        let loaded = policy::load_effective_policy(Some(file), profile)?;
        if let Some(downgrade) = loaded.downgrade() {
            policy::emit_downgrade(downgrade, ErrorFormat::Human)?;
            anyhow::bail!("guarantee policy downgrade requires reason ledger entry");
        }
        Some(loaded)
    } else {
        None
    };
    let session_policy = guarantee_policy
        .as_ref()
        .map(|policy| policy.compiler_policy().clone())
        .or(cli_policy);
    let mut session = build_session(file, session_policy)?;

    let compile_result = if erase_lifetimes {
        run_and_compile_with_lifetime_erasure(&mut session, file)
    } else {
        run_and_compile(&mut session, file)
    };
    let binary_path = compile_result.map_err(|()| {
        render_diagnostics(&session);
        anyhow::anyhow!("compilation failed")
    })?;
    // v0.6 §3.3b: Render K-code warnings on success path [BUG-01 / R02].
    render_diagnostics(&session);
    if let Some(policy) = guarantee_policy.as_ref() {
        policy::emit_policy_summary(policy);
    }
    if !binary_path.is_file() {
        anyhow::bail!("compiled binary missing at {}", binary_path.display());
    }

    let mut run_cmd = Command::new(&binary_path);
    // Runtime layer [G1 §1.4 / R6-04]: in checked profile DiagOwner output must
    // appear without requiring the user to set KOBO_DIAG=1 manually.
    // We propagate KOBO_CHECKED_MODE=1 to the child so kobo-diag::DiagOwner
    // knows to emit on Drop even without the manual opt-in env var.
    if session.guarantee_policy().is_checked() {
        run_cmd.env("KOBO_CHECKED_MODE", "1");
    }
    let run_status = run_cmd
        .status()
        .with_context(|| format!("failed to run {}", binary_path.display()))?;

    if !run_status.success() {
        anyhow::bail!("program exited with non-zero status");
    }

    Ok(())
}

pub(super) fn cmd_inspect(
    file: &Path,
    cli_policy: Option<GuaranteePolicy>,
    clean: bool,
    erase_lifetimes: bool,
    scenario_metadata: bool,
    sim: bool,
    harness: bool,
    cargo_dir: Option<&Path>,
    profile: Option<&str>,
    trait_default: Option<&str>,
    audit: Option<&str>,
) -> anyhow::Result<()> {
    let mut session = build_session(file, cli_policy.clone())?;
    let summary_usages = validate_configured_summaries(&session)?;

    if audit == Some("json") {
        let source = std::fs::read_to_string(file)
            .with_context(|| format!("failed to read {}", file.display()))?;
        print!("{}", audit_json_output(file, &source)?);
        return Ok(());
    }

    if sim {
        let source = std::fs::read_to_string(file)
            .with_context(|| format!("failed to read {}", file.display()))?;
        let output = simulation_transparency_output(&source, harness);
        eprintln!(
            "// effective guarantee profile: {}",
            session.guarantee_profile().as_str()
        );
        print!("{output}");
        return Ok(());
    }

    if let Some(dir) = cargo_dir {
        let InspectCargoOutput {
            project_config,
            source_files,
            main_output,
        } = build_inspect_cargo_output(file, cli_policy.clone(), erase_lifetimes)?;

        kobo_codegen::cargo_gen::generate_cargo_project(&project_config, &source_files, dir)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        eprintln!("cargo project generated at {}", dir.display());
        eprintln!(
            "// effective guarantee profile: {}",
            session.guarantee_profile().as_str()
        );
        print!("{main_output}");
        return Ok(());
    }

    let CodegenArtifacts {
        rs_source,
        must_call_obligations,
        ..
    } = run_codegen_pipeline(&mut session, file).map_err(|()| {
        render_diagnostics(&session);
        anyhow::anyhow!("compilation failed")
    })?;
    // v0.6 §3.3b: Render K-code warnings on success path too [R6-06].
    render_diagnostics(&session);

    // S-21: Apply lifetime erasure when requested for compatibility-profile output.
    let rs_source = if erase_lifetimes {
        kobo_driver::apply_lifetime_erasure(&rs_source, session.guarantee_policy())
    } else {
        rs_source
    };

    let output = if scenario_metadata {
        let source = std::fs::read_to_string(file)
            .with_context(|| format!("failed to read {}", file.display()))?;
        scenario_metadata_output(&source)
    } else if clean {
        kobo_codegen::clean::strip_kobo_wrappers(&rs_source)
    } else {
        // Annotate lock acquisition order for inspect output.
        kobo_codegen::annotate_lock_order(&rs_source)
    };
    let output = if scenario_metadata {
        output
    } else {
        let source = std::fs::read_to_string(file)
            .with_context(|| format!("failed to read {}", file.display()))?;
        let output = append_must_call_metadata(output, &must_call_obligations);
        append_trait_facade_code(output, &source, profile, trait_default)
    };

    // S-26 compatibility: show the profile-equivalent view of legacy mode resolution.
    eprintln!(
        "// effective guarantee profile: {}",
        session.guarantee_profile().as_str()
    );
    for usage in &summary_usages {
        println!(
            "// kobo-summary: crate={} hash={} version={}",
            usage.crate_name, usage.hash, usage.schema_version
        );
    }
    print!("{output}");
    Ok(())
}

struct SummaryUsage {
    crate_name: String,
    hash: String,
    schema_version: u64,
}

fn validate_configured_summaries(
    session: &kobo_driver::CompileSession,
) -> anyhow::Result<Vec<SummaryUsage>> {
    let mut usages = Vec::new();
    for summary in &session.config.ecosystem_policy.summaries {
        let source = fs::read_to_string(&summary.path)
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
        let actual = parsed
            .get("summary_hash")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| stable_hash(&source));
        if actual != summary.hash {
            anyhow::bail!(
                "K0126: .kobo-summary hash mismatch for {}: expected {}, found {}",
                summary.crate_name,
                summary.hash,
                actual
            );
        }
        usages.push(SummaryUsage {
            crate_name: summary.crate_name.clone(),
            hash: actual,
            schema_version,
        });
    }
    Ok(usages)
}

fn stable_hash(source: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in source.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn audit_json_output(file: &Path, source: &str) -> anyhow::Result<String> {
    let file_label = cli_relative_path(file)?;
    let analysis = ownership_analysis::analyze_source(source);
    let entries = analysis
        .audit_records
        .into_iter()
        .map(|record| {
            serde_json::json!({
                "tier": record.tier,
                "kind": record.kind,
                "file": file_label,
                "line": record.line,
                "span": {
                    "line": record.line,
                    "column": record.column,
                },
                "evidence": record.evidence,
                "provenance": {
                    "source": "parser-kir-solver",
                    "rule": "ownership-audit-record",
                },
            })
        })
        .collect::<Vec<_>>();

    let value = serde_json::json!({
        "schema_version": 1,
        "provenance": {
            "source": "parser-kir-solver",
            "confidence": "evidence-backed",
        },
        "audit": entries,
    });
    Ok(format!("{}\n", serde_json::to_string(&value)?))
}

fn simulation_transparency_output(source: &str, harness: bool) -> String {
    let command = if harness {
        "inspect --sim --harness"
    } else {
        "inspect --sim"
    };
    let mut output = String::new();
    output.push_str(&format!(
        "// kobo: {command} v0.10 simulation facade transparency path built on the v0.9 checked simulation MVP\n"
    ));
    output.push_str(
        "// kobo: posture: Kobo is Rust-shaped and Cargo-native; normal Kobo source stays framework-shaped\n",
    );
    output.push_str("// kobo: simulation facade surfaces: time, random, spawn, task, select, sync, storage, network, failpoint\n");
    output.push_str("// kobo: generated facades apply only to sim/replay builds and inspectable harness artifacts\n");
    output.push_str("// kobo: possible backend adapter engines: Loom, Shuttle, Turmoil, Madsim, proptest, failpoints\n");
    if harness {
        output.push_str("// kobo: backend adapter boundary: generated harness owns backend-native imports; user source remains normal\n");
    } else {
        output.push_str("// kobo: use --harness to inspect generated backend adapter boundaries\n");
    }
    if source.contains("kobo::scenario") {
        output.push_str("// kobo: scenario metadata detected; scout can explain backend fit\n");
    }
    if source.contains("tokio::spawn") || source.contains("async fn") {
        output.push_str("// kobo: async facade: spawn/task/select/cancellation schedule points are modeled in sim\n");
    }
    if source.contains("ward.storage") {
        output.push_str("// kobo: storage facade: crash/write/recover hooks are modeled in sim\n");
    }
    if source.contains("ward.network") || source.contains("reqwest::") {
        output.push_str("// kobo: network facade: modeled ports can drop, delay, reorder, or require boundary policy\n");
    }
    output
}

fn append_scenario_metadata(mut output: String, source: &str) -> String {
    let scenarios = scenario_metadata_from_source(source);
    if scenarios.is_empty() {
        return output;
    }

    if !output.ends_with('\n') {
        output.push('\n');
    }
    for scenario in scenarios {
        if scenario.tags.is_empty() {
            output.push_str(&format!("// kobo: scenario {}\n", scenario.name));
        } else {
            output.push_str(&format!(
                "// kobo: scenario {} tags: {}\n",
                scenario.name,
                scenario.tags.join(", ")
            ));
        }
    }
    output
}

fn scenario_metadata_output(source: &str) -> String {
    append_scenario_metadata(String::new(), source)
}

struct ScenarioMetadata {
    name: String,
    tags: Vec<String>,
}

fn scenario_metadata_from_source(source: &str) -> Vec<ScenarioMetadata> {
    source
        .lines()
        .map(str::trim)
        .filter(|line| line.contains("kobo::scenario"))
        .filter_map(parse_scenario_metadata_line)
        .collect()
}

fn parse_scenario_metadata_line(line: &str) -> Option<ScenarioMetadata> {
    let name = extract_named_string(line, "name")?;
    let tags = extract_tag_list(line);
    Some(ScenarioMetadata { name, tags })
}

fn extract_named_string(line: &str, key: &str) -> Option<String> {
    let key_start = line.find(key)?;
    let after_key = &line[key_start + key.len()..];
    let quote_start = after_key.find('"')?;
    let rest = &after_key[quote_start + 1..];
    let quote_end = rest.find('"')?;
    Some(rest[..quote_end].to_owned())
}

fn extract_tag_list(line: &str) -> Vec<String> {
    let Some(tags_start) = line.find("tags") else {
        return Vec::new();
    };
    let after_tags = &line[tags_start..];
    let Some(list_start) = after_tags.find('[') else {
        return Vec::new();
    };
    let Some(list_end) = after_tags[list_start + 1..].find(']') else {
        return Vec::new();
    };
    after_tags[list_start + 1..list_start + 1 + list_end]
        .split(',')
        .filter_map(|part| {
            let trimmed = part.trim();
            let first = trimmed.find('"')?;
            let rest = &trimmed[first + 1..];
            let second = rest.find('"')?;
            Some(rest[..second].to_owned())
        })
        .collect()
}

fn append_must_call_metadata(mut output: String, obligations: &[MustCallObligation]) -> String {
    if obligations.is_empty() {
        return output;
    }

    if !output.ends_with('\n') {
        output.push('\n');
    }
    for obligation in obligations {
        let actions = obligation
            .actions
            .iter()
            .map(|action| action.name.as_str())
            .collect::<Vec<_>>()
            .join(" | ");
        output.push_str(&format!(
            "// kobo: must_call {} => {}\n",
            obligation.owner_type, actions
        ));
    }
    output
}

fn append_trait_facade_code(
    mut output: String,
    source: &str,
    profile: Option<&str>,
    trait_default: Option<&str>,
) -> String {
    let default = trait_default.unwrap_or_else(|| {
        if source.contains("kobo::generic") || profile == Some("game-server-core") {
            "generic"
        } else {
            "trait-object-first"
        }
    });

    let facade = if default == "generic" || source.contains("kobo::generic") {
        generic_facade_code(source)
    } else {
        trait_object_facade_code(source, profile)
    };
    let Some(facade) = facade else {
        return output;
    };

    if !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str(&facade);
    output.push('\n');
    output
}

fn trait_object_facade_code(source: &str, profile: Option<&str>) -> Option<String> {
    let target = trait_object_facade_target(source)?;
    let profile = profile.unwrap_or("script");
    Some(format!(
        "#[doc = \"thin facade trait-object-first profile {profile}\"]\n\
         #[allow(dead_code)]\n\
         pub fn {name}_thin_facade({param}: Box<dyn {trait_name}>) {return_ty} {{\n\
             {name}({param})\n\
         }}",
        name = target.function,
        param = target.param,
        trait_name = target.trait_name,
        return_ty = target.return_ty.unwrap_or_default()
    ))
}

fn generic_facade_code(source: &str) -> Option<String> {
    let target = generic_facade_target(source)?;
    Some(format!(
        "#[doc = \"generic facade\"]\n\
         #[allow(dead_code)]\n\
         pub fn {name}_generic_facade<{type_param}: {trait_name}>({param}: {type_param}) {return_ty} {{\n\
             {name}({param})\n\
         }}",
        name = target.function,
        type_param = target.type_param,
        trait_name = target.trait_name,
        param = target.param,
        return_ty = target.return_ty.unwrap_or_default()
    ))
}

struct TraitObjectFacadeTarget {
    function: String,
    param: String,
    trait_name: String,
    return_ty: Option<String>,
}

struct GenericFacadeTarget {
    function: String,
    type_param: String,
    trait_name: String,
    param: String,
    return_ty: Option<String>,
}

fn trait_object_facade_target(source: &str) -> Option<TraitObjectFacadeTarget> {
    source.lines().find_map(|line| {
        let signature = function_signature(line)?;
        let open = signature.find('(')?;
        let close = signature.rfind(')')?;
        let function = inspect_ident_suffix(&signature[..open])?;
        let param_text = signature[open + 1..close].split(',').next()?.trim();
        let (param, ty) = param_text.split_once(':')?;
        let trait_name = ty.trim().strip_prefix("dyn ")?;
        Some(TraitObjectFacadeTarget {
            function,
            param: inspect_ident_prefix(param)?,
            trait_name: inspect_ident_prefix(trait_name)?,
            return_ty: return_type_from_signature(signature),
        })
    })
}

fn generic_facade_target(source: &str) -> Option<GenericFacadeTarget> {
    source.lines().find_map(|line| {
        let signature = function_signature(line)?;
        let open_generics = signature.find('<')?;
        let close_generics = signature[open_generics + 1..].find('>')? + open_generics + 1;
        let open_params = signature[close_generics + 1..].find('(')? + close_generics + 1;
        let close_params = signature.rfind(')')?;
        let function = inspect_ident_suffix(&signature[..open_generics])?;
        let generic = signature[open_generics + 1..close_generics].trim();
        let (type_param, trait_name) = generic.split_once(':')?;
        let param_text = signature[open_params + 1..close_params]
            .split(',')
            .next()?
            .trim();
        let (param, _) = param_text.split_once(':')?;
        Some(GenericFacadeTarget {
            function,
            type_param: inspect_ident_prefix(type_param)?,
            trait_name: inspect_ident_prefix(trait_name.trim())?,
            param: inspect_ident_prefix(param)?,
            return_ty: return_type_from_signature(signature),
        })
    })
}

fn function_signature(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let signature = trimmed
        .strip_prefix("pub fn ")
        .or_else(|| trimmed.strip_prefix("fn "))?;
    Some(signature.trim_end_matches('{').trim())
}

fn return_type_from_signature(signature: &str) -> Option<String> {
    let arrow = signature.find("->")?;
    let return_ty = signature[arrow..].trim();
    (!return_ty.is_empty()).then(|| format!(" {return_ty}"))
}

fn inspect_ident_prefix(input: &str) -> Option<String> {
    let ident = input
        .trim_start()
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect::<String>();
    (!ident.is_empty()).then_some(ident)
}

fn inspect_ident_suffix(input: &str) -> Option<String> {
    input
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .filter(|part| !part.is_empty())
        .next_back()
        .map(str::to_owned)
}

struct InspectCargoOutput {
    project_config: kobo_codegen::cargo_gen::KoboProjectConfig,
    source_files: Vec<(PathBuf, String)>,
    main_output: String,
}

fn build_inspect_cargo_output(
    file: &Path,
    cli_policy: Option<GuaranteePolicy>,
    erase_lifetimes: bool,
) -> anyhow::Result<InspectCargoOutput> {
    let absolute_file = absolute_input_path(file)?;
    let Some(project_root) = find_nearest_kobo_project_root(&absolute_file) else {
        return build_single_file_inspect_cargo_output(file, cli_policy, erase_lifetimes);
    };

    let driver_config = load_config(&project_root)
        .with_context(|| format!("failed to load config for {}", project_root.display()))?;
    let project_config = cargo_project_config_from_root(&project_root)?;
    let src_dir = project_root.join(&driver_config.src_dir);
    let mut kobo_files = Vec::new();
    collect_kobo_files(&src_dir, &mut kobo_files);
    if kobo_files.is_empty() {
        return build_single_file_inspect_cargo_output(file, cli_policy, erase_lifetimes);
    }

    let canonical_input = absolute_file
        .canonicalize()
        .unwrap_or(absolute_file.clone());
    let mut source_files = Vec::new();
    let mut main_output = None;

    for kobo_file in kobo_files {
        let mut session = build_session(&kobo_file, cli_policy.clone())?;
        let CodegenArtifacts { rs_source, .. } = run_codegen_pipeline(&mut session, &kobo_file)
            .map_err(|()| {
                render_diagnostics(&session);
                anyhow::anyhow!("compilation failed")
            })?;
        render_diagnostics(&session);

        let rs_source = if erase_lifetimes {
            kobo_driver::apply_lifetime_erasure(&rs_source, session.guarantee_policy())
        } else {
            rs_source
        };
        let clean_source = kobo_codegen::clean::strip_kobo_wrappers(&rs_source);
        let rel = kobo_file
            .strip_prefix(&src_dir)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| {
                kobo_file
                    .file_name()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("main.kobo"))
            });

        if kobo_file.canonicalize().unwrap_or(kobo_file.clone()) == canonical_input {
            main_output = Some(clean_source.clone());
        }
        source_files.push((rel, clean_source));
    }

    Ok(InspectCargoOutput {
        project_config,
        source_files,
        main_output: main_output.unwrap_or_default(),
    })
}

fn build_single_file_inspect_cargo_output(
    file: &Path,
    cli_policy: Option<GuaranteePolicy>,
    erase_lifetimes: bool,
) -> anyhow::Result<InspectCargoOutput> {
    let mut session = build_session(file, cli_policy)?;
    let CodegenArtifacts { rs_source, .. } =
        run_codegen_pipeline(&mut session, file).map_err(|()| {
            render_diagnostics(&session);
            anyhow::anyhow!("compilation failed")
        })?;
    render_diagnostics(&session);

    let rs_source = if erase_lifetimes {
        kobo_driver::apply_lifetime_erasure(&rs_source, session.guarantee_policy())
    } else {
        rs_source
    };
    let output = kobo_codegen::clean::strip_kobo_wrappers(&rs_source);
    let project_config = cargo_project_config_from_optional_adjacent_toml(file)?;

    Ok(InspectCargoOutput {
        project_config,
        source_files: vec![(file.to_path_buf(), output.clone())],
        main_output: output,
    })
}

fn cargo_project_config_from_root(
    project_root: &Path,
) -> anyhow::Result<kobo_codegen::cargo_gen::KoboProjectConfig> {
    let cargo_toml_path = project_root.join("Cargo.toml");
    if cargo_toml_path.exists() {
        let cargo_toml = fs::read_to_string(&cargo_toml_path)
            .with_context(|| format!("failed to read {}", cargo_toml_path.display()))?;
        let rewritten = cargo_toml_with_absolute_dependency_paths(project_root, &cargo_toml)?;
        return kobo_codegen::cargo_gen::KoboProjectConfig::from_toml(&rewritten)
            .map_err(|e| anyhow::anyhow!("{e}"));
    }

    let kobo_toml_path = project_root.join("Kobo.toml");
    if !kobo_toml_path.exists() {
        return Ok(kobo_codegen::cargo_gen::KoboProjectConfig::default());
    }

    let toml_str = fs::read_to_string(&kobo_toml_path)
        .with_context(|| format!("failed to read {}", kobo_toml_path.display()))?;
    kobo_codegen::cargo_gen::KoboProjectConfig::from_toml(&toml_str)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

fn cargo_toml_with_absolute_dependency_paths(
    project_root: &Path,
    source: &str,
) -> anyhow::Result<String> {
    let mut parsed = source
        .parse::<toml::Value>()
        .context("failed to parse Cargo.toml")?;
    rewrite_dependency_paths(project_root, &mut parsed);
    toml::to_string(&parsed).context("failed to serialize Cargo.toml")
}

fn rewrite_dependency_paths(project_root: &Path, manifest: &mut toml::Value) {
    let Some(dependencies) = manifest
        .get_mut("dependencies")
        .and_then(toml::Value::as_table_mut)
    else {
        return;
    };
    for (_, value) in dependencies.iter_mut() {
        let Some(table) = value.as_table_mut() else {
            continue;
        };
        let Some(path_value) = table
            .get("path")
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        if Path::new(&path_value).is_absolute() {
            continue;
        }
        table.insert(
            "path".to_owned(),
            toml::Value::String(normalize_manifest_path(project_root.join(path_value))),
        );
    }
}

fn normalize_manifest_path(path: PathBuf) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn cargo_project_config_from_optional_adjacent_toml(
    file: &Path,
) -> anyhow::Result<kobo_codegen::cargo_gen::KoboProjectConfig> {
    let kobo_toml_path = file.parent().unwrap_or(Path::new(".")).join("Kobo.toml");
    if !kobo_toml_path.exists() {
        return Ok(kobo_codegen::cargo_gen::KoboProjectConfig::default());
    }

    let toml_str = fs::read_to_string(&kobo_toml_path)
        .with_context(|| format!("failed to read {}", kobo_toml_path.display()))?;
    kobo_codegen::cargo_gen::KoboProjectConfig::from_toml(&toml_str)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

fn collect_kobo_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_kobo_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "kobo") {
            out.push(path);
        }
    }
    out.sort();
}

fn absolute_input_path(file: &Path) -> anyhow::Result<PathBuf> {
    if file.is_absolute() {
        Ok(file.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .context("failed to determine current directory")?
            .join(file))
    }
}

fn cli_relative_path(file: &Path) -> anyhow::Result<String> {
    let absolute = absolute_input_path(file)?;
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let display_path = absolute.strip_prefix(&cwd).unwrap_or(&absolute);
    Ok(display_path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn find_nearest_kobo_project_root(file: &Path) -> Option<PathBuf> {
    let search_root = file.parent().unwrap_or(file);
    for ancestor in search_root.ancestors() {
        if ancestor.join("Kobo.toml").is_file() || ancestor.join("Cargo.toml").is_file() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

pub(super) fn cmd_dump(file: &Path) -> anyhow::Result<()> {
    let mut session = build_session(file, None)?;
    let (_, kir) = run_kir_phase(&mut session, file)
        .map_err(|()| anyhow::anyhow!("failed to build KIR for {}", file.display()))?;

    for node in kir.iter_nodes() {
        let entry = session
            .file_set()
            .get(node.span.file_id)
            .context("missing file entry for KIR node")?;
        let line = line_number_for_offset(&entry.source, node.span.start);
        let ast_id = node
            .ast_id
            .map(|id| id.0.to_string())
            .unwrap_or_else(|| "-".to_owned());

        println!(
            "[{}] kind={:?} span={}:{} tier={:?} ast={}",
            node.id.0,
            node.kind,
            entry.path.display(),
            line,
            node.ownership,
            ast_id,
        );
    }

    Ok(())
}
