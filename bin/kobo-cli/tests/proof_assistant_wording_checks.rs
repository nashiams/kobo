#[test]
fn allowed_mechanized_soundness_wording_is_documented() {
    let docs = proof_docs();

    assert!(
        docs.contains("mechanized model covers Kobo's obligation core and proof verifier"),
        "docs must use the allowed mechanized-soundness wording"
    );
    assert!(
        docs.contains("sample accepted .kproof trace has a matching mechanized trace model"),
        "docs must describe the certificate bridge as sample-scoped"
    );
}

#[test]
fn docs_do_not_claim_whole_program_or_rust_runtime_proofs() {
    let docs = proof_docs();
    let forbidden = [
        "formally verifies all Kobo programs",
        "proves generated Rust binary behavior",
        "proves generated Rust",
        "proves Tokio",
        "proves third-party crates",
        "full-program verification",
    ];

    for phrase in forbidden {
        assert!(
            !docs.contains(phrase),
            "proof assistant docs must not overclaim with forbidden phrase `{phrase}`"
        );
    }
}

#[test]
fn proof_ci_rejects_unchecked_lean_debt_markers() {
    let workflow =
        std::fs::read_to_string(repo_root().join(".github/workflows/proof-mechanization.yml"))
            .expect("proof mechanization workflow should exist");

    for marker in ["sorry", "admit", "axiom", "constant\\s+.*:"] {
        assert!(
            workflow.contains(marker),
            "proof CI must reject Lean debt marker `{marker}`"
        );
    }
}

#[test]
fn proof_ci_builds_named_lean_library_target() {
    let workflow =
        std::fs::read_to_string(repo_root().join(".github/workflows/proof-mechanization.yml"))
            .expect("proof mechanization workflow should exist");

    assert!(
        workflow.contains("lake build KoboProof"),
        "proof CI must build the named Lean library target, not only the default Lake target"
    );
    assert!(
        workflow.contains("working-directory: proof/lean"),
        "proof CI must run the named Lean build from the Lean package directory"
    );
}

#[test]
fn proof_docs_use_named_lean_library_target() {
    let docs = proof_docs();

    assert!(
        docs.contains("lake build KoboProof"),
        "proof docs must tell release reviewers to build the named Lean library target"
    );
    assert!(
        !docs.contains("lake build\n"),
        "proof docs must not document plain `lake build`, which can miss the release library target"
    );
}

fn proof_docs() -> String {
    [
        "proof/README.md",
        "docs/proof-assistant-mechanization.md",
        "README.md",
    ]
    .into_iter()
    .map(|relative| {
        std::fs::read_to_string(repo_root().join(relative))
            .unwrap_or_else(|error| panic!("{relative} should exist: {error}"))
    })
    .collect::<Vec<_>>()
    .join("\n")
}

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("kobo-cli manifest should live under bin/kobo-cli")
        .to_path_buf()
}
