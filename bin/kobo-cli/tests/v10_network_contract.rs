mod v09_common;

use v09_common::{
    assert_contains, assert_failure, assert_success, path_arg, run_kobo, s, TestProject,
};

#[test]
fn modeled_network_drop_delay_reorder_events_are_serialized() {
    let project = TestProject::new("v10-network-modeled");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "network")]
fn network_island() {
    ward.network.send("request");
    ward.network.delay("reply");
    ward.network.reorder("reply");
    ward.network.drop("stale");
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("21"),
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(&output, "modeled in-process network island should run");
    let text = output.combined();
    for event in [
        "network-send",
        "network-delay",
        "network-reorder",
        "network-drop",
    ] {
        assert_contains(
            &text,
            event,
            "network event stream should include modeled hooks",
        );
    }
}

#[test]
fn unknown_external_network_boundary_is_not_silently_modeled() {
    let project = TestProject::new("v10-network-boundary");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "network")]
fn real_network_boundary() {
    let _client = reqwest::Client::new();
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_failure(&output, "real network boundary should require policy");
    let text = output.combined();
    assert_contains(
        &text,
        "K0107",
        "external network should emit boundary diagnostic",
    );
    assert_contains(
        &text,
        "reqwest",
        "diagnostic should name the external crate",
    );
    assert_contains(
        &text,
        "model",
        "diagnostic should offer boundary policy choices",
    );
}
