use crate::node_id::KirNodeId;
use crate::span::KoboSpan;

/// Whether a dead-original assignment can move instead of cloning.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum CloneElisionDecision {
    Move,
    Clone,
}

/// A syntactic `let y = x` site that is a candidate for move elision.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct CloneElisionCandidate {
    pub source: KirNodeId,
    pub alias: KirNodeId,
    pub move_span: KoboSpan,
}

/// Conservative reasons for refusing clone elision.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum ElisionFallbackReason {
    MoveSafetyCheckFailed,
}

/// Reasons a plain clone/elision shortcut was skipped at an alias site.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum ElisionSkipReason {
    FieldTypeUnknownMayAllocate,
}

impl CloneElisionDecision {
    /// Returns whether this decision allows moving instead of cloning.
    pub fn is_move(self) -> bool {
        matches!(self, CloneElisionDecision::Move)
    }
}

impl ElisionSkipReason {
    /// Renders the user-facing annotation text for this skip reason.
    pub fn annotation_text(self) -> &'static str {
        match self {
            ElisionSkipReason::FieldTypeUnknownMayAllocate => {
                "clone-elision skipped: field type unknown, may allocate"
            }
        }
    }
}
