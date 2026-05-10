#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

static CASE_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub struct TestProject {
    pub root: PathBuf,
}

impl TestProject {
    pub fn new(label: &str) -> Self {
        let counter = CASE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "kobo-v09-{label}-{}-{counter}",
            std::process::id()
        ));
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
    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("kobo command should launch");
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

pub fn json_lines(output: &CliOutput) -> Vec<Value> {
    output
        .combined()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .collect()
}

pub fn first_json(output: &CliOutput, context: &str) -> Value {
    json_lines(output)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("{context}: expected at least one JSON line\n{}", output.combined()))
}

pub fn assert_json_has_path(value: &Value, path: &[&str], context: &str) {
    let mut cursor = value;
    for segment in path {
        cursor = cursor
            .get(*segment)
            .unwrap_or_else(|| panic!("{context}: missing JSON path `{}` in {value}", path.join(".")));
    }
}
