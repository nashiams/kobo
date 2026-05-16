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
    Transfer {
        binding: String,
        callee: String,
    },
    MoveBinding {
        binding: String,
    },
    ModeledEffect {
        boundary: ScenarioModeledBoundary,
    },
    StorageEvent {
        action: String,
    },
    NetworkEvent {
        action: String,
    },
    Select {
        branch_count: u32,
    },
    RawNondeterminism {
        operation: String,
    },
    UncontrolledEffect {
        operation: String,
    },
    ExternalBoundary {
        crate_name: String,
        policy: ScenarioBoundaryPolicy,
        reason: Option<String>,
    },
    Loop,
    Return,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ScenarioBoundaryPolicy {
    Model,
    Record,
    Stub,
    Outside,
    Opaque,
    Debt,
    Unselected,
}

impl ScenarioBoundaryPolicy {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Record => "record",
            Self::Stub => "stub",
            Self::Outside => "outside",
            Self::Opaque => "opaque",
            Self::Debt => "debt",
            Self::Unselected => "unselected",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "model" => Self::Model,
            "record" => Self::Record,
            "stub" => Self::Stub,
            "outside" => Self::Outside,
            "opaque" => Self::Opaque,
            "debt" => Self::Debt,
            _ => Self::Unselected,
        }
    }
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
    #[serde(default)]
    pub call_graph_sccs: Vec<ScenarioCallGraphScc>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScenarioCallGraphScc {
    pub functions: Vec<String>,
    pub is_recursive: bool,
}
