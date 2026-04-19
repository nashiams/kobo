/// Generate fixed-timestep loop from #[kobo::tick(rate=N)].
///
/// Input:
///   #[kobo::tick(rate=20)]
///   fn game_loop(state: &mut GameState) {
///       state.physics.step();
///       state.renderer.draw();
///   }
///
/// Output:
///   async fn game_loop(state: &mut GameState) {
///       let mut interval = tokio::time::interval(Duration::from_millis(50));
///       interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
///       loop {
///           interval.tick().await;
///           state.physics.step();
///           state.renderer.draw();
///       }
///   }
///
/// rate=N means N ticks per second → interval = 1000/N ms
/// MissedTickBehavior::Skip prevents drift accumulation.

/// Parse tick rate from `#[kobo::tick(rate=N)]` attribute.
pub(crate) fn parse_tick_rate(attr: &syn::Attribute) -> Option<u32> {
    // Match #[kobo::tick(rate=N)]
    let path = attr.path();
    let segments: Vec<_> = path.segments.iter().collect();
    if segments.len() != 2 {
        return None;
    }
    if segments[0].ident != "kobo" || segments[1].ident != "tick" {
        return None;
    }

    // Parse the rate=N from the attribute arguments.
    if let syn::Meta::List(list) = &attr.meta {
        let tokens = list.tokens.to_string();
        // Parse "rate = N" or "rate=N"
        let stripped = tokens.replace(' ', "");
        if let Some(rest) = stripped.strip_prefix("rate=") {
            return rest.parse::<u32>().ok();
        }
    }
    None
}

/// Check if an attribute is `#[kobo::tick(...)]`.
pub(crate) fn is_tick_attribute(attr: &syn::Attribute) -> bool {
    let path = attr.path();
    let segments: Vec<_> = path.segments.iter().collect();
    segments.len() == 2 && segments[0].ident == "kobo" && segments[1].ident == "tick"
}

/// Generate the tick loop source code for a given rate and function body.
///
/// Returns a string with the async fn wrapping the body in an interval loop.
pub(crate) fn generate_tick_loop_source(
    fn_name: &str,
    params: &str,
    body: &str,
    rate: u32,
) -> String {
    let interval_ms = if rate > 0 { 1000 / rate } else { 1000 };
    format!(
        r#"async fn {fn_name}({params}) {{
    use std::time::Duration;
    use tokio::time::MissedTickBehavior;
    let mut interval = tokio::time::interval(Duration::from_millis({interval_ms}));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {{
        interval.tick().await;
{body}
    }}
}}"#
    )
}

/// Strip #[kobo::tick(...)] attributes from a function source string.
pub(crate) fn strip_tick_attributes(source: &str) -> String {
    let mut result = String::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("#[kobo::tick") {
            continue;
        }
        result.push_str(line);
        result.push('\n');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_loop_20hz() {
        let output = generate_tick_loop_source(
            "game_loop",
            "state: &mut GameState",
            "        state.physics.step();",
            20,
        );
        assert!(output.contains("tokio::time::interval"), "output: {output}");
        assert!(output.contains("Duration::from_millis(50)"), "output: {output}");
        assert!(output.contains("MissedTickBehavior::Skip"), "output: {output}");
        assert!(output.contains("loop {"), "output: {output}");
    }

    #[test]
    fn tick_loop_60hz() {
        let output = generate_tick_loop_source(
            "render",
            "state: &mut RenderState",
            "        state.draw_frame();",
            60,
        );
        // 1000/60 = 16
        assert!(output.contains("Duration::from_millis(16)"), "output: {output}");
    }

    #[test]
    fn parse_tick_rate_from_attr() {
        let input: syn::ItemFn = syn::parse_str(
            r#"
            #[kobo::tick(rate = 20)]
            fn game_loop() {}
        "#,
        )
        .unwrap();
        let attr = &input.attrs[0];
        let rate = parse_tick_rate(attr);
        assert_eq!(rate, Some(20));
    }

    #[test]
    fn is_tick_attr_check() {
        let input: syn::ItemFn = syn::parse_str(
            r#"
            #[kobo::tick(rate = 30)]
            fn tick_fn() {}
        "#,
        )
        .unwrap();
        assert!(is_tick_attribute(&input.attrs[0]));
    }

    #[test]
    fn not_tick_attr_for_other() {
        let input: syn::ItemFn = syn::parse_str(
            r#"
            #[kobo::engine]
            fn engine_fn() {}
        "#,
        )
        .unwrap();
        assert!(!is_tick_attribute(&input.attrs[0]));
    }

    #[test]
    fn strip_tick_attrs() {
        let input = r#"#[kobo::tick(rate=20)]
fn game_loop() {
    step();
}
"#;
        let output = strip_tick_attributes(input);
        assert!(!output.contains("kobo::tick"));
        assert!(output.contains("fn game_loop()"));
    }
}
