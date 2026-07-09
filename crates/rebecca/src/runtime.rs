use anyhow::{Context, Result};
use rebecca_core::scan::ScanCancellationToken;

#[derive(Debug, Clone)]
pub(crate) struct CliRuntime {
    cancellation: ScanCancellationToken,
}

impl CliRuntime {
    #[cfg(test)]
    pub(crate) fn new(cancellation: ScanCancellationToken) -> Self {
        Self { cancellation }
    }

    pub(crate) fn child_task(&self) -> Self {
        Self {
            cancellation: self.cancellation.child_token(),
        }
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

    #[test]
    fn task_runtime_can_cancel_without_poisoning_parent() {
        let token = ScanCancellationToken::new();
        let runtime = CliRuntime::new(token.clone());
        let task_runtime = runtime.child_task();

        task_runtime.cancellation().cancel();

        assert!(task_runtime.cancellation().is_cancelled());
        assert!(!runtime.cancellation().is_cancelled());

        token.cancel();

        assert!(runtime.cancellation().is_cancelled());
    }
}
