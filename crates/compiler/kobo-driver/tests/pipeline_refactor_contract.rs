use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use kobo_driver::{run_pipeline, CompileSession, KoboConfig};

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn src_path(relative: &str) -> PathBuf {
    crate_root().join("src").join(relative)
}

fn read_required(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| {
        panic!(
            "failed to read required source file {}: {error}",
            path.display()
        )
    })
}

fn strip_comments_and_strings(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            if i < bytes.len() {
                out.push('\n');
                i += 1;
            }
            continue;
        }

        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                if bytes[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }

        if let Some(end) = raw_string_end(bytes, i) {
            out.push_str("r\"\"");
            for &b in &bytes[i..end] {
                if b == b'\n' {
                    out.push('\n');
                }
            }
            i = end;
            continue;
        }

        if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                if bytes[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            out.push_str("\"\"");
            continue;
        }

        out.push(bytes[i] as char);
        i += 1;
    }

    out
}

fn raw_string_end(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start) != Some(&b'r') {
        return None;
    }

    let mut cursor = start + 1;
    let mut hashes = 0;
    while bytes.get(cursor) == Some(&b'#') {
        hashes += 1;
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'"') {
        return None;
    }
    cursor += 1;

    while cursor < bytes.len() {
        if bytes[cursor] == b'"' {
            let mut matched = true;
            for offset in 0..hashes {
                if bytes.get(cursor + 1 + offset) != Some(&b'#') {
                    matched = false;
                    break;
                }
            }
            if matched {
                return Some(cursor + 1 + hashes);
            }
        }
        cursor += 1;
    }

    Some(bytes.len())
}

fn function_names(source: &str) -> Vec<String> {
    let stripped = strip_comments_and_strings(source);
    let normalized: String = stripped
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                ' '
            }
        })
        .collect();
    let tokens: Vec<&str> = normalized.split_whitespace().collect();
    let mut names = Vec::new();
    for window in tokens.windows(2) {
        if window[0] == "fn" {
            names.push(window[1].to_owned());
        }
    }
    names
}

fn function_line_count(source: &str, function_name: &str) -> Option<usize> {
    let lines = source.lines().collect::<Vec<_>>();
    for (start_index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("fn ")
            && !trimmed.starts_with("pub fn ")
            && !trimmed.starts_with("pub(crate) fn ")
        {
            continue;
        }
        if !trimmed.contains(&format!("fn {function_name}")) {
            continue;
        }

        let mut brace_depth = 0isize;
        let mut has_body = false;
        for (end_index, body_line) in lines.iter().enumerate().skip(start_index) {
            for ch in body_line.chars() {
                match ch {
                    '{' => {
                        brace_depth += 1;
                        has_body = true;
                    }
                    '}' => brace_depth -= 1,
                    _ => {}
                }
            }
            if has_body && brace_depth <= 0 {
                return Some(end_index - start_index + 1);
            }
        }
    }
    None
}

fn count_nonblank_noncomment_lines(source: &str) -> usize {
    strip_comments_and_strings(source)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count()
}

fn collect_pipeline_sources() -> BTreeMap<String, String> {
    let mut sources = BTreeMap::new();
    let legacy = src_path("pipeline.rs");
    if legacy.exists() {
        sources.insert("pipeline.rs".to_owned(), read_required(&legacy));
    }

    let module_dir = src_path("pipeline");
    if module_dir.exists() {
        for entry in fs::read_dir(&module_dir).expect("pipeline directory should be readable") {
            let entry = entry.expect("pipeline directory entry should be readable");
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                let name = format!(
                    "pipeline/{}",
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .expect("pipeline filename should be UTF-8")
                );
                sources.insert(name, read_required(&path));
            }
        }
    }

    sources
}

#[test]
fn analysis_phase_entrypoint_is_not_a_god_function() {
    let source = read_required(&src_path("pipeline/analysis.rs"));
    let line_count = function_line_count(&source, "run_analysis_phase")
        .expect("pipeline/analysis.rs must define run_analysis_phase");

    assert!(
        line_count <= 120,
        "run_analysis_phase should orchestrate named projection steps, not own every diagnostic block; found {line_count} lines"
    );
}

#[test]
fn pipeline_entrypoint_is_facade_not_phase_dump() {
    let legacy = src_path("pipeline.rs");
    if legacy.exists() {
        let source = read_required(&legacy);
        let line_count = count_nonblank_noncomment_lines(&source);
        assert!(
            line_count <= 80,
            "src/pipeline.rs must be removed or reduced to a tiny facade; found {line_count} nonblank code lines"
        );

        let names = function_names(&source);
        let forbidden = [
            "run_kir_phase",
            "run_analysis_phase",
            "run_codegen_pipeline",
            "compile_codegen_artifacts",
            "project_solver_diagnostics",
            "inject_solver_evidence",
            "apply_engine_ceiling",
        ];
        let present: Vec<&str> = forbidden
            .into_iter()
            .filter(|name| names.iter().any(|found| found == name))
            .collect();
        assert!(
            present.is_empty(),
            "src/pipeline.rs facade must not define phase implementation functions: {present:?}"
        );
    }

    assert!(
        src_path("pipeline/mod.rs").exists(),
        "pipeline facade must live at src/pipeline/mod.rs after the split"
    );
}

#[test]
fn pipeline_phase_modules_own_expected_functions_once() {
    let expected: [(&str, &[&str]); 6] = [
        ("pipeline/parse.rs", &["run_kir_phase"]),
        (
            "pipeline/analysis.rs",
            &[
                "run_check_pipeline",
                "run_pipeline_ordering_check",
                "run_analysis_phase",
                "project_live_borrow_liveness_diagnostics",
            ],
        ),
        (
            "pipeline/solver.rs",
            &[
                "resolve_solution",
                "project_solver_diagnostics",
                "apply_engine_ceiling",
                "project_engine_ceiling_diagnostics",
                "inject_solver_evidence",
            ],
        ),
        ("pipeline/codegen.rs", &["run_codegen_pipeline"]),
        (
            "pipeline/compile.rs",
            &[
                "run_and_compile",
                "run_and_compile_with_lifetime_erasure",
                "compile_codegen_artifacts",
            ],
        ),
        (
            "pipeline/util.rs",
            &[
                "effective_mode",
                "apply_lifetime_erasure",
                "lifetime_erasure_debt_report",
                "extract_before_borrow_rewrite",
            ],
        ),
    ];

    let sources = collect_pipeline_sources();

    for (file, functions) in expected {
        let source = sources
            .get(file)
            .unwrap_or_else(|| panic!("expected pipeline phase file {file} to exist"));
        let names = function_names(source);
        for function in functions {
            assert!(
                names.iter().any(|name| name == function),
                "{file} must define function `{function}` as real Rust code, not only in a comment or string"
            );
        }
    }

    for (expected_file, functions) in expected {
        for function in functions {
            let owners: Vec<&str> = sources
                .iter()
                .filter_map(|(file, source)| {
                    if function_names(source).iter().any(|name| name == function) {
                        Some(file.as_str())
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(
                owners,
                vec![expected_file],
                "function `{function}` must be defined exactly once, in {expected_file}"
            );
        }
    }
}

#[test]
fn public_run_pipeline_contract_survives_module_split() {
    let root = unique_temp_dir("kobo-driver-pipeline-contract");
    let source_path = root.join("contract.kobo");
    let output_dir = root.join("generated");
    fs::create_dir_all(&root).expect("test temp root should be creatable");
    fs::write(
        &source_path,
        r#"
fn main() {
    let name = String::from("Ada");
    println!("{}", name);
}
"#,
    )
    .expect("test source should be writable");

    let config = KoboConfig {
        output_dir: Some(output_dir.clone()),
        ..KoboConfig::default()
    };
    let mut session = CompileSession::new(config);

    let rs_source = run_pipeline(&mut session, &source_path)
        .expect("public run_pipeline should generate Rust for a simple Kobo source");

    assert!(
        rs_source.contains("fn main"),
        "generated Rust should still contain the source main function:\n{rs_source}"
    );
    assert!(
        output_dir.join("contract.rs").exists(),
        "run_pipeline should still write the generated Rust file"
    );
    assert!(
        output_dir.join("contract.kobo.map").exists(),
        "run_pipeline should still write the source map file"
    );
    assert!(
        !session.has_errors(),
        "simple pipeline contract source should not accumulate error diagnostics: {:?}",
        session.diagnostics
    );

    let _ = fs::remove_dir_all(root);
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after UNIX_EPOCH")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()))
}
