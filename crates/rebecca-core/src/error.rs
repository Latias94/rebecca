use thiserror::Error;

pub type Result<T> = std::result::Result<T, RebeccaError>;

#[derive(Debug, Error)]
pub enum RebeccaError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("toml error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("could not locate the current user's standard directories")]
    UserDirsUnavailable,

    #[error("invalid rule id: {0}")]
    InvalidRuleId(String),

    #[error("invalid rule catalog: {0}")]
    RuleCatalogInvalid(String),

    #[error("path template expansion failed: {0}")]
    PathExpansionFailed(String),

    #[error("cleanup target was blocked by safety policy: {0}")]
    SafetyBlocked(String),

    #[error("scan failed: {0}")]
    ScanFailed(String),

    #[error("operation cancelled: {0}")]
    OperationCancelled(String),

    #[error("platform feature is not available: {0}")]
    PlatformUnavailable(String),

    #[error("history is unavailable: {0}")]
    HistoryUnavailable(String),

    #[error("history record was corrupted: {0}")]
    HistoryCorrupted(String),

    #[error("cleanup execution failed: {0}")]
    ExecutionFailed(String),
}
