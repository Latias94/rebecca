use rebecca::core::cleanup_advice::CleanupAdviceStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CleanupBasketItem {
    pub(crate) rule_id: String,
    pub(crate) status: CleanupAdviceStatus,
    pub(crate) reason: String,
    pub(crate) required_flags: Vec<String>,
    pub(crate) required_warnings: Vec<String>,
}
