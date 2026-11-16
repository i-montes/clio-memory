#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! Each agent connected by skill or by MCP, and setup remembering which: under a temporary home
//! and data folder, with fake `claude` and `codex`.

mod common;

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use clio_memory::setup::agents::{AgentId, Env};
use clio_memory::setup::mcp::{McpState, mcp_state};
use clio_memory::setup::modes::{self, Mode, default_mode};
use clio_memory::setup::permission::{self, RULE};
use clio_memory::setup::plan::{Selection, Step, apply, connected};
use clio_memory::setup::skill::SkillDir;
use clio_memory::setup::{INSTRUCTIONS_LINE, OLD_INSTRUCTIONS_LINES};
use common::fakes::{
    CLAUDE_USER_SCOPE, calls, claude_get_output, claude_registry, fake_claude_mcp, fake_codex,
};
use serde_json::{Value, json};

use AgentId::{Antigravity, ClaudeCode, Codex, Gemini, OpenCode};

const BY_SKILL: &str = "conectado por skill: corre «clio-memory m …» (sin proceso por sesión)";

struct Rig {
    _t: tempfile::TempDir,
    home: PathBuf,
    data: PathBuf,
    bin: PathBuf,
    codex_home: PathBuf,
    exe: PathBuf,
}

fn rig() -> Rig {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let bin = t.path().join("bin");
    let codex_home = t.path().join("codex");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&codex_home).unwrap();
    fake_claude_mcp(&bin);
    fake_codex(&bin, false);
    Rig {
        data: t.path().join("data"),
        exe: t.path().join("opt/clio-memory"),
        home,
        bin,
        codex_home,
        _t: t,
    }
}

fn no_service(_: &[&str]) -> Result<(), String> {
    Ok(())
}

fn never(q: &str) -> bool {
    panic!("nothing should have been asked: {q}")
}

impl Rig {
    fn env(&self) -> Env<'static> {
        Env {
            home: self.home.clone(),
            data: self.data.clone(),
            exe: self.exe.clone(),
            path: vec![self.bin.clone()],
            codex_home: Some(self.codex_home.clone()),
            claude: None,
            bin_dir: None,
            service: &no_service,
        }
    }

    fn sel(agents: &[AgentId], mode: Option<Mode>) -> Selection {
        Selection {
            agents: agents.to_vec(),
            instructions: false,
            replace: false,
            exclusive: false,
            mode,
        }
    }

    fn apply(&self, agents: &[AgentId], mode: Option<Mode>) -> Vec<Step> {
        apply(&self.env(), &Rig::sel(agents, mode), &never)
    }

    fn mode(&self, agent: AgentId) -> Option<Mode> {
        modes::load(&self.data).get(&agent).copied()
    }

    fn settings(&self) -> PathBuf {
        self.home.join(".claude/settings.json")
    }

    /// The copies of our skill that `agent` reads.
    fn skills_read_by(&self, agent: AgentId) -> usize {
        SkillDir::ALL
            .into_iter()
            .filter(|d| d.readers().contains(&agent))
            .filter(|d| d.path(&self.home).is_file())
            .count()
    }
}

fn all_ok(steps: &[Step]) {
    for s in steps {
        assert!(s.outcome.is_ok(), "{steps:#?}");
    }
}

/// What `agent`'s connecting step said.
fn said(steps: &[Step], agent: AgentId) -> String {
    let s: Vec<&Step> = steps.iter().filter(|s| s.agent == Some(agent)).collect();
    assert_eq!(s.len(), 1, "{steps:#?}");
    s[0].outcome.clone().unwrap()
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn mtimes(dir: &Path, out: &mut BTreeMap<PathBuf, SystemTime>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            mtimes(&p, out);
        } else {
            out.insert(
                p.clone(),
                std::fs::metadata(&p).unwrap().modified().unwrap(),
            );
        }
    }
}

#[test]
fn each_agent_has_its_default() {
    assert_eq!(default_mode(ClaudeCode), Mode::Skill);
    assert_eq!(default_mode(OpenCode), Mode::Skill);
    assert_eq!(default_mode(Codex), Mode::Mcp);
    assert_eq!(default_mode(Gemini), Mode::Mcp);
    assert_eq!(default_mode(Antigravity), Mode::Mcp);
    for m in [Mode::Skill, Mode::Mcp] {
        assert_eq!(Mode::from_key(m.key()), Some(m));
    }
    assert_eq!(Mode::Skill.key(), "skill");
    assert_eq!(Mode::Mcp.key(), "mcp");
}

#[test]
fn claude_code_by_skill_has_no_mcp_server() {
    let r = rig();
    let steps = r.apply(&[ClaudeCode], None);
    all_ok(&steps);
    assert_eq!(said(&steps, ClaudeCode), BY_SKILL);
    assert_eq!(mcp_state(&r.env(), ClaudeCode).unwrap(), McpState::Absent);
    assert!(calls(&r.bin, "claude").iter().all(|c| c[1] != "add"));
    assert_eq!(r.mode(ClaudeCode), Some(Mode::Skill));
    let file = modes::file(&r.data);
    assert_eq!(file, r.data.join("agents.json"));
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(r.home.join(".claude/skills/clio-memory/SKILL.md").is_file());
    assert_eq!(read(&r.settings())["permissions"]["allow"], json!([RULE]));
    assert_eq!(connected(&r.env()), vec![ClaudeCode]);
}

#[test]
fn codex_by_default_keeps_its_mcp_server() {
    let r = rig();
    all_ok(&r.apply(&[Codex], None));
    assert_eq!(mcp_state(&r.env(), Codex).unwrap(), McpState::Ours);
    assert_eq!(r.mode(Codex), Some(Mode::Mcp));
}

#[test]
fn switching_to_skill_takes_the_server_away() {
    let r = rig();
    all_ok(&r.apply(&[Codex], None));
    let steps = r.apply(&[Codex], Some(Mode::Skill));
    all_ok(&steps);
    assert_eq!(said(&steps, Codex), BY_SKILL);
    assert_eq!(mcp_state(&r.env(), Codex).unwrap(), McpState::Absent);
    assert_eq!(r.mode(Codex), Some(Mode::Skill));
    assert_eq!(r.skills_read_by(Codex), 1);
    assert_eq!(connected(&r.env()), vec![Codex]);
}

#[test]
fn switching_back_puts_it_again() {
    let r = rig();
    all_ok(&r.apply(&[Codex], None));
    all_ok(&r.apply(&[Codex], Some(Mode::Skill)));
    all_ok(&r.apply(&[Codex], Some(Mode::Mcp)));
    assert_eq!(mcp_state(&r.env(), Codex).unwrap(), McpState::Ours);
    assert_eq!(r.mode(Codex), Some(Mode::Mcp));
    // And Claude Code's rule goes when it goes back to MCP.
    all_ok(&r.apply(&[ClaudeCode], None));
    all_ok(&r.apply(&[ClaudeCode], Some(Mode::Mcp)));
    assert_eq!(mcp_state(&r.env(), ClaudeCode).unwrap(), McpState::Ours);
    assert_eq!(read(&r.settings())["permissions"]["allow"], json!([]));
}

#[test]
fn a_foreign_server_is_left_alone_without_replace() {
    let r = rig();
    let foreign = claude_get_output(CLAUDE_USER_SCOPE, "user", "clio", "memory mcp");
    std::fs::write(claude_registry(&r.bin), &foreign).unwrap();
    let asked = std::cell::Cell::new(0);
    let steps = apply(
        &r.env(),
        &Rig::sel(&[ClaudeCode], Some(Mode::Skill)),
        &|q: &str| {
            assert!(q.contains("un servidor MCP llamado clio-memory"), "{q}");
            asked.set(asked.get() + 1);
            false
        },
    );
    assert_eq!(asked.get(), 1);
    assert_eq!(steps.len(), 1, "{steps:#?}");
    assert!(steps[0].outcome.is_err(), "{steps:#?}");
    assert_eq!(
        std::fs::read_to_string(claude_registry(&r.bin)).unwrap(),
        foreign
    );
    assert!(modes::load(&r.data).is_empty());
    assert!(!r.settings().exists());
}

#[test]
fn the_permission_rule_touches_nothing_else() {
    let r = rig();
    let original = r#"{"permissions":{"allow":["Bash(ls:*)"],"deny":["Read(.env)"]},"model":"x"}"#;
    std::fs::create_dir_all(r.settings().parent().unwrap()).unwrap();
    std::fs::write(r.settings(), original).unwrap();
    let keys = |v: &Value| -> Vec<String> { v.as_object().unwrap().keys().cloned().collect() };

    assert!(permission::allow(&r.home).unwrap());
    let after = read(&r.settings());
    assert_eq!(
        after["permissions"]["allow"],
        json!(["Bash(ls:*)", "Bash(clio-memory m:*)"])
    );
    assert_eq!(after["permissions"]["deny"], json!(["Read(.env)"]));
    assert_eq!(after["model"], json!("x"));
    assert_eq!(keys(&after), ["permissions", "model"]);
    assert_eq!(keys(&after["permissions"]), ["allow", "deny"]);
    assert!(!permission::allow(&r.home).unwrap());

    assert!(permission::revoke(&r.home).unwrap());
    let back = read(&r.settings());
    let want: Value = serde_json::from_str(original).unwrap();
    assert_eq!(back, want);
    assert_eq!(keys(&back), ["permissions", "model"]);
    assert_eq!(keys(&back["permissions"]), ["allow", "deny"]);
    assert!(!permission::revoke(&r.home).unwrap());
}

#[test]
fn the_permission_rule_on_a_missing_or_broken_file() {
    let r = rig();
    assert!(!permission::revoke(&r.home).unwrap());
    assert!(!r.settings().exists());
    assert!(permission::allow(&r.home).unwrap());
    assert_eq!(
        read(&r.settings()),
        json!({"permissions": {"allow": [RULE]}})
    );
    let broken = "{ // mis permisos\n}\n";
    std::fs::write(r.settings(), broken).unwrap();
    assert!(permission::allow(&r.home).is_err());
    assert!(permission::revoke(&r.home).is_err());
    assert_eq!(std::fs::read_to_string(r.settings()).unwrap(), broken);
}

#[test]
fn a_rerun_keeps_each_mode() {
    let r = rig();
    all_ok(&r.apply(&[ClaudeCode, Codex], None));
    assert_eq!(r.mode(ClaudeCode), Some(Mode::Skill));
    assert_eq!(r.mode(Codex), Some(Mode::Mcp));
    let mut before = BTreeMap::new();
    for dir in [&r.home, &r.codex_home, &r.data] {
        mtimes(dir, &mut before);
    }
    let steps = r.apply(&[ClaudeCode, Codex], None);
    all_ok(&steps);
    let mut after = BTreeMap::new();
    for dir in [&r.home, &r.codex_home, &r.data] {
        mtimes(dir, &mut after);
    }
    assert_eq!(before, after);
    assert_eq!(said(&steps, ClaudeCode), format!("ya estaba {BY_SKILL}"));
    assert!(said(&steps, Codex).contains("ya está"), "{steps:#?}");
    assert_eq!(r.mode(ClaudeCode), Some(Mode::Skill));
    assert_eq!(r.mode(Codex), Some(Mode::Mcp));
    assert_eq!(mcp_state(&r.env(), ClaudeCode).unwrap(), McpState::Absent);
    assert_eq!(mcp_state(&r.env(), Codex).unwrap(), McpState::Ours);
}

#[test]
fn exclusive_takes_the_skill_way_away() {
    let r = rig();
    all_ok(&r.apply(&[ClaudeCode, Codex], None));
    let steps = apply(
        &r.env(),
        &Selection {
            exclusive: true,
            ..Rig::sel(&[Codex], None)
        },
        &never,
    );
    all_ok(&steps);
    assert_eq!(connected(&r.env()), vec![Codex]);
    assert_eq!(r.mode(ClaudeCode), None);
    assert_eq!(r.mode(Codex), Some(Mode::Mcp));
    assert_eq!(read(&r.settings())["permissions"]["allow"], json!([]));
    assert!(!r.home.join(".claude/skills/clio-memory/SKILL.md").exists());
}

// `update` runs `setup --yes`, which adds no line on a re-run: an earlier version's line of ours
// is still renewed, since it is ours (added with a yes) and renewing is not adding.
#[test]
fn a_rerun_renews_an_old_instructions_line() {
    let r = rig();
    let md = r.home.join(".claude/CLAUDE.md");
    std::fs::create_dir_all(md.parent().unwrap()).unwrap();
    std::fs::write(
        &md,
        format!("# Reglas\n{}\n- corto\n", OLD_INSTRUCTIONS_LINES[0]),
    )
    .unwrap();
    all_ok(&r.apply(&[ClaudeCode], None));
    assert_eq!(
        std::fs::read_to_string(&md).unwrap(),
        format!("# Reglas\n{INSTRUCTIONS_LINE}\n- corto\n")
    );
    // Without a line of ours, none is added.
    let agents = r.codex_home.join("AGENTS.md");
    std::fs::write(&agents, "# Codex\n").unwrap();
    all_ok(&r.apply(&[Codex], None));
    assert_eq!(std::fs::read_to_string(&agents).unwrap(), "# Codex\n");
}

// The skill way is noted before the server goes: a removal that fails leaves the agent counted as
// connected (noted by skill), and the next run removes it.
#[test]
fn a_failed_removal_still_leaves_the_skill_way_noted() {
    let r = rig();
    all_ok(&r.apply(&[Codex], None));
    let codex = r.bin.join("codex");
    let working = std::fs::read(&codex).unwrap();
    std::fs::write(
        &codex,
        format!(
            "#!/bin/sh\ncase \"$1 $2\" in\n  \"mcp get\") cat '{}';;\n  *) echo 'cannot write config.toml' >&2; exit 1;;\nesac\n",
            common::fakes::codex_registry(&r.codex_home).display()
        ),
    )
    .unwrap();
    let steps = r.apply(&[Codex], Some(Mode::Skill));
    assert!(steps.iter().any(|s| s.outcome.is_err()), "{steps:#?}");
    assert_eq!(r.mode(Codex), Some(Mode::Skill));
    assert_eq!(connected(&r.env()), vec![Codex]);
    std::fs::write(&codex, working).unwrap();
    all_ok(&r.apply(&[Codex], None));
    assert_eq!(mcp_state(&r.env(), Codex).unwrap(), McpState::Absent);
}

// Someone who never used the skill way: a settings.json that is not JSON does not make Claude
// Code's MCP connect or disconnect fail, and is left as it is. Once by skill, it does.
#[test]
fn a_broken_settings_file_only_matters_to_the_skill_way() {
    let r = rig();
    let broken = "{ // mis permisos\n}\n";
    std::fs::create_dir_all(r.settings().parent().unwrap()).unwrap();
    std::fs::write(r.settings(), broken).unwrap();
    all_ok(&r.apply(&[ClaudeCode], Some(Mode::Mcp)));
    let steps = apply(
        &r.env(),
        &Selection {
            exclusive: true,
            ..Rig::sel(&[], None)
        },
        &never,
    );
    all_ok(&steps);
    assert_eq!(connected(&r.env()), vec![]);
    assert_eq!(std::fs::read_to_string(r.settings()).unwrap(), broken);
    // Noted by skill, the rule must go: the broken file is an error naming it.
    modes::set(&r.data, ClaudeCode, Some(Mode::Skill)).unwrap();
    let steps = r.apply(&[ClaudeCode], Some(Mode::Mcp));
    let why = steps[0].outcome.as_ref().unwrap_err();
    assert!(why.contains("settings.json"), "{steps:#?}");
    assert_eq!(std::fs::read_to_string(r.settings()).unwrap(), broken);
}

#[test]
fn an_unreadable_modes_file_is_never_written_over() {
    let r = rig();
    let file = modes::file(&r.data);
    std::fs::create_dir_all(&r.data).unwrap();
    std::fs::write(&file, "{ roto").unwrap();
    let e = modes::set(&r.data, ClaudeCode, Some(Mode::Skill)).unwrap_err();
    assert!(e.contains("agents.json"), "{e}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ roto");
    assert!(modes::load(&r.data).is_empty());
    // Entries it does not know are kept.
    std::fs::write(&file, r#"{"cursor": "skill"}"#).unwrap();
    modes::set(&r.data, ClaudeCode, Some(Mode::Skill)).unwrap();
    assert_eq!(
        read(&file),
        json!({"cursor": "skill", "claude-code": "skill"})
    );
}
