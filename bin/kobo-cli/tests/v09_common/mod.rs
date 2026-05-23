#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

static CASE_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub struct TestProject {
    pub root: PathBuf,
}

impl TestProject {
    pub fn new(label: &str) -> Self {
        let counter = CASE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("kobo-v09-{label}-{}-{counter}", std::process::id()));
        if root.exists() {
            let _ = fs::remove_dir_all(&root);
        }
        fs::create_dir_all(root.join("src")).expect("test project should be creatable");
        Self { root }
    }

    pub fn write(&self, relative: &str, contents: &str) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("test project parent should be creatable");
        }
        fs::write(&path, contents).expect("test project file should be writable");
        path
    }

    pub fn main_file(&self, contents: &str) -> PathBuf {
        self.write("src/main.kobo", contents)
    }

    pub fn copy_fixture(&self, fixture_relative: &str, project_relative: &str) -> PathBuf {
        let contents = fixture_text(fixture_relative);
        self.write(project_relative, &contents)
    }

    pub fn copy_fixture_template(
        &self,
        fixture_relative: &str,
        project_relative: &str,
        replacements: &[(&str, &str)],
    ) -> PathBuf {
        let contents = fixture_template(fixture_relative, replacements);
        self.write(project_relative, &contents)
    }

    pub fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.root.join(relative)).expect("test project file should be readable")
    }

    pub fn find_files_with_ext(&self, extension: &str) -> Vec<PathBuf> {
        let mut out = Vec::new();
        collect_ext(&self.root, extension, &mut out);
        out.sort();
        out
    }
}

impl Drop for TestProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn collect_ext(root: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_ext(&path, extension, out);
        } else if path.extension().and_then(|value| value.to_str()) == Some(extension) {
            out.push(path);
        }
    }
}

pub struct CliOutput {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl CliOutput {
    pub fn combined(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

pub fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, Duration::from_secs(60))
}

pub fn run_kobo_with_timeout(args: &[String], cwd: &Path, timeout: Duration) -> CliOutput {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kobo"));
    command.args(args).current_dir(cwd);
    run_command_with_timeout(command, timeout)
}

pub fn run_kobo_with_env(args: &[String], cwd: &Path, envs: &[(&str, &str)]) -> CliOutput {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kobo"));
    command.args(args).current_dir(cwd);
    for (key, value) in envs {
        command.env(key, value);
    }
    run_command_with_timeout(command, Duration::from_secs(60))
}

pub fn run_kobo_lsp_stdio(input: &str, cwd: &Path) -> CliOutput {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kobo-lsp"));
    command
        .arg("--stdio")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("kobo-lsp command should launch");
    {
        let mut stdin = child
            .stdin
            .take()
            .expect("kobo-lsp stdin should be available");
        stdin
            .write_all(input.as_bytes())
            .expect("kobo-lsp stdin should be writable");
    }
    let output = child
        .wait_with_output()
        .expect("kobo-lsp command output should be readable");
    output_to_cli(output)
}

fn run_command_with_timeout(mut command: Command, timeout: Duration) -> CliOutput {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().expect("kobo command should launch");
    let started_at = Instant::now();
    loop {
        if child
            .try_wait()
            .expect("kobo command status should be observable")
            .is_some()
        {
            let output = child
                .wait_with_output()
                .expect("kobo command output should be readable");
            return output_to_cli(output);
        }
        if started_at.elapsed() >= timeout {
            return kill_timed_out_child(child, timeout);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn kill_timed_out_child(mut child: Child, timeout: Duration) -> CliOutput {
    let process_id = child.id();
    let _ = child.kill();
    let output = child
        .wait_with_output()
        .expect("timed out kobo command output should be readable");
    let mut cli_output = output_to_cli(output);
    if !cli_output.stderr.is_empty() && !cli_output.stderr.ends_with('\n') {
        cli_output.stderr.push('\n');
    }
    cli_output.stderr.push_str(&format!(
        "kobo command timed out after {}s; killed process {process_id}\n",
        timeout.as_secs()
    ));
    cli_output
}

fn output_to_cli(output: std::process::Output) -> CliOutput {
    CliOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

pub fn s(value: impl Into<String>) -> String {
    value.into()
}

pub fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub fn fixture_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("v0.9")
        .join(relative)
}

pub fn fixture_text(relative: &str) -> String {
    let path = fixture_path(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("fixture `{}` should be readable: {error}", path.display()))
}

pub fn fixture_template(relative: &str, replacements: &[(&str, &str)]) -> String {
    let mut contents = fixture_text(relative);
    for (placeholder, replacement) in replacements {
        contents = contents.replace(placeholder, replacement);
    }
    contents
}

pub fn assert_success(output: &CliOutput, context: &str) {
    assert!(
        output.status.success(),
        "{context} must succeed\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
}

pub fn assert_failure(output: &CliOutput, context: &str) {
    assert!(
        !output.status.success(),
        "{context} must fail\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
}

pub fn assert_contains(text: &str, needle: &str, context: &str) {
    assert!(
        text.contains(needle),
        "{context}\nmissing `{needle}` in:\n{text}"
    );
}

pub fn assert_not_contains(text: &str, needle: &str, context: &str) {
    assert!(
        !text.contains(needle),
        "{context}\nunexpected `{needle}` in:\n{text}"
    );
}

pub fn unique_symbol(prefix: &str) -> String {
    let counter = CASE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let normalized: String = prefix
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    format!("{normalized}_{}_{}", std::process::id(), counter)
}

pub fn one_based_line_of(source: &str, needle: &str) -> usize {
    source
        .lines()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("source must contain `{needle}`:\n{source}"))
        + 1
}

pub fn assert_mentions_line(output: &CliOutput, line: usize, context: &str) {
    let text = output.combined();
    let patterns = [
        format!(":{line}:"),
        format!("\"line\":{line}"),
        format!("\"line\": {line}"),
        format!("line {line}"),
    ];
    assert!(
        patterns.iter().any(|pattern| text.contains(pattern)),
        "{context}\nexpected output to mention source line {line}\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
}

pub fn json_lines(output: &CliOutput) -> Vec<Value> {
    output
        .combined()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .collect()
}

pub fn first_json(output: &CliOutput, context: &str) -> Value {
    json_lines(output).into_iter().next().unwrap_or_else(|| {
        panic!(
            "{context}: expected at least one JSON line\n{}",
            output.combined()
        )
    })
}

pub fn assert_json_has_path(value: &Value, path: &[&str], context: &str) {
    let mut cursor = value;
    for segment in path {
        cursor = cursor.get(*segment).unwrap_or_else(|| {
            panic!(
                "{context}: missing JSON path `{}` in {value}",
                path.join(".")
            )
        });
    }
}
