use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{RebeccaError, Result};

pub trait ApplicationDiscovery {
    fn steam_installation(&self) -> Result<Option<SteamInstallation>>;

    fn installed_applications(&self) -> Result<Vec<InstalledApplication>> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledApplication {
    pub stable_id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub install_locations: Vec<PathBuf>,
}

impl InstalledApplication {
    pub fn new(
        stable_id: impl Into<String>,
        display_name: impl Into<String>,
        install_locations: impl IntoIterator<Item = PathBuf>,
    ) -> Self {
        Self {
            stable_id: stable_id.into(),
            display_name: display_name.into(),
            publisher: None,
            install_locations: dedupe_paths(install_locations),
        }
    }

    pub fn with_publisher(mut self, publisher: impl Into<String>) -> Self {
        let publisher = publisher.into();
        if !publisher.trim().is_empty() {
            self.publisher = Some(publisher);
        }
        self
    }

    pub fn with_install_location(mut self, install_location: impl Into<PathBuf>) -> Self {
        push_deduped_path(&mut self.install_locations, install_location.into());
        self
    }

    pub fn stable_id(&self) -> &str {
        &self.stable_id
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn install_locations(&self) -> &[PathBuf] {
        &self.install_locations
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoopApplicationDiscovery;

impl NoopApplicationDiscovery {
    pub fn new() -> Self {
        Self
    }
}

impl ApplicationDiscovery for NoopApplicationDiscovery {
    fn steam_installation(&self) -> Result<Option<SteamInstallation>> {
        Ok(None)
    }

    fn installed_applications(&self) -> Result<Vec<InstalledApplication>> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LinuxApplicationDiscovery;

impl LinuxApplicationDiscovery {
    pub fn new() -> Self {
        Self
    }
}

impl ApplicationDiscovery for LinuxApplicationDiscovery {
    fn steam_installation(&self) -> Result<Option<SteamInstallation>> {
        discover_linux_steam_installation()
    }
}

#[derive(Debug, Clone, Default)]
pub struct StaticApplicationDiscovery {
    steam_installation: Option<SteamInstallation>,
    installed_applications: Vec<InstalledApplication>,
}

impl StaticApplicationDiscovery {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_steam_installation(mut self, installation: SteamInstallation) -> Self {
        self.steam_installation = Some(installation);
        self
    }

    pub fn with_installed_applications(
        mut self,
        applications: impl IntoIterator<Item = InstalledApplication>,
    ) -> Self {
        self.installed_applications = dedupe_applications(applications);
        self
    }
}

impl ApplicationDiscovery for StaticApplicationDiscovery {
    fn steam_installation(&self) -> Result<Option<SteamInstallation>> {
        Ok(self.steam_installation.clone())
    }

    fn installed_applications(&self) -> Result<Vec<InstalledApplication>> {
        Ok(self.installed_applications.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamInstallation {
    install_path: PathBuf,
    library_paths: Vec<PathBuf>,
}

impl SteamInstallation {
    pub fn new(
        install_path: impl Into<PathBuf>,
        library_paths: impl IntoIterator<Item = PathBuf>,
    ) -> Self {
        let install_path = install_path.into();
        let paths = library_paths
            .into_iter()
            .filter(|path| !same_path_ignore_case(path, &install_path))
            .collect::<Vec<_>>();
        Self {
            install_path,
            library_paths: dedupe_paths(paths),
        }
    }

    pub fn from_install_path(install_path: impl Into<PathBuf>) -> Result<Self> {
        let install_path = install_path.into();
        let library_paths = read_steam_libraryfolders(&install_path)?;

        Ok(Self::new(install_path, library_paths))
    }

    pub fn from_install_path_best_effort(install_path: impl Into<PathBuf>) -> Self {
        let install_path = install_path.into();

        Self::from_install_path(&install_path)
            .unwrap_or_else(|_| Self::new(install_path, Vec::new()))
    }

    pub fn install_path(&self) -> &Path {
        &self.install_path
    }

    pub fn library_paths(&self) -> &[PathBuf] {
        &self.library_paths
    }
}

pub fn parse_steam_libraryfolders(raw: &str) -> Result<Vec<PathBuf>> {
    let tokens = tokenize_vdf(raw)?;
    let mut paths = Vec::new();
    let mut index = 0usize;

    while index + 1 < tokens.len() {
        let is_libraryfolders = match &tokens[index] {
            VdfToken::String(value) => value.eq_ignore_ascii_case("libraryfolders"),
            _ => false,
        };

        if is_libraryfolders && matches!(tokens.get(index + 1), Some(VdfToken::OpenBrace)) {
            index += 2;
            parse_steam_libraryfolders_object(&tokens, &mut index, &mut paths)?;
            return Ok(dedupe_paths(paths));
        }
        index += 1;
    }

    Ok(dedupe_paths(paths))
}

const STEAM_LIBRARYFOLDERS_CANDIDATES: [&str; 2] =
    ["config/libraryfolders.vdf", "steamapps/libraryfolders.vdf"];

fn read_steam_libraryfolders(install_path: &Path) -> Result<Vec<PathBuf>> {
    let candidates = STEAM_LIBRARYFOLDERS_CANDIDATES
        .iter()
        .map(|relative_path| install_path.join(relative_path));

    let mut paths = Vec::new();
    let mut first_error = None;

    for library_file in candidates {
        match read_steam_libraryfolders_file(&library_file) {
            Ok(Some(mut discovered)) => paths.append(&mut discovered),
            Ok(None) => {}
            Err(err) if first_error.is_none() => first_error = Some(err),
            Err(_) => {}
        }
    }

    if paths.is_empty()
        && let Some(err) = first_error
    {
        return Err(err);
    }

    Ok(dedupe_paths(paths))
}

fn read_steam_libraryfolders_file(library_file: &Path) -> Result<Option<Vec<PathBuf>>> {
    let raw = match fs::read_to_string(library_file) {
        Ok(raw) => raw,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(RebeccaError::ApplicationDiscoveryFailed(format!(
                "could not read Steam library folders at {}: {err}",
                library_file.display()
            )));
        }
    };

    parse_steam_libraryfolders(&raw).map(Some)
}

fn parse_steam_libraryfolders_object(
    tokens: &[VdfToken],
    index: &mut usize,
    paths: &mut Vec<PathBuf>,
) -> Result<()> {
    while *index < tokens.len() {
        match tokens.get(*index) {
            Some(VdfToken::CloseBrace) => {
                *index += 1;
                return Ok(());
            }
            Some(VdfToken::OpenBrace) => {
                *index += 1;
            }
            Some(VdfToken::String(key)) => {
                let key = key.clone();
                *index += 1;

                match tokens.get(*index) {
                    Some(VdfToken::OpenBrace) => {
                        *index += 1;
                        parse_steam_libraryfolders_object(tokens, index, paths)?;
                    }
                    Some(VdfToken::String(value)) => {
                        if let Some(path) = steam_library_path_value(&key, value) {
                            paths.push(path);
                        }
                        *index += 1;
                    }
                    Some(VdfToken::CloseBrace) => return Ok(()),
                    None => return Ok(()),
                }
            }
            None => return Ok(()),
        }
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum VdfToken {
    String(String),
    OpenBrace,
    CloseBrace,
}

fn tokenize_vdf(raw: &str) -> Result<Vec<VdfToken>> {
    let mut tokens = Vec::new();
    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '"' => tokens.push(VdfToken::String(read_vdf_string(&mut chars)?)),
            '{' => tokens.push(VdfToken::OpenBrace),
            '}' => tokens.push(VdfToken::CloseBrace),
            '/' if chars.peek() == Some(&'/') => {
                for comment_ch in chars.by_ref() {
                    if comment_ch == '\n' {
                        break;
                    }
                }
            }
            ch if ch.is_whitespace() => {}
            _ => {}
        }
    }

    Ok(tokens)
}

fn read_vdf_string<I>(chars: &mut std::iter::Peekable<I>) -> Result<String>
where
    I: Iterator<Item = char>,
{
    let mut value = String::new();

    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Ok(value),
            '\\' => {
                if let Some(escaped) = chars.next() {
                    match escaped {
                        '\\' | '"' => value.push(escaped),
                        'n' => value.push('\n'),
                        't' => value.push('\t'),
                        other => {
                            value.push('\\');
                            value.push(other);
                        }
                    }
                } else {
                    value.push('\\');
                }
            }
            other => value.push(other),
        }
    }

    Err(RebeccaError::ApplicationDiscoveryFailed(
        "unterminated string in Steam libraryfolders.vdf".to_string(),
    ))
}

fn is_legacy_library_key(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit())
}

fn looks_like_path_value(value: &str) -> bool {
    value.contains(':') || value.contains('\\') || value.contains('/')
}

fn steam_library_path_value(key: &str, value: &str) -> Option<PathBuf> {
    if key.eq_ignore_ascii_case("path")
        || (is_legacy_library_key(key) && looks_like_path_value(value))
    {
        let trimmed = value.trim();
        if trimmed.is_empty() || !looks_like_absolute_path(trimmed) {
            return None;
        }

        Some(PathBuf::from(trimmed))
    } else {
        None
    }
}

fn dedupe_paths(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();

    for path in paths {
        if seen.insert(path_key(&path)) {
            deduped.push(path);
        }
    }

    deduped
}

fn dedupe_applications(
    applications: impl IntoIterator<Item = InstalledApplication>,
) -> Vec<InstalledApplication> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();

    for application in applications {
        if seen.insert(application_key(&application)) {
            deduped.push(application);
        }
    }

    deduped
}

fn application_key(application: &InstalledApplication) -> String {
    let mut key = application.stable_id.trim().to_ascii_lowercase();
    key.push('|');
    key.push_str(&application.display_name.trim().to_ascii_lowercase());
    key.push('|');
    key.push_str(
        &application
            .install_locations
            .iter()
            .map(|path| path_key(path))
            .collect::<Vec<_>>()
            .join(";"),
    );
    key
}

fn push_deduped_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if paths
        .iter()
        .all(|existing| !same_path_ignore_case(existing, &path))
    {
        paths.push(path);
    }
}

fn path_key(path: &Path) -> String {
    let mut normalized = path
        .as_os_str()
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();

    while normalized.ends_with('/') && normalized.len() > 3 {
        normalized.pop();
    }

    normalized
}

fn same_path_ignore_case(left: &Path, right: &Path) -> bool {
    path_key(left) == path_key(right)
}

fn discover_linux_steam_installation() -> Result<Option<SteamInstallation>> {
    let Some(home) = std::env::var_os("HOME") else {
        return Ok(None);
    };
    if home.is_empty() {
        return Ok(None);
    }

    let home = PathBuf::from(home);
    for candidate in [
        home.join(".steam").join("steam"),
        home.join(".local").join("share").join("Steam"),
    ] {
        if candidate.is_dir() {
            return Ok(Some(SteamInstallation::from_install_path_best_effort(
                candidate,
            )));
        }
    }

    Ok(None)
}

fn looks_like_absolute_path(value: &str) -> bool {
    value.starts_with('/') || looks_like_windows_absolute_path(value)
}

fn looks_like_windows_absolute_path(value: &str) -> bool {
    let normalized = value.replace('/', "\\");

    if normalized.starts_with("\\\\") {
        return true;
    }

    let bytes = normalized.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\'
}
