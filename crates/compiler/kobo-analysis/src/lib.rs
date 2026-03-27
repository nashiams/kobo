mod debug;
mod diagnostics;
mod liveness;
mod ownership_facts;
mod passes;
mod runner;

pub use diagnostics::facts_to_diagnostics;
pub use ownership_facts::{BorrowFact, BorrowKind, MoveFact};
pub use runner::{run_analysis, AnalysisFacts};
