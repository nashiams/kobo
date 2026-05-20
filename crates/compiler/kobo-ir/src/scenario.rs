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
        #[serde(default)]
        template: Option<ScenarioLifecycleTemplate>,
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
    BranchUnresolved {
        binding: String,
    },
    UnsupportedContainer {
        binding: String,
        type_name: String,
        container: String,
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
        call_path: Option<String>,
        call_arguments: Vec<ScenarioBoundaryCallArgument>,
        return_type: Option<String>,
        call_shape: ScenarioExternalCallShape,
        policy: ScenarioBoundaryPolicy,
        reason: Option<String>,
    },
    CoreTerminator {
        kind: ScenarioCoreTerminatorKind,
        boundary: Option<String>,
        policy: Option<ScenarioBoundaryPolicy>,
        edges: Vec<String>,
    },
    Loop,
    Return,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScenarioLifecycleTemplate {
    pub id: String,
    pub kind: String,
    pub version: String,
    pub confidence: String,
    pub source: ScenarioLifecycleTemplateSource,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ScenarioLifecycleTemplateSource {
    Declaration,
    Inference,
}

impl ScenarioLifecycleTemplate {
    pub fn inferred(id: &str, kind: &str) -> Self {
        Self {
            id: id.to_owned(),
            kind: kind.to_owned(),
            version: "v0.13.0".to_owned(),
            confidence: "exact_template".to_owned(),
            source: ScenarioLifecycleTemplateSource::Inference,
        }
    }

    pub fn declared(type_name: &str) -> Self {
        Self {
            id: format!("declared_must_call:{}", type_name),
            kind: "declared_must_call".to_owned(),
            version: "v0.13.0".to_owned(),
            confidence: "declared_contract".to_owned(),
            source: ScenarioLifecycleTemplateSource::Declaration,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ScenarioCoreTerminatorKind {
    Return,
    ErrorExit,
    Panic,
    Await,
    OpaqueBoundary,
}

impl ScenarioCoreTerminatorKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Return => "return",
            Self::ErrorExit => "error_exit",
            Self::Panic => "panic",
            Self::Await => "await",
            Self::OpaqueBoundary => "opaque_boundary",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScenarioBoundaryCallArgument {
    pub index: usize,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ScenarioExternalCallShape {
    FreeFunction,
    AssociatedFunction,
    Method,
}

impl ScenarioExternalCallShape {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::FreeFunction => "free_function",
            Self::AssociatedFunction => "associated_function",
            Self::Method => "method",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ScenarioBoundaryPolicy {
    Typed,
    Model,
    Record,
    Activity,
    Stub,
    Outside,
    Opaque,
    Debt,
    Unselected,
}

impl ScenarioBoundaryPolicy {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Typed => "typed",
            Self::Model => "model",
            Self::Record => "record",
            Self::Activity => "activity",
            Self::Stub => "stub",
            Self::Outside => "outside",
            Self::Opaque => "opaque",
            Self::Debt => "debt",
            Self::Unselected => "unselected",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "typed" => Self::Typed,
            "model" => Self::Model,
            "record" => Self::Record,
            "activity" => Self::Activity,
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
    WardTaskLocal,
}

impl ScenarioModeledBoundary {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::WardTime => "ward.time",
            Self::WardRandom => "ward.random",
            Self::WardTask => "ward.task",
            Self::WardTaskLocal => "ward.task.local",
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_cfg: Option<ScenarioCoreCfgFacts>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScenarioCallGraphScc {
    pub functions: Vec<String>,
    pub is_recursive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScenarioCoreCfgFacts {
    pub blocks: Vec<ScenarioCoreCfgBlock>,
    pub edges: Vec<ScenarioCoreCfgEdge>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScenarioCoreCfgBlock {
    pub id: u32,
    pub kir_nodes: Vec<u32>,
    pub span_start: usize,
    pub span_end: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScenarioCoreCfgEdge {
    pub from: u32,
    pub to: u32,
}
