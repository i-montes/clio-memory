//! What setup does with a choice of agents: connect them, disconnect them, report on them.
//!
//! Every action is a `Step` with its own outcome, and a step that fails never stops the others:
//! an agent whose program breaks leaves the rest connected, and says why. Nothing of the
//! person's (another server or skill called `clio-memory`) is replaced without a yes or
//! `replace`; a declined agent gets nothing written at all.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::agents::{AgentId, Env, detect};
use super::instructions::{add_instructions, instructions_file, remove_instructions};
use super::mcp::{McpState, mcp_connect, mcp_disconnect, mcp_state};
use super::skill::{SkillDir, SkillFile, place_skill, placement, skill_state};
use super::{ensure_sleep, remove_sleep, sleep_is_clio_memorys, sleep_present};

/// What to connect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub agents: Vec<AgentId>,
    /// Add the instructions line to each of them.
    pub instructions: bool,
    /// Replace another server or skill called `clio-memory` without asking.
    pub replace: bool,
    /// The connected agents not in `agents` are disconnected (only the TUI sets it: flags only
    /// add).
    pub exclusive: bool,
}

/// One thing done (or not), for an agent or for all of them (`agent: None`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub agent: Option<AgentId>,
    pub what: String,
    pub outcome: Result<String, String>,
}

/// `Step::what` of the sleep's step.
pub const SLEEP: &str = "sueño";
/// `Step::what` of the step that deletes this binary (always the last).
pub const BINARY: &str = "binario";
/// What `agent_report` says of an agent that is there but not connected.
pub const NOT_CONNECTED: &str = "no conectado";

impl Step {
    fn new(agent: Option<AgentId>, what: &str, outcome: Result<String, String>) -> Step {
        Step {
            agent,
            what: what.to_owned(),
            outcome,
        }
    }

    /// The step as one line for the person: `✔ Claude Code: …`, `✖ Codex: …`.
    pub fn line(&self) -> String {
        let (mark, text) = match &self.outcome {
            Ok(text) => ("✔", text),
            Err(text) => ("✖", text),
        };
        match self.agent {
            Some(agent) => format!("{mark} {}: {text}", agent.label()),
            None => format!("{mark} {text}"),
        }
    }
}

fn is_connected(state: &McpState) -> bool {
    matches!(state, McpState::Ours | McpState::OtherPath(_))
}

/// The agents with a `clio-memory` server of ours, current or to be redone.
pub fn connected(env: &Env) -> Vec<AgentId> {
    AgentId::ALL
        .into_iter()
        .filter(|&a| mcp_state(env, a).is_ok_and(|s| is_connected(&s)))
        .collect()
}

/// Whether each agent is connected; `None` when its state cannot be read (its program is not
/// there, or fails).
type Connections = BTreeMap<AgentId, Option<bool>>;

fn connections(env: &Env) -> Connections {
    AgentId::ALL
        .into_iter()
        .map(|a| (a, mcp_state(env, a).ok().map(|s| is_connected(&s))))
        .collect()
}

/// After trying to change `agent`: what it is now, as far as can be read.
fn reread(env: &Env, agent: AgentId) -> Option<bool> {
    mcp_state(env, agent).ok().map(|s| is_connected(&s))
}

/// The agents the skill is for: the connected ones, and those whose state cannot be read but
/// that see our skill now (what cannot be checked is left as it is, never taken away on a guess).
fn skill_agents(env: &Env, conn: &Connections) -> Vec<AgentId> {
    conn.iter()
        .filter(|(a, c)| c.unwrap_or_else(|| sees_our_skill(&env.home, **a)))
        .map(|(a, _)| *a)
        .collect()
}

/// The skill folders `agent` reads.
fn read_by(agent: AgentId) -> impl Iterator<Item = SkillDir> {
    SkillDir::ALL
        .into_iter()
        .filter(move |d| d.readers().contains(&agent))
}

fn sees_our_skill(home: &Path, agent: AgentId) -> bool {
    read_by(agent).any(|d| {
        matches!(
            skill_state(&d.path(home)),
            SkillFile::Current | SkillFile::Old
        )
    })
}

/// Connects `sel.agents`; then, with `exclusive`, disconnects the others; then puts the skill
/// where every connected agent sees it, and installs the nightly sleep.
pub fn apply(env: &Env, sel: &Selection, ask: &dyn Fn(&str) -> bool) -> Vec<Step> {
    let mut steps = Vec::new();
    let found: Vec<AgentId> = detect(env).into_iter().map(|f| f.agent).collect();
    let mut conn = connections(env);
    let chosen: Vec<AgentId> = AgentId::ALL
        .into_iter()
        .filter(|a| sel.agents.contains(a))
        .collect();
    // Where the skill is going, to ask about a foreign one before anything is written.
    let planned: Vec<AgentId> = AgentId::ALL
        .into_iter()
        .filter(|a| chosen.contains(a) || (!sel.exclusive && conn[a] == Some(true)))
        .collect();
    let planned_dirs = placement(&planned);
    let mut approved: Vec<PathBuf> = Vec::new();
    // Who sees our skill now: outside `exclusive` nobody loses it (flags only add, and a no
    // leaves that agent as it was).
    let saw: Vec<AgentId> = AgentId::ALL
        .into_iter()
        .filter(|a| sees_our_skill(&env.home, *a))
        .collect();
    // Whether anything was connected or disconnected: if not, there is nothing to place.
    let mut touched = false;

    for agent in chosen.iter().copied() {
        let label = agent.label();
        if !found.contains(&agent) {
            steps.push(Step::new(
                Some(agent),
                "MCP",
                Err(format!("no se encontró {label}")),
            ));
            continue;
        }
        let state = match mcp_state(env, agent) {
            Ok(state) => state,
            Err(e) => {
                steps.push(Step::new(Some(agent), "MCP", Err(e)));
                continue;
            }
        };
        let foreign_skills: Vec<PathBuf> = planned_dirs
            .iter()
            .filter(|d| d.readers().contains(&agent))
            .map(|d| d.path(&env.home))
            .filter(|p| skill_state(p) == SkillFile::Foreign)
            .collect();
        let foreign_mcp = matches!(state, McpState::Foreign(_));
        if (foreign_mcp || !foreign_skills.is_empty()) && !sel.replace {
            let what = match (foreign_mcp, !foreign_skills.is_empty()) {
                (true, true) => "un servidor MCP y una skill llamados clio-memory",
                (true, false) => "un servidor MCP llamado clio-memory",
                _ => "una skill clio-memory distinta",
            };
            if !ask(&format!("{label} ya tiene {what}. ¿Reemplazarlo?")) {
                steps.push(Step::new(
                    Some(agent),
                    "MCP",
                    Err(format!(
                        "{label} ya tiene {what}: no se cambió nada (usa --replace para reemplazarlo)"
                    )),
                ));
                continue;
            }
            approved.extend(foreign_skills);
        }
        let exe = env.exe.display();
        let done = match &state {
            McpState::Ours => Ok("el servidor MCP clio-memory ya está registrado".to_owned()),
            other => mcp_connect(env, agent).map(|()| match other {
                McpState::OtherPath(p) if *p == env.exe => {
                    "el servidor MCP clio-memory estaba desactivado; reactivado".to_owned()
                }
                McpState::OtherPath(p) => format!(
                    "el servidor MCP clio-memory apuntaba a {}; ahora a {exe}",
                    p.display()
                ),
                _ => format!("servidor MCP clio-memory registrado: {exe} mcp"),
            }),
        };
        let ok = done.is_ok();
        // A failed connect may have removed what was there before adding: read it again.
        conn.insert(agent, if ok { Some(true) } else { reread(env, agent) });
        steps.push(Step::new(Some(agent), "MCP", done));
        touched |= ok;
        if ok && sel.instructions {
            steps.push(Step::new(
                Some(agent),
                "instrucciones",
                add_line(env, agent),
            ));
        }
    }

    if sel.exclusive {
        let extra: Vec<AgentId> = AgentId::ALL
            .into_iter()
            .filter(|a| conn[a] == Some(true) && !chosen.contains(a))
            .collect();
        let still: Vec<AgentId> = chosen
            .iter()
            .copied()
            .filter(|a| conn[a] == Some(true))
            .collect();
        for agent in extra {
            touched = true;
            steps.extend(disconnect(env, agent, &still, &mut conn));
        }
    }
    if !touched {
        return steps;
    }

    // The connected agents, and those that saw the skill unless `exclusive` took them out.
    let mut agents = skill_agents(env, &conn);
    for a in saw {
        if !agents.contains(&a) && (!sel.exclusive || chosen.contains(&a)) {
            agents.push(a);
        }
    }
    agents.sort();
    let blocked = placement(&agents)
        .iter()
        .map(|d| d.path(&env.home))
        .any(|p| skill_state(&p) == SkillFile::Foreign && !approved.contains(&p));
    steps.extend(skill_step(env, &agents, sel.replace || !blocked));
    // The sleep is part of the memory, once some agent uses it.
    if !agents.is_empty() {
        steps.push(Step::new(None, SLEEP, ensure_sleep(env)));
    }
    steps
}

/// The instructions line in `agent`, as a step's outcome.
fn add_line(env: &Env, agent: AgentId) -> Result<String, String> {
    let file = instructions_file(env, agent);
    Ok(match add_instructions(env, agent)? {
        Some(at) => format!("línea añadida a {}", at.display()),
        None => format!(
            "la línea de {} ya está",
            file.file_name()
                .unwrap_or(file.as_os_str())
                .to_string_lossy()
        ),
    })
}

/// Takes `agent`'s line out; `still` stay connected (see `remove_instructions`).
fn remove_line(env: &Env, agent: AgentId, still: &[AgentId]) -> Option<Step> {
    let done = match remove_instructions(env, agent, still) {
        Ok(None) => return None,
        Ok(Some(file)) => Ok(format!("línea quitada de {}", file.display())),
        Err(e) => Err(e),
    };
    Some(Step::new(Some(agent), "instrucciones", done))
}

/// Disconnects an agent that is connected: its server, then its line.
fn disconnect(env: &Env, agent: AgentId, still: &[AgentId], conn: &mut Connections) -> Vec<Step> {
    let done = mcp_disconnect(env, agent).map(|()| "servidor MCP clio-memory quitado".to_owned());
    let ok = done.is_ok();
    conn.insert(agent, if ok { Some(false) } else { reread(env, agent) });
    let mut steps = vec![Step::new(Some(agent), "MCP", done)];
    if ok {
        steps.extend(remove_line(env, agent, still));
    }
    steps
}

/// The skill placed for `agents` (and taken from the folders they do not need), as one step;
/// none when there was nothing to do or say.
fn skill_step(env: &Env, agents: &[AgentId], replace: bool) -> Option<Step> {
    let done = place_skill(&env.home, agents, replace).map(|said| {
        said.iter()
            .map(|l| match l.strip_prefix("skill al día en ") {
                Some(at) => format!("la skill ya está en {at}"),
                None => l.clone(),
            })
            .collect::<Vec<_>>()
            .join("; ")
    });
    if done.as_ref().is_ok_and(String::is_empty) {
        return None;
    }
    Some(Step::new(None, "skill", done))
}

/// Takes clio-memory out of `agents` (and the skill out of the folders only they needed), or
/// with `None` out of every agent, with the nightly sleep, and last this binary when it is where
/// the installer put it and nothing failed before. A server or skill called `clio-memory` that
/// is not ours is never touched; the memory and the models stay.
pub fn uninstall(env: &Env, agents: Option<&[AgentId]>) -> Vec<Step> {
    let mut steps = Vec::new();
    let found: Vec<AgentId> = detect(env).into_iter().map(|f| f.agent).collect();
    let mut conn = connections(env);
    let leaving: Vec<AgentId> = match agents {
        Some(list) => AgentId::ALL
            .into_iter()
            .filter(|a| list.contains(a))
            .collect(),
        None => AgentId::ALL.to_vec(),
    };
    let still: Vec<AgentId> = AgentId::ALL
        .into_iter()
        .filter(|a| !leaving.contains(a) && conn[a] == Some(true))
        .collect();
    for agent in leaving.iter().copied() {
        match mcp_state(env, agent) {
            Ok(state) if is_connected(&state) => {
                steps.extend(disconnect(env, agent, &still, &mut conn));
                continue;
            }
            Ok(McpState::Foreign(_)) => steps.push(Step::new(
                Some(agent),
                "MCP",
                Ok("el servidor MCP llamado clio-memory no es este: no se tocó".to_owned()),
            )),
            Ok(_) => {}
            // Asked for by name, or there to be asked: it says why it could not be read. An
            // agent that is not on this machine has nothing to take out.
            Err(e) if agents.is_some() || found.contains(&agent) => {
                steps.push(Step::new(Some(agent), "MCP", Err(e)));
            }
            Err(_) => {}
        }
        steps.extend(remove_line(env, agent, &still));
    }

    let rest = match agents {
        Some(_) => skill_agents(env, &conn)
            .into_iter()
            .filter(|a| !leaving.contains(a))
            .collect(),
        None => Vec::new(),
    };
    steps.extend(skill_step(env, &rest, false));
    if agents.is_some() {
        return steps;
    }

    if sleep_present(&env.home) {
        steps.push(Step::new(
            None,
            SLEEP,
            if sleep_is_clio_memorys(&env.home) {
                remove_sleep(&env.home, env.service).map(|()| "sueño nocturno quitado".to_owned())
            } else {
                Ok("hay otro sueño nocturno (de Clio u otro binario): no se tocó".to_owned())
            },
        ));
    }
    if placed_by_installer(env) {
        let exe = env.exe.display();
        // With something left undone, the binary stays, so `uninstall` can run again.
        let done = if steps.iter().any(|s| s.outcome.is_err()) {
            Err(format!(
                "se dejó {exe}: algo no se pudo quitar; corre `clio-memory uninstall` otra vez"
            ))
        } else {
            std::fs::remove_file(&env.exe)
                .map(|()| format!("binario borrado: {exe}"))
                .map_err(|e| format!("{exe}: {e}"))
        };
        steps.push(Step::new(None, BINARY, done));
    }
    steps
}

/// Whether this binary is in the folder `install.sh` puts it in: `env.bin_dir`
/// (`$CLIO_MEMORY_BIN_DIR`), else `~/.local/bin`.
fn placed_by_installer(env: &Env) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let Some(dir) = env.exe.parent().map(canon) else {
        return false;
    };
    env.bin_dir
        .iter()
        .cloned()
        .chain([env.home.join(".local/bin")])
        .any(|d| canon(&d) == dir)
}

/// How an agent found on this machine is, for `doctor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    /// Connected, with the current skill.
    Ok,
    /// Not connected (or, as far as can be told, not): nothing to fix.
    Off,
    /// Something to fix.
    Fail,
}

/// Each agent found on this machine: how it is and what is the matter: `conectado`,
/// `no conectado` (`NOT_CONNECTED`), or what is wrong. An agent whose state cannot be read is
/// `Off` while it does not see our skill (most likely it was never connected), else `Fail`.
pub fn agent_report(env: &Env) -> Vec<(AgentId, Health, String)> {
    detect(env)
        .into_iter()
        .map(|f| {
            let agent = f.agent;
            let (health, text) = match mcp_state(env, agent) {
                Err(e) if sees_our_skill(&env.home, agent) => (Health::Fail, e),
                Err(e) => (Health::Off, format!("no se pudo leer su estado: {e}")),
                Ok(McpState::Absent) => (Health::Off, NOT_CONNECTED.to_owned()),
                Ok(McpState::Foreign(what)) => (
                    Health::Fail,
                    format!("otro servidor MCP se llama clio-memory: {what}"),
                ),
                Ok(McpState::OtherPath(p)) if p == env.exe => {
                    (Health::Fail, "el MCP está desactivado".to_owned())
                }
                Ok(McpState::OtherPath(p)) => {
                    (Health::Fail, format!("el MCP apunta a {}", p.display()))
                }
                Ok(McpState::Ours) => skill_report(&env.home, agent),
            };
            (agent, health, text)
        })
        .collect()
}

/// A connected agent's skill: ours and current in a folder it reads, and nothing foreign there.
fn skill_report(home: &Path, agent: AgentId) -> (Health, String) {
    let states: Vec<(PathBuf, SkillFile)> = read_by(agent)
        .map(|d| d.path(home))
        .map(|p| {
            let s = skill_state(&p);
            (p, s)
        })
        .collect();
    let first = |want: SkillFile| states.iter().find(|(_, s)| *s == want).map(|(p, _)| p);
    if let Some(p) = first(SkillFile::Foreign) {
        (Health::Fail, format!("skill ajena en {}", p.display()))
    } else if first(SkillFile::Current).is_some() {
        (Health::Ok, "conectado".to_owned())
    } else if let Some(p) = first(SkillFile::Old) {
        (
            Health::Fail,
            format!("skill desactualizada en {}", p.display()),
        )
    } else {
        (Health::Fail, "falta la skill".to_owned())
    }
}
