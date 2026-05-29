use std::path::{Path, PathBuf};

pub(super) fn cmd_explain(code: &str, location: Option<&str>, verbose: bool) -> anyhow::Result<()> {
    if let Some(text) = explain_profile(code) {
        anyhow::ensure!(
            location.is_none(),
            "profile explanations do not accept a source location"
        );
        println!("{text}");
        return Ok(());
    }

    let detail = if verbose {
        kobo_errors::ExplainDetail::Verbose
    } else {
        kobo_errors::ExplainDetail::Human
    };

    match kobo_errors::explain_code_with_detail(code, detail) {
        Some(mut text) => {
            if let Some(location) = location {
                append_location_context(&mut text, code, location)?;
            }
            println!("{text}");
            Ok(())
        }
        None => anyhow::bail!("{}", kobo_errors::unknown_code_message(code)),
    }
}

#[derive(Debug, Eq, PartialEq)]
struct ExplainLocation {
    path: PathBuf,
    line: usize,
    column: Option<usize>,
}

fn append_location_context(text: &mut String, code: &str, location: &str) -> anyhow::Result<()> {
    let location = parse_explain_location(location)?;

    text.push_str("\nSource context\n");
    text.push_str("location: ");
    text.push_str(&render_explain_location(&location));
    text.push('\n');

    if code == "K0107" {
        text.push_str(
            "boundary choices: model, record, outside, opaque, or debt keep replay policy explicit.\n",
        );
    }

    if let Some(line) = read_location_line(&location)? {
        text.push_str("source: ");
        text.push_str(line.trim_end());
        text.push('\n');
    } else {
        text.push_str("source: unavailable; file was not found at this path.\n");
    }

    Ok(())
}

fn parse_explain_location(raw: &str) -> anyhow::Result<ExplainLocation> {
    let Some((path_and_line, line_or_column)) = raw.rsplit_once(':') else {
        anyhow::bail!("expected FILE:LINE[:COLUMN], got `{raw}`");
    };

    let line_or_column = parse_position("line", line_or_column)?;
    let (path, line, column) = if let Some((path, maybe_line)) = path_and_line.rsplit_once(':') {
        match parse_position("line", maybe_line) {
            Ok(line) => (path, line, Some(line_or_column)),
            Err(_) => (path_and_line, line_or_column, None),
        }
    } else {
        (path_and_line, line_or_column, None)
    };

    anyhow::ensure!(!path.trim().is_empty(), "explain location path is empty");
    Ok(ExplainLocation {
        path: PathBuf::from(path),
        line,
        column,
    })
}

fn parse_position(name: &str, raw: &str) -> anyhow::Result<usize> {
    let value = raw
        .parse::<usize>()
        .map_err(|_| anyhow::anyhow!("explain location {name} must be a positive integer"))?;
    anyhow::ensure!(
        value > 0,
        "explain location {name} must be a positive integer"
    );
    Ok(value)
}

fn render_explain_location(location: &ExplainLocation) -> String {
    match location.column {
        Some(column) => format!("{}:{}:{column}", location.path.display(), location.line),
        None => format!("{}:{}", location.path.display(), location.line),
    }
}

fn read_location_line(location: &ExplainLocation) -> anyhow::Result<Option<String>> {
    if !Path::new(&location.path).exists() {
        return Ok(None);
    }

    let source = std::fs::read_to_string(&location.path)?;
    Ok(source
        .lines()
        .nth(location.line.saturating_sub(1))
        .map(str::to_owned))
}

fn explain_profile(query: &str) -> Option<&'static str> {
    match query {
        "profile:sync" => Some(sync_profile_explain()),
        "profile:async" => Some(async_profile_explain()),
        "profile:stateful-input" => Some(stateful_input_profile_explain()),
        "profile:failpoint" => Some(failpoint_profile_explain()),
        "profile:network" => Some(network_profile_explain()),
        _ => None,
    }
}

fn sync_profile_explain() -> &'static str {
    "profile:sync\n\n\
     I found:\n\
     Kobo selected the stable sync simulation profile.\n\n\
     Why I care:\n\
     This is a stable recommendation for lock, atomic, and small shared-state code. Kobo records Loom as backend evidence when the linked adapter runs, but ordinary Kobo source does not gain Loom imports.\n\n\
     Try this:\n\
     - Keep this profile for sync shared-state islands.\n\
     - Pin `--profile sync` when you want stable metadata.\n\
     - Use `kobo sim scout --why` when the inferred shape looks wrong.\n\n\
     Example\n\
     Problem:\nA target mostly uses mutexes or atomics.\n\n\
     Fix:\nRun `kobo sim init --profile sync --minimal --target FILE:SYMBOL`."
}

fn async_profile_explain() -> &'static str {
    "profile:async\n\n\
     I found:\n\
     Kobo selected the stable async simulation profile.\n\n\
     Why I care:\n\
     This is a stable recommendation for async functions, `tokio::spawn`, `tokio::select!`, cancellation, and request-handler shapes. Kobo may record shuttle as reserved backend metadata while keeping the quick runner Kobo-level and avoiding user-source rewrites into Shuttle code.\n\n\
     Try this:\n\
     - Keep this profile for async spawn/select/cancellation islands.\n\
     - Pin `--profile async` when the target shape is intentionally async.\n\
     - Move network-only effects behind a boundary policy before claiming exact replay.\n\n\
     Example\n\
     Problem:\nA target uses `tokio::spawn` and can be cancelled around an await point.\n\n\
     Fix:\nRun `kobo sim init --profile async --minimal --target FILE:SYMBOL`."
}

fn stateful_input_profile_explain() -> &'static str {
    "profile:stateful-input\n\n\
     I found:\n\
     Kobo selected the stable stateful-input simulation profile.\n\n\
     Why I care:\n\
     This is a stable recommendation for parser, reducer, and operation-stream targets. Kobo can use generated input evidence without exposing backend-native controls in ordinary source.\n\n\
     Try this:\n\
     - Keep this profile for input/state transition islands.\n\
     - Add explicit scenario inputs when generated fixtures are not enough.\n\
     - Move IO outside the state-machine target.\n\n\
     Example\n\
     Problem:\nA reducer accepts operation strings and mutates state.\n\n\
     Fix:\nUse the generated island as the first property-style scenario."
}

fn failpoint_profile_explain() -> &'static str {
    "profile:failpoint\n\n\
     I found:\n\
     Kobo selected the stable failpoint simulation profile.\n\n\
     Why I care:\n\
     This is a stable recommendation for explicit failure branches and retry paths. Failure hooks remain sim-only and must not alter normal build/run behavior.\n\n\
     Try this:\n\
     - Keep this profile when the target already marks failure points.\n\
     - Use `--inject cancel,preempt,time-jump,crash` for quick modeled hook evidence.\n\
     - Keep production failure behavior in normal Rust code.\n\n\
     Example\n\
     Problem:\nA retry path should be checked against injected send failure.\n\n\
     Fix:\nRun a quick sim with an explicit failure hook list."
}

fn network_profile_explain() -> &'static str {
    "profile:network\n\n\
     I found:\n\
     Kobo selected the stable network simulation profile.\n\n\
     Why I care:\n\
     Network profile evidence is boundary-first. Kobo records the boundary and recommendation, but does not claim arbitrary external network replay.\n\n\
     Try this:\n\
     - Treat this as a boundary-design prompt.\n\
     - Choose typed, model, record, activity, stub, outside, opaque, or debt for replay-critical calls.\n\
     - Wait for later scheduler/network backend work before claiming exact network replay.\n\n\
     Example\n\
     Problem:\nA target constructs `reqwest::Client` inside a scenario.\n\n\
     Fix:\nAdd a boundary policy before treating the witness as replay evidence."
}
