pub(super) fn cmd_explain(code: &str, verbose: bool) -> anyhow::Result<()> {
    let detail = if verbose {
        kobo_errors::ExplainDetail::Verbose
    } else {
        kobo_errors::ExplainDetail::Human
    };

    match kobo_errors::explain_code_with_detail(code, detail) {
        Some(text) => {
            println!("{text}");
            Ok(())
        }
        None => anyhow::bail!("{}", kobo_errors::unknown_code_message(code)),
    }
}
