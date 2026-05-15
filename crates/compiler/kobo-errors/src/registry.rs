use std::collections::BTreeMap;

use crate::{KErrorCode, Severity};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticRegistry {
    entries: BTreeMap<&'static str, DiagnosticRegistryEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticRegistryEntry {
    pub code: KErrorCode,
    pub code_text: &'static str,
    pub slug: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    pub explain: &'static str,
    pub category: DiagnosticCategory,
    pub status: DiagnosticStatus,
    pub default_severity: Severity,
    pub severity_policy: SeverityPolicy,
    pub mode_behavior: ModeBehavior,
    pub suggestion_policy: SuggestionPolicy,
    pub machine_edit_policy: MachineEditPolicy,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DiagnosticCategory {
    Ownership,
    Performance,
    StrictBoundary,
    Async,
    Solver,
    Parser,
    RustcRemap,
    MigrationBoundary,
    Liveness,
    Nondeterminism,
    BoundaryPolicy,
    Replay,
    Simulation,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DiagnosticStatus {
    Active,
    Reserved,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SeverityPolicy {
    Always(Severity),
    OwnershipModeDependent,
    AsyncModeDependent,
    ScriptDebtCheckedWarningStrictError,
    HiddenUntilActive,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ModeBehavior {
    Unspecified,
    NoModeDependency,
    OwnershipGuaranteeProfile,
    AsyncGuaranteeProfile,
    ScriptDebtStrictError,
    ReplayBoundaryPrompt,
    ParserRecovery,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SuggestionPolicy {
    Unspecified,
    None,
    HelpOnly,
    ReviewOnly,
    BoundaryPolicy,
    MachineApplicableAllowed,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MachineEditPolicy {
    Unspecified,
    NotApplicable,
    RefuseByDefault,
    AllowedWhenSuggestionMachineApplicable,
}

impl DiagnosticCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ownership => "ownership",
            Self::Performance => "performance",
            Self::StrictBoundary => "strict-boundary",
            Self::Async => "async",
            Self::Solver => "solver",
            Self::Parser => "parser",
            Self::RustcRemap => "rustc-remap",
            Self::MigrationBoundary => "migration-boundary",
            Self::Liveness => "liveness",
            Self::Nondeterminism => "nondeterminism",
            Self::BoundaryPolicy => "boundary-policy",
            Self::Replay => "replay",
            Self::Simulation => "simulation",
        }
    }
}

impl DiagnosticStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Reserved => "reserved",
        }
    }
}

impl SeverityPolicy {
    pub const fn resolve(self, mode: kobo_ir::KoboMode) -> Option<Severity> {
        match self {
            Self::Always(severity) => Some(severity),
            Self::OwnershipModeDependent => match mode {
                kobo_ir::KoboMode::Script => None,
                kobo_ir::KoboMode::Checked => Some(Severity::Warning),
                kobo_ir::KoboMode::Strict => Some(Severity::Error),
            },
            Self::AsyncModeDependent => match mode {
                kobo_ir::KoboMode::Script | kobo_ir::KoboMode::Checked => Some(Severity::Warning),
                kobo_ir::KoboMode::Strict => Some(Severity::Error),
            },
            Self::ScriptDebtCheckedWarningStrictError => match mode {
                kobo_ir::KoboMode::Script | kobo_ir::KoboMode::Checked => Some(Severity::Warning),
                kobo_ir::KoboMode::Strict => Some(Severity::Error),
            },
            Self::HiddenUntilActive => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Always(Severity::Error) => "always-error",
            Self::Always(Severity::Warning) => "always-warning",
            Self::Always(Severity::Note) => "always-note",
            Self::OwnershipModeDependent => "ownership-mode-dependent",
            Self::AsyncModeDependent => "async-mode-dependent",
            Self::ScriptDebtCheckedWarningStrictError => "script-debt-checked-warning-strict-error",
            Self::HiddenUntilActive => "hidden-until-active",
        }
    }
}

impl ModeBehavior {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::NoModeDependency => "no mode dependency",
            Self::OwnershipGuaranteeProfile => "ownership guarantee profile",
            Self::AsyncGuaranteeProfile => "async guarantee profile",
            Self::ScriptDebtStrictError => "Script debt, Strict error",
            Self::ReplayBoundaryPrompt => "normal Rust allowed; replay requires boundary policy",
            Self::ParserRecovery => "parser recovery",
        }
    }
}

impl SuggestionPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::None => "none",
            Self::HelpOnly => "help-only",
            Self::ReviewOnly => "review-only",
            Self::BoundaryPolicy => "boundary-policy",
            Self::MachineApplicableAllowed => "machine-applicable-allowed",
        }
    }
}

impl MachineEditPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::NotApplicable => "not-applicable",
            Self::RefuseByDefault => "refuse-by-default",
            Self::AllowedWhenSuggestionMachineApplicable => {
                "allowed-when-suggestion-machine-applicable"
            }
        }
    }
}

impl DiagnosticRegistry {
    pub fn active_entries(&self) -> impl Iterator<Item = &DiagnosticRegistryEntry> {
        self.entries
            .values()
            .filter(|entry| entry.status == DiagnosticStatus::Active)
    }

    pub fn get(&self, code: KErrorCode) -> Option<&DiagnosticRegistryEntry> {
        self.find_by_code_text(code.as_str())
    }

    pub fn find_by_code_text(&self, code_text: &str) -> Option<&DiagnosticRegistryEntry> {
        let normalized = normalize_code_text(code_text);
        self.entries.get(normalized.as_str())
    }

    pub fn nearest_code_text(&self, code_text: &str) -> Option<&'static str> {
        let target = code_number(&normalize_code_text(code_text))?;
        self.entries
            .keys()
            .filter_map(|code| code_number(code).map(|number| (*code, number.abs_diff(target))))
            .min_by_key(|(_, distance)| *distance)
            .map(|(code, _)| code)
    }
}

pub fn diagnostic_registry() -> DiagnosticRegistry {
    let mut entries = BTreeMap::new();
    for entry in registry_entries() {
        entries.insert(entry.code_text, entry);
    }
    DiagnosticRegistry { entries }
}

fn registry_entries() -> Vec<DiagnosticRegistryEntry> {
    let mut entries = Vec::new();
    entries.extend(ownership_entries());
    entries.extend(performance_entries());
    entries.extend(strict_boundary_entries());
    entries.extend(async_entries());
    entries.extend(solver_entries());
    entries.extend(migration_entries());
    entries.extend(v085_entries());
    entries.extend(parser_recovery_entries());
    append_reserved_entries(&mut entries);
    entries
}

fn append_reserved_entries(entries: &mut Vec<DiagnosticRegistryEntry>) {
    for code in KErrorCode::ALL {
        if !entries.iter().any(|entry| entry.code == *code) {
            entries.push(reserved_entry(*code));
        }
    }
}

fn reserved_entry(code: KErrorCode) -> DiagnosticRegistryEntry {
    DiagnosticRegistryEntry {
        code,
        code_text: code.as_str(),
        slug: "reserved-kobo-diagnostic-slot",
        title: "reserved Kobo diagnostic slot",
        summary: "This K-code is reserved by this Kobo version and is not emitted as a user diagnostic yet.",
        explain: "Kobo keeps stable K-code ranges so future diagnostics can be added without renumbering existing errors. A reserved slot is part of that public catalog, but normal compiler output should not produce it until the slot is promoted into an active diagnostic.",
        category: reserved_category(code),
        status: DiagnosticStatus::Reserved,
        default_severity: Severity::Note,
        severity_policy: SeverityPolicy::HiddenUntilActive,
        mode_behavior: ModeBehavior::NoModeDependency,
        suggestion_policy: SuggestionPolicy::HelpOnly,
        machine_edit_policy: MachineEditPolicy::NotApplicable,
    }
}

fn reserved_category(code: KErrorCode) -> DiagnosticCategory {
    match code_number(code.as_str()).unwrap_or_default() {
        1..=19 => DiagnosticCategory::Ownership,
        20..=39 => DiagnosticCategory::Performance,
        40..=59 => DiagnosticCategory::StrictBoundary,
        60..=79 => DiagnosticCategory::Async,
        80..=89 => DiagnosticCategory::Solver,
        90..=99 => DiagnosticCategory::MigrationBoundary,
        100..=109 => DiagnosticCategory::Replay,
        110..=116 => DiagnosticCategory::Parser,
        _ => DiagnosticCategory::Simulation,
    }
}

fn ownership_entries() -> Vec<DiagnosticRegistryEntry> {
    use DiagnosticCategory::Ownership;
    use MachineEditPolicy::AllowedWhenSuggestionMachineApplicable;
    use ModeBehavior::OwnershipGuaranteeProfile;
    use Severity::Warning;
    use SeverityPolicy::OwnershipModeDependent;
    use SuggestionPolicy::MachineApplicableAllowed;

    vec![
        entry(
            KErrorCode::K0001,
            "value-used-after-move",
            "value used after move",
            "A value is used after ownership has moved away from it.",
            "The later use would need the original value, but an earlier operation already took ownership of it. Kobo reports this before lowering so the source-level move is visible.",
            Ownership,
            Warning,
            OwnershipModeDependent,
            OwnershipGuaranteeProfile,
            MachineApplicableAllowed,
            AllowedWhenSuggestionMachineApplicable,
        ),
        entry(
            KErrorCode::K0002,
            "mutable-borrow-conflict",
            "cannot borrow as mutable - already borrowed",
            "A mutable borrow conflicts with an existing borrow.",
            "The mutable access overlaps another live borrow. Kobo reports the overlap so the source can shorten one borrow or make shared mutation explicit.",
            Ownership,
            Warning,
            OwnershipModeDependent,
            OwnershipGuaranteeProfile,
            MachineApplicableAllowed,
            AllowedWhenSuggestionMachineApplicable,
        ),
        entry(
            KErrorCode::K0019,
            "ownership-diagnostic",
            "ownership diagnostic",
            "An ownership condition needs a stronger mode-dependent guarantee.",
            "K0019 is the compatibility bucket for ownership diagnostics that have not yet been assigned a narrower code.",
            Ownership,
            Warning,
            OwnershipModeDependent,
            OwnershipGuaranteeProfile,
            SuggestionPolicy::ReviewOnly,
            MachineEditPolicy::RefuseByDefault,
        ),
    ]
}

fn performance_entries() -> Vec<DiagnosticRegistryEntry> {
    use DiagnosticCategory::Performance;
    use MachineEditPolicy::NotApplicable;
    use ModeBehavior::NoModeDependency;
    use Severity::Warning;
    use SeverityPolicy::Always;
    use SuggestionPolicy::HelpOnly;

    vec![
        entry(
            KErrorCode::K0020,
            "hot-refcell-borrow-counter",
            "RefCell accessed >10,000 times in hot path",
            "A diagnostic owner recorded many dynamic borrow checks in a hot path.",
            "Kobo records runtime borrow counters so migration work can prioritize expensive shared-mutable paths.",
            Performance,
            Warning,
            Always(Warning),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0021,
            "borrow-counter-saturated",
            "DiagOwner borrow counter saturated - count understated",
            "A diagnostic owner counter reached its maximum value.",
            "The program continued, but the reported count is a lower bound because the counter saturated.",
            Performance,
            Warning,
            Always(Warning),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0026,
            "relax-attribute-ineffective",
            "relax attribute has no effect or is malformed",
            "A relax attribute is ineffective or invalid for the current context.",
            "Kobo reports relax misuse so mode boundaries stay explicit and reviewable.",
            Performance,
            Warning,
            Always(Warning),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0031,
            "engine-owned-capped",
            "engine-owned binding capped to PlainOwned",
            "A framework-managed engine binding was prevented from escalating to shared ownership.",
            "Engine-managed state must stay owned by the framework unless the source explicitly opts into a supported escape path.",
            Performance,
            Warning,
            Always(Warning),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0032,
            "live-borrow-at-move",
            "live borrow at move forces shared ownership",
            "A borrow remains live when a value moves.",
            "Kobo uses KIR liveness to keep this move from invalidating a later borrow use.",
            Performance,
            Warning,
            Always(Warning),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
    ]
}

fn strict_boundary_entries() -> Vec<DiagnosticRegistryEntry> {
    use DiagnosticCategory::StrictBoundary;
    use MachineEditPolicy::RefuseByDefault;
    use ModeBehavior::NoModeDependency;
    use Severity::Error;
    use SeverityPolicy::Always;
    use SuggestionPolicy::ReviewOnly;

    vec![
        entry(
            KErrorCode::K0025,
            "ownership-hint-cannot-be-used",
            "Kobo cannot use this ownership hint",
            "A hint asks Kobo for an ownership shape this value's actual use cannot support.",
            "A moved value has only one owner. Code that needs repeated mutable use must use a shape that supports that sharing.",
            StrictBoundary,
            Error,
            Always(Error),
            NoModeDependency,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0030,
            "resource-handle-moved",
            "resource handle moved - cannot alias file handle",
            "A resource handle cannot be safely aliased through ownership wrapping.",
            "Kobo keeps resource ownership explicit because duplicating or aliasing OS-backed handles can change behavior.",
            StrictBoundary,
            Error,
            Always(Error),
            NoModeDependency,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0041,
            "aliased-strict-entry",
            "cannot enter @strict block - value has active aliases",
            "An @strict block would start while aliases are still active.",
            "Strict regions promise a simple ownership shape at the boundary. Any live alias must end before entry so the generated guards can be introduced and removed in one clear scope.",
            StrictBoundary,
            Error,
            Always(Error),
            NoModeDependency,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0042,
            "closure-crosses-strict-boundary",
            "closure captures value across @strict boundary",
            "A closure captures a value whose strict-boundary guard must stay local to the block.",
            "The closure could run after the strict boundary has finished. Kobo rejects that capture so guard lifetimes remain visible in source.",
            StrictBoundary,
            Error,
            Always(Error),
            NoModeDependency,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0043,
            "moved-inside-strict-block",
            "value moved inside @strict block - cannot restore on exit",
            "A value moved inside a strict block cannot be restored to its wrapper on exit.",
            "Kobo keeps strict boundary exit behavior explicit so generated ownership state remains coherent.",
            StrictBoundary,
            Error,
            Always(Error),
            NoModeDependency,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0044,
            "labeled-jump-crosses-strict-boundary",
            "labeled break or continue crosses @strict boundary",
            "A labeled control-flow jump would exit an @strict region unsafely.",
            "Kobo rejects labeled jumps that bypass strict-region guard teardown.",
            StrictBoundary,
            Error,
            Always(Error),
            NoModeDependency,
            ReviewOnly,
            RefuseByDefault,
        ),
    ]
}

fn async_entries() -> Vec<DiagnosticRegistryEntry> {
    use DiagnosticCategory::Async;
    use MachineEditPolicy::RefuseByDefault;
    use ModeBehavior::AsyncGuaranteeProfile;
    use Severity::Warning;
    use SeverityPolicy::AsyncModeDependent;
    use SuggestionPolicy::ReviewOnly;

    vec![
        entry(
            KErrorCode::K0060,
            "refcell-borrow-live-at-await",
            "RefCell borrow is live at suspend point",
            "A non-Send borrow would remain live across async suspension.",
            "Kobo reports async ownership hazards before lowering code that would fail executor requirements.",
            Async,
            Warning,
            AsyncModeDependent,
            AsyncGuaranteeProfile,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0061,
            "future-send-boundary",
            "future requires Send but value cannot safely cross thread boundary",
            "An async future requires Send but a captured value cannot satisfy that boundary.",
            "The future may run on another worker thread, but one captured value is only safe on the current thread.\n\
Kobo does not hide that by inserting shared mutation for you.\n\
Use LocalSet when the task is intentionally single-thread local.\n\
Use #[kobo::async_shared] when shared async ownership is intentional.\n\
Capture only thread-safe data when the task really should move between worker threads.\n\
If mutation crosses many tasks, prefer message passing or actor ownership.\n\
If this crosses an external runtime boundary, record the boundary tradeoff explicitly.",
            Async,
            Warning,
            AsyncModeDependent,
            AsyncGuaranteeProfile,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0062,
            "mutex-guard-across-await",
            "async executor dependency is missing",
            "Async code was found, but Kobo could not find an executor dependency.",
            "Kobo needs an executor dependency such as tokio or async-std before it can lower async entry points with a concrete runtime.",
            Async,
            Warning,
            AsyncModeDependent,
            AsyncGuaranteeProfile,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0063,
            "strict-block-inside-async",
            "this strict borrow is inside code that can pause",
            "This strict block runs inside an async function.",
            "Kobo needs strict borrows to end before the function can pause or be cancelled.",
            Async,
            Warning,
            AsyncModeDependent,
            AsyncGuaranteeProfile,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0064,
            "strict-guard-live-across-yield",
            "@strict inside async block - ownership cannot be tracked across yield",
            "A guard-like value is live across an await point.",
            "Kobo reports guard liveness because it can produce deadlocks or non-Send futures.",
            Async,
            Warning,
            AsyncModeDependent,
            AsyncGuaranteeProfile,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0065,
            "select-branch-cancel-safety",
            "select branch may not be cancel-safe",
            "A select branch contains an operation that may lose progress if cancelled.",
            "Kobo flags non-cancel-safe operations so async control flow stays reviewable before stronger simulation checks arrive.",
            Async,
            Warning,
            AsyncModeDependent,
            AsyncGuaranteeProfile,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0067,
            "handler-state-leaks-across-task",
            "handler request-state leaks across async boundary",
            "A handler-local request value is captured by a longer-lived async task.",
            "Request state should not outlive its request unless it is cloned, modeled, or moved into an explicit actor.",
            Async,
            Warning,
            AsyncModeDependent,
            AsyncGuaranteeProfile,
            ReviewOnly,
            RefuseByDefault,
        ),
    ]
}

fn solver_entries() -> Vec<DiagnosticRegistryEntry> {
    use DiagnosticCategory::Solver;
    use MachineEditPolicy::NotApplicable;
    use ModeBehavior::NoModeDependency;
    use Severity::{Error, Note, Warning};
    use SeverityPolicy::Always;
    use SuggestionPolicy::{HelpOnly, ReviewOnly};

    vec![
        entry(
            KErrorCode::K0080,
            "structural-ownership-conflict",
            "structural ownership conflict - no automatic fix possible",
            "Kobo found an ownership shape that needs a human design choice.",
            "More than one architecture could be correct, or the current shape needs a design boundary. Kobo reports the conflict instead of fabricating a silent rewrite.",
            Solver,
            Note,
            Always(Note),
            NoModeDependency,
            ReviewOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0080P1,
            "migration-architecture-decision",
            "ownership pattern will require architectural decision at migration",
            "A precursor ownership pattern is likely to need human migration design.",
            "Kobo reports this as advisory debt so it can be handled before stricter migration gates.",
            Solver,
            Note,
            Always(Note),
            NoModeDependency,
            ReviewOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0080P2,
            "rc-back-pointer-cycle-risk",
            "parent-child Rc back-pointer tree - cycle risk",
            "Parent and child links can create an Rc cycle.",
            "Use Weak links or a different ownership shape when migrating this structure.",
            Solver,
            Note,
            Always(Note),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0080P3,
            "shared-mutable-callsite-hotspot",
            "shared mutable state at 3+ call sites",
            "A value is mutated through several call sites and may need a design decision.",
            "Kobo tracks this as migration debt because automatic wrapping may hide an architectural choice.",
            Solver,
            Note,
            Always(Note),
            NoModeDependency,
            ReviewOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0080P4,
            "self-referential-struct",
            "self-referential struct - infinite size without indirection",
            "A structure refers to itself without an indirection boundary.",
            "Kobo reports this early because Rust requires an indirection such as Box, Rc, or a redesigned structure.",
            Solver,
            Note,
            Always(Note),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0081,
            "ownership-cluster-too-large",
            "ownership problem is too large to choose automatically",
            "Too many ownership choices are connected in one part of the code.",
            "Kobo stops instead of making a silent partial decision. Split the code or add a local ownership annotation so the intended owner is clear.",
            Solver,
            Error,
            Always(Error),
            NoModeDependency,
            ReviewOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0082,
            "ownership-budget-exceeded",
            "ownership analysis took too long",
            "Kobo ran out of budget before reaching a complete answer.",
            "Budget exhaustion is distinct from no-solution so users can decide whether to increase budget or simplify the code.",
            Solver,
            Error,
            Always(Error),
            NoModeDependency,
            ReviewOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0083,
            "ownership-choice-needs-review",
            "ownership choice requires human review",
            "Kobo found more than one valid ownership candidate.",
            "Kobo may choose a lowest-risk candidate for codegen, but review output must expose the alternatives.",
            Solver,
            Warning,
            Always(Warning),
            NoModeDependency,
            ReviewOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0084,
            "ownership-medium-confidence",
            "ownership choice needs confirmation",
            "Kobo found an ownership choice that is plausible but not strong enough to hide from review.",
            "Kobo reports medium-confidence choices so migration remains auditable.",
            Solver,
            Warning,
            Always(Warning),
            NoModeDependency,
            ReviewOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0085,
            "ownership-low-confidence",
            "ownership suggestion needs manual review",
            "A low-confidence ownership choice requires human review.",
            "Kobo keeps low-confidence picks visible rather than treating them as final migration decisions.",
            Solver,
            Warning,
            Always(Warning),
            NoModeDependency,
            ReviewOnly,
            NotApplicable,
        ),
    ]
}

fn migration_entries() -> Vec<DiagnosticRegistryEntry> {
    use DiagnosticCategory::{MigrationBoundary, RustcRemap};
    use MachineEditPolicy::{NotApplicable, RefuseByDefault};
    use ModeBehavior::NoModeDependency;
    use Severity::Error;
    use SeverityPolicy::Always;
    use SuggestionPolicy::{HelpOnly, ReviewOnly};

    vec![
        entry(
            KErrorCode::K0090,
            "external-crate-migration-boundary",
            "migration cannot continue - value crosses into external crate",
            "Ownership depends on a boundary Kobo cannot currently solve.",
            "External crate boundaries require an explicit ownership hint, summary, or deferral instead of silent inference.",
            MigrationBoundary,
            Error,
            Always(Error),
            NoModeDependency,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0095,
            "macro-generated-ownership-unknown",
            "ownership of macro-generated value cannot be inferred",
            "A macro-generated value lacks enough source structure for ownership inference.",
            "Kobo reports macro boundaries explicitly because generated ownership facts may not map cleanly back to source.",
            MigrationBoundary,
            Error,
            Always(Error),
            NoModeDependency,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0099,
            "rustc-remapped",
            "rustc error remapped to Kobo source",
            "Rust reported an error in generated code and Kobo mapped it back to the original source.",
            "Kobo users should not need to inspect generated Rust for routine errors. K0099 is the bridge diagnostic when rustc remains the source of truth.",
            RustcRemap,
            Error,
            Always(Error),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
    ]
}

fn v085_entries() -> Vec<DiagnosticRegistryEntry> {
    use DiagnosticCategory::{BoundaryPolicy, Liveness, Nondeterminism, Replay, Simulation};
    use MachineEditPolicy::{NotApplicable, RefuseByDefault};
    use ModeBehavior::{NoModeDependency, ReplayBoundaryPrompt, ScriptDebtStrictError};
    use Severity::{Error, Warning};
    use SeverityPolicy::{Always, ScriptDebtCheckedWarningStrictError};
    use SuggestionPolicy::{BoundaryPolicy as BoundarySuggestion, HelpOnly, ReviewOnly};

    vec![
        entry(
            KErrorCode::K0100,
            "checked-runtime-liveness-token-dropped",
            "checked scenario dropped an unresolved liveness token",
            "A checked-profile scenario dropped a runtime liveness token before ack, nack, requeue, commit, rollback, or another required action.",
            "Checked simulation records liveness obligations at runtime. A token cannot disappear without one of its required actions or an explicit debt record.",
            Liveness,
            Warning,
            ScriptDebtCheckedWarningStrictError,
            ScriptDebtStrictError,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0101,
            "liveness-obligation-escaped",
            "liveness obligation escapes local analysis",
            "A must_call value escapes through a return, task, store, or external call where local analysis cannot prove the required action happens.",
            "Kobo keeps this as reviewable debt because the obligation may be handled elsewhere, but the local function no longer proves it.",
            Liveness,
            Warning,
            ScriptDebtCheckedWarningStrictError,
            ScriptDebtStrictError,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0102,
            "raw-nondeterminism-on-replay-path",
            "raw nondeterminism appears on a replay path",
            "A replay-critical path uses raw nondeterminism such as time, random, spawn, filesystem, network, process, or external I/O.",
            "Exact replay needs the same event stream each time. Raw nondeterminism must be modeled, recorded, or acknowledged as replay debt.",
            Nondeterminism,
            Warning,
            Always(Warning),
            ScriptDebtStrictError,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0103,
            "uncontrolled-effect-blocks-replay",
            "scenario cannot replay an uncontrolled effect",
            "A scenario reached an uncontrolled effect that was neither modeled nor recorded, so Kobo cannot replay the failure exactly.",
            "Add a deterministic model, record the effect stream, choose a boundary policy, or mark this scenario as partial replay debt.",
            Replay,
            Error,
            Always(Error),
            ReplayBoundaryPrompt,
            BoundarySuggestion,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0104,
            "kwit-replay-diverged",
            ".kwit replay diverged from recorded history",
            "Replay observed an event stream that diverged from the history recorded in the .kwit witness.",
            "Compare the expected and observed events, verify source identity, and regenerate the witness after intentional source or boundary changes.",
            Replay,
            Error,
            Always(Error),
            NoModeDependency,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0105,
            "sim-quick-budget-exceeded",
            "scenario exceeded quick-profile budget",
            "A deterministic quick-profile scenario exceeded its configured tick or event budget.",
            "Increase the simulation profile budget, reduce the scenario, or run a deeper profile when the longer exploration is intentional.",
            Simulation,
            Error,
            Always(Error),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0114,
            "malformed-must-call-attribute",
            "malformed must_call attribute",
            "A must_call obligation attribute could not be parsed into one or more named actions.",
            "Use `#[kobo::must_call(commit | rollback)]` with action names separated by a single `|`.",
            Liveness,
            Error,
            Always(Error),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0115,
            "invalid-kwit-witness-schema",
            "invalid kwit witness schema",
            "A .kwit witness is missing required metadata or uses an unsupported schema version.",
            "Legacy schema_version 0 witnesses validate metadata only; v0.9 exact replay requires schema_version 1 event history and uses K0104 for divergence.",
            BoundaryPolicy,
            Error,
            Always(Error),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0116,
            "malformed-scenario-metadata",
            "malformed scenario metadata",
            "A scenario attribute is missing required metadata such as its stable scenario name.",
            "Use `#[kobo::scenario(name = \"case_name\")]` to index metadata without changing runtime behavior.",
            Nondeterminism,
            Error,
            Always(Error),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0106,
            "witness-shrink-unsafe",
            "witness shrink is unsafe",
            "A requested witness shrink would remove replay evidence needed to preserve deterministic or boundary obligations.",
            "Keep the witness evidence or regenerate a smaller witness through a checked shrink pass that preserves required replay and boundary facts.",
            BoundaryPolicy,
            Error,
            Always(Error),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0109,
            "invalid-field-capability-view",
            "invalid field capability view",
            "A using field capability list names duplicate or unavailable fields.",
            "Kobo validates field capability views before lowering so a `using { ... }` list cannot silently claim access to a field that does not exist.",
            BoundaryPolicy,
            Error,
            Always(Error),
            NoModeDependency,
            HelpOnly,
            NotApplicable,
        ),
        entry(
            KErrorCode::K0107,
            "unmodeled-external-boundary",
            "unmodeled external crate boundary",
            "An external crate boundary is usable for normal Rust compatibility but blocks exact replay until a boundary policy is chosen.",
            "normal Rust crates remain allowed in Kobo. Exact replay cannot cross this boundary until you choose one policy: model, record, outside, opaque, or debt.",
            BoundaryPolicy,
            Warning,
            Always(Warning),
            ReplayBoundaryPrompt,
            BoundarySuggestion,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0108,
            "replay-obligation-suppressed",
            "replay obligation suppressed",
            "A replay, liveness, or boundary obligation was suppressed with a recorded reason.",
            "Kobo keeps reason-bearing suppression as reviewable evidence instead of pretending the boundary or obligation was modeled.",
            BoundaryPolicy,
            Warning,
            Always(Warning),
            ReplayBoundaryPrompt,
            BoundarySuggestion,
            RefuseByDefault,
        ),
    ]
}

fn parser_recovery_entries() -> Vec<DiagnosticRegistryEntry> {
    use DiagnosticCategory::Parser;
    use MachineEditPolicy::RefuseByDefault;
    use ModeBehavior::ParserRecovery;
    use Severity::Error;
    use SeverityPolicy::Always;
    use SuggestionPolicy::ReviewOnly;

    vec![
        entry(
            KErrorCode::K0110,
            "syntax-error-recovered",
            "syntax error recovered",
            "Kobo recovered from invalid syntax and continued compiling the remaining trustworthy source regions.",
            "Kobo keeps parsing after localized syntax errors so it can report other independent diagnostics. The broken source range is skipped by later compiler phases to avoid cascades.",
            Parser,
            Error,
            Always(Error),
            ParserRecovery,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0111,
            "unclosed-delimiter",
            "unclosed delimiter",
            "A delimiter was opened but not closed before the parser reached a synchronization boundary.",
            "Kobo resumes at the next safe item or statement boundary so one missing delimiter does not hide unrelated diagnostics.",
            Parser,
            Error,
            Always(Error),
            ParserRecovery,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0112,
            "invalid-item-skipped",
            "invalid item skipped",
            "Kobo skipped an invalid item while preserving later parseable items.",
            "The skipped item is not analyzed further. Later phases operate only on parseable source regions so follow-on diagnostics stay trustworthy.",
            Parser,
            Error,
            Always(Error),
            ParserRecovery,
            ReviewOnly,
            RefuseByDefault,
        ),
        entry(
            KErrorCode::K0113,
            "parser-recovery-limit-reached",
            "parser recovery limit reached",
            "Kobo stopped recovery after too many syntax errors to avoid misleading cascades.",
            "Fix the first reported syntax errors and rerun Kobo. The parser intentionally stops after the recovery budget is exhausted.",
            Parser,
            Error,
            Always(Error),
            ParserRecovery,
            ReviewOnly,
            RefuseByDefault,
        ),
    ]
}

fn entry(
    code: KErrorCode,
    slug: &'static str,
    title: &'static str,
    summary: &'static str,
    explain: &'static str,
    category: DiagnosticCategory,
    default_severity: Severity,
    severity_policy: SeverityPolicy,
    mode_behavior: ModeBehavior,
    suggestion_policy: SuggestionPolicy,
    machine_edit_policy: MachineEditPolicy,
) -> DiagnosticRegistryEntry {
    DiagnosticRegistryEntry {
        code,
        code_text: code.as_str(),
        slug,
        title,
        summary,
        explain,
        category,
        status: DiagnosticStatus::Active,
        default_severity,
        severity_policy,
        mode_behavior,
        suggestion_policy,
        machine_edit_policy,
    }
}

fn normalize_code_text(code_text: &str) -> String {
    code_text.trim().to_ascii_uppercase()
}

fn code_number(code_text: &str) -> Option<u32> {
    let digits = code_text
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    digits.parse().ok()
}
