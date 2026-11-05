//! The `clio-memory` MCP server in each agent's configuration.
//!
//! Claude Code, Codex and Gemini CLI are driven through their own command line (`mcp add` and
//! `mcp remove`); OpenCode and Antigravity have none, so their JSON config is edited, touching
//! only our key. The server is always the binary's absolute path and `mcp` as separate arguments,
//! never one string, so a path with spaces stays whole.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Map, Value, json};

use super::agents::{AgentId, Env};
use super::config_file::{opencode_config, read_json, write_json};
use super::{command_output, run_command, run_program};

const NAME: &str = "clio-memory";

/// What an agent has under the name `clio-memory`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpState {
    Absent,
    /// This binary, with `mcp`.
    Ours,
    /// A `clio-memory` binary elsewhere, with `mcp`: an older install (`~/.cargo/bin`), ours to
    /// re-point. Also this binary when the agent has it switched off (`"enabled": false`,
    /// `"disabled": true`): ours, but not as connecting leaves it, so connecting redoes it.
    OtherPath(PathBuf),
    /// Anything else, as the agent describes it.
    Foreign(String),
}

/// What `agent` has registered as `clio-memory`.
pub fn mcp_state(env: &Env, agent: AgentId) -> Result<McpState, String> {
    let exe = &env.exe;
    match agent {
        AgentId::ClaudeCode => {
            let out = command_output(Command::new(claude(env)?).args(["mcp", "get", NAME]))?;
            Ok(if out.status.success() {
                claude_entry(exe, &String::from_utf8_lossy(&out.stdout))
            } else {
                McpState::Absent
            })
        }
        AgentId::Codex => {
            let out = command_output(&mut codex(env, &["mcp", "get", NAME, "--json"])?)?;
            if out.status.success() {
                codex_entry(exe, &String::from_utf8_lossy(&out.stdout))
            } else {
                Ok(McpState::Absent)
            }
        }
        AgentId::Gemini => {
            let entry = json_entry(&gemini_settings(env), "mcpServers")?;
            Ok(entry.map_or(McpState::Absent, |e| command_args_entry(exe, &e)))
        }
        AgentId::OpenCode => {
            let entry = json_entry(&opencode_config(&env.home), "mcp")?;
            Ok(entry.map_or(McpState::Absent, |e| opencode_entry(exe, &e)))
        }
        AgentId::Antigravity => {
            let entry = json_entry(&antigravity_config(env), "mcpServers")?;
            Ok(entry.map_or(McpState::Absent, |e| command_args_entry(exe, &e)))
        }
    }
}

/// Registers this binary as `clio-memory` in `agent`, replacing whatever has that name.
pub fn mcp_connect(env: &Env, agent: AgentId) -> Result<(), String> {
    let exe = env
        .exe
        .to_str()
        .ok_or_else(|| format!("{}: la ruta del binario no es UTF-8", env.exe.display()))?;
    match agent {
        AgentId::OpenCode => json_set(
            &opencode_config(&env.home),
            "mcp",
            Some(json!({"type": "local", "command": [exe, "mcp"], "enabled": true})),
        ),
        AgentId::Antigravity => json_set(
            &antigravity_config(env),
            "mcpServers",
            Some(json!({"command": exe, "args": ["mcp"]})),
        ),
        AgentId::ClaudeCode => cli_connect(env, agent, || {
            run_program(
                &claude(env)?,
                &["mcp", "add", "--scope", "user", NAME, "--", exe, "mcp"],
            )
        }),
        AgentId::Codex => cli_connect(env, agent, || {
            run_command(&mut codex(env, &["mcp", "add", NAME, "--", exe, "mcp"])?)
        }),
        AgentId::Gemini => cli_connect(env, agent, || {
            run_program(
                &program(env, "gemini")?,
                &["mcp", "add", "-s", "user", NAME, exe, "mcp"],
            )
        }),
    }
}

/// Connecting through an agent's command line: nothing when it is ours already, else what is
/// there is removed and `add` runs.
fn cli_connect(
    env: &Env,
    agent: AgentId,
    add: impl FnOnce() -> Result<String, String>,
) -> Result<(), String> {
    match mcp_state(env, agent)? {
        McpState::Ours => return Ok(()),
        McpState::Absent => {}
        McpState::OtherPath(_) | McpState::Foreign(_) => mcp_disconnect(env, agent)?,
    }
    add().map(drop)
}

/// Removes `clio-memory` from `agent`; nothing to do when it is not there.
pub fn mcp_disconnect(env: &Env, agent: AgentId) -> Result<(), String> {
    match agent {
        AgentId::OpenCode => json_set(&opencode_config(&env.home), "mcp", None),
        AgentId::Antigravity => json_set(&antigravity_config(env), "mcpServers", None),
        AgentId::ClaudeCode => cli_disconnect(env, agent, || {
            run_program(&claude(env)?, &["mcp", "remove", "--scope", "user", NAME])
        }),
        AgentId::Codex => cli_disconnect(env, agent, || {
            run_command(&mut codex(env, &["mcp", "remove", NAME])?)
        }),
        AgentId::Gemini => cli_disconnect(env, agent, || {
            run_program(
                &program(env, "gemini")?,
                &["mcp", "remove", "-s", "user", NAME],
            )
        }),
    }
}

/// Disconnecting through an agent's command line: `remove` runs only when something is there.
fn cli_disconnect(
    env: &Env,
    agent: AgentId,
    remove: impl FnOnce() -> Result<String, String>,
) -> Result<(), String> {
    if mcp_state(env, agent)? == McpState::Absent {
        return Ok(());
    }
    remove().map(drop)
}

fn program(env: &Env, name: &str) -> Result<PathBuf, String> {
    env.program(name)
        .ok_or_else(|| format!("no se encontró `{name}` en el PATH"))
}

/// `CLIO_MEMORY_CLAUDE` if set, else the `claude` on the `PATH`.
fn claude(env: &Env) -> Result<PathBuf, String> {
    env.claude
        .clone()
        .map_or_else(|| program(env, "claude"), Ok)
}

/// `codex args`, with `CODEX_HOME` exactly as the environment has it.
fn codex(env: &Env, args: &[&str]) -> Result<Command, String> {
    let mut cmd = Command::new(program(env, "codex")?);
    cmd.args(args);
    match &env.codex_home {
        Some(home) => cmd.env("CODEX_HOME", home),
        None => cmd.env_remove("CODEX_HOME"),
    };
    Ok(cmd)
}

fn gemini_settings(env: &Env) -> PathBuf {
    env.home.join(".gemini/settings.json")
}

fn antigravity_config(env: &Env) -> PathBuf {
    env.home.join(".gemini/config/mcp_config.json")
}

/// Whether the agent has the entry switched off: OpenCode's and Codex's `"enabled": false`, the
/// `"disabled": true` of the `mcpServers` configs.
fn switched_off(entry: &Value) -> bool {
    entry.get("enabled") == Some(&Value::Bool(false))
        || entry.get("disabled") == Some(&Value::Bool(true))
}

/// `state` for an entry that may be switched off: this binary switched off is to be redone,
/// as a binary elsewhere is (see `McpState::OtherPath`).
fn unless_off(exe: &Path, state: McpState, entry: &Value) -> McpState {
    match state {
        McpState::Ours if switched_off(entry) => McpState::OtherPath(exe.to_path_buf()),
        state => state,
    }
}

/// A server that runs `command args`.
fn classify(exe: &Path, command: &str, args: &[&str]) -> McpState {
    let path = Path::new(command);
    if args == ["mcp"] {
        if path == exe {
            return McpState::Ours;
        }
        if path.file_name() == Some(OsStr::new(NAME)) {
            return McpState::OtherPath(path.to_path_buf());
        }
    }
    McpState::Foreign(
        std::iter::once(command)
            .chain(args.iter().copied())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// `claude mcp get clio-memory`, which prints (Claude Code 2.1) a stdio server as
///
/// ```text
/// clio-memory:
///   Scope: User config (available in all your projects)
///   Status: ✔ Connected
///   Type: stdio
///   Command: /home/x/.local/bin/clio-memory
///   Args: mcp
///   Environment:
///
/// To remove this server, run: claude mcp remove clio-memory -s user
/// ```
///
/// with the arguments joined by spaces, and a remote one with `URL:` instead. `get` also finds
/// the local and project entries of the directory it runs in; only the user's scope is what setup
/// adds and removes, so an entry in any other scope reads as `Absent`.
fn claude_entry(exe: &Path, text: &str) -> McpState {
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.trim_start().strip_prefix(key))
            .map(str::trim_end)
    };
    if !field("Scope: ").is_some_and(|s| s.starts_with("User config")) {
        return McpState::Absent;
    }
    match field("Command: ") {
        Some(command) => {
            let args: Vec<&str> = field("Args: ").unwrap_or("").split_whitespace().collect();
            classify(exe, command, &args)
        }
        None => McpState::Foreign(field("URL: ").unwrap_or("un servidor remoto").to_owned()),
    }
}

/// `codex mcp get clio-memory --json`, which prints (codex-cli 0.159.2)
///
/// ```text
/// {"name": "clio-memory", "enabled": true, "disabled_reason": null,
///  "transport": {"type": "stdio", "command": "/bin/true", "args": ["mcp"],
///                "env": null, "env_vars": [], "cwd": null},
///  "enabled_tools": null, "disabled_tools": null,
///  "startup_timeout_sec": null, "tool_timeout_sec": null}
/// ```
fn codex_entry(exe: &Path, text: &str) -> Result<McpState, String> {
    let v: Value = serde_json::from_str(text)
        .map_err(|e| format!("`codex mcp get {NAME} --json` no respondió JSON: {e}"))?;
    let transport = v.get("transport").unwrap_or(&Value::Null);
    Ok(unless_off(exe, command_args_entry(exe, transport), &v))
}

/// An entry `{"command": "...", "args": [...]}` (Gemini, Antigravity, Codex's transport).
fn command_args_entry(exe: &Path, entry: &Value) -> McpState {
    let command = entry.get("command").and_then(Value::as_str);
    let args = match entry.get("args") {
        None | Some(Value::Null) => Some(Vec::new()),
        Some(a) => strings(a),
    };
    match (command, args) {
        (Some(command), Some(args)) => unless_off(exe, classify(exe, command, &args), entry),
        _ => McpState::Foreign(entry.to_string()),
    }
}

/// OpenCode's entry: `{"type": "local", "command": ["<program>", "<arg>", ...]}`.
fn opencode_entry(exe: &Path, entry: &Value) -> McpState {
    match entry.get("command").and_then(strings).as_deref() {
        Some([command, args @ ..]) => unless_off(exe, classify(exe, command, args), entry),
        _ => McpState::Foreign(entry.to_string()),
    }
}

/// A JSON array of strings.
fn strings(v: &Value) -> Option<Vec<&str>> {
    v.as_array()?.iter().map(Value::as_str).collect()
}

/// The config's top level and its server table (`table`), checked to be objects: a config of
/// another shape is never indexed into nor rewritten.
fn tables<'a>(
    path: &Path,
    config: &'a mut Value,
    table: &str,
) -> Result<&'a mut Map<String, Value>, String> {
    let root = config
        .as_object_mut()
        .ok_or_else(|| format!("{} no es un objeto JSON: no se tocó", path.display()))?;
    match root.get(table) {
        None | Some(Value::Object(_)) => Ok(root),
        Some(_) => Err(format!(
            "{}: «{table}» no es un objeto JSON: no se tocó",
            path.display()
        )),
    }
}

/// Our entry in `path`'s `table`, if there is one.
fn json_entry(path: &Path, table: &str) -> Result<Option<Value>, String> {
    let mut config = read_json(path)?;
    let root = tables(path, &mut config, table)?;
    Ok(root.get(table).and_then(|t| t.get(NAME)).cloned())
}

/// Puts `entry` as ours in `path`'s `table` (in the place it had, if it was there), or with
/// `None` removes ours and then `table` if that leaves it empty. Every other key keeps its place;
/// a file that would not change is not written.
fn json_set(path: &Path, table: &str, entry: Option<Value>) -> Result<(), String> {
    let mut config = read_json(path)?;
    let root = tables(path, &mut config, table)?;
    let servers = root.get(table).and_then(Value::as_object);
    if servers.and_then(|s| s.get(NAME)) == entry.as_ref() {
        return Ok(());
    }
    match entry {
        Some(entry) => {
            let servers = root
                .entry(table)
                .or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(servers) = servers {
                servers.insert(NAME.to_owned(), entry);
            }
        }
        None => {
            // `retain`: with `preserve_order`, `remove` would move the last key into the hole.
            if let Some(Value::Object(servers)) = root.get_mut(table) {
                servers.retain(|k, _| k != NAME);
                if servers.is_empty() {
                    root.retain(|k, _| k != table);
                }
            }
        }
    }
    write_json(path, &config)
}
