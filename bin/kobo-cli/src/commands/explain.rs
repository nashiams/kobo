pub(super) fn cmd_explain(code: &str) -> anyhow::Result<()> {
    match kobo_errors::explain_code(code) {
        Some(text) => {
            println!("{text}");
            Ok(())
        }
        None => anyhow::bail!("unknown Kobo diagnostic code `{code}`"),
    }
}
