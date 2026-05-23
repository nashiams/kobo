pub(super) fn cmd_explain(code: &str, verbose: bool) -> anyhow::Result<()> {
    if let Some(text) = explain_profile(code) {
        println!("{text}");
        return Ok(());
    }

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
     What happened\n\
     Kobo selected the stable sync simulation profile.\n\n\
     Why this matters\n\
     This is a stable recommendation for lock, atomic, and small shared-state code. Kobo records Loom as backend evidence when the linked adapter runs, but ordinary Kobo source does not gain Loom imports.\n\n\
     How to fix\n\
     Option 1: Keep this profile for sync shared-state islands.\n\
     Option 2: Pin `--profile sync` when you want stable metadata.\n\
     Option 3: Use `kobo sim scout --why` when the inferred shape looks wrong.\n\n\
     Example\n\
     Problem:\nA target mostly uses mutexes or atomics.\n\n\
     Fix:\nRun `kobo sim init --profile sync --minimal --target FILE:SYMBOL`."
}

fn async_profile_explain() -> &'static str {
    "profile:async\n\n\
     What happened\n\
     Kobo selected the stable async simulation profile.\n\n\
     Why this matters\n\
     This is a stable recommendation for async functions, `tokio::spawn`, `tokio::select!`, cancellation, and request-handler shapes. Kobo may record shuttle as reserved backend metadata while keeping the quick runner Kobo-level and avoiding user-source rewrites into Shuttle code.\n\n\
     How to fix\n\
     Option 1: Keep this profile for async spawn/select/cancellation islands.\n\
     Option 2: Pin `--profile async` when the target shape is intentionally async.\n\
     Option 3: Move network-only effects behind a boundary policy before claiming exact replay.\n\n\
     Example\n\
     Problem:\nA target uses `tokio::spawn` and can be cancelled around an await point.\n\n\
     Fix:\nRun `kobo sim init --profile async --minimal --target FILE:SYMBOL`."
}

fn stateful_input_profile_explain() -> &'static str {
    "profile:stateful-input\n\n\
     What happened\n\
     Kobo selected the stable stateful-input simulation profile.\n\n\
     Why this matters\n\
     This is a stable recommendation for parser, reducer, and operation-stream targets. Kobo can use generated input evidence without exposing backend-native controls in ordinary source.\n\n\
     How to fix\n\
     Option 1: Keep this profile for input/state transition islands.\n\
     Option 2: Add explicit scenario inputs when generated fixtures are not enough.\n\
     Option 3: Move IO outside the state-machine target.\n\n\
     Example\n\
     Problem:\nA reducer accepts operation strings and mutates state.\n\n\
     Fix:\nUse the generated island as the first property-style scenario."
}

fn failpoint_profile_explain() -> &'static str {
    "profile:failpoint\n\n\
     What happened\n\
     Kobo selected the stable failpoint simulation profile.\n\n\
     Why this matters\n\
     This is a stable recommendation for explicit failure branches and retry paths. Failure hooks remain sim-only and must not alter normal build/run behavior.\n\n\
     How to fix\n\
     Option 1: Keep this profile when the target already marks failure points.\n\
     Option 2: Use `--inject cancel,preempt,time-jump,crash` for quick modeled hook evidence.\n\
     Option 3: Keep production failure behavior in normal Rust code.\n\n\
     Example\n\
     Problem:\nA retry path should be checked against injected send failure.\n\n\
     Fix:\nRun a quick sim with an explicit failure hook list."
}

fn network_profile_explain() -> &'static str {
    "profile:network\n\n\
     What happened\n\
     Kobo selected the stable network simulation profile.\n\n\
     Why this matters\n\
     Network profile evidence is boundary-first. Kobo records the boundary and recommendation, but does not claim arbitrary external network replay.\n\n\
     How to fix\n\
     Option 1: Treat this as a boundary-design prompt.\n\
     Option 2: Choose typed, model, record, activity, stub, outside, opaque, or debt for replay-critical calls.\n\
     Option 3: Wait for later scheduler/network backend work before claiming exact network replay.\n\n\
     Example\n\
     Problem:\nA target constructs `reqwest::Client` inside a scenario.\n\n\
     Fix:\nAdd a boundary policy before treating the witness as replay evidence."
}
