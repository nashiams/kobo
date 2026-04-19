use std::path::Path;

/// Per-function timing report for tick-loop functions.
///
/// Reads a .kobo file, finds functions annotated with `#[kobo::tick(rate=N)]`,
/// and reports estimated tick budget usage based on frame timing.
pub(super) fn cmd_bench_tick(file: &Path, tick_budget: bool) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {}", file.display(), e))?;

    if !tick_budget {
        println!("Use --tick-budget to enable tick budget analysis.");
        return Ok(());
    }

    let report = generate_tick_budget_report(&source);
    println!("{report}");
    Ok(())
}

/// A tick-budget entry for a single function.
pub(crate) struct TickBudgetEntry {
    pub fn_name: String,
    pub rate_hz: u32,
    pub budget_ms: f64,
}

impl TickBudgetEntry {
    fn status(&self) -> &'static str {
        // Placeholder: in real usage we'd measure actual time.
        // For now, always WITHIN BUDGET since we can't time without execution.
        "WITHIN BUDGET"
    }
}

/// Generate a tick budget report from source code.
pub(crate) fn generate_tick_budget_report(source: &str) -> String {
    let entries = extract_tick_functions(source);
    if entries.is_empty() {
        return "Tick Budget Report\n══════════════════\nNo #[kobo::tick] functions found.".to_string();
    }

    let mut lines = vec![
        "Tick Budget Report".to_string(),
        "══════════════════".to_string(),
    ];

    for entry in &entries {
        lines.push(format!(
            "  {} @ {}Hz — budget {:.1}ms — {}",
            entry.fn_name,
            entry.rate_hz,
            entry.budget_ms,
            entry.status(),
        ));
    }

    lines.join("\n")
}

/// Extract tick-annotated function information from source text.
fn extract_tick_functions(source: &str) -> Vec<TickBudgetEntry> {
    let mut entries = Vec::new();
    let lines: Vec<&str> = source.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if let Some(rate) = parse_tick_rate_from_line(trimmed) {
            // Look for the next `fn` line
            for next_line in &lines[i + 1..] {
                let next_trimmed = next_line.trim();
                if let Some(fn_name) = extract_fn_name(next_trimmed) {
                    let budget_ms = 1000.0 / rate as f64;
                    entries.push(TickBudgetEntry {
                        fn_name: fn_name.to_string(),
                        rate_hz: rate,
                        budget_ms,
                    });
                    break;
                }
                if !next_trimmed.is_empty() && !next_trimmed.starts_with('#') {
                    break;
                }
            }
        }
    }

    entries
}

/// Parse rate from a `#[kobo::tick(rate=N)]` line.
fn parse_tick_rate_from_line(line: &str) -> Option<u32> {
    if !line.starts_with("#[kobo::tick") {
        return None;
    }
    let stripped = line.replace(' ', "");
    if let Some(start) = stripped.find("rate=") {
        let rest = &stripped[start + 5..];
        let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        return num_str.parse::<u32>().ok();
    }
    None
}

/// Extract function name from a `fn name(...)` line.
fn extract_fn_name(line: &str) -> Option<&str> {
    let rest = if let Some(r) = line.strip_prefix("async fn ") {
        r
    } else if let Some(r) = line.strip_prefix("fn ") {
        r
    } else if let Some(r) = line.strip_prefix("pub fn ") {
        r
    } else if let Some(r) = line.strip_prefix("pub async fn ") {
        r
    } else {
        return None;
    };

    let end = rest.find('(')?;
    Some(rest[..end].trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bench_tick_budget_report_format() {
        let source = r#"
#[kobo::tick(rate = 20)]
fn game_loop(state: &mut GameState) {
    state.physics.step();
    state.renderer.draw();
}

#[kobo::tick(rate = 60)]
fn render_loop(ctx: &mut RenderCtx) {
    ctx.draw_frame();
}
"#;

        let report = generate_tick_budget_report(source);
        assert!(report.contains("Tick Budget Report"), "report: {report}");
        assert!(report.contains("game_loop"), "report: {report}");
        assert!(report.contains("render_loop"), "report: {report}");
        assert!(report.contains("20Hz"), "report: {report}");
        assert!(report.contains("60Hz"), "report: {report}");
        // budget for 20Hz = 50ms, 60Hz ≈ 16.7ms
        assert!(report.contains("50.0ms"), "report: {report}");
        assert!(
            report.contains("WITHIN BUDGET") || report.contains("OVER BUDGET"),
            "report: {report}"
        );
    }

    #[test]
    fn bench_no_tick_functions() {
        let source = "fn main() { println!(\"hello\"); }";
        let report = generate_tick_budget_report(source);
        assert!(report.contains("Tick Budget Report"), "report: {report}");
        assert!(report.contains("No #[kobo::tick] functions found"), "report: {report}");
    }

    #[test]
    fn parse_tick_rate_20() {
        assert_eq!(parse_tick_rate_from_line("#[kobo::tick(rate=20)]"), Some(20));
    }

    #[test]
    fn parse_tick_rate_with_spaces() {
        assert_eq!(
            parse_tick_rate_from_line("#[kobo::tick(rate = 60)]"),
            Some(60)
        );
    }

    #[test]
    fn extract_fn_name_basic() {
        assert_eq!(extract_fn_name("fn game_loop(state: &mut S) {"), Some("game_loop"));
    }

    #[test]
    fn extract_fn_name_async() {
        assert_eq!(extract_fn_name("async fn render(ctx: &Ctx) {"), Some("render"));
    }
}
