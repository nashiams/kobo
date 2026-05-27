use kobo_ir::{FileId, NodeIdGen, ScenarioOpKind, ScenarioProgram};

use crate::diagnostics::{compiler_liveness_diagnostics, parse_recovery_diagnostics};
use crate::navigation::compiler_rust_navigation;
use crate::protocol::{ProtocolDocumentAnalysis, ProtocolHoverKind};

pub(crate) fn protocol_document_analysis(uri: &str, source: &str) -> ProtocolDocumentAnalysis {
    compiler_document_analysis(uri, source)
}

fn compiler_document_analysis(uri: &str, source: &str) -> ProtocolDocumentAnalysis {
    let mut id_gen = NodeIdGen::new();
    let file_id = FileId(0);
    let ward_mapped = kobo_parser::preprocess_ward_syntax_mapped(source, file_id);
    let has_ward_syntax = !ward_mapped.metadata.wards.is_empty();
    let preprocess_source_map = ward_mapped.source_map;
    let rewritten = ward_mapped.rewritten;
    let outcome = kobo_parser::parse_file_recovering(
        &rewritten,
        file_id,
        &mut id_gen,
        kobo_parser::RecoveryMode::Recover,
    );
    let mut diagnostics =
        parse_recovery_diagnostics(source, &outcome.diagnostics, &preprocess_source_map);
    let Some(ast) = outcome.file else {
        return ProtocolDocumentAnalysis {
            diagnostics,
            hover: if has_ward_syntax {
                ProtocolHoverKind::Ward
            } else {
                ProtocolHoverKind::RustShape
            },
            rust_navigation: None,
        };
    };
    let kir = kobo_transform::build_kir(
        &ast,
        &mut id_gen,
        kobo_transform::TransformOptions::default(),
    );
    let programs = kir.scenario_programs();
    diagnostics.extend(programs.iter().flat_map(|program| {
        compiler_liveness_diagnostics(source, program, &preprocess_source_map)
    }));
    let hover = if has_ward_syntax {
        ProtocolHoverKind::Ward
    } else if programs.iter().any(program_has_obligation) || !diagnostics.is_empty() {
        ProtocolHoverKind::MustCall
    } else {
        ProtocolHoverKind::RustShape
    };
    let rust_navigation =
        compiler_rust_navigation(uri, source, &preprocess_source_map, &ast, &kir, &programs);
    ProtocolDocumentAnalysis {
        diagnostics,
        hover,
        rust_navigation,
    }
}

fn program_has_obligation(program: &ScenarioProgram) -> bool {
    program
        .operations
        .iter()
        .any(|operation| matches!(operation.kind, ScenarioOpKind::CreateObligation { .. }))
}
