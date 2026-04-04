use kobo_ir::{KirNodeId, KoboSpan, OwnershipTier};

#[derive(Clone, Debug)]
pub struct LoweringSite {
    pub node: KirNodeId,
    pub binding_name: String,
    pub ownership_tier: OwnershipTier,
    pub kobo_span: KoboSpan,
    pub kobo_line: usize,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct AnnotationNote {
    pub node: KirNodeId,
    pub binding_name: String,
    pub kobo_line: usize,
    pub reason: String,
}

impl LoweringSite {
    pub(crate) fn display_name(&self) -> &str {
        &self.binding_name
    }

    #[cfg(test)]
    pub(crate) fn new(
        node: KirNodeId,
        binding_name: impl Into<String>,
        ownership_tier: OwnershipTier,
        kobo_span: KoboSpan,
        kobo_line: usize,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            node,
            binding_name: binding_name.into(),
            ownership_tier,
            kobo_span,
            kobo_line,
            reason: reason.into(),
        }
    }
}

impl AnnotationNote {
    pub(crate) fn display_name(&self) -> &str {
        &self.binding_name
    }

    #[cfg(test)]
    pub(crate) fn new(
        node: KirNodeId,
        binding_name: impl Into<String>,
        kobo_line: usize,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            node,
            binding_name: binding_name.into(),
            kobo_line,
            reason: reason.into(),
        }
    }
}
