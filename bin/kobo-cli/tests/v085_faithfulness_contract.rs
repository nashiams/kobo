use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

static CASE_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    root: PathBuf,
    file: PathBuf,
}

impl Fixture {
    fn source(label: &str, file_name: &str, source: &str) -> Self {
        let root = unique_temp_root(label);
        fs::create_dir_all(&root).expect("fixture root should be creatable");
        let file = root.join(file_name);
        fs::write(&file, source).expect("fixture source should be writable");
        Self { root, file }
    }

    fn project(label: &str, files: &[(&str, &str)]) -> Self {
        let root = unique_temp_root(label);
        for (relative, contents) in files {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("fixture parent should be creatable");
            }
            fs::write(path, contents).expect("fixture project file should be writable");
        }
        let file = root.join("src/main.kobo");
        Self { root, file }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct CliOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

fn unique_temp_root(label: &str) -> PathBuf {
    let counter = CASE_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{label}-{}-{counter}", std::process::id()))
}

fn run_kobo(args: &[String]) -> CliOutput {
    run_command(env!("CARGO_BIN_EXE_kobo"), args, None)
}

fn run_kobo_in(args: &[String], cwd: &Path) -> CliOutput {
    run_command(env!("CARGO_BIN_EXE_kobo"), args, Some(cwd))
}

fn run_command(binary: &str, args: &[String], cwd: Option<&Path>) -> CliOutput {
    let mut command = Command::new(binary);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command.output().expect("command should launch");
    CliOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn arg(path: impl AsRef<Path>) -> String {
    path.as_ref().to_string_lossy().into_owned()
}

fn assert_success(output: &CliOutput, context: &str) {
    assert!(
        output.status.success(),
        "{context} must succeed\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
}

#[test]
fn field_capability_views_lower_to_real_field_borrow_statements() {
    let fixture = Fixture::source(
        "v085-field-cap-lowered",
        "route.kobo",
        r#"
struct Request {
    header: String,
    body: String,
}

fn route(req: Request) using { req.header, req.body } {
    println!("{}", req.header);
}
"#,
    );

    let output = run_kobo(&[
        "inspect".to_owned(),
        "--clean".to_owned(),
        arg(&fixture.file),
    ]);
    assert_success(&output, "field capability inspect");

    let real_borrows: Vec<_> = output
        .stdout
        .lines()
        .map(str::trim_start)
        .filter(|line| !line.starts_with("//"))
        .filter(|line| line.contains("&req.header") || line.contains("&req.body"))
        .collect();

    assert!(
        real_borrows
            .iter()
            .any(|line| line.contains("let _kobo_using_req_header = &req.header;")),
        "using req.header must lower to a real Rust borrow statement, not only metadata comments\n{}",
        output.stdout
    );
    assert!(
        real_borrows
            .iter()
            .any(|line| line.contains("let _kobo_using_req_body = &req.body;")),
        "using req.body must lower to a real Rust borrow statement, not only metadata comments\n{}",
        output.stdout
    );
}

#[test]
fn owner_qualified_invalid_field_capability_reports_field_name() {
    let fixture = Fixture::source(
        "v085-field-cap-qualified-invalid",
        "route.kobo",
        r#"
struct Request {
    header: String,
}

fn route(req: Request) using { req.missing } {
    println!("{}", req.header);
}
"#,
    );

    let output = run_kobo(&[
        "check".to_owned(),
        "--error-format=json".to_owned(),
        arg(&fixture.file),
    ]);
    assert!(
        !output.status.success(),
        "invalid owner-qualified field capability must fail\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );

    let value: Value = serde_json::from_str(output.stdout.trim()).unwrap_or_else(|error| {
        panic!(
            "field capability diagnostic must be JSON: {error}\n{}",
            output.stdout
        )
    });
    assert_eq!(value["code"], "K0109");
    assert!(
        value["message"]
            .as_str()
            .is_some_and(|message| message.contains("missing")),
        "diagnostic must report the missing field name, not the owner name\n{}",
        output.stdout
    );
}

#[test]
fn trait_object_default_emits_actual_facade_code() {
    let fixture = Fixture::source(
        "v085-trait-facade",
        "store.kobo",
        r#"
trait Store {
    fn get(&self) -> i32;
}

fn handler(store: dyn Store) {
    println!("{}", store.get());
}
"#,
    );

    let output = run_kobo(&[
        "inspect".to_owned(),
        "--clean".to_owned(),
        "--profile".to_owned(),
        "checked".to_owned(),
        "--trait-default".to_owned(),
        "trait-object-first".to_owned(),
        arg(&fixture.file),
    ]);
    assert_success(&output, "trait facade inspect");

    assert!(
        output
            .stdout
            .lines()
            .map(str::trim_start)
            .any(|line| line.starts_with("pub fn handler_thin_facade(store: Box<dyn Store>)")),
        "trait-object-first default must emit actual facade code, not only comments\n{}",
        output.stdout
    );
    assert!(
        output
            .stdout
            .lines()
            .map(str::trim_start)
            .any(|line| line.starts_with("fn handler(store: Box<dyn Store>)")),
        "the lowered callable must use the trait-object surface promised by the profile\n{}",
        output.stdout
    );
}

#[test]
fn doctor_deps_json_reports_dependency_evidence() {
    let fixture = Fixture::project(
        "v085-doctor-evidence",
        &[
            (
                "Cargo.toml",
                r#"
[package]
name = "doctor-evidence"
version = "0.1.0"
edition = "2021"
rust-version = "1.74"
build = "build.rs"

[dependencies]
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
serde_derive = "1"

[build-dependencies]
prost-build = "0.12"
"#,
            ),
            ("build.rs", "fn main() {}\n"),
            ("src/main.kobo", "fn main() {}\n"),
        ],
    );

    let output = run_kobo_in(
        &[
            "doctor".to_owned(),
            "--deps".to_owned(),
            "--json".to_owned(),
        ],
        &fixture.root,
    );
    assert_success(&output, "doctor --deps --json");

    let value: Value = serde_json::from_str(&output.stdout)
        .unwrap_or_else(|error| panic!("doctor JSON must parse: {error}\n{}", output.stdout));
    assert_eq!(value["rust_version"], "1.74");
    assert_eq!(value["build_rs"], true);

    let dependencies = value["dependencies"]
        .as_array()
        .expect("doctor JSON must include dependency evidence array");
    assert!(
        dependencies.iter().any(|dep| {
            dep["name"] == "reqwest"
                && dep["section"] == "dependencies"
                && dep["features"]
                    .as_array()
                    .is_some_and(|features| features.iter().any(|feature| feature == "json"))
                && dep["default_features"] == false
        }),
        "doctor must report direct dependency shape from Cargo.toml\n{}",
        output.stdout
    );
    assert!(
        dependencies
            .iter()
            .any(|dep| dep["name"] == "prost-build" && dep["section"] == "build-dependencies"),
        "doctor must report build dependency evidence\n{}",
        output.stdout
    );
    assert!(
        value["proc_macro_candidates"]
            .as_array()
            .is_some_and(|deps| deps.iter().any(|dep| dep == "serde_derive")),
        "doctor must surface proc-macro-like dependency candidates\n{}",
        output.stdout
    );
}

#[test]
fn kobo_lsp_binary_exposes_entrypoint() {
    let Some(binary) = option_env!("CARGO_BIN_EXE_kobo-lsp") else {
        panic!("workspace must expose a kobo-lsp binary for v0.8.5 LSP MVP");
    };

    let output = run_command(binary, &["--version".to_owned()], None);
    assert_success(&output, "kobo-lsp --version");
    assert!(
        output.stdout.contains("kobo-lsp"),
        "kobo-lsp --version must identify the binary\n{}",
        output.stdout
    );
}
