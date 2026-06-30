use anyhow::{Context, Result};
use rebecca::core::scan::ScanCancellationToken;

#[derive(Debug, Clone)]
pub(crate) struct CliRuntime {
    cancellation: ScanCancellationToken,
}

impl CliRuntime {
    #[cfg(test)]
    pub(crate) fn new(cancellation: ScanCancellationToken) -> Self {
        Self { cancellation }
    }

    pub(crate) fn with_ctrlc_handler() -> Result<Self> {
        let cancellation = ScanCancellationToken::new();
        ctrlc::set_handler({
            let cancellation = cancellation.clone();
            move || cancellation.cancel()
        })
        .context("failed to install Ctrl+C handler")?;

        Ok(Self { cancellation })
    }

    pub(crate) fn cancellation(&self) -> &ScanCancellationToken {
        &self.cancellation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_exposes_shared_cancellation_token() {
        let token = ScanCancellationToken::new();
        let runtime = CliRuntime::new(token.clone());

        assert!(!runtime.cancellation().is_cancelled());

        token.cancel();

        assert!(runtime.cancellation().is_cancelled());
    }
}
