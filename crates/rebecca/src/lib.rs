//! Product-level library surface for Rebecca.
//!
//! This crate gives embedders one stable namespace while the implementation
//! remains split across focused crates.

pub mod core {
    //! Core planning, safety, scanning, configuration, and history types.

    pub use rebecca_core::*;
}

#[cfg(feature = "rules")]
pub mod rules {
    //! Built-in Rebecca cleanup rules.

    pub use rebecca_rules::*;
}

#[cfg(feature = "windows")]
pub mod windows {
    //! Windows-specific adapters for discovery and cleanup execution.

    pub use rebecca_windows::*;
}

pub mod prelude {
    //! Common Rebecca types for library callers.

    pub use rebecca_core::{
        CleanupPlan, CleanupSummary, CleanupTarget, CleanupWorkflow, DeleteMode, PlanRequest,
        Platform, RebeccaError, Result, RuleDefinition, RuleSelection, SafetyLevel, TargetStatus,
    };

    #[cfg(feature = "rules")]
    pub use rebecca_rules::{builtin_rules, builtin_safety_knowledge};
}

pub use rebecca_core::{
    CleanupPlan, CleanupSummary, CleanupTarget, CleanupWorkflow, DeleteMode, PlanRequest, Platform,
    RebeccaError, Result, RuleDefinition, RuleSelection, SafetyLevel, TargetStatus,
};

#[cfg(feature = "rules")]
pub use rebecca_rules::{builtin_rules, builtin_safety_knowledge};

#[cfg(test)]
mod tests {
    #[test]
    fn facade_exposes_builtin_rules() {
        let rules = crate::builtin_rules().expect("built-in rules should load");
        assert!(!rules.is_empty());
    }
}
