pub(super) fn scenario_failure_actions(message: &str) -> Option<String> {
    let actions = message
        .split("finish with ")
        .nth(1)
        .or_else(|| message.split("discharge with ").nth(1))?;
    let actions = actions
        .split(" or pass ")
        .next()
        .unwrap_or(actions)
        .trim_end_matches('.');
    Some(actions.to_owned())
}
