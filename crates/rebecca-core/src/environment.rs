use std::collections::HashMap;
use std::ffi::OsString;

pub trait Environment {
    fn get(&self, key: &str) -> Option<OsString>;
}

#[derive(Debug, Clone, Default)]
pub struct SystemEnvironment;

impl Environment for SystemEnvironment {
    fn get(&self, key: &str) -> Option<OsString> {
        std::env::var_os(key)
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
