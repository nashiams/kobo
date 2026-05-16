mod v09_common;

use std::fs;

use serde_json::Value;
use v09_common::{
    assert_contains, assert_failure, assert_not_contains, assert_success, path_arg, run_kobo, s,
    TestProject,
};

#[test]
fn sync_backend_adapter_writes_replay_token_and_stays_out_of_user_source() {
    let project = TestProject::new("v10-backend-adapter");
    let file = project.main_file(
        r#"
#[kobo::must_call(commit | rollback)]
struct Transaction {}

#[kobo::scenario(profile = "sync")]
fn sync_transaction() {
    let tx = Transaction {};
    let _lost = tx;
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("23"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(&output, "sync backend failure should emit witness");

    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness should exist");
    let witness: Value =
        serde_json::from_str(&fs::read_to_string(witness_path).expect("witness should read"))
            .expect("witness should parse");
    assert_eq!(witness["backend_profile"], "sync");
    assert_eq!(witness["backend"], "loom");
    assert!(witness["backend_replay_token"]
        .as_str()
        .is_some_and(|token| !token.is_empty()));

    let inspect = run_kobo(
        &[s("inspect"), s("--sim"), s("--harness"), path_arg(&file)],
        &project.root,
    );
    assert_success(
        &inspect,
        "inspect --sim --harness should show adapter boundary",
    );
    assert_contains(
        &inspect.combined(),
        "backend adapter",
        "inspect output should expose backend adapter boundary",
    );

    let source = project.read("src/main.kobo");
    assert_not_contains(&source, "loom::", "user source must not import Loom");
    assert_not_contains(&source, "shuttle::", "user source must not import Shuttle");
}
