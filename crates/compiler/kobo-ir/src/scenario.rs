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
        #[serde(default = "default_transfer_proven")]
        proven: bool,
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
    LoopStart {
        loop_id: String,
        label: Option<String>,
    },
    LoopBackEdge {
        loop_id: String,
        can_exit: bool,
    },
    LoopContinue {
        loop_id: String,
    },
    LoopBreak {
        loop_id: String,
    },
    Loop,
    Return,
}

const fn default_transfer_proven() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScenarioLifecycleTemplate {
    pub id: String,
    pub kind: String,
    pub template_schema: String,
    pub schema_version: u64,
    pub confidence: String,
    pub source: ScenarioLifecycleTemplateSource,
    pub lifecycle_owner: String,
    pub cancel_policy: String,
    pub registry_source: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ScenarioLifecycleTemplateSource {
    Declaration,
    Inference,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtocolTemplateDefinition {
    pub id: &'static str,
    pub kind: &'static str,
    pub obligation_kind: &'static str,
    pub create_methods: &'static [&'static str],
    pub terminal_actions: &'static [&'static str],
    pub lifecycle_owner: &'static str,
    pub cancel_policy: &'static str,
    pub schema_version: u64,
    pub confidence: &'static str,
    pub registry_source: &'static str,
}

pub struct ProtocolTemplateRegistry;

impl ScenarioLifecycleTemplate {
    pub fn inferred(id: &str, kind: &str) -> Self {
        if let Some(definition) = ProtocolTemplateRegistry::find(id) {
            return Self::from_protocol_definition(definition);
        }
        Self {
            id: id.to_owned(),
            kind: kind.to_owned(),
            template_schema: "lifecycle-template".to_owned(),
            schema_version: 1,
            confidence: "exact_template".to_owned(),
            source: ScenarioLifecycleTemplateSource::Inference,
            lifecycle_owner: kind.to_owned(),
            cancel_policy: "explicit_terminal_action".to_owned(),
            registry_source: "inferred_fallback".to_owned(),
        }
    }

    pub fn declared(type_name: &str) -> Self {
        Self {
            id: format!("declared_must_call:{}", type_name),
            kind: "declared_must_call".to_owned(),
            template_schema: "lifecycle-template".to_owned(),
            schema_version: 1,
            confidence: "declared_contract".to_owned(),
            source: ScenarioLifecycleTemplateSource::Declaration,
            lifecycle_owner: type_name.to_owned(),
            cancel_policy: "declared_terminal_action".to_owned(),
            registry_source: "declaration".to_owned(),
        }
    }

    pub fn from_protocol_definition(definition: ProtocolTemplateDefinition) -> Self {
        Self {
            id: definition.id.to_owned(),
            kind: definition.kind.to_owned(),
            template_schema: "lifecycle-template".to_owned(),
            schema_version: definition.schema_version,
            confidence: definition.confidence.to_owned(),
            source: ScenarioLifecycleTemplateSource::Inference,
            lifecycle_owner: definition.lifecycle_owner.to_owned(),
            cancel_policy: definition.cancel_policy.to_owned(),
            registry_source: definition.registry_source.to_owned(),
        }
    }
}

impl ProtocolTemplateDefinition {
    pub fn has_create_method(&self, method_name: &str) -> bool {
        self.create_methods
            .iter()
            .any(|candidate| *candidate == method_name)
    }

    pub fn terminal_action_strings(&self) -> Vec<String> {
        self.terminal_actions
            .iter()
            .map(|action| (*action).to_owned())
            .collect()
    }
}

impl ProtocolTemplateRegistry {
    pub fn builtin_templates() -> &'static [ProtocolTemplateDefinition] {
        &[
            ProtocolTemplateDefinition {
                id: "queue_delivery",
                kind: "queue_delivery",
                obligation_kind: "Delivery",
                create_methods: &["recv"],
                terminal_actions: &["ack", "nack", "requeue"],
                lifecycle_owner: "queue",
                cancel_policy: "terminal_action_or_requeue",
                schema_version: 1,
                confidence: "exact_template",
                registry_source: "builtin_protocol_registry",
            },
            ProtocolTemplateDefinition {
                id: "transaction",
                kind: "transaction",
                obligation_kind: "Transaction",
                create_methods: &["begin"],
                terminal_actions: &["commit", "rollback"],
                lifecycle_owner: "transaction_manager",
                cancel_policy: "commit_or_rollback",
                schema_version: 1,
                confidence: "exact_template",
                registry_source: "builtin_protocol_registry",
            },
            ProtocolTemplateDefinition {
                id: "stream_item",
                kind: "stream_item",
                obligation_kind: "StreamItem",
                create_methods: &["next", "poll_next"],
                terminal_actions: &["consume", "skip"],
                lifecycle_owner: "stream",
                cancel_policy: "consume_or_skip",
                schema_version: 1,
                confidence: "exact_template",
                registry_source: "builtin_protocol_registry",
            },
            ProtocolTemplateDefinition {
                id: "retry_attempt",
                kind: "retry_attempt",
                obligation_kind: "RetryAttempt",
                create_methods: &["attempt", "next_attempt"],
                terminal_actions: &["succeed", "retry", "give_up"],
                lifecycle_owner: "retry_policy",
                cancel_policy: "succeed_retry_or_give_up",
                schema_version: 1,
                confidence: "exact_template",
                registry_source: "builtin_protocol_registry",
            },
            ProtocolTemplateDefinition {
                id: "handler_reply",
                kind: "handler_reply",
                obligation_kind: "HandlerReply",
                create_methods: &["request"],
                terminal_actions: &["reply", "reject", "cancel"],
                lifecycle_owner: "service_request",
                cancel_policy: "reply_reject_or_cancel",
                schema_version: 1,
                confidence: "exact_template",
                registry_source: "builtin_protocol_registry",
            },
            ProtocolTemplateDefinition {
                id: "spawned_task",
                kind: "spawned_task",
                obligation_kind: "SpawnedTask",
                create_methods: &["spawn"],
                terminal_actions: &["await", "abort", "detach-with-policy"],
                lifecycle_owner: "task_runtime",
                cancel_policy: "await_abort_or_detach",
                schema_version: 1,
                confidence: "exact_template",
                registry_source: "builtin_protocol_registry",
            },
            ProtocolTemplateDefinition {
                id: "lock_permit",
                kind: "lock_permit",
                obligation_kind: "LockPermit",
                create_methods: &["acquire", "lock", "try_acquire"],
                terminal_actions: &["release", "drop-at-safe-boundary"],
                lifecycle_owner: "lock",
                cancel_policy: "release_or_safe_drop",
                schema_version: 1,
                confidence: "exact_template",
                registry_source: "builtin_protocol_registry",
            },
            ProtocolTemplateDefinition {
                id: "file_socket",
                kind: "file_socket",
                obligation_kind: "FileSocket",
                create_methods: &["open", "connect", "accept"],
                terminal_actions: &["close", "transfer", "opaque-boundary"],
                lifecycle_owner: "io_resource",
                cancel_policy: "close_transfer_or_opaque_boundary",
                schema_version: 1,
                confidence: "exact_template",
                registry_source: "builtin_protocol_registry",
            },
        ]
    }

    pub fn find(id: &str) -> Option<ProtocolTemplateDefinition> {
        Self::builtin_templates()
            .iter()
            .copied()
            .find(|definition| definition.id == id)
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
