use std::fs;
use std::path::{Path, PathBuf};

use super::ProjectArtifactContextMatch;
use super::definitions::ProjectArtifactContext;

pub(super) fn project_artifact_context_match(
    path: &Path,
    context: ProjectArtifactContext,
) -> Option<ProjectArtifactContextMatch> {
    let parent = path.parent()?;

    match context {
        ProjectArtifactContext::NodeProject => node_project_context(parent),
        ProjectArtifactContext::TargetProject => target_project_context(parent),
        ProjectArtifactContext::PythonProject => python_project_context(parent),
        ProjectArtifactContext::PythonCache => {
            project_anchor_ancestor(parent, python_cache_context, 4)
        }
        ProjectArtifactContext::GradleProject => gradle_project_context(parent),
        ProjectArtifactContext::DartProject => {
            regular_file_context(parent, "dart-project", &["pubspec.yaml"])
        }
        ProjectArtifactContext::ZigProject => zig_project_context(parent),
        ProjectArtifactContext::GenericProjectOutput => project_output_context(parent),
        ProjectArtifactContext::CocoapodsProject => cocoapods_project_context(parent),
        ProjectArtifactContext::CxxProject => cxx_project_context(parent),
        ProjectArtifactContext::ExpoProject => expo_project_context(parent),
        ProjectArtifactContext::SwiftPackage => {
            regular_file_context(parent, "swift-package", &["Package.swift"])
        }
        ProjectArtifactContext::DotnetBin => dotnet_bin_context(path),
        ProjectArtifactContext::DotnetObj => dotnet_obj_context(path),
        ProjectArtifactContext::ComposerVendor => {
            regular_file_context(parent, "composer-vendor", &["composer.json"])
        }
        ProjectArtifactContext::CachedirTag => Some(cachedir_tag_context_match(path)),
    }
}

pub(super) fn cachedir_tag_context_match(dir: &Path) -> ProjectArtifactContextMatch {
    context_match("cachedir-tag", dir, dir.join("CACHEDIR.TAG"))
}

fn context_match(
    matched_context: &'static str,
    project_root: &Path,
    project_anchor: PathBuf,
) -> ProjectArtifactContextMatch {
    ProjectArtifactContextMatch {
        matched_context: matched_context.to_string(),
        project_root: project_root.to_path_buf(),
        project_anchor,
    }
}

fn regular_file_context(
    dir: &Path,
    matched_context: &'static str,
    markers: &[&str],
) -> Option<ProjectArtifactContextMatch> {
    find_regular_file_anchor(dir, markers).map(|anchor| context_match(matched_context, dir, anchor))
}

fn node_project_context(dir: &Path) -> Option<ProjectArtifactContextMatch> {
    regular_file_context(
        dir,
        "node-project",
        &[
            "package.json",
            "pnpm-workspace.yaml",
            "nx.json",
            "rush.json",
            "lerna.json",
        ],
    )
}

fn target_project_context(dir: &Path) -> Option<ProjectArtifactContextMatch> {
    regular_file_context(dir, "target-project", &["Cargo.toml", "pom.xml"])
}

fn python_project_context(dir: &Path) -> Option<ProjectArtifactContextMatch> {
    regular_file_context(
        dir,
        "python-project",
        &[
            "pyproject.toml",
            "requirements.txt",
            "Pipfile",
            "poetry.lock",
            "setup.py",
            "setup.cfg",
            "tox.ini",
            "pytest.ini",
            "ruff.toml",
            ".python-version",
        ],
    )
}

fn python_cache_context(dir: &Path) -> Option<ProjectArtifactContextMatch> {
    regular_file_context(
        dir,
        "python-cache",
        &[
            "pyproject.toml",
            "requirements.txt",
            "Pipfile",
            "poetry.lock",
            "tox.ini",
            "pytest.ini",
            "ruff.toml",
        ],
    )
    .or_else(|| {
        has_child_dir(dir, ".git").then(|| context_match("python-cache", dir, dir.join(".git")))
    })
}

fn gradle_project_context(dir: &Path) -> Option<ProjectArtifactContextMatch> {
    regular_file_context(
        dir,
        "gradle-project",
        &[
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
        ],
    )
}

fn zig_project_context(dir: &Path) -> Option<ProjectArtifactContextMatch> {
    regular_file_context(dir, "zig-project", &["build.zig", "build.zig.zon"])
}

fn project_output_context(dir: &Path) -> Option<ProjectArtifactContextMatch> {
    regular_file_context(
        dir,
        "project-output",
        &[
            "package.json",
            "pnpm-workspace.yaml",
            "nx.json",
            "rush.json",
            "lerna.json",
            "pyproject.toml",
            "requirements.txt",
            "Pipfile",
            "Cargo.toml",
            "pom.xml",
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
            "CMakeLists.txt",
            "Makefile",
            "go.mod",
            "Gemfile",
            "composer.json",
            "pubspec.yaml",
            "Package.swift",
            "Podfile",
            "build.zig",
            "build.zig.zon",
        ],
    )
    .or_else(|| {
        has_child_dir(dir, ".git").then(|| context_match("project-output", dir, dir.join(".git")))
    })
    .or_else(|| {
        dotnet_project_file_anchor(dir).map(|anchor| context_match("project-output", dir, anchor))
    })
    .or_else(|| {
        file_with_extension_anchor(dir, &["sln", "vcxproj"])
            .map(|anchor| context_match("project-output", dir, anchor))
    })
}

fn cocoapods_project_context(dir: &Path) -> Option<ProjectArtifactContextMatch> {
    regular_file_context(dir, "cocoapods-project", &["Podfile", "Podfile.lock"])
}

fn cxx_project_context(dir: &Path) -> Option<ProjectArtifactContextMatch> {
    regular_file_context(dir, "cxx-project", &["CMakeLists.txt"]).or_else(|| {
        gradle_project_context(dir).map(|mut context| {
            context.matched_context = "cxx-project".to_string();
            context
        })
    })
}

fn expo_project_context(dir: &Path) -> Option<ProjectArtifactContextMatch> {
    regular_file_context(
        dir,
        "expo-project",
        &[
            "package.json",
            "app.json",
            "app.config.js",
            "app.config.ts",
            "app.config.mjs",
        ],
    )
}

fn dotnet_bin_context(path: &Path) -> Option<ProjectArtifactContextMatch> {
    let parent = path.parent()?;

    if has_child_dir(path, "Debug") || has_child_dir(path, "Release") {
        dotnet_project_file_anchor(parent).map(|anchor| context_match("dotnet-bin", parent, anchor))
    } else {
        None
    }
}

fn dotnet_obj_context(path: &Path) -> Option<ProjectArtifactContextMatch> {
    let parent = path.parent()?;

    dotnet_project_file_anchor(parent).map(|anchor| context_match("dotnet-obj", parent, anchor))
}

fn project_anchor_ancestor(
    start: &Path,
    predicate: fn(&Path) -> Option<ProjectArtifactContextMatch>,
    max_parent_hops: usize,
) -> Option<ProjectArtifactContextMatch> {
    let mut current = Some(start);
    for _ in 0..=max_parent_hops {
        let dir = current?;
        if let Some(context) = predicate(dir) {
            return Some(context);
        }
        current = dir.parent();
    }

    None
}

fn find_regular_file_anchor(dir: &Path, markers: &[&str]) -> Option<PathBuf> {
    markers
        .iter()
        .map(|marker| dir.join(marker))
        .find(|path| has_regular_file(path))
}

fn dotnet_project_file_anchor(dir: &Path) -> Option<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return None;
    };

    entries.flatten().find_map(|entry| {
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            return None;
        };
        if !metadata.is_file() || crate::safety::is_reparse_like(&metadata) {
            return None;
        }

        let is_dotnet_project = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                ["csproj", "fsproj", "vbproj"]
                    .iter()
                    .any(|expected| extension.eq_ignore_ascii_case(expected))
            });

        is_dotnet_project.then_some(path)
    })
}

fn file_with_extension_anchor(dir: &Path, extensions: &[&str]) -> Option<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return None;
    };

    entries.flatten().find_map(|entry| {
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            return None;
        };
        if !metadata.is_file() || crate::safety::is_reparse_like(&metadata) {
            return None;
        }

        let matches_extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extensions
                    .iter()
                    .any(|expected| extension.eq_ignore_ascii_case(expected))
            });

        matches_extension.then_some(path)
    })
}

fn has_regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.is_file() && !crate::safety::is_reparse_like(&metadata))
        .unwrap_or(false)
}

fn has_child_dir(parent: &Path, child: &str) -> bool {
    let path = parent.join(child);
    fs::symlink_metadata(path)
        .map(|metadata| metadata.is_dir() && !crate::safety::is_reparse_like(&metadata))
        .unwrap_or(false)
}
