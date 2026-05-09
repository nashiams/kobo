pub fn explain_code(code_text: &str) -> Option<String> {
    let registry = crate::diagnostic_registry();
    registry.find_by_code_text(code_text).map(|entry| {
        format!(
            "{} - {}\n\n{}\n",
            entry.code_text, entry.title, entry.explain
        )
    })
}
