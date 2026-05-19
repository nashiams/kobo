mod v09_common;

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::Value;
use v09_common::{
    assert_contains, assert_not_contains, assert_success, path_arg, run_kobo_with_timeout, s,
    CliOutput, TestProject,
};

const V13_TIMEOUT: Duration = Duration::from_secs(90);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V13_TIMEOUT)
}

fn clean_exit_source() -> &'static str {
    r#"
fn double(value: i32) -> i32 {
    value * 2
}

fn main() {
    println!("{}", double(21));
}
"#
}

#[test]
fn complete_cli_surface_documents_required_commands() {
    let project = TestProject::new("v13-cli-help");
    let help = run_kobo(&[s("--help")], &project.root);
    assert_success(&help, "top-level help should render");
    for command in [
        "run", "build", "check", "inspect", "migrate", "debt", "perf", "sim", "replay",
    ] {
        assert_contains(
            &help.stdout,
            command,
            "top-level help should document required command",
        );
    }

    let inspect = run_kobo(&[s("inspect"), s("--help")], &project.root);
    assert_success(&inspect, "inspect help should render");
    for flag in ["--clean", "--cargo", "--scenario-metadata"] {
        assert_contains(
            &inspect.stdout,
            flag,
            "inspect help should document subflag",
        );
    }

    let debt = run_kobo(&[s("debt"), s("--help")], &project.root);
    assert_success(&debt, "debt help should render");
    for flag in ["--json", "--summary", "--watch"] {
        assert_contains(&debt.stdout, flag, "debt help should document subflag");
    }

    let sim = run_kobo(&[s("sim"), s("--help")], &project.root);
    assert_success(&sim, "sim help should render");
    for subcommand in ["init", "backends", "scout"] {
        assert_contains(
            &sim.stdout,
            subcommand,
            "sim help should document subcommand",
        );
    }
}

#[test]
fn inspect_clean_and_inspect_cargo_preserve_clean_rust_exit_ramp() {
    let project = TestProject::new("v13-clean-exit-ramp");
    let file = project.main_file(clean_exit_source());

    let clean = run_kobo(
        &[s("inspect"), s("--clean"), path_arg(&file)],
        &project.root,
    );
    assert_success(&clean, "inspect --clean should render clean Rust");
    assert_contains(
        &clean.stdout,
        "fn main()",
        "clean output should retain main",
    );
    assert_not_contains(
        &clean.stdout,
        "kobo::",
        "clean output should not depend on Kobo attributes",
    );

    let out_dir = project.root.join("target/clean-rust-cargo");
    let cargo_export = run_kobo(
        &[
            s("inspect"),
            s("--clean"),
            s("--cargo"),
            path_arg(&out_dir),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(
        &cargo_export,
        "inspect --clean --cargo should export Cargo project",
    );

    let check = Command::new("cargo")
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(out_dir.join("Cargo.toml"))
        .output()
        .expect("cargo check should launch");
    assert_success(
        &command_output(check),
        "clean Rust Cargo output should build",
    );

    let clippy = Command::new("cargo")
        .arg("clippy")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(out_dir.join("Cargo.toml"))
        .arg("--")
        .arg("-D")
        .arg("warnings")
        .output()
        .expect("cargo clippy should launch");
    assert_success(
        &command_output(clippy),
        "clean Rust Cargo output should satisfy clippy warning baseline",
    );
}

fn command_output(output: std::process::Output) -> CliOutput {
    CliOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

#[test]
fn lsp_reports_diagnostics_hovers_runnables_and_witness_links() {
    let capabilities = kobo_lsp::editor_capabilities();
    assert_eq!(capabilities["diagnostics"]["provider"], "kobo-lsp");
    assert_eq!(capabilities["hoverProvider"], Value::Bool(true));
    assert_eq!(capabilities["codeActionProvider"], Value::Bool(true));
    assert_eq!(capabilities["documentLinkProvider"], Value::Bool(true));
    assert_eq!(
        capabilities["definitionProvider"]["delegate"],
        "rust-analyzer"
    );
    for command in [
        "kobo.testScenario",
        "kobo.replayWitness",
        "kobo.explainDiagnostic",
    ] {
        assert_contains(
            &capabilities["runnables"].to_string(),
            command,
            "LSP capabilities should expose runnable workflow commands",
        );
    }

    let snapshot = kobo_lsp::protocol_document_snapshot(
        "file:///workspace/src/main.kobo",
        r#"
ward Demo {
    obligation Token must close
    scenario run {
        let token = Token {};
    }
}
"#,
        Some(".kobo/witnesses/run-1.kwit"),
    );
    assert_contains(
        &snapshot["diagnostics"].to_string(),
        "K0100",
        "protocol snapshot should publish diagnostics for a real document",
    );
    assert_eq!(
        snapshot["diagnostics"][0]["range"]["start"]["line"],
        Value::from(4),
        "protocol diagnostics should point at the unresolved token site"
    );
    assert_contains(
        &snapshot["hover"].to_string(),
        "ward model",
        "protocol snapshot should expose hover text",
    );
    assert_contains(
        &snapshot["documentLinks"].to_string(),
        ".kobo/witnesses/run-1.kwit",
        "protocol snapshot should expose witness links",
    );
    assert_contains(
        &snapshot["codeActions"].to_string(),
        "kobo replay .kobo/witnesses/run-1.kwit",
        "protocol snapshot should expose replay actions",
    );
}

#[test]
fn lsp_stdio_publishes_document_diagnostics_for_opened_document() {
    let lsp = env!("CARGO_BIN_EXE_kobo-lsp");
    let source = r#"
ward Demo {
    obligation Token must close
    scenario run {
        let token = Token {};
    }
}
"#;
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": "file:///workspace/src/main.kobo",
                "text": source
            }
        }
    })
    .to_string();
    let mut child = Command::new(lsp)
        .arg("--stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("kobo-lsp --stdio should launch");
    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(request.as_bytes())
        .expect("didOpen request should write");
    let output = child
        .wait_with_output()
        .expect("kobo-lsp should finish bounded stdio request");
    let lsp_output = command_output(output);
    assert_success(&lsp_output, "kobo-lsp --stdio didOpen request");
    assert_contains(
        &lsp_output.stdout,
        "textDocument/publishDiagnostics",
        "stdio server should publish diagnostics for opened documents",
    );
    assert_contains(
        &lsp_output.stdout,
        "K0100",
        "stdio diagnostics should carry compiler diagnostic code",
    );
}

#[test]
fn vscode_extension_contract_exposes_syntax_and_actions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let package = root.join("editors/vscode/package.json");
    let syntax = root.join("editors/vscode/syntaxes/kobo.tmLanguage.json");
    let language = root.join("editors/vscode/language-configuration.json");

    let package_json: Value = serde_json::from_str(
        &std::fs::read_to_string(&package).expect("VS Code package should exist"),
    )
    .expect("VS Code package should parse");
    assert_contains(
        &package_json["contributes"]["languages"].to_string(),
        "kobo",
        "VS Code extension should register Kobo syntax",
    );
    for command in [
        "kobo.runScenario",
        "kobo.replayWitness",
        "kobo.explainDiagnostic",
    ] {
        assert_contains(
            &package_json["contributes"]["commands"].to_string(),
            command,
            "VS Code extension should expose workflow action",
        );
    }
    assert!(
        syntax.is_file(),
        "VS Code extension should ship a TextMate grammar"
    );
    assert!(
        language.is_file(),
        "VS Code extension should ship language configuration"
    );
}

#[test]
fn formatting_delegates_rust_shaped_code_to_rustfmt() {
    let rust_project = TestProject::new("v13-fmt-rust-shaped");
    let rust_file = rust_project.main_file("fn main(){println!(\"hi\");}\n");
    let rust_fmt = run_kobo(&[s("fmt"), path_arg(&rust_file)], &rust_project.root);
    assert_success(&rust_fmt, "rust-shaped formatting should succeed");
    let formatted_rust =
        std::fs::read_to_string(&rust_file).expect("formatted Rust-shaped source should read");
    assert_contains(
        &formatted_rust,
        "fn main() {",
        "rust-shaped Kobo source should be delegated to rustfmt",
    );

    let ward_project = TestProject::new("v13-fmt-ward-syntax");
    let ward_file =
        ward_project.main_file("ward Demo{state log: Vec<String>\nscenario run{let text = \"brace { stays }\"; ward.task();}}\n");
    let ward_fmt = run_kobo(&[s("fmt"), path_arg(&ward_file)], &ward_project.root);
    assert_success(&ward_fmt, "Kobo-only ward formatting should succeed");
    let formatted_ward =
        std::fs::read_to_string(&ward_file).expect("formatted ward source should read");
    assert_contains(
        &formatted_ward,
        "ward Demo {",
        "Kobo-only formatter should handle ward syntax without rustfmt",
    );
    assert_contains(
        &formatted_ward,
        "scenario run {",
        "Kobo-only formatter should preserve scenario syntax",
    );
    assert_contains(
        &formatted_ward,
        "\"brace { stays }\"",
        "Kobo-only formatter should not split braces inside strings",
    );
}

#[test]
fn docs_explain_gradual_guarantees_without_gradual_typing_claim() {
    let readme = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("README.md"),
    )
    .expect("README should read");
    for expected in [
        "modeled wards",
        "ports",
        "recordings",
        "opaque boundaries",
        "scoped modes",
        "gradual guarantees",
        "mode invariant",
        "Script, Checked, and Strict preserve the same ordinary runtime behavior",
    ] {
        assert_contains(&readme, expected, "README should document workflow claim");
    }
    for forbidden in ["gradual typing", "formally proves arbitrary"] {
        assert_not_contains(&readme, forbidden, "README should avoid overclaim");
    }
}
