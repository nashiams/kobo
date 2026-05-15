use kobo_driver::test_utils::run_codegen_for_source_with_policy;

#[test]
fn question_marks_in_strings_and_comments_are_not_error_policy_sites() {
    let source = r#"
fn main() {
    let text = "this ? is not an error operator";
    // this ? is also not an error operator
    println!("{}", text);
}
"#;

    let artifacts = run_codegen_for_source_with_policy(source, "typed").expect("codegen artifacts");

    assert!(
        artifacts.error_policy_sites.is_empty(),
        "only real codegen-emitted ? operators may become error policy sites"
    );
    assert!(
        !artifacts.rs_source.contains("KoboTypedError"),
        "typed policy must not inject error enum for comments or string literals"
    );
}
