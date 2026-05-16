#[test]
fn error_policy_sites_ignore_literals_comments_and_raw_strings() {
    let source = r##"
fn main() -> Result<(), std::io::Error> {
    println!("literal ?");
    let raw = r#"raw ? string"#;
    // comment ?
    std::fs::read_to_string("missing.txt")?;
    Ok(())
}
"##;
    let artifacts = kobo_driver::test_utils::run_codegen_for_source_with_policy(source, "typed")
        .expect("codegen should succeed");
    assert_eq!(
        artifacts.error_policy_sites.len(),
        1,
        "only the real ? operator should become an error policy site"
    );
    assert!(
        artifacts
            .rs_source
            .contains(".map_err(KoboTypedError::ReadToString)?"),
        "generated Rust must map the real error operator"
    );
    assert!(
        !artifacts.rs_source.contains("__kobo_error_policy_site"),
        "codegen-owned error policy markers must not leak into generated Rust"
    );
    assert!(
        artifacts.rs_source.contains("literal ?") && artifacts.rs_source.contains("raw ? string"),
        "non-operator question marks must survive unchanged"
    );
}
