//! Product-level library surface for Rebecca.
//!
//! This crate gives embedders a small stable namespace while the implementation
//! remains split across focused crates. Internal modules live in
//! `rebecca-core`, `rebecca-rules`, and `rebecca-windows`.

pub mod prelude {
    //! Common Rebecca types for library callers.

    pub use rebecca_core::{
        CleanupPlan, CleanupSummary, CleanupTarget, CleanupWorkflow, DeleteMode, PlanRequest,
        Platform, RebeccaError, Result, RuleDefinition, RuleSelection, SafetyLevel, TargetStatus,
    };

    #[cfg(feature = "rules")]
    pub use rebecca_rules::{
        builtin_rules, builtin_safety_catalog, builtin_safety_knowledge,
        builtin_safety_knowledge_for_platform,
    };
}

pub use rebecca_core::{
    CleanupPlan, CleanupSummary, CleanupTarget, CleanupWorkflow, DeleteMode, PlanRequest, Platform,
    RebeccaError, Result, RuleDefinition, RuleSelection, SafetyLevel, TargetStatus,
};

#[cfg(feature = "rules")]
pub use rebecca_rules::{
    builtin_rules, builtin_safety_catalog, builtin_safety_knowledge,
    builtin_safety_knowledge_for_platform,
};

#[cfg(test)]
mod tests {
    #[test]
    fn facade_exposes_builtin_rules() {
        let rules = crate::builtin_rules().expect("built-in rules should load");
        assert!(!rules.is_empty());
    }

    #[test]
    fn facade_exposes_platform_safety_knowledge() {
        let knowledge = crate::builtin_safety_knowledge_for_platform(crate::Platform::Linux)
            .expect("Linux safety knowledge should load");

        assert_eq!(knowledge.platform(), crate::Platform::Linux);
    }
}
