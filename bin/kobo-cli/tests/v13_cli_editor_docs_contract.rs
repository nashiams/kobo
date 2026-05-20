mod v09_common;

use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
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
fn lsp_diagnostics_use_compiler_scenario_facts_for_attribute_sources() {
    let snapshot = kobo_lsp::protocol_document_snapshot(
        "file:///workspace/src/main.kobo",
        r#"
#[kobo::must_call(close)]
struct Token {}

impl Token {
    fn close(self) {}
}

#[kobo::scenario(profile = "sync")]
fn run() {
    let token = Token {};
}
"#,
        None,
    );
    assert_contains(
        &snapshot["diagnostics"].to_string(),
        "compiler-scenario-program",
        "LSP diagnostics should be fed by compiler scenario facts",
    );
    assert_eq!(
        snapshot["diagnostics"][0]["source"], "kobo-compiler",
        "attribute-form diagnostics should not use the source-heuristic fallback"
    );
    assert_contains(
        &snapshot["hover"].to_string(),
        "must_call obligation",
        "hover should be derived from compiler facts for attribute sources",
    );
    assert_contains(
        &snapshot["documentLinks"].to_string(),
        "generated-rust",
        "Rust navigation should be present for compiler-parsed documents",
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
fn lsp_stdio_handles_framed_hover_actions_links_and_navigation() {
    let lsp = env!("CARGO_BIN_EXE_kobo-lsp");
    let source = r#"
#[kobo::must_call(close)]
struct Token {}

impl Token {
    fn close(self) {}
}

#[kobo::scenario(profile = "sync")]
fn run() {
    let token = Token {};
}
"#;
    let uri = "file:///workspace/src/main.kobo";
    let requests = [
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {"textDocument": {"uri": uri, "text": source}}
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/hover",
            "params": {"textDocument": {"uri": uri}, "position": {"line": 10, "character": 12}}
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "textDocument/codeAction",
            "params": {"textDocument": {"uri": uri}, "range": {"start": {"line": 10, "character": 8}, "end": {"line": 10, "character": 13}}}
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "textDocument/documentLink",
            "params": {"textDocument": {"uri": uri}}
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "textDocument/definition",
            "params": {"textDocument": {"uri": uri}, "position": {"line": 10, "character": 12}}
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "kobo/runnables",
            "params": {"textDocument": {"uri": uri}}
        }),
    ];
    let request = requests
        .into_iter()
        .map(|value| {
            let body = value.to_string();
            format!("Content-Length: {}\r\n\r\n{}", body.len(), body)
        })
        .collect::<String>();

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
        .expect("framed requests should write");
    let output = child
        .wait_with_output()
        .expect("kobo-lsp should finish bounded framed stdio request");
    let lsp_output = command_output(output);
    assert_success(&lsp_output, "kobo-lsp framed stdio session");
    assert_contains(
        &lsp_output.stdout,
        "Content-Length:",
        "stdio server should use standard LSP message framing",
    );
    for expected in [
        "textDocument/publishDiagnostics",
        "must_call obligation",
        "kobo explain K0100",
        "generated-rust",
        "rust-analyzer",
        "kobo.testScenario",
    ] {
        assert_contains(
            &lsp_output.stdout,
            expected,
            "framed LSP response should include protocol feature",
        );
    }
}

#[test]
fn lsp_stdio_responds_before_stdin_eof_in_live_session() {
    let lsp = env!("CARGO_BIN_EXE_kobo-lsp");
    let uri = "file:///workspace/src/main.kobo";
    let source = r#"
#[kobo::must_call(close)]
struct Token {}

#[kobo::scenario(profile = "sync")]
fn run() {
    let token = Token {};
}
"#;
    let mut child = Command::new(lsp)
        .arg("--stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("kobo-lsp --stdio should launch");
    let mut stdin = child.stdin.take().expect("stdin should be piped");
    let stdout = child.stdout.take().expect("stdout should be piped");
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut stdout = stdout;
        while let Ok(message) = read_lsp_frame(&mut stdout) {
            if sender.send(message).is_err() {
                break;
            }
        }
    });

    write_lsp_frame(
        &mut stdin,
        &serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
    );
    let initialize = receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("live LSP should answer initialize before stdin EOF");
    assert_contains(
        &initialize,
        "kobo-lsp",
        "initialize response should be emitted during live session",
    );
    assert!(
        child
            .try_wait()
            .expect("child process status should be available")
            .is_none(),
        "stdio server should keep running after initialize in a live session",
    );

    write_lsp_frame(
        &mut stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {"textDocument": {"uri": uri, "text": source}}
        }),
    );
    let diagnostics = receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("live LSP should publish diagnostics before stdin EOF");
    assert_contains(
        &diagnostics,
        "textDocument/publishDiagnostics",
        "live session should publish diagnostics for opened documents",
    );
    assert_contains(
        &diagnostics,
        "compiler-scenario-program",
        "live session diagnostics should come from compiler scenario facts",
    );

    write_lsp_frame(
        &mut stdin,
        &serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": null}),
    );
    let shutdown = receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("live LSP should respond to shutdown");
    assert_contains(&shutdown, r#""id":2"#, "shutdown response should carry id");
    write_lsp_frame(
        &mut stdin,
        &serde_json::json!({"jsonrpc": "2.0", "method": "exit"}),
    );
    drop(stdin);
    let output = child
        .wait_with_output()
        .expect("kobo-lsp should finish after exit");
    assert_success(&command_output(output), "kobo-lsp live stdio exit");
    let _ = reader.join();
}

fn write_lsp_frame(stdin: &mut impl Write, value: &Value) {
    let body = value.to_string();
    write!(stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body)
        .expect("LSP frame should write");
    stdin.flush().expect("LSP frame should flush");
}

fn read_lsp_frame(stdout: &mut impl Read) -> std::io::Result<String> {
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stdout.read_exact(&mut byte)?;
        header.push(byte[0]);
        if header.ends_with(b"\r\n\r\n") || header.ends_with(b"\n\n") {
            break;
        }
    }
    let header = String::from_utf8_lossy(&header);
    let length = header
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .expect("LSP response should include Content-Length");
    let mut body = vec![0u8; length];
    stdout.read_exact(&mut body)?;
    Ok(String::from_utf8_lossy(&body).to_string())
}

#[test]
fn vscode_extension_contract_exposes_syntax_and_actions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let package = root.join("editors/vscode/package.json");
    let syntax = root.join("editors/vscode/syntaxes/kobo.tmLanguage.json");
    let language = root.join("editors/vscode/language-configuration.json");
    let extension = root.join("editors/vscode/extension.js");

    let package_json: Value = serde_json::from_str(
        &std::fs::read_to_string(&package).expect("VS Code package should exist"),
    )
    .expect("VS Code package should parse");
    assert_eq!(
        package_json["main"], "./extension.js",
        "VS Code extension should have an activation entrypoint",
    );
    assert_eq!(
        package_json["dependencies"]["vscode-languageclient"],
        Value::String("^9.0.1".to_owned()),
        "VS Code extension should declare the language client it imports",
    );
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
    let extension_js =
        std::fs::read_to_string(&extension).expect("VS Code extension entrypoint should exist");
    for command in [
        "kobo.runScenario",
        "kobo.replayWitness",
        "kobo.explainDiagnostic",
    ] {
        assert_contains(
            &extension_js,
            &format!("registerCommand('{command}'"),
            "VS Code extension should register contributed command",
        );
    }
    assert_contains(
        &extension_js,
        "LanguageClient",
        "VS Code extension should start kobo-lsp through a language client",
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
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let readme = std::fs::read_to_string(repo_root.join("README.md")).expect("README should read");
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

    let docs = [
        (
            "docs/modeled-wards.md",
            [
                "Modeled Ward Tutorial",
                "attribute form",
                "first-class ward syntax",
                "ports",
                "recordings",
                "opaque boundaries",
                "source spans",
            ],
        ),
        (
            "docs/replay-and-strict-liveness.md",
            [
                "Strict Liveness Reference",
                "modeled scenario Core CFG evidence",
                "join",
                "transfer",
                "proof failure",
                "replay validation",
                "mode invariant",
            ],
        ),
        (
            "docs/failure-lab.md",
            [
                "Failure Lab",
                "durable queue",
                "async gateway",
                "crash-after-ack",
                "cancellation",
                "orphan task",
                "request-token",
            ],
        ),
        (
            "docs/editor-clean-rust-workflow.md",
            [
                "Editor Workflow",
                "kobo-lsp",
                "witness links",
                "generated-Rust navigation hints",
                "inspect --clean --cargo",
                "zero Kobo dependency",
                "rustfmt",
            ],
        ),
        (
            "docs/migration-guide.md",
            [
                "Migration Guide",
                "Script",
                "Checked",
                "Strict",
                "scoped modes",
                "debt",
                "mode invariant",
            ],
        ),
    ];

    let mut combined = readme;
    for (relative, required_terms) in docs {
        let path = repo_root.join(relative);
        assert!(path.is_file(), "{relative} should be shipped documentation");
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{relative} should read: {error}"));
        for expected in required_terms {
            assert_contains(
                &contents,
                expected,
                "documentation page should cover required workflow detail",
            );
        }
        combined.push_str(&contents);
    }
    for forbidden in [
        "gradual typing",
        "formally proves arbitrary",
        "arbitrary crate internals are deterministic",
        "first-class ward syntax is required",
        "It is built around the Core CFG",
        "Rust navigation delegation from source-mapped compiler facts",
        "use rust-analyzer delegation and source maps",
    ] {
        assert_not_contains(
            &combined,
            forbidden,
            "v0.13 docs should avoid overclaim language",
        );
    }
}
