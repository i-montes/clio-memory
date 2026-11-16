//! Claude Code's permission rule for the skill way: `Bash(clio-memory m:*)` in
//! `permissions.allow` of `~/.claude/settings.json`, so the agent runs `clio-memory m …` without
//! asking each time. Only that one entry is ever added or taken out; every other key stays where
//! it was (see `config_file`).

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::config_file::{read_json, write_json};

pub const RULE: &str = "Bash(clio-memory m:*)";

fn settings(home: &Path) -> PathBuf {
    home.join(".claude/settings.json")
}

fn not_object(path: &Path, what: &str) -> String {
    format!(
        "{}: {what} no es un objeto JSON: no se tocó",
        path.display()
    )
}

/// Adds the rule; whether anything changed.
pub fn allow(home: &Path) -> Result<bool, String> {
    let path = settings(home);
    let mut config = read_json(&path)?;
    let root = config
        .as_object_mut()
        .ok_or_else(|| not_object(&path, "el archivo"))?;
    let list = root
        .entry("permissions")
        .or_insert_with(|| Value::Object(Default::default()))
        .as_object_mut()
        .ok_or_else(|| not_object(&path, "`permissions`"))?
        .entry("allow")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| {
            format!(
                "{}: `permissions.allow` no es un arreglo: no se tocó",
                path.display()
            )
        })?;
    if list.iter().any(|r| r == RULE) {
        return Ok(false);
    }
    list.push(Value::from(RULE));
    write_json(&path, &config)?;
    Ok(true)
}

/// Whether the rule is there; `false` when the file cannot be read.
pub fn has(home: &Path) -> bool {
    read_json(&settings(home)).is_ok_and(|c| {
        c.pointer("/permissions/allow")
            .and_then(Value::as_array)
            .is_some_and(|l| l.iter().any(|r| r == RULE))
    })
}

/// Takes the rule out; whether anything changed (a missing file changes nothing).
pub fn revoke(home: &Path) -> Result<bool, String> {
    let path = settings(home);
    let mut config = read_json(&path)?;
    let Some(list) = config
        .get_mut("permissions")
        .and_then(|p| p.get_mut("allow"))
        .and_then(Value::as_array_mut)
    else {
        return Ok(false);
    };
    let before = list.len();
    // `retain`: the other rules keep their order.
    list.retain(|r| r != RULE);
    if list.len() == before {
        return Ok(false);
    }
    write_json(&path, &config)?;
    Ok(true)
}
