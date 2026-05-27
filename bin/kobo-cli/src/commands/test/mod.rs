use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use kobo_errors::{
    diagnostic_to_json_value, ColorMode, DiagDecision, DiagLabel, DiagnosticOutputFormat,
    DiagnosticRenderer, KDiagnostic, KErrorCode, Severity,
};
use kobo_ir::{
    FileId, FileSetBuilder, GuaranteePolicy, GuaranteeProfile, KoboSpan, ScenarioOpKind,
    ScenarioProgram,
};
use kobo_parser::{parse_ward_syntax, WardItem};
use kobo_sim_core::{EngineMode, FullDepthRun, ReplayGuarantee, ScenarioEvent, ScenarioFailure};
use proptest::prelude::{any, Strategy};
use proptest::strategy::ValueTree;
use proptest::test_runner::{
    Config as ProptestConfig, RngAlgorithm, TestRng, TestRunner as ProptestRunner,
};

use crate::ErrorFormat;

use super::formal_core;
use super::sim_model::{self, ScenarioDocument};
use super::witness_evidence;
use super::{
    backend_debt::{self, DebtControlSource},
    declarations, summary_validation,
};

mod backend;
mod failure;
mod fuzz;
mod json_schema;
mod model_compare;
mod proof;
mod replay_scope;
mod run;
mod runtime_boundary;
mod seed_portfolio;
mod trace_checks;
mod witness;

pub(super) use backend::BackendExpertOptions;
use backend::{parse_engine, reserved_backend_fit_json, ProfileRoles};
use failure::emit_failure;
use fuzz::{run_fuzz_portfolio, FuzzPlan};
use json_schema::scheduler_json;
use model_compare::apply_model_vs_implementation;
use replay_scope::validate_run_boundary_declarations;
pub(super) use run::cmd_test;
use runtime_boundary::runtime_boundary_evidence;
use seed_portfolio::run_seed_portfolio;
use trace_checks::apply_trace_checks;
use witness::{print_events, write_run_witness};

pub(super) use json_schema::flagship_demo_json;
pub(super) use model_compare::model_vs_implementation_json;
pub(super) use replay_scope::{backend_replay_evidence_json, backend_replay_id};
pub(super) use trace_checks::trace_checks_json;
