//! Which way each agent is connected: by skill (no server; the agent runs `clio-memory m …`) or by
//! the thin MCP client. Setup notes it in `<data>/agents.json` (`{"claude-code": "skill", …}`), so
//! a re-run keeps each agent the way it was.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::agents::{AgentId, Env, shadowing, shadowing_note};
use super::config_file::{read_json, write_json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    Skill,
    Mcp,
}

impl Mode {
    pub fn key(self) -> &'static str {
        match self {
            Mode::Skill => "skill",
            Mode::Mcp => "mcp",
        }
    }

    pub fn from_key(s: &str) -> Option<Mode> {
        [Mode::Skill, Mode::Mcp].into_iter().find(|m| m.key() == s)
    }
}

/// The way an agent is connected when nothing was chosen or noted: by skill where the agent runs
/// commands well on its own, by MCP elsewhere.
pub fn default_mode(agent: AgentId) -> Mode {
    match agent {
        AgentId::ClaudeCode | AgentId::OpenCode => Mode::Skill,
        AgentId::Codex | AgentId::Gemini | AgentId::Antigravity => Mode::Mcp,
    }
}

/// The mode `agent` gets: the one chosen, else the one noted, else its default; with, when the
/// default by skill fell back to MCP, why. The skill way runs a bare `clio-memory`, so with none
/// on the `PATH` (or another one first) an upgrade would cut the agent off: it gets MCP instead.
/// A skill chosen or noted is kept (doctor names the problem).
pub fn resolve(
    env: &Env,
    chosen: Option<Mode>,
    noted: &BTreeMap<AgentId, Mode>,
    agent: AgentId,
) -> (Mode, Option<String>) {
    if let Some(mode) = chosen.or_else(|| noted.get(&agent).copied()) {
        return (mode, None);
    }
    let then = "corre `clio-memory setup --mode skill`";
    let why = match default_mode(agent) {
        Mode::Mcp => None,
        Mode::Skill if env.program("clio-memory").is_none() => Some(format!(
            "por MCP y no por skill: «clio-memory» no está en el PATH; agrega {} al PATH y {then}",
            env.exe.parent().unwrap_or(&env.exe).display()
        )),
        Mode::Skill => shadowing(env).map(|o| {
            format!(
                "por MCP y no por skill: {}; luego {then}",
                shadowing_note(env, &o)
            )
        }),
    };
    match why {
        Some(why) => (Mode::Mcp, Some(why)),
        None => (default_mode(agent), None),
    }
}

pub fn file(data: &Path) -> PathBuf {
    data.join("agents.json")
}

/// The file as it is: empty when there is none; an error when it cannot be read or is not a JSON
/// object.
fn read(data: &Path) -> Result<Map<String, Value>, String> {
    let path = file(data);
    match read_json(&path)? {
        Value::Object(all) => Ok(all),
        _ => Err(format!(
            "{} no es un objeto JSON: no se tocó",
            path.display()
        )),
    }
}

/// The modes noted; empty when there is no file or it cannot be read.
pub fn load(data: &Path) -> BTreeMap<AgentId, Mode> {
    read(data)
        .unwrap_or_default()
        .iter()
        .filter_map(|(a, m)| Some((AgentId::from_key(a)?, Mode::from_key(m.as_str()?)?)))
        .collect()
}

/// Notes `agent`'s mode (`None` takes its entry out). The file is written whole, 0600, and only
/// when something changes; one that cannot be read is refused, never written over (it holds
/// the other agents' modes).
pub fn set(data: &Path, agent: AgentId, mode: Option<Mode>) -> Result<(), String> {
    let mut all = read(data)?;
    let key = agent.key();
    let value = mode.map(|m| Value::from(m.key()));
    if all.get(key) == value.as_ref() {
        return Ok(());
    }
    match value {
        Some(v) => {
            all.insert(key.to_owned(), v);
        }
        None => all.retain(|k, _| k != key),
    }
    crate::agent::memory::private_dir(data)?;
    write_json(&file(data), &Value::Object(all))
}
