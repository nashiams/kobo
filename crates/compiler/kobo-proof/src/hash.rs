use crate::ProofCertificate;

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
