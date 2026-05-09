use std::fs;
use std::path::{Path, PathBuf};

const PHASE_COUNT: usize = 17;

#[test]
fn v085_oracle_gates_cover_every_phase_with_required_sections() {
    let roadmap = roadmap_root();
    let oracle = read(roadmap.join("acceptance/oracle_gates.md"));

    for phase in 0..PHASE_COUNT {
        let marker = format!("## Phase {phase:02}:");
        assert_contains(
            &oracle,
            &marker,
            "oracle_gates.md must define one section per v0.8.5 phase",
        );

        let section = section_after_marker(&oracle, &marker);
        for required in [
            "### Required Tests",
            "### Oracle Gates",
            "cargo test",
            "rg -n",
        ] {
            assert_contains(
                section,
                required,
                &format!("phase {phase:02} oracle section is not executable enough"),
            );
        }
    }

    for required in [
        "## Universal Oracle Rules",
        "### RED Evidence",
        "### Public Surface Rule",
        "### Mutation Rule",
        "### No-Hardcode Rule",
        "### Cross-Layer Rule",
        "### Source Probe Rule",
        "## Final Release Oracle",
        "spec-driven TDD",
        "Passing old tests is not enough",
        "Missing test targets are blockers",
    ] {
        assert_contains(
            &oracle,
            required,
            "oracle_gates.md lost a hard-to-game universal rule",
        );
    }
}

#[test]
fn every_v085_phase_file_requires_the_central_oracle_matrix() {
    let phases_root = roadmap_root().join("phases");
    let entries = fs::read_dir(&phases_root)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", phases_root.display()));
    let mut phase_files = entries
        .map(|entry| entry.expect("phase entry should be readable").path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("md"))
        .collect::<Vec<_>>();
    phase_files.sort();

    assert_eq!(
        phase_files.len(),
        PHASE_COUNT,
        "v0.8.5 must keep exactly {PHASE_COUNT} phase files"
    );

    for path in phase_files {
        let text = read(&path);
        let file_name = path.file_name().and_then(|value| value.to_str()).unwrap();

        for required in [
            "## Test First",
            "## Acceptance",
            "## Anti-Lie Verification",
            "acceptance/oracle_gates.md",
        ] {
            assert_contains(
                &text,
                required,
                &format!("{file_name} must require test-first oracle gates"),
            );
        }
    }
}

#[test]
fn v085_release_docs_make_oracle_gates_release_blockers() {
    let roadmap = roadmap_root();
    let implement = read(roadmap.join("IMPLEMENT.md"));
    let gate_criteria = read(roadmap.join("acceptance/gate_criteria.md"));
    let todo = read(roadmap.join("todo.md"));
    let combined = format!("{implement}\n{gate_criteria}\n{todo}");

    for required in [
        "acceptance/oracle_gates.md",
        "RED and GREEN command evidence",
        "public-surface oracle gates",
        "Missing test targets are blockers",
        "helper-only tests",
        "constant string",
        "Every phase-specific gate",
    ] {
        assert_contains(
            &combined,
            required,
            "v0.8.5 release docs must make oracle gates non-optional",
        );
    }
}

#[test]
fn v085_oracle_gates_are_phase_specific_not_generic_checklists() {
    let oracle = read(roadmap_root().join("acceptance/oracle_gates.md"));

    for phase_specific in [
        "parser_recovery_uses_non_conflicting_codes",
        "query_key_uses_source_content_config_and_version",
        "lsp_payload_matches_cli_json_for_same_diagnostic",
        "must_call_metadata_reaches_ast_and_kir",
        "early_return_before_commit_or_rollback_emits_k0100",
        "scout_changes_top_rank_when_fixture_signals_change",
        "scenario_raw_time_random_spawn_filesystem_network_process_emit_k0102",
        "kwit_invalid_version_fails_with_registry_diagnostic",
        "scenario_attribute_does_not_enable_hidden_simulation",
        "normal_external_crate_use_builds_without_k0107_wall",
        "backend_registry_does_not_execute_backend",
        "suggestions_absent_when_pattern_absent",
        "visible_region_diagnostics_survive_budget_cap",
        "k0061_card_has_four_source_mapped_spans",
        "fix_refuses_maybe_incorrect_or_placeholder_suggestions",
        "lowering_creates_visible_rust_field_borrows",
        "normal_crate_without_kobo_metadata_still_builds",
    ] {
        assert_contains(
            &oracle,
            phase_specific,
            "oracle_gates.md must name mutation-sensitive phase-specific tests",
        );
    }
}

fn section_after_marker<'a>(text: &'a str, marker: &str) -> &'a str {
    let start = text
        .find(marker)
        .unwrap_or_else(|| panic!("missing section marker {marker}"));
    let rest = &text[start + marker.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    &rest[..end]
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root should exist")
}

fn roadmap_root() -> PathBuf {
    repo_root().join(".claude/prompt/roadmap/v0.8.5")
}

fn read(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

fn assert_contains(haystack: &str, needle: &str, message: &str) {
    assert!(haystack.contains(needle), "{message}\nmissing: {needle}");
}
