use anyhow::Result;
use rebecca_core::cache::{CachePurgeMode, CachePurgeReport, purge_app_cache};
use rebecca_core::config::load_app_paths;

use crate::output::format_bytes;

#[derive(Debug)]
pub struct CachePurgeOptions {
    pub dry_run: bool,
    pub json: bool,
    pub yes: bool,
}

pub fn purge(options: CachePurgeOptions) -> Result<()> {
    let paths = load_app_paths()?;
    let mode = if options.yes && !options.dry_run {
        CachePurgeMode::Delete
    } else {
        CachePurgeMode::DryRun
    };
    let report = purge_app_cache(&paths, mode)?;

    if options.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    print_cache_purge_report(&report);
    Ok(())
}

fn print_cache_purge_report(report: &CachePurgeReport) {
    println!("Rebecca cache: {}", report.cache_dir.display());
    println!("Mode: {}", mode_label(report.mode));
    println!(
        "Entries: {}, files: {}, directories: {}",
        report.summary.total_entries, report.summary.files, report.summary.directories
    );
    println!(
        "Estimated bytes: {} ({})",
        report.summary.estimated_bytes,
        format_bytes(report.summary.estimated_bytes)
    );
    println!(
        "Reclaimed bytes: {} ({})",
        report.summary.reclaimed_bytes,
        format_bytes(report.summary.reclaimed_bytes)
    );

    if report.entries.is_empty() {
        println!("No cache entries found.");
        return;
    }

    if report.mode == CachePurgeMode::DryRun {
        println!("Run with --yes to purge these rebuildable cache entries.");
    }

    println!("Cache entries:");
    for entry in &report.entries {
        let reason = entry
            .reason
            .as_deref()
            .map(|reason| format!(" - {reason}"))
            .unwrap_or_default();
        println!(
            "- {}: {} ({}; {} file(s), {} dir(s)){}",
            entry.status.label(),
            entry.path.display(),
            format_bytes(entry.estimated_bytes),
            entry.files,
            entry.directories,
            reason
        );
    }
}

fn mode_label(mode: CachePurgeMode) -> &'static str {
    match mode {
        CachePurgeMode::DryRun => "dry-run",
        CachePurgeMode::Delete => "delete",
    }
}
