use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;
use kobo_driver::{
    load_config, run_and_compile, run_and_compile_with_lifetime_erasure, run_codegen_pipeline,
    run_kir_phase, CodegenArtifacts,
};
use kobo_ir::{KoboMode, MustCallObligation};

use super::session::{build_session, line_number_for_offset, render_diagnostics};

pub(super) fn cmd_run(
    file: &Path,
    cli_mode: Option<KoboMode>,
    erase_lifetimes: bool,
) -> anyhow::Result<()> {
    let mut session = build_session(file, cli_mode)?;
    if session.mode() == KoboMode::Strict {
        eprintln!("error: --strict mode is not yet implemented (target: v0.9)");
        eprintln!("hint: use --checked for advisory ownership warnings");
        std::process::exit(1);
    }

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
    if !binary_path.is_file() {
        anyhow::bail!("compiled binary missing at {}", binary_path.display());
    }

    let mut run_cmd = Command::new(&binary_path);
    // Runtime layer [G1 §1.4 / R6-04]: in checked mode DiagOwner output must
    // appear without requiring the user to set KOBO_DIAG=1 manually.
    // We propagate KOBO_CHECKED_MODE=1 to the child so kobo-diag::DiagOwner
    // knows to emit on Drop even without the manual opt-in env var.
    if session.mode().is_checked() {
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
    cli_mode: Option<KoboMode>,
    clean: bool,
    erase_lifetimes: bool,
    scenario_metadata: bool,
    cargo_dir: Option<&Path>,
    profile: Option<&str>,
    trait_default: Option<&str>,
) -> anyhow::Result<()> {
    let mut session = build_session(file, cli_mode)?;
    if session.mode() == KoboMode::Strict {
        eprintln!("error: --strict mode is not yet implemented (target: v0.9)");
        eprintln!("hint: use --checked for advisory ownership warnings");
        std::process::exit(1);
    }

    if let Some(dir) = cargo_dir {
        let InspectCargoOutput {
            project_config,
            source_files,
            main_output,
        } = build_inspect_cargo_output(file, cli_mode, erase_lifetimes)?;

        kobo_codegen::cargo_gen::generate_cargo_project(&project_config, &source_files, dir)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        eprintln!("cargo project generated at {}", dir.display());
        eprintln!("// effective mode: {}", session.mode());
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

    // S-21: Apply lifetime erasure when requested (script mode only).
    let rs_source = if erase_lifetimes {
        kobo_driver::apply_lifetime_erasure(&rs_source, session.mode())
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
        let output = append_field_capability_metadata(output, &source);
        append_trait_default_metadata(output, &source, profile, trait_default)
    };

    // S-26: Show effective mode so user can verify per-module mode resolution.
    eprintln!("// effective mode: {}", session.mode());
    print!("{output}");
    Ok(())
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

fn append_field_capability_metadata(mut output: String, source: &str) -> String {
    let views = inspect_field_capability_views(source);
    if views.is_empty() {
        return output;
    }

    if !output.ends_with('\n') {
        output.push('\n');
    }
    for view in views {
        for field in view.fields {
            output.push_str(&format!(
                "// kobo: using {} => &{}.{}\n",
                view.owner, view.owner, field
            ));
        }
    }
    output
}

fn append_trait_default_metadata(
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

    let metadata = if default == "generic" || source.contains("kobo::generic") {
        generic_facade_metadata(source)
    } else {
        trait_object_facade_metadata(source, profile)
    };
    let Some(metadata) = metadata else {
        return output;
    };

    if !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str(&metadata);
    output.push('\n');
    output
}

fn trait_object_facade_metadata(source: &str, profile: Option<&str>) -> Option<String> {
    let trait_name = first_trait_name(source)?;
    let profile = profile.unwrap_or("script");
    Some(format!(
        "// kobo: thin facade trait-object-first Box<dyn {trait_name}> profile {profile}"
    ))
}

fn generic_facade_metadata(source: &str) -> Option<String> {
    let signature = source.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.starts_with("fn ") && trimmed.contains('<') {
            return Some(trimmed.trim_end_matches('{').trim().to_owned());
        }
        if trimmed.starts_with("pub fn ") && trimmed.contains('<') {
            return Some(trimmed.trim_end_matches('{').trim().to_owned());
        }
        None
    })?;
    Some(format!("// kobo: generic facade {signature}"))
}

fn first_trait_name(source: &str) -> Option<String> {
    source.lines().find_map(|line| {
        let trimmed = line.trim();
        let rest = trimmed.strip_prefix("trait ")?;
        inspect_ident_prefix(rest)
    })
}

struct InspectFieldCapabilityView {
    owner: String,
    fields: Vec<String>,
}

fn inspect_field_capability_views(source: &str) -> Vec<InspectFieldCapabilityView> {
    let mut views = Vec::new();
    let mut search_start = 0usize;
    while let Some(relative) = source[search_start..].find(" using {") {
        let using_start = search_start + relative;
        let list_start = using_start + " using {".len();
        let Some(close_relative) = source[list_start..].find('}') else {
            break;
        };
        let list_end = list_start + close_relative;
        let owner = inspect_field_capability_owner(source, using_start)
            .unwrap_or_else(|| "value".to_owned());
        let fields = source[list_start..list_end]
            .split(',')
            .filter_map(|part| inspect_field_capability_name(part.trim()))
            .collect::<Vec<_>>();
        views.push(InspectFieldCapabilityView { owner, fields });
        search_start = list_end + 1;
    }
    views
}

fn inspect_field_capability_owner(source: &str, using_start: usize) -> Option<String> {
    let prefix = &source[..using_start];
    let arg_start = prefix
        .rfind(|ch| ch == '(' || ch == ',')
        .map(|index| index + 1)
        .unwrap_or(0);
    let arg = prefix[arg_start..].trim();
    let name = arg.split_once(':').map(|(name, _)| name).unwrap_or(arg);
    inspect_ident_suffix(name)
}

fn inspect_field_capability_name(part: &str) -> Option<String> {
    let part = if let Some(rest) = part.strip_prefix("mut") {
        if rest.chars().next().is_some_and(char::is_whitespace) {
            rest.trim_start()
        } else {
            part
        }
    } else {
        part
    };
    inspect_ident_prefix(part)
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
    cli_mode: Option<KoboMode>,
    erase_lifetimes: bool,
) -> anyhow::Result<InspectCargoOutput> {
    let absolute_file = absolute_input_path(file)?;
    let Some(project_root) = find_nearest_kobo_project_root(&absolute_file) else {
        return build_single_file_inspect_cargo_output(file, cli_mode, erase_lifetimes);
    };

    let driver_config = load_config(&project_root)
        .with_context(|| format!("failed to load config for {}", project_root.display()))?;
    let project_config = cargo_project_config_from_root(&project_root)?;
    let src_dir = project_root.join(&driver_config.src_dir);
    let mut kobo_files = Vec::new();
    collect_kobo_files(&src_dir, &mut kobo_files);
    if kobo_files.is_empty() {
        return build_single_file_inspect_cargo_output(file, cli_mode, erase_lifetimes);
    }

    let canonical_input = absolute_file
        .canonicalize()
        .unwrap_or(absolute_file.clone());
    let mut source_files = Vec::new();
    let mut main_output = None;

    for kobo_file in kobo_files {
        let mut session = build_session(&kobo_file, cli_mode)?;
        let CodegenArtifacts { rs_source, .. } = run_codegen_pipeline(&mut session, &kobo_file)
            .map_err(|()| {
                render_diagnostics(&session);
                anyhow::anyhow!("compilation failed")
            })?;
        render_diagnostics(&session);

        let rs_source = if erase_lifetimes {
            kobo_driver::apply_lifetime_erasure(&rs_source, session.mode())
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
    cli_mode: Option<KoboMode>,
    erase_lifetimes: bool,
) -> anyhow::Result<InspectCargoOutput> {
    let mut session = build_session(file, cli_mode)?;
    let CodegenArtifacts { rs_source, .. } =
        run_codegen_pipeline(&mut session, file).map_err(|()| {
            render_diagnostics(&session);
            anyhow::anyhow!("compilation failed")
        })?;
    render_diagnostics(&session);

    let rs_source = if erase_lifetimes {
        kobo_driver::apply_lifetime_erasure(&rs_source, session.mode())
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
    let kobo_toml_path = project_root.join("Kobo.toml");
    if !kobo_toml_path.exists() {
        return Ok(kobo_codegen::cargo_gen::KoboProjectConfig::default());
    }

    let toml_str = fs::read_to_string(&kobo_toml_path)
        .with_context(|| format!("failed to read {}", kobo_toml_path.display()))?;
    kobo_codegen::cargo_gen::KoboProjectConfig::from_toml(&toml_str)
        .map_err(|e| anyhow::anyhow!("{e}"))
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

fn find_nearest_kobo_project_root(file: &Path) -> Option<PathBuf> {
    let search_root = file.parent().unwrap_or(file);
    for ancestor in search_root.ancestors() {
        if ancestor.join("Kobo.toml").is_file() {
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
