//! The instructions line in each agent: the one line that tells the agent clio-memory is its
//! memory, in the file where that agent reads its standing instructions.
//!
//! Claude Code, Codex and Gemini CLI read one file; Antigravity reads Gemini's `GEMINI.md`, so
//! the two share it. OpenCode has no such file of ours to append to (its `AGENTS.md` is the
//! person's), so it gets `clio-memory.md` beside its config and that file's absolute path in the
//! `instructions` array of `opencode.json`. Only lines equal to ours are ever removed; every
//! other byte of a file, and every other key and entry of the config, stays where it was. A file
//! left with nothing but blank lines is deleted (unless it is a link).

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::agents::{AgentId, Env};
use super::config_file::{opencode_config, read_json, write_json};
use super::{INSTRUCTIONS_LINE, write_keeping_mode, write_private};

/// The file `agent`'s line goes in.
pub fn instructions_file(env: &Env, agent: AgentId) -> PathBuf {
    match agent {
        AgentId::ClaudeCode => env.home.join(".claude/CLAUDE.md"),
        AgentId::Codex => {
            let dir = codex_dir(env);
            // Codex reads the override instead of `AGENTS.md` when it has something in it.
            let over = dir.join("AGENTS.override.md");
            if std::fs::metadata(&over).is_ok_and(|m| m.len() > 0) {
                over
            } else {
                dir.join("AGENTS.md")
            }
        }
        AgentId::Gemini | AgentId::Antigravity => env.home.join(".gemini/GEMINI.md"),
        AgentId::OpenCode => opencode_md(env),
    }
}

/// `CODEX_HOME`, else `~/.codex`.
fn codex_dir(env: &Env) -> PathBuf {
    env.codex_home
        .clone()
        .unwrap_or_else(|| env.home.join(".codex"))
}

fn opencode_md(env: &Env) -> PathBuf {
    env.home.join(".config/opencode/clio-memory.md")
}

/// Whether `agent` has the line (for OpenCode: the file with it, and the config pointing at it).
pub fn has_instructions(env: &Env, agent: AgentId) -> bool {
    let has_line = |text: &str| text.lines().any(|l| l.trim() == INSTRUCTIONS_LINE);
    let file = std::fs::read_to_string(instructions_file(env, agent));
    if !file.as_deref().is_ok_and(has_line) {
        return false;
    }
    if agent != AgentId::OpenCode {
        return true;
    }
    let path = config_path(env);
    let Ok(mut config) = read_json(&path) else {
        return false;
    };
    matches!(
        entries(&path, &mut config),
        Ok(Some(list)) if holds(list, &opencode_md(env))
    )
}

/// Adds the line to `agent`; `None` when it was there already, else the file it went in.
pub fn add_instructions(env: &Env, agent: AgentId) -> Result<Option<PathBuf>, String> {
    if has_instructions(env, agent) {
        return Ok(None);
    }
    let file = instructions_file(env, agent);
    if agent != AgentId::OpenCode {
        let text = read_text(&file)?;
        if !text.lines().any(|l| l.trim() == INSTRUCTIONS_LINE) {
            write_through(&file, &format!("{text}{}{INSTRUCTIONS_LINE}\n", sep(&text)))?;
        }
        return Ok(Some(file));
    }

    // The config is read and checked before anything is written: one that cannot be edited
    // leaves no `clio-memory.md` behind.
    let path = config_path(env);
    let mut config = read_json(&path)?;
    let mut changed = false;
    {
        let root = object(&path, &mut config)?;
        let list = root
            .entry("instructions")
            .or_insert_with(|| Value::Array(Vec::new()));
        let Value::Array(list) = list else {
            return Err(not_array(&path));
        };
        if !holds(list, &file) {
            list.push(Value::String(path_text(&file)?));
            changed = true;
        }
    }
    write_private(&file, &format!("{INSTRUCTIONS_LINE}\n"))?;
    if changed {
        write_json(&path, &config)?;
    }
    Ok(Some(file))
}

/// Removes the line from `agent`; `None` when there was nothing to remove, else the file it was
/// removed from. `still` are the agents that stay connected: Gemini CLI and Antigravity share
/// `GEMINI.md`, so the line stays while the other is in it.
pub fn remove_instructions(
    env: &Env,
    agent: AgentId,
    still: &[AgentId],
) -> Result<Option<PathBuf>, String> {
    let file = instructions_file(env, agent);
    let shared_with = match agent {
        AgentId::Gemini => Some(AgentId::Antigravity),
        AgentId::Antigravity => Some(AgentId::Gemini),
        _ => None,
    };
    if shared_with.is_some_and(|other| still.contains(&other)) {
        return Ok(None);
    }
    if agent == AgentId::OpenCode {
        return remove_opencode(env, file);
    }
    // Codex reads one of two files, and which one changes when the person writes an override:
    // the line goes from whichever holds it.
    let files = match agent {
        AgentId::Codex => ["AGENTS.override.md", "AGENTS.md"]
            .map(|name| codex_dir(env).join(name))
            .to_vec(),
        _ => vec![file],
    };
    let mut removed = None;
    for file in files {
        let text = read_text(&file)?;
        if let Some(rest) = without_line(&text) {
            // Only blank lines left (setup most likely created it): the file goes, unless it is
            // a link, which stays and is emptied.
            let link = std::fs::symlink_metadata(&file).is_ok_and(|m| m.file_type().is_symlink());
            if rest.trim().is_empty() && !link {
                std::fs::remove_file(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            } else {
                write_through(&file, &rest)?;
            }
            removed.get_or_insert(file);
        }
    }
    Ok(removed)
}

/// OpenCode: our entry out of `instructions` (and the key with it when that leaves it empty),
/// then `clio-memory.md`. The config goes first, so one that cannot be edited deletes nothing.
fn remove_opencode(env: &Env, md: PathBuf) -> Result<Option<PathBuf>, String> {
    let path = config_path(env);
    let mut config = read_json(&path)?;
    let mut changed = false;
    if let Some(list) = entries(&path, &mut config)? {
        let before = list.len();
        // `retain`, not a swap: the other entries keep their order.
        list.retain(|v| v.as_str() != md.to_str());
        changed = list.len() != before;
        if changed && list.is_empty() {
            // `retain`: with `preserve_order`, `remove` would move the last key into the hole.
            object(&path, &mut config)?.retain(|k, _| k != "instructions");
        }
    }
    if changed {
        write_json(&path, &config)?;
    }
    let deleted = match std::fs::remove_file(&md) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(format!("{}: {e}", md.display())),
    };
    Ok((changed || deleted).then_some(md))
}

fn config_path(env: &Env) -> PathBuf {
    opencode_config(&env.home)
}

/// The config as an object, or an error naming the file (never indexed into blindly).
fn object<'a>(path: &Path, config: &'a mut Value) -> Result<&'a mut Map<String, Value>, String> {
    config
        .as_object_mut()
        .ok_or_else(|| format!("{} no es un objeto JSON: no se tocó", path.display()))
}

/// The `instructions` array of the config, if it has one.
fn entries<'a>(path: &Path, config: &'a mut Value) -> Result<Option<&'a mut Vec<Value>>, String> {
    match object(path, config)?.get_mut("instructions") {
        None => Ok(None),
        Some(Value::Array(list)) => Ok(Some(list)),
        Some(_) => Err(not_array(path)),
    }
}

fn not_array(path: &Path) -> String {
    format!(
        "{}: `instructions` no es un arreglo: no se tocó",
        path.display()
    )
}

/// Whether `list` has `file`'s path, compared exactly.
fn holds(list: &[Value], file: &Path) -> bool {
    list.iter().any(|v| v.as_str() == file.to_str())
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("{}: la ruta no es UTF-8", path.display()))
}

/// The file's text; a file that is not there is empty, one that cannot be read is an error (it
/// is never written over).
fn read_text(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// What goes between `text` and a line added after it: a newline unless it ends in one already.
fn sep(text: &str) -> &'static str {
    if text.is_empty() || text.ends_with('\n') {
        ""
    } else {
        "\n"
    }
}

/// `text` without the lines that are ours (compared trimmed); `None` when it had none.
fn without_line(text: &str) -> Option<String> {
    let kept: String = text
        .split_inclusive('\n')
        .filter(|l| l.trim() != INSTRUCTIONS_LINE)
        .collect();
    (kept.len() != text.len()).then_some(kept)
}

/// `write_keeping_mode`, through a symlink (a dotfiles repository) rather than replacing it.
fn write_through(path: &Path, text: &str) -> Result<(), String> {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    write_keeping_mode(&path, text)
}
