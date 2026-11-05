#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! The `clio-memory` MCP server in each agent: through the agent's own command line where it has
//! one (fake `claude`, `codex` and `gemini` here), through its JSON config where it does not.
//! Everything runs under a temporary home.

mod common;

use std::path::{Path, PathBuf};

use clio_memory::setup::agents::{AgentId, Env};
use clio_memory::setup::mcp::{McpState, mcp_connect, mcp_disconnect, mcp_state};
use common::fakes::{
    CLAUDE_LOCAL_SCOPE, CLAUDE_USER_SCOPE, calls, claude_get_output, claude_registry,
    codex_get_json, codex_registry, fake_claude_mcp, fake_codex, fake_gemini,
};
use serde_json::{Value, json};

const EXE: &str = "/opt/my bin/clio-memory";

struct Rig {
    _t: tempfile::TempDir,
    home: PathBuf,
    bin: PathBuf,
    codex_home: PathBuf,
}

fn rig(codex_fails: bool) -> Rig {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let bin = t.path().join("bin");
    let codex_home = t.path().join("codex home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&codex_home).unwrap();
    fake_claude_mcp(&bin);
    fake_codex(&bin, codex_fails);
    fake_gemini(&bin, &home);
    Rig {
        _t: t,
        home,
        bin,
        codex_home,
    }
}

fn no_service(_: &[&str]) -> Result<(), String> {
    Ok(())
}

impl Rig {
    fn env(&self) -> Env<'static> {
        Env {
            home: self.home.clone(),
            exe: PathBuf::from(EXE),
            path: vec![self.bin.clone()],
            codex_home: Some(self.codex_home.clone()),
            claude: None,
            bin_dir: None,
            service: &no_service,
        }
    }

    fn opencode(&self) -> PathBuf {
        self.home.join(".config/opencode/opencode.json")
    }

    fn antigravity(&self) -> PathBuf {
        self.home.join(".gemini/config/mcp_config.json")
    }
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn write(path: &Path, v: &Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_string_pretty(v).unwrap() + "\n").unwrap();
}

/// The `mcp add` call a fake received.
fn add_call(r: &Rig, program: &str) -> Vec<String> {
    calls(&r.bin, program)
        .into_iter()
        .find(|c| c.first().map(String::as_str) == Some("mcp") && c[1] == "add")
        .unwrap_or_else(|| panic!("{program} got no `mcp add`"))
}

#[test]
fn connect_each_agent() {
    let r = rig(false);
    // Antigravity's config as it is on Iván's machine: there, and 0 bytes.
    std::fs::create_dir_all(r.antigravity().parent().unwrap()).unwrap();
    std::fs::write(r.antigravity(), "").unwrap();
    let env = r.env();
    for agent in AgentId::ALL {
        assert_eq!(
            mcp_state(&env, agent).unwrap(),
            McpState::Absent,
            "{agent:?}"
        );
        mcp_connect(&env, agent).unwrap();
        assert_eq!(mcp_state(&env, agent).unwrap(), McpState::Ours, "{agent:?}");
    }

    // The path with a space travels as one argument.
    assert_eq!(
        add_call(&r, "claude"),
        [
            "mcp",
            "add",
            "--scope",
            "user",
            "clio-memory",
            "--",
            EXE,
            "mcp"
        ]
    );
    assert_eq!(
        add_call(&r, "codex"),
        ["mcp", "add", "clio-memory", "--", EXE, "mcp"]
    );
    assert_eq!(
        add_call(&r, "gemini"),
        ["mcp", "add", "-s", "user", "clio-memory", EXE, "mcp"]
    );
    assert_eq!(
        read(&r.opencode())["mcp"]["clio-memory"],
        json!({"type": "local", "command": [EXE, "mcp"], "enabled": true})
    );
    assert_eq!(
        read(&r.antigravity())["mcpServers"]["clio-memory"],
        json!({"command": EXE, "args": ["mcp"]})
    );

    // Connecting again finds it there and changes nothing.
    let before = calls(&r.bin, "claude").len();
    mcp_connect(&env, AgentId::ClaudeCode).unwrap();
    assert!(
        !calls(&r.bin, "claude")[before..]
            .iter()
            .any(|c| c[1] == "add" || c[1] == "remove")
    );

    for agent in AgentId::ALL {
        mcp_disconnect(&env, agent).unwrap();
        assert_eq!(
            mcp_state(&env, agent).unwrap(),
            McpState::Absent,
            "{agent:?}"
        );
    }
}

#[test]
fn old_cargo_install_is_other_path() {
    let r = rig(false);
    let env = r.env();
    let cargo = "/home/x/.cargo/bin/clio-memory";

    let entry =
        |command: Value| json!({"mcp": {"clio-memory": {"type": "local", "command": command}}});
    write(&r.opencode(), &entry(json!([cargo, "mcp"])));
    assert_eq!(
        mcp_state(&env, AgentId::OpenCode).unwrap(),
        McpState::OtherPath(PathBuf::from(cargo))
    );
    write(&r.opencode(), &entry(json!(["clio", "memory", "mcp"])));
    assert!(matches!(
        mcp_state(&env, AgentId::OpenCode).unwrap(),
        McpState::Foreign(_)
    ));

    // The same old install, as each command line reports it.
    std::fs::write(
        claude_registry(&r.bin),
        claude_get_output(CLAUDE_USER_SCOPE, "user", cargo, "mcp"),
    )
    .unwrap();
    assert_eq!(
        mcp_state(&env, AgentId::ClaudeCode).unwrap(),
        McpState::OtherPath(PathBuf::from(cargo))
    );
    std::fs::write(
        claude_registry(&r.bin),
        claude_get_output(CLAUDE_USER_SCOPE, "user", "clio", "memory mcp"),
    )
    .unwrap();
    assert!(matches!(
        mcp_state(&env, AgentId::ClaudeCode).unwrap(),
        McpState::Foreign(_)
    ));
    std::fs::write(
        codex_registry(&r.codex_home),
        codex_get_json(cargo, &["mcp"]),
    )
    .unwrap();
    assert_eq!(
        mcp_state(&env, AgentId::Codex).unwrap(),
        McpState::OtherPath(PathBuf::from(cargo))
    );

    // Connecting re-points it.
    mcp_connect(&env, AgentId::Codex).unwrap();
    assert_eq!(mcp_state(&env, AgentId::Codex).unwrap(), McpState::Ours);
}

#[test]
fn a_claude_entry_in_another_scope_is_not_the_users() {
    // `claude mcp get` also finds the entries of the directory it runs in: those are not the
    // user's, so for setup there is nothing there.
    for command in [EXE, "/home/x/.cargo/bin/clio-memory"] {
        let r = rig(false);
        let env = r.env();
        let local = claude_get_output(CLAUDE_LOCAL_SCOPE, "local", command, "mcp");
        std::fs::write(claude_registry(&r.bin), local).unwrap();
        assert_eq!(
            mcp_state(&env, AgentId::ClaudeCode).unwrap(),
            McpState::Absent,
            "{command}"
        );
        mcp_disconnect(&env, AgentId::ClaudeCode).unwrap();
        mcp_connect(&env, AgentId::ClaudeCode).unwrap();
        assert_eq!(
            add_call(&r, "claude"),
            [
                "mcp",
                "add",
                "--scope",
                "user",
                "clio-memory",
                "--",
                EXE,
                "mcp"
            ]
        );
        assert!(
            !calls(&r.bin, "claude").iter().any(|c| c[1] == "remove"),
            "{command}"
        );
        assert_eq!(
            mcp_state(&env, AgentId::ClaudeCode).unwrap(),
            McpState::Ours
        );
    }
}

#[test]
fn a_disabled_entry_is_reconnected() {
    // Ours, but switched off: read as an install to re-point (connecting turns it back on).
    let r = rig(false);
    let env = r.env();
    let ours_off = McpState::OtherPath(PathBuf::from(EXE));

    write(
        &r.opencode(),
        &json!({"mcp": {"clio-memory": {"type": "local", "command": [EXE, "mcp"], "enabled": false}}}),
    );
    assert_eq!(mcp_state(&env, AgentId::OpenCode).unwrap(), ours_off);
    mcp_connect(&env, AgentId::OpenCode).unwrap();
    assert_eq!(
        read(&r.opencode())["mcp"]["clio-memory"]["enabled"],
        json!(true)
    );
    assert_eq!(mcp_state(&env, AgentId::OpenCode).unwrap(), McpState::Ours);

    write(
        &r.antigravity(),
        &json!({"mcpServers": {"clio-memory": {"command": EXE, "args": ["mcp"], "disabled": true}}}),
    );
    assert_eq!(mcp_state(&env, AgentId::Antigravity).unwrap(), ours_off);
    mcp_connect(&env, AgentId::Antigravity).unwrap();
    assert_eq!(
        mcp_state(&env, AgentId::Antigravity).unwrap(),
        McpState::Ours
    );

    let mut off: Value = serde_json::from_str(&codex_get_json(EXE, &["mcp"])).unwrap();
    off["enabled"] = json!(false);
    std::fs::write(codex_registry(&r.codex_home), off.to_string()).unwrap();
    assert_eq!(mcp_state(&env, AgentId::Codex).unwrap(), ours_off);
    mcp_connect(&env, AgentId::Codex).unwrap();
    assert_eq!(mcp_state(&env, AgentId::Codex).unwrap(), McpState::Ours);
    let removed = calls(&r.bin, "codex");
    assert!(removed.iter().any(|c| c[1] == "remove"), "{removed:?}");
}

#[test]
fn disconnect_leaves_the_rest() {
    let r = rig(false);
    let env = r.env();
    let server = |c: &str| json!({"command": c, "args": []});

    // Another server, connected and disconnected around: the file comes back as it was.
    let original = json!({"mcpServers": {"x": server("/bin/x")}, "theme": "dark"});
    write(&r.antigravity(), &original);
    let text = std::fs::read_to_string(r.antigravity()).unwrap();
    mcp_connect(&env, AgentId::Antigravity).unwrap();
    mcp_disconnect(&env, AgentId::Antigravity).unwrap();
    assert_eq!(std::fs::read_to_string(r.antigravity()).unwrap(), text);

    // Ours in the middle, with servers after it: the rest keep their order.
    write(
        &r.antigravity(),
        &json!({"mcpServers": {
            "a": server("/bin/a"),
            "clio-memory": server("/old/clio-memory"),
            "y": server("/bin/y"),
            "z": server("/bin/z"),
        }}),
    );
    mcp_connect(&env, AgentId::Antigravity).unwrap();
    mcp_disconnect(&env, AgentId::Antigravity).unwrap();
    let want = json!({"mcpServers": {"a": server("/bin/a"), "y": server("/bin/y"), "z": server("/bin/z")}});
    assert_eq!(
        std::fs::read_to_string(r.antigravity()).unwrap(),
        serde_json::to_string_pretty(&want).unwrap() + "\n"
    );

    // OpenCode: `mcp` left empty goes, and the keys after it keep their place.
    write(
        &r.opencode(),
        &json!({"$schema": "s", "mcp": {}, "theme": "t", "autoupdate": false}),
    );
    mcp_connect(&env, AgentId::OpenCode).unwrap();
    mcp_disconnect(&env, AgentId::OpenCode).unwrap();
    let want = json!({"$schema": "s", "theme": "t", "autoupdate": false});
    assert_eq!(
        std::fs::read_to_string(r.opencode()).unwrap(),
        serde_json::to_string_pretty(&want).unwrap() + "\n"
    );
}

#[test]
fn a_config_of_another_shape_is_an_error() {
    let r = rig(false);
    let env = r.env();
    for bad in [json!([1, 2]), json!({"mcpServers": "x"})] {
        write(&r.antigravity(), &bad);
        let e = mcp_connect(&env, AgentId::Antigravity).unwrap_err();
        assert!(e.contains("mcp_config.json"), "{e}");
        assert_eq!(read(&r.antigravity()), bad, "left as it was");
    }
    write(&r.opencode(), &json!({"mcp": [1]}));
    let e = mcp_state(&env, AgentId::OpenCode).unwrap_err();
    assert!(e.contains("opencode.json"), "{e}");
}

#[test]
fn failing_program_is_an_error() {
    let r = rig(true);
    let e = mcp_connect(&r.env(), AgentId::Codex).unwrap_err();
    assert!(e.contains("codex"), "{e}");
    assert!(e.contains("could not write config.toml"), "{e}");
}

#[test]
fn a_missing_program_is_an_error_not_absent() {
    let r = rig(false);
    let mut env = r.env();
    env.path = vec![r.home.join("nowhere")];
    for agent in [AgentId::ClaudeCode, AgentId::Codex] {
        let e = mcp_state(&env, agent).unwrap_err();
        assert!(e.contains(agent.key().trim_end_matches("-code")), "{e}");
    }
}
