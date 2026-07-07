use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;

use crate::{Platform, macos_paths};

pub trait Environment {
    fn get(&self, key: &str) -> Option<OsString>;
}

impl<E> Environment for &E
where
    E: Environment + ?Sized,
{
    fn get(&self, key: &str) -> Option<OsString> {
        (*self).get(key)
    }
}

#[derive(Debug, Clone, Default)]
pub struct SystemEnvironment;

impl Environment for SystemEnvironment {
    fn get(&self, key: &str) -> Option<OsString> {
        std::env::var_os(key)
    }
}

#[derive(Debug, Clone)]
pub struct PlatformEnvironment<E> {
    platform: Platform,
    inner: E,
}

impl<E> PlatformEnvironment<E> {
    pub fn new(platform: Platform, inner: E) -> Self {
        Self { platform, inner }
    }

    pub fn current(inner: E) -> Self {
        Self::new(Platform::current(), inner)
    }
}

impl<E> Environment for PlatformEnvironment<E>
where
    E: Environment,
{
    fn get(&self, key: &str) -> Option<OsString> {
        let value = self.inner.get(key);
        if self.platform == Platform::Linux && linux_xdg_default_suffix(key).is_some() {
            return value
                .filter(|value| !value.is_empty())
                .or_else(|| linux_xdg_default(key, &self.inner));
        }
        if self.platform == Platform::Macos && macos_paths::default_home_suffix(key).is_some() {
            return value
                .filter(|value| !value.is_empty())
                .or_else(|| macos_default(key, &self.inner));
        }
        value
    }
}

#[derive(Debug, Clone, Default)]
pub struct MapEnvironment {
    values: HashMap<String, OsString>,
}

impl MapEnvironment {
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
        }
    }

    pub fn with_var(mut self, key: impl Into<String>, value: impl Into<OsString>) -> Self {
        self.values.insert(key.into(), value.into());
        self
    }
}

impl Environment for MapEnvironment {
    fn get(&self, key: &str) -> Option<OsString> {
        self.values.get(key).cloned()
    }
}

impl<K, V> FromIterator<(K, V)> for MapEnvironment
where
    K: Into<String>,
    V: Into<OsString>,
{
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        let mut values = HashMap::new();
        for (key, value) in iter {
            values.insert(key.into(), value.into());
        }
        Self { values }
    }
}

fn linux_xdg_default(key: &str, env: &impl Environment) -> Option<OsString> {
    let suffix = linux_xdg_default_suffix(key)?;
    home_relative_default(env, suffix)
}

fn macos_default(key: &str, env: &impl Environment) -> Option<OsString> {
    let suffix = macos_paths::default_home_suffix(key)?;
    home_relative_default(env, suffix)
}

fn home_relative_default(env: &impl Environment, suffix: &[&str]) -> Option<OsString> {
    let home = env.get("HOME")?;
    if home.is_empty() {
        return None;
    }

    let mut path = PathBuf::from(home);
    for segment in suffix {
        path.push(segment);
    }
    Some(path.into_os_string())
}

fn linux_xdg_default_suffix(key: &str) -> Option<&'static [&'static str]> {
    match key {
        "XDG_CACHE_HOME" => Some(&[".cache"]),
        "XDG_CONFIG_HOME" => Some(&[".config"]),
        "XDG_DATA_HOME" => Some(&[".local", "share"]),
        "XDG_STATE_HOME" => Some(&[".local", "state"]),
        _ => None,
    }
}
