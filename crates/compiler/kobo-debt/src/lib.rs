mod complexity;
mod report;
mod warn_early;

pub use complexity::classify_site;
pub use kobo_ir::debt::DebtReport;
pub use report::build_debt_report;
pub use warn_early::{format_warn_early, group_by_pattern, GroupedWarnings};
