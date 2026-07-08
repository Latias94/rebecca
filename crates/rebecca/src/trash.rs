use anyhow::{Result, anyhow};
use serde::Serialize;

use crate::cli::OutputMode;
use crate::output::{CliApiContract, format_bytes, format_shell_command};

const BYTE_ACCURACY_EXACT: &str = "exact";
const BYTE_ACCURACY_KNOWN_FILE_BYTES: &str = "known-file-bytes";

#[derive(Debug)]
pub(crate) struct TrashEmptyOptions {
    pub(crate) output_mode: OutputMode,
    pub(crate) yes: bool,
    pub(crate) drives: Vec<String>,
}

#[derive(Debug, Serialize)]
struct TrashReport {
    platform: &'static str,
    mode: &'static str,
    emptied: bool,
    summary: TrashSummary,
    targets: Vec<TrashTarget>,
}

#[derive(Debug, Default, Serialize)]
struct TrashSummary {
    targets: usize,
    items: u64,
    bytes: u64,
    byte_accuracy: &'static str,
    metadata_errors: u64,
    freed_bytes: u64,
    pending_reclaim_bytes: u64,
}

#[derive(Debug, Serialize)]
struct TrashTarget {
    root: Option<String>,
    items: u64,
    bytes: u64,
    byte_accuracy: &'static str,
    metadata_errors: u64,
    status: &'static str,
}

#[derive(Debug)]
struct TrashState {
    root: Option<String>,
    bytes: u64,
    byte_accuracy: &'static str,
    metadata_errors: u64,
    items: u64,
}

pub(crate) fn empty(options: TrashEmptyOptions) -> Result<()> {
    let roots = trash_roots(options.drives)?;
    let mut targets = Vec::with_capacity(roots.len());

    for root in &roots {
        let state = if options.yes {
            empty_trash(root.as_deref())?
        } else {
            query_trash(root.as_deref())?
        };
        targets.push(TrashTarget {
            root: state.root,
            items: state.items,
            bytes: state.bytes,
            byte_accuracy: state.byte_accuracy,
            metadata_errors: state.metadata_errors,
            status: if options.yes {
                "emptied"
            } else {
                "would-empty"
            },
        });
    }

    let bytes = targets.iter().map(|target| target.bytes).sum();
    let items = targets.iter().map(|target| target.items).sum();
    let metadata_errors = targets.iter().map(|target| target.metadata_errors).sum();
    let byte_accuracy = merged_byte_accuracy(&targets);
    let report = TrashReport {
        platform: std::env::consts::OS,
        mode: if options.yes { "empty" } else { "dry-run" },
        emptied: options.yes,
        summary: TrashSummary {
            targets: targets.len(),
            items,
            bytes,
            byte_accuracy,
            metadata_errors,
            freed_bytes: if options.yes { bytes } else { 0 },
            pending_reclaim_bytes: if options.yes { 0 } else { bytes },
        },
        targets,
    };

    crate::output::print_command_success_with_contract(
        CliApiContract::v1("trash empty", "trash-report"),
        options.output_mode,
        || &report,
        || {
            print_human_report(&report, &roots);
            Ok(())
        },
    )
}

fn trash_roots(drives: Vec<String>) -> Result<Vec<Option<String>>> {
    if drives.is_empty() {
        return Ok(vec![None]);
    }
    if !cfg!(windows) {
        return Err(anyhow!("--drive is only supported on Windows"));
    }
    drives
        .into_iter()
        .map(|drive| normalize_drive_arg(&drive).map(Some))
        .collect()
}

fn normalize_drive_arg(drive: &str) -> Result<String> {
    let trimmed = drive.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("drive cannot be empty"));
    }

    let mut chars = trimmed.chars();
    let Some(letter) = chars.next() else {
        return Err(anyhow!("drive cannot be empty"));
    };
    if !letter.is_ascii_alphabetic() {
        return Err(anyhow!("drive must start with a letter, got {trimmed}"));
    }
    let rest = chars.as_str();
    if !matches!(rest, "" | ":" | ":\\" | ":/") {
        return Err(anyhow!("drive must look like C or C:, got {trimmed}"));
    }

    Ok(format!("{}:", letter.to_ascii_uppercase()))
}

fn print_human_report(report: &TrashReport, roots: &[Option<String>]) {
    println!("{}", trash_label(report.platform));
    println!("Mode: {}", report.mode);
    println!("Scope: {}", trash_scope_label(roots));
    println!("Items: {}", report.summary.items);
    println!(
        "{}: {} ({})",
        byte_line_label(report.summary.byte_accuracy),
        report.summary.bytes,
        format_bytes(report.summary.bytes)
    );
    if report.summary.metadata_errors > 0 {
        println!(
            "Skipped metadata: {} trash item(s) could not be measured.",
            report.summary.metadata_errors
        );
    }
    println!(
        "Freed bytes: {} ({})",
        report.summary.freed_bytes,
        format_bytes(report.summary.freed_bytes)
    );
    println!(
        "Pending reclaim bytes: {} ({})",
        report.summary.pending_reclaim_bytes,
        format_bytes(report.summary.pending_reclaim_bytes)
    );

    if report.emptied {
        println!("Decision: {} emptied.", trash_label(report.platform));
    } else {
        println!(
            "Decision: preview only; {} was not emptied.",
            trash_label(report.platform)
        );
        println!(
            "Next command: {}",
            format_shell_command("rebecca", &trash_empty_args(roots))
        );
    }

    if report.targets.len() > 1 {
        println!();
        println!("Targets:");
        for target in &report.targets {
            println!(
                "  - {}: {}, {} ({})",
                target.root.as_deref().unwrap_or("all trash locations"),
                target.items,
                target.bytes,
                format_bytes(target.bytes)
            );
        }
    }
}

fn trash_label(platform: &str) -> &'static str {
    if platform == "windows" {
        "Windows Recycle Bin"
    } else {
        "System trash"
    }
}

fn byte_line_label(byte_accuracy: &str) -> &'static str {
    match byte_accuracy {
        BYTE_ACCURACY_EXACT => "Size",
        BYTE_ACCURACY_KNOWN_FILE_BYTES => "Known file bytes",
        _ => "Bytes",
    }
}

fn trash_scope_label(roots: &[Option<String>]) -> String {
    if roots.iter().any(Option::is_none) {
        return "all trash locations".to_string();
    }

    roots
        .iter()
        .filter_map(|root| root.as_deref())
        .collect::<Vec<_>>()
        .join(", ")
}

fn trash_empty_args(roots: &[Option<String>]) -> Vec<String> {
    let mut args = vec![
        "trash".to_string(),
        "empty".to_string(),
        "--yes".to_string(),
    ];
    for root in roots.iter().filter_map(|root| root.as_deref()) {
        args.push("--drive".to_string());
        args.push(root.to_string());
    }
    args
}

fn merged_byte_accuracy(targets: &[TrashTarget]) -> &'static str {
    if targets
        .iter()
        .all(|target| target.byte_accuracy == BYTE_ACCURACY_EXACT)
    {
        BYTE_ACCURACY_EXACT
    } else {
        BYTE_ACCURACY_KNOWN_FILE_BYTES
    }
}

fn query_trash(root: Option<&str>) -> Result<TrashState> {
    if root.is_some() {
        return query_windows_recycle_bin(root);
    }
    query_all_trash()
}

fn empty_trash(root: Option<&str>) -> Result<TrashState> {
    if root.is_some() {
        return empty_windows_recycle_bin(root);
    }
    empty_all_trash()
}

#[cfg(all(windows, feature = "windows"))]
fn query_all_trash() -> Result<TrashState> {
    query_windows_recycle_bin(None)
}

#[cfg(not(all(windows, feature = "windows")))]
fn query_all_trash() -> Result<TrashState> {
    query_supported_system_trash()
}

#[cfg(all(windows, feature = "windows"))]
fn empty_all_trash() -> Result<TrashState> {
    empty_windows_recycle_bin(None)
}

#[cfg(not(all(windows, feature = "windows")))]
fn empty_all_trash() -> Result<TrashState> {
    empty_supported_system_trash()
}

#[cfg(all(windows, feature = "windows"))]
fn query_windows_recycle_bin(root: Option<&str>) -> Result<TrashState> {
    rebecca_windows::recycle_bin::query_recycle_bin(root)
        .map(|state| TrashState {
            root: state.root,
            bytes: state.bytes,
            byte_accuracy: BYTE_ACCURACY_EXACT,
            metadata_errors: 0,
            items: state.items,
        })
        .map_err(|message| anyhow!(message))
}

#[cfg(not(all(windows, feature = "windows")))]
fn query_windows_recycle_bin(_root: Option<&str>) -> Result<TrashState> {
    Err(anyhow!(
        "--drive requires Rebecca's Windows adapter and is only available on Windows"
    ))
}

#[cfg(all(windows, feature = "windows"))]
fn empty_windows_recycle_bin(root: Option<&str>) -> Result<TrashState> {
    rebecca_windows::recycle_bin::empty_recycle_bin(root)
        .map(|state| TrashState {
            root: state.root,
            bytes: state.bytes,
            byte_accuracy: BYTE_ACCURACY_EXACT,
            metadata_errors: 0,
            items: state.items,
        })
        .map_err(|message| anyhow!(message))
}

#[cfg(not(all(windows, feature = "windows")))]
fn empty_windows_recycle_bin(_root: Option<&str>) -> Result<TrashState> {
    Err(anyhow!(
        "--drive requires Rebecca's Windows adapter and is only available on Windows"
    ))
}

#[cfg(any(
    all(windows, not(feature = "windows")),
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
fn query_supported_system_trash() -> Result<TrashState> {
    let items =
        trash::os_limited::list().map_err(|err| anyhow!("failed to list system trash: {err}"))?;
    Ok(summarize_system_trash_items(&items))
}

#[cfg(all(
    not(all(windows, feature = "windows")),
    not(any(
        all(windows, not(feature = "windows")),
        all(
            unix,
            not(target_os = "macos"),
            not(target_os = "ios"),
            not(target_os = "android")
        )
    ))
))]
fn query_supported_system_trash() -> Result<TrashState> {
    Err(anyhow!(
        "system trash listing is not supported on this platform yet"
    ))
}

#[cfg(any(
    all(windows, not(feature = "windows")),
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
fn empty_supported_system_trash() -> Result<TrashState> {
    let items =
        trash::os_limited::list().map_err(|err| anyhow!("failed to list system trash: {err}"))?;
    let state = summarize_system_trash_items(&items);
    trash::os_limited::purge_all(items)
        .map_err(|err| anyhow!("failed to empty system trash: {err}"))?;
    Ok(state)
}

#[cfg(all(
    not(all(windows, feature = "windows")),
    not(any(
        all(windows, not(feature = "windows")),
        all(
            unix,
            not(target_os = "macos"),
            not(target_os = "ios"),
            not(target_os = "android")
        )
    ))
))]
fn empty_supported_system_trash() -> Result<TrashState> {
    Err(anyhow!(
        "system trash emptying is not supported on this platform yet"
    ))
}

#[cfg(any(
    all(windows, not(feature = "windows")),
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
fn summarize_system_trash_items(items: &[trash::TrashItem]) -> TrashState {
    let mut bytes = 0_u64;
    let mut byte_accuracy = BYTE_ACCURACY_EXACT;
    let mut metadata_errors = 0_u64;

    for item in items {
        match trash::os_limited::metadata(item) {
            Ok(metadata) => match metadata.size {
                trash::TrashItemSize::Bytes(size) => {
                    bytes = bytes.saturating_add(size);
                }
                trash::TrashItemSize::Entries(_) => {
                    byte_accuracy = BYTE_ACCURACY_KNOWN_FILE_BYTES;
                }
            },
            Err(_) => {
                byte_accuracy = BYTE_ACCURACY_KNOWN_FILE_BYTES;
                metadata_errors = metadata_errors.saturating_add(1);
            }
        }
    }

    TrashState {
        root: None,
        bytes,
        byte_accuracy,
        metadata_errors,
        items: items.len() as u64,
    }
}
