use std::fs;
use std::path::{Component, Path, PathBuf};

use globset::{GlobBuilder, GlobMatcher};

use crate::applications::{ApplicationDiscovery, NoopApplicationDiscovery, SteamInstallation};
use crate::environment::Environment;
use crate::error::{RebeccaError, Result};
use crate::model::RuleTargetSpec;
use crate::path_template::expand_template;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetResolution {
    Paths(Vec<PathBuf>),
    Skipped(String),
}

pub fn resolve_rule_target(
    target: &RuleTargetSpec,
    env: &impl Environment,
) -> Result<TargetResolution> {
    resolve_rule_target_with_applications(target, env, &NoopApplicationDiscovery::new())
}

pub fn resolve_rule_target_with_applications<A>(
    target: &RuleTargetSpec,
    env: &impl Environment,
    applications: &A,
) -> Result<TargetResolution>
where
    A: ApplicationDiscovery + ?Sized,
{
    match target {
        RuleTargetSpec::Template(template) => match expand_template(template, env)? {
            Some(path) => Ok(TargetResolution::Paths(vec![path])),
            None => Ok(TargetResolution::Skipped(
                "path template could not be resolved in the current environment".to_string(),
            )),
        },
        RuleTargetSpec::ExactPath(path) => Ok(TargetResolution::Paths(vec![path.clone()])),
        RuleTargetSpec::GlobTemplate(template) => {
            let pattern = match expand_template(template, env)? {
                Some(path) => path,
                None => {
                    return Ok(TargetResolution::Skipped(
                        "glob template could not be resolved in the current environment"
                            .to_string(),
                    ));
                }
            };

            let paths = discover_glob_paths(&pattern)?;
            if paths.is_empty() {
                Ok(TargetResolution::Skipped(
                    "glob pattern matched no existing paths".to_string(),
                ))
            } else {
                Ok(TargetResolution::Paths(paths))
            }
        }
        RuleTargetSpec::SteamInstallTemplate(template) => {
            resolve_with_steam_installation(applications, |steam| {
                match append_steam_relative_target(steam.install_path(), template, env)? {
                    Some(path) => Ok(TargetResolution::Paths(vec![path])),
                    None => Ok(TargetResolution::Skipped(
                        "Steam install template could not be resolved in the current environment"
                            .to_string(),
                    )),
                }
            })
        }
        RuleTargetSpec::SteamLibraryTemplate(template) => {
            resolve_with_steam_installation(applications, |steam| {
                resolve_steam_library_template(steam, template, env)
            })
        }
    }
}

fn resolve_with_steam_installation<A, F>(applications: &A, resolve: F) -> Result<TargetResolution>
where
    A: ApplicationDiscovery + ?Sized,
    F: FnOnce(&SteamInstallation) -> Result<TargetResolution>,
{
    let Some(steam) = applications.steam_installation()? else {
        return Ok(TargetResolution::Skipped(
            "Steam installation was not discovered".to_string(),
        ));
    };

    resolve(&steam)
}

fn resolve_steam_library_template(
    steam: &SteamInstallation,
    template: &crate::PathTemplate,
    env: &impl Environment,
) -> Result<TargetResolution> {
    let mut paths = Vec::with_capacity(1 + steam.library_paths().len());
    for library_path in std::iter::once(steam.install_path())
        .chain(steam.library_paths().iter().map(PathBuf::as_path))
    {
        if let Some(path) = append_steam_relative_target(&library_path, template, env)? {
            paths.push(path);
        } else {
            return Ok(TargetResolution::Skipped(
                "Steam library template could not be resolved in the current environment"
                    .to_string(),
            ));
        }
    }

    Ok(TargetResolution::Paths(paths))
}

fn append_steam_relative_target(
    root: &Path,
    template: &crate::PathTemplate,
    env: &impl Environment,
) -> Result<Option<PathBuf>> {
    let Some(relative) = expand_template(template, env)? else {
        return Ok(None);
    };

    ensure_safe_relative_steam_target(&relative)?;
    Ok(Some(root.join(relative)))
}

fn ensure_safe_relative_steam_target(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || looks_absolute(path) {
        return Err(unsafe_steam_relative_path_error(path));
    }

    for component in path.components() {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err(unsafe_steam_relative_path_error(path));
        }
    }

    Ok(())
}

fn looks_absolute(path: &Path) -> bool {
    if path.is_absolute() {
        return true;
    }

    let raw = path.as_os_str().to_string_lossy().replace('\\', "/");
    raw.starts_with('/') || raw.starts_with("//") || raw.as_bytes().get(1) == Some(&b':')
}

fn unsafe_steam_relative_path_error(path: &Path) -> RebeccaError {
    RebeccaError::PathExpansionFailed(format!(
        "Steam target {} must be a safe relative path",
        path.display()
    ))
}

fn discover_glob_paths(pattern: &Path) -> Result<Vec<PathBuf>> {
    let normalized = normalize_separators(&pattern.as_os_str().to_string_lossy());
    let segments = split_segments(&normalized);

    let mut results = Vec::new();
    expand_segments(root_path(&normalized), &segments, &mut results)?;
    results.sort();
    results.dedup();

    Ok(results)
}

fn expand_segments(
    current: PathBuf,
    remaining: &[String],
    results: &mut Vec<PathBuf>,
) -> Result<()> {
    let Some((segment, tail)) = remaining.split_first() else {
        if current.exists() {
            results.push(current);
        }
        return Ok(());
    };

    if !has_wildcards(segment) {
        let mut next = current;
        next.push(segment);
        return expand_segments(next, tail, results);
    }

    if !current.is_dir() {
        return Ok(());
    }

    let matcher = segment_matcher(segment)?;
    for entry in fs::read_dir(&current)? {
        let entry = entry?;
        if matcher.is_match(entry.file_name()) {
            expand_segments(entry.path(), tail, results)?;
        }
    }

    Ok(())
}

fn normalize_separators(raw: &str) -> String {
    if std::path::MAIN_SEPARATOR == '\\' {
        raw.replace('/', "\\")
    } else {
        raw.replace('\\', "/")
    }
}

fn split_segments(normalized: &str) -> Vec<String> {
    let mut segments = normalized
        .split(std::path::MAIN_SEPARATOR)
        .filter(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    if is_drive_absolute(normalized)
        && segments
            .first()
            .is_some_and(|segment| segment.ends_with(':'))
    {
        segments.remove(0);
    }

    segments
}

fn root_path(normalized: &str) -> PathBuf {
    let separator = std::path::MAIN_SEPARATOR;

    if is_drive_absolute(normalized) {
        return PathBuf::from(format!("{}{}", &normalized[..2], separator));
    }

    if normalized.starts_with(separator) {
        return PathBuf::from(separator.to_string());
    }

    PathBuf::new()
}

fn is_drive_absolute(normalized: &str) -> bool {
    let separator = std::path::MAIN_SEPARATOR;
    let bytes = normalized.as_bytes();

    bytes.len() >= 3 && bytes[1] == b':' && bytes[2] == separator as u8
}

fn has_wildcards(segment: &str) -> bool {
    segment.contains('*') || segment.contains('?') || segment.contains('[')
}

fn segment_matcher(segment: &str) -> Result<GlobMatcher> {
    let mut builder = GlobBuilder::new(segment);
    builder.literal_separator(true);

    if cfg!(windows) {
        builder.case_insensitive(true);
    }

    builder
        .build()
        .map(|glob| glob.compile_matcher())
        .map_err(|err| {
            RebeccaError::PathExpansionFailed(format!("invalid glob segment {segment:?}: {err}"))
        })
}
