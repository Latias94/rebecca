use std::num::NonZeroUsize;

use rebecca_core::history::HistoryEntry;

const HISTORY_LARGEST_RUN_LIMIT: usize = 3;

#[derive(Debug, Clone)]
pub(crate) struct HistoryProjection {
    entries: Vec<HistoryEntry>,
    summary: HistoryAggregateSummary,
    largest_runs: Vec<HistoryRunHighlight>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct HistoryAggregateSummary {
    pub(crate) runs: usize,
    pub(crate) completed_targets: usize,
    pub(crate) skipped_targets: usize,
    pub(crate) blocked_targets: usize,
    pub(crate) failed_targets: usize,
    pub(crate) freed_bytes: u64,
    pub(crate) pending_reclaim_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HistoryRunHighlight {
    pub(crate) recorded_at_unix_seconds: u64,
    pub(crate) total_bytes: u64,
    pub(crate) freed_bytes: u64,
    pub(crate) pending_reclaim_bytes: u64,
}

impl HistoryProjection {
    pub(crate) fn new(entries: Vec<HistoryEntry>, limit: Option<NonZeroUsize>) -> Self {
        let entries = limit_history_entries(entries, limit);
        let summary = HistoryAggregateSummary::from_entries(&entries);
        let largest_runs = largest_history_runs(&entries);

        Self {
            entries,
            summary,
            largest_runs,
        }
    }

    pub(crate) fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    pub(crate) fn summary(&self) -> &HistoryAggregateSummary {
        &self.summary
    }

    pub(crate) fn largest_runs(&self) -> &[HistoryRunHighlight] {
        &self.largest_runs
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl HistoryAggregateSummary {
    fn from_entries(entries: &[HistoryEntry]) -> Self {
        let mut summary = Self::default();

        for entry in entries {
            summary.runs = summary.runs.saturating_add(1);
            summary.completed_targets = summary
                .completed_targets
                .saturating_add(entry.summary.completed_targets);
            summary.skipped_targets = summary
                .skipped_targets
                .saturating_add(entry.summary.skipped_targets);
            summary.blocked_targets = summary
                .blocked_targets
                .saturating_add(entry.summary.blocked_targets);
            summary.failed_targets = summary
                .failed_targets
                .saturating_add(entry.summary.failed_targets);
            summary.freed_bytes = summary
                .freed_bytes
                .saturating_add(entry.summary.freed_bytes);
            summary.pending_reclaim_bytes = summary
                .pending_reclaim_bytes
                .saturating_add(entry.summary.pending_reclaim_bytes);
        }

        summary
    }
}

fn limit_history_entries(
    mut entries: Vec<HistoryEntry>,
    limit: Option<NonZeroUsize>,
) -> Vec<HistoryEntry> {
    let Some(limit) = limit else {
        return entries;
    };

    let limit = limit.get();
    if entries.len() <= limit {
        return entries;
    }

    entries.split_off(entries.len() - limit)
}

fn largest_history_runs(entries: &[HistoryEntry]) -> Vec<HistoryRunHighlight> {
    let mut runs = entries
        .iter()
        .filter_map(|entry| {
            let total_bytes = history_cleanup_bytes(entry);
            (total_bytes > 0).then_some(HistoryRunHighlight {
                recorded_at_unix_seconds: entry.recorded_at_unix_seconds,
                total_bytes,
                freed_bytes: entry.summary.freed_bytes,
                pending_reclaim_bytes: entry.summary.pending_reclaim_bytes,
            })
        })
        .collect::<Vec<_>>();

    runs.sort_by(|left, right| {
        right
            .total_bytes
            .cmp(&left.total_bytes)
            .then_with(|| right.freed_bytes.cmp(&left.freed_bytes))
            .then_with(|| right.pending_reclaim_bytes.cmp(&left.pending_reclaim_bytes))
            .then_with(|| {
                right
                    .recorded_at_unix_seconds
                    .cmp(&left.recorded_at_unix_seconds)
            })
    });

    runs.truncate(HISTORY_LARGEST_RUN_LIMIT);
    runs
}

fn history_cleanup_bytes(entry: &HistoryEntry) -> u64 {
    entry
        .summary
        .freed_bytes
        .saturating_add(entry.summary.pending_reclaim_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rebecca_core::plan::CleanupSummary;
    use rebecca_core::{DeleteMode, PlanRequest, Platform};

    fn history_entry(recorded_at_unix_seconds: u64, summary: CleanupSummary) -> HistoryEntry {
        HistoryEntry {
            recorded_at_unix_seconds,
            request: PlanRequest::for_platform(Platform::Windows, DeleteMode::RecoverableDelete),
            summary,
            targets: Vec::new(),
        }
    }

    #[test]
    fn projection_applies_limit_before_summarizing() {
        let projection = HistoryProjection::new(
            vec![
                history_entry(
                    10,
                    CleanupSummary {
                        completed_targets: 1,
                        skipped_targets: 1,
                        freed_bytes: 100,
                        pending_reclaim_bytes: 10,
                        ..CleanupSummary::default()
                    },
                ),
                history_entry(
                    20,
                    CleanupSummary {
                        completed_targets: 2,
                        blocked_targets: 1,
                        freed_bytes: 200,
                        pending_reclaim_bytes: 20,
                        ..CleanupSummary::default()
                    },
                ),
                history_entry(
                    30,
                    CleanupSummary {
                        completed_targets: 4,
                        failed_targets: 1,
                        freed_bytes: 400,
                        pending_reclaim_bytes: 40,
                        ..CleanupSummary::default()
                    },
                ),
            ],
            NonZeroUsize::new(2),
        );

        assert_eq!(projection.entries().len(), 2);
        assert_eq!(projection.summary().runs, 2);
        assert_eq!(projection.summary().completed_targets, 6);
        assert_eq!(projection.summary().blocked_targets, 1);
        assert_eq!(projection.summary().failed_targets, 1);
        assert_eq!(projection.summary().freed_bytes, 600);
        assert_eq!(projection.summary().pending_reclaim_bytes, 60);
    }

    #[test]
    fn projection_orders_largest_runs_by_cleanup_bytes() {
        let projection = HistoryProjection::new(
            vec![
                history_entry(
                    10,
                    CleanupSummary {
                        freed_bytes: 100,
                        pending_reclaim_bytes: 0,
                        ..CleanupSummary::default()
                    },
                ),
                history_entry(
                    20,
                    CleanupSummary {
                        freed_bytes: 0,
                        pending_reclaim_bytes: 400,
                        ..CleanupSummary::default()
                    },
                ),
                history_entry(
                    30,
                    CleanupSummary {
                        freed_bytes: 200,
                        pending_reclaim_bytes: 100,
                        ..CleanupSummary::default()
                    },
                ),
                history_entry(
                    40,
                    CleanupSummary {
                        freed_bytes: 0,
                        pending_reclaim_bytes: 200,
                        ..CleanupSummary::default()
                    },
                ),
                history_entry(50, CleanupSummary::default()),
            ],
            None,
        );

        let runs = projection.largest_runs();
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].recorded_at_unix_seconds, 20);
        assert_eq!(runs[0].total_bytes, 400);
        assert_eq!(runs[1].recorded_at_unix_seconds, 30);
        assert_eq!(runs[1].total_bytes, 300);
        assert_eq!(runs[2].recorded_at_unix_seconds, 40);
        assert_eq!(runs[2].total_bytes, 200);
    }

    #[test]
    fn projection_omits_zero_byte_runs() {
        let projection = HistoryProjection::new(
            vec![
                history_entry(10, CleanupSummary::default()),
                history_entry(
                    20,
                    CleanupSummary {
                        freed_bytes: 1,
                        pending_reclaim_bytes: 0,
                        ..CleanupSummary::default()
                    },
                ),
            ],
            None,
        );

        let runs = projection.largest_runs();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].recorded_at_unix_seconds, 20);
    }
}
