use crate::{
    AsyncModelEvidence, CoreCfgEdge, CoreCfgNode, ProofCertificate, TemplateSchemaEvidence,
};

pub fn stable_hash(material: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in material.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

pub fn certificate_material_hash(
    certificate: &ProofCertificate,
) -> Result<String, serde_json::Error> {
    let mut material = certificate.clone();
    material.certificate_material_hash.clear();
    serde_json::to_string(&material).map(|source| stable_hash(&source))
}

pub fn core_material_hash(
    core_version: &str,
    cfg_nodes: &[CoreCfgNode],
    cfg_edges: &[CoreCfgEdge],
    async_model: &AsyncModelEvidence,
) -> Result<String, serde_json::Error> {
    let material = serde_json::json!({
        "version": core_version,
        "cfg_nodes": cfg_nodes,
        "cfg_edges": cfg_edges,
        "async_model": async_model,
    });
    serde_json::to_string(&material).map(|source| stable_hash(&source))
}

pub fn template_schema_hash(
    template: &TemplateSchemaEvidence,
) -> Result<String, serde_json::Error> {
    let material = serde_json::json!({
        "id": &template.id,
        "kind": &template.kind,
        "template_schema": &template.template_schema,
        "schema_version": template.schema_version,
        "confidence": &template.confidence,
        "source": &template.source,
    });
    serde_json::to_string(&material).map(|source| stable_hash(&source))
}
