use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static CASE_COUNTER: AtomicUsize = AtomicUsize::new(0);

const UC_PROJECTS: &[UcProject] = &[
    UcProject {
        name: "ops_cli_pipeline",
        generated_files: &[
            "src/main.rs",
            "src/config.rs",
            "src/jobs.rs",
            "src/report.rs",
        ],
        cargo_dependencies: &["anyhow", "clap", "serde_json"],
        run_args: &["--limit", "2", "--region", "eu-west"],
        stdout_contains: &[
            "\"region\":\"eu-west\"",
            "\"total_cost\":89",
            "ingest",
            "transcode",
        ],
    },
    UcProject {
        name: "tokio_worker_pipeline",
        generated_files: &[
            "src/main.rs",
            "src/metrics.rs",
            "src/queue.rs",
            "src/worker.rs",
        ],
        cargo_dependencies: &["anyhow", "tokio"],
        run_args: &[],
        stdout_contains: &["processed=3", "total_ms=51"],
    },
    UcProject {
        name: "service_router",
        generated_files: &[
            "src/main.rs",
            "src/domain/mod.rs",
            "src/domain/order.rs",
            "src/domain/pricing.rs",
            "src/http/mod.rs",
            "src/http/routes.rs",
        ],
        cargo_dependencies: &["serde_json", "thiserror"],
        run_args: &[],
        stdout_contains: &[
            "\"status\":\"ok\"",
            "\"customer\":\"lulus\"",
            "\"total_cents\":5000",
        ],
    },
];

struct UcProject {
    name: &'static str,
    generated_files: &'static [&'static str],
    cargo_dependencies: &'static [&'static str],
    run_args: &'static [&'static str],
    stdout_contains: &'static [&'static str],
}

struct ProjectCase {
    root: PathBuf,
}

impl ProjectCase {
    fn new(project: &UcProject) -> Self {
        let workspace = workspace_root();
        let unique_id = CASE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = workspace.join("target-test-fixtures").join(format!(
            "uc-project-{}-{}-{unique_id}",
            project.name,
            std::process::id()
        ));

        if root.exists() {
            let _ = fs::remove_dir_all(&root);
        }

        let fixture = workspace
            .join("tests")
            .join("fixtures")
            .join("uc_projects")
            .join(project.name);
        copy_dir_all(&fixture, &root).unwrap_or_else(|error| {
            panic!(
                "{} fixture should copy from {} to {}: {error}",
                project.name,
                fixture.display(),
                root.display()
            )
        });

        Self { root }
    }

    fn generated_dir(&self) -> PathBuf {
        self.root.join("target").join("kobo-gen")
    }

    fn inspect_dir(&self) -> PathBuf {
        self.root.join("target").join("inspect-cargo")
    }
}

impl Drop for ProjectCase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct CmdOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

#[test]
fn uc_projects_build_and_run_as_real_multi_file_cargo_apps() {
    for project in UC_PROJECTS {
        let case = ProjectCase::new(project);
        assert_no_stub_markers(&case.root, project.name);

        let build = run_kobo_build(&case.root);
        assert_success(&build, project.name, "kobo build");

        assert_generated_project_shape(project, &case.generated_dir());
        assert_no_stub_markers(&case.generated_dir(), project.name);

        let run = run_generated_project(&case.generated_dir(), project.run_args);
        assert_success(&run, project.name, "generated cargo run");
        for expected in project.stdout_contains {
            assert!(
                run.stdout.contains(expected),
                "{} generated app stdout should contain {expected:?}\nstdout:\n{}\nstderr:\n{}",
                project.name,
                run.stdout,
                run.stderr
            );
        }
    }
}

#[test]
fn inspect_cargo_packages_real_multi_file_projects() {
    for project in UC_PROJECTS {
        let case = ProjectCase::new(project);
        assert_no_stub_markers(&case.root, project.name);

        let inspect = run_kobo_inspect_cargo(&case.root, &case.inspect_dir());
        assert_success(&inspect, project.name, "kobo inspect --cargo");

        assert_generated_project_shape(project, &case.inspect_dir());
        assert_no_stub_markers(&case.inspect_dir(), project.name);

        let run = run_generated_project(&case.inspect_dir(), project.run_args);
        assert_success(&run, project.name, "inspect-generated cargo run");
        for expected in project.stdout_contains {
            assert!(
                run.stdout.contains(expected),
                "{} inspect-generated app stdout should contain {expected:?}\nstdout:\n{}\nstderr:\n{}",
                project.name,
                run.stdout,
                run.stderr
            );
        }
    }
}

fn assert_generated_project_shape(project: &UcProject, generated_dir: &Path) {
    let cargo_toml_path = generated_dir.join("Cargo.toml");
    let cargo_toml = fs::read_to_string(&cargo_toml_path).unwrap_or_else(|error| {
        panic!(
            "{} generated Cargo.toml should read at {}: {error}",
            project.name,
            cargo_toml_path.display()
        )
    });

    for dependency in project.cargo_dependencies {
        assert!(
            cargo_toml.contains(dependency),
            "{} generated Cargo.toml should include dependency {dependency}\n{cargo_toml}",
            project.name
        );
    }

    for generated_file in project.generated_files {
        let path = generated_dir.join(generated_file);
        let source = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "{} generated file should exist and read at {}: {error}",
                project.name,
                path.display()
            )
        });
        assert!(
            !source.trim().is_empty(),
            "{} generated file {} should contain real code",
            project.name,
            generated_file
        );
    }
}

fn assert_no_stub_markers(root: &Path, project_name: &str) {
    let files = collect_files(root).unwrap_or_else(|error| {
        panic!(
            "{} files should be collectable under {}: {error}",
            project_name,
            root.display()
        )
    });
    for file in files {
        let extension = file.extension().and_then(|ext| ext.to_str());
        if !matches!(extension, Some("kobo") | Some("rs") | Some("toml")) {
            continue;
        }

        let source = fs::read_to_string(&file).unwrap_or_else(|error| {
            panic!(
                "{} file should read at {}: {error}",
                project_name,
                file.display()
            )
        });
        let lowercase = source.to_lowercase();
        for marker in ["todo!", "unimplemented!", "stub", "placeholder"] {
            assert!(
                !lowercase.contains(marker),
                "{} should not contain stub marker {marker:?} in {}",
                project_name,
                file.display()
            );
        }
    }
}

fn run_kobo_build(project_dir: &Path) -> CmdOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .arg("build")
        .current_dir(project_dir)
        .output()
        .expect("kobo build should run");
    cmd_output(output)
}

fn run_kobo_inspect_cargo(project_dir: &Path, output_dir: &Path) -> CmdOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .args([
            "inspect",
            "--clean",
            "--cargo",
            output_dir
                .to_str()
                .expect("inspect cargo output path should be UTF-8"),
            "src/main.kobo",
        ])
        .current_dir(project_dir)
        .output()
        .expect("kobo inspect --cargo should run");
    cmd_output(output)
}

fn run_generated_project(generated_dir: &Path, args: &[&str]) -> CmdOutput {
    let mut command = Command::new("cargo");
    command.arg("run").arg("--quiet").current_dir(generated_dir);
    if !args.is_empty() {
        command.arg("--").args(args);
    }

    let output = command.output().expect("generated cargo run should start");
    cmd_output(output)
}

fn assert_success(output: &CmdOutput, project_name: &str, step: &str) {
    assert!(
        output.status.success(),
        "{project_name} {step} failed\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
}

fn cmd_output(output: std::process::Output) -> CmdOutput {
    CmdOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn copy_dir_all(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn collect_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_files_into(root, &mut files)?;
    Ok(files)
}

fn collect_files_into(root: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !root.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files_into(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root should exist")
}
