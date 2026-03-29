pub use kobo_ir::BorrowKind;
pub use kobo_ir::HintConflictFact;

use kobo_ir::{KirNodeId, KoboSpan};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoveFact {
    pub binding: KirNodeId,
    pub move_site: KoboSpan,
    pub later_use: KoboSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BorrowFact {
    pub binding: KirNodeId,
    pub borrow_site: KoboSpan,
    pub conflict_site: KoboSpan,
    pub borrow_kind: BorrowKind,
}
