pub mod borrow_report;
mod complexity;
pub mod patterns;
mod report;
mod warn_early;

#[cfg(test)]
mod borrow_report_tests;

pub use complexity::classify_site;
pub use kobo_ir::debt::DebtReport;
pub use patterns::{detect_migration_patterns, format_patterns, MigrationPattern, PatternRisk};
pub use report::build_debt_report;
pub use warn_early::{format_warn_early, group_by_pattern, GroupedWarnings};
