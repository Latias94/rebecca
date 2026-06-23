use std::ffi::OsStr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::environment::Environment;
use crate::error::{RebeccaError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathTemplate(pub String);

impl PathTemplate {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    pub fn raw(&self) -> &str {
        &self.0
    }
}

impl From<&str> for PathTemplate {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for PathTemplate {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

pub fn expand_template(template: &PathTemplate, env: &impl Environment) -> Result<Option<PathBuf>> {
    let mut expanded = String::new();
    let mut chars = template.raw().chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '%' {
            expanded.push(ch);
            continue;
        }

        let mut key = String::new();
        let mut closed = false;

        for next in chars.by_ref() {
            if next == '%' {
                closed = true;
                break;
            }
            key.push(next);
        }

        if !closed {
            return Err(RebeccaError::PathExpansionFailed(format!(
                "unterminated variable in template {template:?}"
            )));
        }

        if key.is_empty() {
            return Err(RebeccaError::PathExpansionFailed(format!(
                "empty variable name in template {template:?}"
            )));
        }

        let value = match env.get(&key) {
            Some(value) => value,
            None => return Ok(None),
        };

        expanded.push_str(&os_str_to_lossy(&value));
    }

    Ok(Some(PathBuf::from(expanded)))
}

fn os_str_to_lossy(value: &OsStr) -> String {
    value.to_string_lossy().into_owned()
}
