use std::path::Path;
use std::time::Instant;

use super::session::{build_session, render_diagnostics};
use kobo_driver::run_codegen_pipeline;

/// Per-function timing report for tick-loop functions.
///
/// Reads a .kobo file, finds functions annotated with `#[kobo::tick(rate=N)]`,
/// and reports estimated tick budget usage based on frame timing.
/// When compilation succeeds, uses the generated Rust body statement count
/// as a rough complexity proxy.
pub(super) fn cmd_bench_tick(file: &Path, tick_budget: bool) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(file)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {}", file.display(), e))?;

    if !tick_budget {
        println!("Use --tick-budget to enable tick budget analysis.");
        return Ok(());
    }

    // Attempt compilation to get generated Rust — used for body complexity.
    let compile_start = Instant::now();
    let compiled_source = match build_session(file, None) {
        Ok(mut session) => match run_codegen_pipeline(&mut session, file) {
            Ok(artifacts) => {
                render_diagnostics(&session);
                Some(artifacts.rs_source)
            }
            Err(()) => {
                render_diagnostics(&session);
                None
            }
        },
        Err(_) => None,
    };
    let measurement = TickBudgetMeasurement {
        pipeline_us: compile_start.elapsed().as_micros(),
        compiled_output: compiled_source.is_some(),
    };

    let report = generate_tick_budget_report_with_measurement(
        &source,
        compiled_source.as_deref(),
        Some(measurement),
    );
    println!("{report}");
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TickBudgetMeasurement {
    pub pipeline_us: u128,
    pub compiled_output: bool,
}

/// A tick-budget entry for a single function.
pub(crate) struct TickBudgetEntry {
    pub fn_name: String,
    pub rate_hz: u32,
    pub budget_ms: f64,
    /// Body statement count from compiled source (0 if unknown).
    pub body_stmts: usize,
    /// S-19: Estimated execution time in microseconds based on statement complexity.
    /// Uses weighted statement analysis instead of flat count.
    pub estimated_us: Option<f64>,
}

impl TickBudgetEntry {
    fn status(&self) -> String {
        if let Some(est_us) = self.estimated_us {
            let est_ms = est_us / 1000.0;
            let pct = (est_ms / self.budget_ms) * 100.0;
            if pct > 80.0 {
                format!(
                    "OVER BUDGET — est. {:.2}ms ({:.0}% of {:.1}ms frame)",
                    est_ms, pct, self.budget_ms
                )
            } else if pct > 50.0 {
                format!(
                    "WARNING — est. {:.2}ms ({:.0}% of {:.1}ms frame)",
                    est_ms, pct, self.budget_ms
                )
            } else {
                format!(
                    "OK — est. {:.2}ms ({:.0}% of {:.1}ms frame)",
                    est_ms, pct, self.budget_ms
                )
            }
        } else if self.body_stmts == 0 {
            "UNKNOWN (no compiled output)".to_string()
        } else if self.body_stmts > 50 {
            "OVER BUDGET (static estimate)".to_string()
        } else {
            "WITHIN BUDGET (static estimate)".to_string()
        }
    }
}

/// S-19: Estimate execution time from compiled source body.
///
/// Uses weighted statement analysis: async calls and method chains are
/// heavier than simple assignments. Returns estimated microseconds.
fn estimate_tick_time_us(compiled: &str, fn_name: &str) -> Option<f64> {
    let pattern = format!("fn {fn_name}(");
    let pos = compiled.find(&pattern)?;
    let rest = &compiled[pos..];
    let brace_pos = rest.find('{')?;
    let body_start = &rest[brace_pos + 1..];

    let mut depth = 1u32;
    let mut weight: f64 = 0.0;
    let mut line_buf = String::new();

    for ch in body_start.chars() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            '\n' if depth == 1 => {
                weight += statement_weight(&line_buf);
                line_buf.clear();
            }
            _ => line_buf.push(ch),
        }
    }
    if !line_buf.is_empty() {
        weight += statement_weight(&line_buf);
    }

    // Base cost: ~0.5us per weight unit (calibrated for release-mode tick loops).
    Some(weight * 0.5)
}

/// Heuristic weight for a single line of generated code.
fn statement_weight(line: &str) -> f64 {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with("//") {
        return 0.0;
    }
    let mut w: f64 = 1.0;
    // Async operations are heavier
    if trimmed.contains(".await") {
        w += 10.0;
    }
    // Method chains add complexity
    w += trimmed.matches('.').count() as f64 * 0.5;
    // Allocations
    if trimmed.contains("Vec::new")
        || trimmed.contains("String::new")
        || trimmed.contains("clone()")
    {
        w += 3.0;
    }
    // I/O operations
    if trimmed.contains("println!") || trimmed.contains("eprintln!") || trimmed.contains("write!") {
        w += 5.0;
    }
    w
}

/// Generate a tick budget report from source code.
///
/// If `compiled_source` is provided, count statements in each tick function's
/// body to give a rough complexity-based budget estimate.
#[cfg(test)]
pub(crate) fn generate_tick_budget_report(source: &str, compiled_source: Option<&str>) -> String {
    generate_tick_budget_report_with_measurement(source, compiled_source, None)
}

pub(crate) fn generate_tick_budget_report_with_measurement(
    source: &str,
    compiled_source: Option<&str>,
    measurement: Option<TickBudgetMeasurement>,
) -> String {
    let entries = extract_tick_functions(source, compiled_source);
    if entries.is_empty() {
        return "Tick Budget Report\n══════════════════\nNo #[kobo::tick] functions found."
            .to_string();
    }

    let mut lines = vec![
        "Tick Budget Report".to_string(),
        "══════════════════".to_string(),
    ];

    if let Some(measurement) = measurement {
        lines.push(format!(
            "measured_pipeline_ms: {:.3} compiled_output={}",
            measurement.pipeline_us as f64 / 1000.0,
            measurement.compiled_output
        ));
    }

    for entry in &entries {
        lines.push(format!(
            "  {} @ {}Hz — budget {:.1}ms — {}",
            entry.fn_name,
            entry.rate_hz,
            entry.budget_ms,
            entry.status(),
        ));
        if entry.body_stmts > 0 {
            lines.push(format!("    body: {} statements", entry.body_stmts));
        }
    }

    lines.join("\n")
}

/// Extract tick-annotated function information from source text.
fn extract_tick_functions(source: &str, compiled_source: Option<&str>) -> Vec<TickBudgetEntry> {
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
                    let body_stmts = compiled_source
                        .map(|src| count_fn_body_stmts(src, fn_name))
                        .unwrap_or(0);
                    let estimated_us =
                        compiled_source.and_then(|src| estimate_tick_time_us(src, fn_name));
                    entries.push(TickBudgetEntry {
                        fn_name: fn_name.to_string(),
                        rate_hz: rate,
                        budget_ms,
                        body_stmts,
                        estimated_us,
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

/// Count semicolons in a function body in compiled source (rough statement count proxy).
fn count_fn_body_stmts(compiled: &str, fn_name: &str) -> usize {
    // Find the function definition in compiled source.
    let pattern = format!("fn {fn_name}(");
    if let Some(pos) = compiled.find(&pattern) {
        // Find the opening brace.
        let rest = &compiled[pos..];
        if let Some(brace_pos) = rest.find('{') {
            let body_start = &rest[brace_pos + 1..];
            // Count statements by tracking braces and counting semicolons at depth 1.
            let mut depth = 1u32;
            let mut stmts = 0usize;
            for ch in body_start.chars() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    ';' if depth == 1 => stmts += 1,
                    _ => {}
                }
            }
            return stmts;
        }
    }
    0
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

        let report = generate_tick_budget_report(source, None);
        assert!(report.contains("Tick Budget Report"), "report: {report}");
        assert!(report.contains("game_loop"), "report: {report}");
        assert!(report.contains("render_loop"), "report: {report}");
        assert!(report.contains("20Hz"), "report: {report}");
        assert!(report.contains("60Hz"), "report: {report}");
        // budget for 20Hz = 50ms, 60Hz ≈ 16.7ms
        assert!(report.contains("50.0ms"), "report: {report}");
        assert!(
            report.contains("WITHIN BUDGET")
                || report.contains("OVER BUDGET")
                || report.contains("UNKNOWN"),
            "report: {report}"
        );
    }

    #[test]
    fn bench_no_tick_functions() {
        let source = "fn main() { println!(\"hello\"); }";
        let report = generate_tick_budget_report(source, None);
        assert!(report.contains("Tick Budget Report"), "report: {report}");
        assert!(
            report.contains("No #[kobo::tick] functions found"),
            "report: {report}"
        );
    }

    #[test]
    fn parse_tick_rate_20() {
        assert_eq!(
            parse_tick_rate_from_line("#[kobo::tick(rate=20)]"),
            Some(20)
        );
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
        assert_eq!(
            extract_fn_name("fn game_loop(state: &mut S) {"),
            Some("game_loop")
        );
    }

    #[test]
    fn extract_fn_name_async() {
        assert_eq!(
            extract_fn_name("async fn render(ctx: &Ctx) {"),
            Some("render")
        );
    }
}
