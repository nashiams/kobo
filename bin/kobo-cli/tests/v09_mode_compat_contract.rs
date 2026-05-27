mod v09_common;

use std::process::Command;

use v09_common::{
    assert_contains, assert_not_contains, assert_success, path_arg, run_kobo, s, TestProject,
};

#[test]
fn accepted_profiles_preserve_ordinary_runtime_output() {
    let project = TestProject::new("mode-profile-output");
    let file = project.copy_fixture("policy/basic.kobo", "src/main.kobo");

    let mut outputs = Vec::new();
    for profile in ["dev", "checked", "release"] {
        let build = run_kobo(
            &[
                s("build"),
                s("--profile"),
                s(profile),
                s("--emit-rust"),
                path_arg(&file),
            ],
            &project.root,
        );
        assert_success(&build, "accepted profile build should succeed");
        outputs.push(run_generated_rust(&project, profile));
    }

    assert_eq!(
        outputs,
        vec![
            "policy\n".to_owned(),
            "policy\n".to_owned(),
            "policy\n".to_owned(),
        ],
        "dev, checked, and release presets must preserve ordinary runtime behavior",
    );
}

#[test]
fn normal_crates_do_not_need_sim_adapters() {
    let project = TestProject::new("mode-normal-crate");
    let file = project.copy_fixture("errors/boundary_normal_http.kobo", "src/main.kobo");

    for profile in ["dev", "checked", "release"] {
        let output = run_kobo(
            &[
                s("build"),
                s("--profile"),
                s(profile),
                s("--emit-rust"),
                path_arg(&file),
            ],
            &project.root,
        );
        assert_success(
            &output,
            "normal external crate build should not require sim adapter",
        );
        let text = output.combined();
        assert_contains(
            &text,
            "reqwest::Client::new",
            "normal crate path should remain ordinary Rust",
        );
        assert_not_contains(
            &text,
            "K0107",
            "normal non-replay path must not emit replay boundary diagnostic",
        );
        assert_not_contains(
            &text,
            "loom::",
            "normal source must not import a simulation backend",
        );
        assert_not_contains(
            &text,
            "shuttle::",
            "normal source must not import a simulation backend",
        );
    }
}

fn run_generated_rust(project: &TestProject, profile: &str) -> String {
    let rust_file = project.root.join("src/main.rs");
    let exe = project.root.join(format!("target-{profile}.exe"));
    let compile = Command::new("rustc")
        .arg(&rust_file)
        .arg("-o")
        .arg(&exe)
        .current_dir(&project.root)
        .output()
        .expect("rustc should launch for generated profile output");
    assert!(
        compile.status.success(),
        "generated Rust for {profile} should compile\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&compile.stdout),
        String::from_utf8_lossy(&compile.stderr),
    );
    let run = Command::new(&exe)
        .current_dir(&project.root)
        .output()
        .expect("generated profile executable should launch");
    assert!(
        run.status.success(),
        "generated Rust for {profile} should run\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    String::from_utf8_lossy(&run.stdout).to_string()
}
