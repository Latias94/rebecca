use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::PlanRequest;
use crate::error::{RebeccaError, Result};
use crate::plan::{CleanupPlan, CleanupSummary, CleanupTarget};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub recorded_at_unix_seconds: u64,
    pub request: PlanRequest,
    pub summary: CleanupSummary,
    pub targets: Vec<CleanupTarget>,
}

impl HistoryEntry {
    pub fn from_plan(plan: &CleanupPlan) -> Self {
        Self {
            recorded_at_unix_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or_default(),
            request: plan.request.clone(),
            summary: plan.summary.clone(),
            targets: plan.targets.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HistoryStore {
    path: PathBuf,
}

impl HistoryStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn append_plan(&self, plan: &CleanupPlan) -> Result<()> {
        self.append_entry(&HistoryEntry::from_plan(plan))
    }

    pub fn append_entry(&self, entry: &HistoryEntry) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;

        serde_json::to_writer(&mut file, entry)?;
        file.write_all(b"\n")?;
        Ok(())
    }

    pub fn load(&self) -> Result<Vec<HistoryEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&self.path)?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();

        for (index, line) in reader.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            let entry = serde_json::from_str::<HistoryEntry>(&line).map_err(|err| {
                RebeccaError::HistoryCorrupted(format!(
                    "{} at line {}: {}",
                    self.path.display(),
                    index + 1,
                    err
                ))
            })?;
            entries.push(entry);
        }

        Ok(entries)
    }
}
