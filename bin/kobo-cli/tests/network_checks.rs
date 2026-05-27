mod cli_test_support;

use std::fs;
use std::time::Duration;

use cli_test_support::{
    assert_contains, assert_failure, assert_success, path_arg, run_kobo, run_kobo_with_timeout, s,
    TestProject,
};
use serde_json::Value;

const NETWORK_REPLAY_TIMEOUT: Duration = Duration::from_secs(180);

#[test]
fn modeled_network_drop_delay_reorder_events_are_serialized() {
    let project = TestProject::new("runtime-network-modeled");
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

    let output = run_kobo_with_timeout(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("21"),
            s("--events=json"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
        NETWORK_REPLAY_TIMEOUT,
    );

    assert_success(&output, "generated loopback network harness should run");
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
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("network success should still write a witness when requested");
    let witness: Value =
        serde_json::from_str(&fs::read_to_string(witness_path).expect("witness should read"))
            .expect("witness should parse");
    let harness_path = witness["harness_manifest"]["harness_rs_path"]
        .as_str()
        .expect("network witness should include harness source");
    let harness_source = fs::read_to_string(harness_path).expect("harness source should read");
    assert_contains(
        &harness_source,
        "std::net::UdpSocket",
        "network facade must execute through the generated Rust loopback network harness",
    );
}

#[test]
fn unknown_external_network_boundary_is_not_silently_modeled() {
    let project = TestProject::new("runtime-network-boundary");
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
