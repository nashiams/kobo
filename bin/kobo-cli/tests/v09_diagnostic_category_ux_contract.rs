mod v09_common;

use std::path::{Path, PathBuf};

use v09_common::{
    assert_contains, assert_failure, assert_not_contains, path_arg, run_kobo, s, TestProject,
};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn ui_fixture(relative: &str) -> PathBuf {
    repo_root().join("tests").join("ui").join(relative)
}

fn assert_elm_level_cli_card(output: &str, code: &str) {
    assert_contains(output, code, "category smoke must emit expected code");
    assert_contains(output, "-->", "category smoke must show source location");
    assert_contains(
        output,
        "What Kobo found:",
        "category smoke must explain the local finding",
    );
    assert_contains(
        output,
        "Why this matters:",
        "category smoke must explain why the user should care",
    );
    assert_contains(output, "Try this:", "category smoke must give an action");
    assert_contains(
        output,
        "More:",
        "category smoke must point to deeper explain text",
    );
    assert_contains(
        output,
        &format!("kobo explain {code}"),
        "category smoke must expose the explain command",
    );

    for forbidden in [
        "kobo decision:",
        "greedy priority",
        "engine ceiling",
        "constraint cluster",
        "constraint conflict",
        "constraint graph",
        "solver",
        "poisoned",
        "Error: analysis failed",
        "<unknown>",
        "[bytes ",
        "\u{00e2}",
        "\u{00c3}\u{00a2}",
        "\u{fffd}",
    ] {
        assert_not_contains(
            output,
            forbidden,
            "category smoke must avoid human-output jank",
        );
    }
}

#[test]
fn real_cli_cards_are_elm_level_across_diagnostic_categories() {
    let ownership = TestProject::new("category-ownership");
    let ownership_out = run_kobo(
        &[
            s("check"),
            s("--checked"),
            path_arg(&ui_fixture("K0001_use_after_move.kobo")),
        ],
        &ownership.root,
    );
    assert_elm_level_cli_card(&ownership_out.combined(), "K0001");

    let strict_boundary = TestProject::new("category-strict-boundary");
    let strict_boundary_out = run_kobo(
        &[
            s("check"),
            s("--strict"),
            path_arg(&ui_fixture("K0025_hint_ignored.kobo")),
        ],
        &strict_boundary.root,
    );
    assert_failure(&strict_boundary_out, "strict boundary fixture should fail");
    assert_elm_level_cli_card(&strict_boundary_out.combined(), "K0025");

    let async_category = TestProject::new("category-async");
    let async_out = run_kobo(
        &[
            s("check"),
            s("--strict"),
            path_arg(&ui_fixture("strict_k0063_async_context.kobo")),
        ],
        &async_category.root,
    );
    assert_failure(&async_out, "async strict fixture should fail");
    assert_elm_level_cli_card(&async_out.combined(), "K0063");

    let design = TestProject::new("category-design");
    let design_out = run_kobo(
        &[s("check"), path_arg(&ui_fixture("K0080_P1_node.kobo"))],
        &design.root,
    );
    assert_elm_level_cli_card(&design_out.combined(), "K0080-P1");

    let parser = TestProject::new("category-parser");
    let parser_file = parser.main_file("fn main() { let broken = }\n");
    let parser_out = run_kobo(&[s("check"), path_arg(&parser_file)], &parser.root);
    assert_failure(&parser_out, "parser fixture should fail");
    assert_elm_level_cli_card(&parser_out.combined(), "K0110");
}

#[test]
fn replay_liveness_boundary_and_perf_cards_use_the_same_human_contract() {
    let liveness = TestProject::new("category-liveness");
    let liveness_file = liveness.copy_fixture("sim/gateway.kobo", "src/gateway.kobo");
    let liveness_out = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--profile"),
            s("checked"),
            s("--seed"),
            s("7"),
            path_arg(&liveness_file),
        ],
        &liveness.root,
    );
    assert_failure(&liveness_out, "liveness fixture should fail");
    let liveness_text = liveness_out.combined();
    assert_elm_level_cli_card(&liveness_text, "K0100");
    assert_not_contains(
        &liveness_text,
        "make replay evidence deterministic",
        "K0100 must not use a generic replay fix",
    );

    let nondeterminism = TestProject::new("category-nondeterminism");
    let nondeterminism_file =
        nondeterminism.copy_fixture("sim/raw_clock.kobo", "src/raw_clock.kobo");
    let nondeterminism_out = run_kobo(
        &[s("check"), path_arg(&nondeterminism_file)],
        &nondeterminism.root,
    );
    assert_failure(&nondeterminism_out, "nondeterminism fixture should fail");
    assert_elm_level_cli_card(&nondeterminism_out.combined(), "K0102");

    let boundary = TestProject::new("category-boundary");
    let boundary_file = boundary.main_file(
        r#"
use external_service::Client;

#[kobo::scenario(name = "fetch_user")]
fn fetch_user() {
    let client = Client::new();
    println!("{:?}", client);
}
"#,
    );
    let boundary_out = run_kobo(
        &[s("check"), s("--replay-critical"), path_arg(&boundary_file)],
        &boundary.root,
    );
    assert_elm_level_cli_card(&boundary_out.combined(), "K0107");

    let perf = TestProject::new("category-perf");
    let perf_file = perf.main_file(
        r#"
fn main() {
    hot_path();
}
"#,
    );
    let perf_log = perf.write(
        "diag.log",
        "[kobo-diag] src/main.kobo:3:5 - (Rc<RefCell<T>>)\n\
         borrow_count: 20000\n\
         mut_borrow_count: 20001\n\
         contention_count: 2\n\
         saturated: false\n",
    );
    let perf_out = run_kobo(
        &[
            s("perf"),
            path_arg(&perf_file),
            s("--from"),
            path_arg(&perf_log),
            s("--threshold"),
            s("10"),
        ],
        &perf.root,
    );
    assert_elm_level_cli_card(&perf_out.combined(), "K0020");
}
