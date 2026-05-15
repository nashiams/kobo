use crate::{FileId, KoboSpan};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScenarioProgram {
    pub file_id: FileId,
    pub target: String,
    pub source_hash: String,
    pub operations: Vec<ScenarioOp>,
    pub boundaries: Vec<ScenarioBoundary>,
    pub coverage: ScenarioCoverageFacts,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScenarioOp {
    pub span: KoboSpan,
    pub kind: ScenarioOpKind,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ScenarioOpKind {
    CreateObligation {
        binding: String,
        type_name: String,
        actions: Vec<String>,
    },
    Discharge {
        binding: String,
        action: String,
    },
    MoveBinding {
        binding: String,
    },
    ModeledEffect {
        boundary: ScenarioModeledBoundary,
    },
    RawNondeterminism {
        operation: String,
    },
    UncontrolledEffect {
        operation: String,
    },
    ExternalBoundary {
        crate_name: String,
    },
    Loop,
    Return,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ScenarioModeledBoundary {
    WardTime,
    WardRandom,
    WardTask,
}

impl ScenarioModeledBoundary {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::WardTime => "ward.time",
            Self::WardRandom => "ward.random",
            Self::WardTask => "ward.task",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScenarioBoundary {
    pub span: KoboSpan,
    pub name: String,
    pub decision: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScenarioCoverageFacts {
    pub unsupported_constructs: Vec<String>,
    pub opaque_boundaries: Vec<String>,
}
