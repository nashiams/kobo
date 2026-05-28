mod cli_test_support;

use cli_test_support::{
    assert_contains, assert_not_contains, assert_success, path_arg, run_kobo, s, TestProject,
};

#[test]
fn inspect_sim_shows_facade_lowering_without_rewriting_user_imports() {
    let project = TestProject::new("runtime-facade-inspect");
    let file = project.main_file(
        r#"
use tokio::time::sleep;

#[kobo::scenario(profile = "async")]
async fn async_gateway() {
    tokio::spawn(async move {});
    ward.task();
    ward.time.now();
}
"#,
    );

    let output = run_kobo(&[s("inspect"), s("--sim"), path_arg(&file)], &project.root);

    assert_success(&output, "inspect --sim should expose facade lowering");
    let text = output.combined();
    assert_contains(
        &text,
        "Rust-shaped and Cargo-native",
        "inspect output should keep Kobo-first product posture",
    );
    assert_contains(
        &text,
        "normal Kobo source stays framework-shaped",
        "inspect output should not make user source backend-shaped",
    );
    assert_contains(
        &text,
        "simulation facade",
        "inspect output should show facade lowering",
    );
    for surface in [
        "time", "spawn", "task", "select", "sync", "storage", "network",
    ] {
        assert_contains(&text, surface, "facade surface should be inspectable");
    }
    assert_not_contains(
        &text,
        "reserved for",
        "inspect --sim implementation must not report reserved harness wording",
    );
    let source = project.read("src/main.kobo");
    assert_not_contains(&source, "shuttle::", "user source must not import Shuttle");
    assert_not_contains(&source, "loom::", "user source must not import Loom");
    assert_not_contains(&source, "turmoil::", "user source must not import Turmoil");
    assert_not_contains(&source, "madsim::", "user source must not import Madsim");
}
