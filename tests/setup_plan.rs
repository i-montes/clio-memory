#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! Applying a selection of agents, uninstalling, and the per-agent report: everything under a
//! temporary home, with fake `claude`, `codex` and `gemini` and the folders of OpenCode and
//! Antigravity.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use clio_memory::setup::INSTRUCTIONS_LINE;
use clio_memory::setup::agents::{AgentId, Env};
use clio_memory::setup::instructions::has_instructions;
use clio_memory::setup::mcp::{McpState, mcp_state};
use clio_memory::setup::plan::{Selection, Step, agent_report, apply, connected, uninstall};
use common::fakes::{calls, fake_claude_mcp, fake_codex, fake_gemini};
use serde_json::{Value, json};

use AgentId::{Antigravity, ClaudeCode, Codex, Gemini, OpenCode};

struct Rig {
    /// `CLIO_MEMORY_BIN_DIR`.
    bin_dir: Option<PathBuf>,
    t: tempfile::TempDir,
    home: PathBuf,
    bin: PathBuf,
    codex_home: PathBuf,
    exe: PathBuf,
}

fn rig_with(codex_fails: bool) -> Rig {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("my home");
    let bin = t.path().join("bin");
    let codex_home = t.path().join("codex home");
    std::fs::create_dir_all(home.join(".config/opencode")).unwrap();
    std::fs::create_dir_all(home.join(".gemini/config")).unwrap();
    std::fs::create_dir_all(&codex_home).unwrap();
    fake_claude_mcp(&bin);
    fake_codex(&bin, codex_fails);
    fake_gemini(&bin, &home);
    let exe = t.path().join("opt bin/clio-memory");
    Rig {
        bin_dir: None,
        home,
        bin,
        codex_home,
        exe,
        t,
    }
}

fn rig() -> Rig {
    rig_with(false)
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
            exe: self.exe.clone(),
            path: vec![self.bin.clone()],
            codex_home: Some(self.codex_home.clone()),
            claude: None,
            bin_dir: self.bin_dir.clone(),
            service: &no_service,
        }
    }

    fn apply(&self, agents: &[AgentId], exclusive: bool) -> Vec<Step> {
        apply(
            &self.env(),
            &Selection {
                agents: agents.to_vec(),
                instructions: true,
                replace: false,
                exclusive,
            },
            &never,
        )
    }

    fn skill(&self, root: &str) -> PathBuf {
        self.home.join(root).join("clio-memory/SKILL.md")
    }

    fn sleep_timer(&self) -> PathBuf {
        self.home
            .join(".config/systemd/user/clio-memory-sleep.timer")
    }
}

fn all_ok(steps: &[Step]) {
    for s in steps {
        assert!(s.outcome.is_ok(), "{steps:#?}");
    }
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Every file under `dir`, with when it was last written.
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
fn apply_all_connects_all_and_places_one_skill_each() {
    let r = rig();
    let steps = r.apply(&AgentId::ALL, false);
    all_ok(&steps);
    assert_eq!(connected(&r.env()), AgentId::ALL.to_vec());
    assert!(r.skill(".claude/skills").is_file());
    assert!(r.skill(".agents/skills").is_file());
    assert!(r.skill(".gemini/config/skills").is_file());
    assert!(!r.skill(".config/opencode/skills").exists());
    assert!(!r.skill(".gemini/skills").exists());
    for agent in AgentId::ALL {
        assert!(has_instructions(&r.env(), agent), "{agent:?}");
    }
    // One skill step for everybody, and the sleep last.
    assert_eq!(
        steps.iter().filter(|s| s.agent.is_none()).count(),
        2,
        "{steps:#?}"
    );
    assert!(r.sleep_timer().is_file() || !cfg!(target_os = "linux"));
}

#[test]
fn one_failing_agent_does_not_stop_the_rest() {
    let r = rig_with(true);
    let steps = r.apply(&AgentId::ALL, false);
    let codex: Vec<&Step> = steps.iter().filter(|s| s.agent == Some(Codex)).collect();
    assert!(
        codex.iter().any(|s| s
            .outcome
            .as_ref()
            .is_err_and(|e| e.contains("could not write config.toml"))),
        "{steps:#?}"
    );
    assert_eq!(
        connected(&r.env()),
        vec![ClaudeCode, OpenCode, Gemini, Antigravity]
    );
    // Nothing of Codex's beyond the failed step: no line in its AGENTS.md.
    assert!(!has_instructions(&r.env(), Codex));
    for s in steps.iter().filter(|s| s.agent != Some(Codex)) {
        assert!(s.outcome.is_ok(), "{s:?}");
    }
}

#[test]
fn declined_conflict_writes_nothing_for_that_agent() {
    let r = rig();
    let settings = r.home.join(".gemini/settings.json");
    std::fs::write(
        &settings,
        "{\"theme\": \"x\", \"mcpServers\": {\"clio-memory\": {\"command\": \"npx\", \"args\": [\"otra-memoria\"]}}}\n",
    )
    .unwrap();
    let before = std::fs::read(&settings).unwrap();
    let asked = std::cell::RefCell::new(Vec::new());
    let steps = apply(
        &r.env(),
        &Selection {
            agents: vec![ClaudeCode, Gemini],
            instructions: true,
            replace: false,
            exclusive: false,
        },
        &|q: &str| {
            asked.borrow_mut().push(q.to_owned());
            false
        },
    );
    assert_eq!(asked.borrow().len(), 1, "{:?}", asked.borrow());
    assert!(
        asked.borrow()[0].starts_with("Gemini CLI ya tiene"),
        "{:?}",
        asked.borrow()
    );
    let gemini: Vec<&Step> = steps.iter().filter(|s| s.agent == Some(Gemini)).collect();
    assert_eq!(gemini.len(), 1, "{steps:#?}");
    assert!(
        gemini[0]
            .outcome
            .as_ref()
            .is_err_and(|e| e.contains("--replace")),
        "{steps:#?}"
    );
    assert_eq!(std::fs::read(&settings).unwrap(), before);
    assert!(calls(&r.bin, "gemini").is_empty(), "gemini was not run");
    assert!(!r.home.join(".gemini/GEMINI.md").exists());
    assert!(!r.skill(".agents/skills").exists());
    // Claude Code, which had no conflict, is connected.
    assert_eq!(connected(&r.env()), vec![ClaudeCode]);
    assert!(r.skill(".claude/skills").is_file());
}

#[test]
fn a_foreign_skill_is_asked_about_and_a_no_keeps_it() {
    let r = rig();
    let skill = r.skill(".claude/skills");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(&skill, "mi propia skill\n").unwrap();
    let steps = apply(
        &r.env(),
        &Selection {
            agents: vec![ClaudeCode],
            instructions: true,
            replace: false,
            exclusive: false,
        },
        &|q: &str| {
            assert!(q.contains("una skill clio-memory distinta"), "{q}");
            false
        },
    );
    assert!(steps[0].outcome.is_err(), "{steps:#?}");
    assert_eq!(
        std::fs::read_to_string(&skill).unwrap(),
        "mi propia skill\n"
    );
    assert!(calls(&r.bin, "claude").iter().all(|c| c[1] != "add"));
    // A yes replaces it.
    let steps = apply(
        &r.env(),
        &Selection {
            agents: vec![ClaudeCode],
            instructions: false,
            replace: false,
            exclusive: false,
        },
        &|_| true,
    );
    all_ok(&steps);
    assert_ne!(
        std::fs::read_to_string(&skill).unwrap(),
        "mi propia skill\n"
    );
}

#[test]
fn old_install_is_repointed() {
    let r = rig();
    let old = r.home.join(".cargo/bin/clio-memory");
    let config = r.home.join(".config/opencode/opencode.json");
    std::fs::write(
        &config,
        serde_json::to_string_pretty(&json!({
            "mcp": {"clio-memory": {"type": "local", "command": [old, "mcp"], "enabled": true}}
        }))
        .unwrap(),
    )
    .unwrap();
    let steps = r.apply(&[OpenCode], false);
    all_ok(&steps);
    let mcp = &steps[0];
    assert_eq!(mcp.agent, Some(OpenCode));
    let said = mcp.outcome.as_ref().unwrap();
    assert!(said.contains("apuntaba a"), "{said}");
    assert!(said.contains(&old.display().to_string()), "{said}");
    assert_eq!(
        read(&config)["mcp"]["clio-memory"]["command"][0],
        json!(r.exe)
    );
    assert_eq!(mcp_state(&r.env(), OpenCode).unwrap(), McpState::Ours);
}

#[test]
fn a_switched_off_entry_is_switched_on() {
    let r = rig();
    let config = r.home.join(".config/opencode/opencode.json");
    std::fs::write(
        &config,
        serde_json::to_string_pretty(&json!({
            "mcp": {"clio-memory": {"type": "local", "command": [r.exe, "mcp"], "enabled": false}}
        }))
        .unwrap(),
    )
    .unwrap();
    let steps = r.apply(&[OpenCode], false);
    let said = steps[0].outcome.as_ref().unwrap();
    assert!(said.contains("estaba desactivado; reactivado"), "{said}");
    assert!(!said.contains("apuntaba"), "{said}");
    assert_eq!(read(&config)["mcp"]["clio-memory"]["enabled"], json!(true));
}

#[test]
fn an_agent_not_found_is_one_failed_step() {
    let r = rig();
    std::fs::remove_file(r.bin.join("codex")).unwrap();
    let steps = r.apply(&[Codex, ClaudeCode], false);
    let codex: Vec<&Step> = steps.iter().filter(|s| s.agent == Some(Codex)).collect();
    assert_eq!(codex.len(), 1);
    assert_eq!(codex[0].outcome, Err("no se encontró Codex".to_owned()));
    assert_eq!(connected(&r.env()), vec![ClaudeCode]);
}

#[test]
fn exclusive_disconnects_the_unchecked() {
    let r = rig();
    all_ok(&r.apply(&[ClaudeCode, Codex], false));
    assert!(r.skill(".agents/skills").is_file());
    let steps = r.apply(&[ClaudeCode], true);
    all_ok(&steps);
    assert_eq!(connected(&r.env()), vec![ClaudeCode]);
    assert_eq!(mcp_state(&r.env(), Codex).unwrap(), McpState::Absent);
    assert!(!has_instructions(&r.env(), Codex));
    assert!(!r.skill(".agents/skills").exists());
    assert!(r.skill(".claude/skills").is_file());
    assert!(has_instructions(&r.env(), ClaudeCode));
}

#[test]
fn non_exclusive_only_adds() {
    let r = rig();
    all_ok(&r.apply(&[ClaudeCode, Codex], false));
    all_ok(&r.apply(&[ClaudeCode], false));
    assert_eq!(connected(&r.env()), vec![ClaudeCode, Codex]);
    assert!(has_instructions(&r.env(), Codex));
    assert!(r.skill(".agents/skills").is_file());
}

#[test]
fn second_apply_changes_nothing() {
    let r = rig();
    all_ok(&r.apply(&AgentId::ALL, false));
    let mut before = BTreeMap::new();
    mtimes(&r.home, &mut before);
    mtimes(&r.codex_home, &mut before);
    let adds = |name| calls(&r.bin, name).iter().filter(|c| c[1] == "add").count();
    let added = (adds("claude"), adds("codex"), adds("gemini"));
    let steps = r.apply(&AgentId::ALL, false);
    all_ok(&steps);
    let mut after = BTreeMap::new();
    mtimes(&r.home, &mut after);
    mtimes(&r.codex_home, &mut after);
    assert_eq!(before, after);
    assert_eq!((adds("claude"), adds("codex"), adds("gemini")), added);
    // And it says so.
    for s in &steps {
        let said = s.outcome.as_ref().unwrap();
        assert!(said.contains("ya"), "{said}");
    }
    // One line each, never two.
    for f in [
        ".claude/CLAUDE.md",
        ".gemini/GEMINI.md",
        ".config/opencode/clio-memory.md",
    ] {
        let text = std::fs::read_to_string(r.home.join(f)).unwrap();
        assert_eq!(text.matches(INSTRUCTIONS_LINE).count(), 1, "{f}");
    }
}

#[test]
fn uninstall_all_leaves_memory_db() {
    let r = rig();
    all_ok(&r.apply(&AgentId::ALL, false));
    let data = r.t.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("memory.db"), "la memoria").unwrap();
    let steps = uninstall(&r.env(), None);
    all_ok(&steps);
    assert!(connected(&r.env()).is_empty());
    for agent in AgentId::ALL {
        assert!(!has_instructions(&r.env(), agent), "{agent:?}");
    }
    for root in [".claude/skills", ".agents/skills", ".gemini/config/skills"] {
        assert!(!r.skill(root).exists(), "{root}");
    }
    let units = r.home.join(".config/systemd/user");
    assert!(!units.join("clio-memory-sleep.timer").exists());
    assert!(!units.join("clio-memory-sleep.service").exists());
    assert_eq!(
        std::fs::read_to_string(data.join("memory.db")).unwrap(),
        "la memoria"
    );
}

#[test]
fn uninstall_all_deletes_the_binary_only_where_the_installer_put_it() {
    let mut r = rig();
    // Elsewhere: kept.
    std::fs::create_dir_all(r.exe.parent().unwrap()).unwrap();
    std::fs::write(&r.exe, "binario").unwrap();
    all_ok(&uninstall(&r.env(), None));
    assert!(r.exe.is_file());
    // In ~/.local/bin: deleted, last.
    r.exe = r.home.join(".local/bin/clio-memory");
    std::fs::create_dir_all(r.exe.parent().unwrap()).unwrap();
    std::fs::write(&r.exe, "binario").unwrap();
    all_ok(&r.apply(&[OpenCode], false));
    let steps = uninstall(&r.env(), None);
    all_ok(&steps);
    assert!(!r.exe.exists());
    let last = steps.last().unwrap().outcome.as_ref().unwrap();
    assert!(last.contains(&r.exe.display().to_string()), "{steps:#?}");
}

#[test]
fn uninstall_some_keeps_the_rest_and_moves_the_skill() {
    let r = rig();
    all_ok(&r.apply(&[ClaudeCode, Codex, Gemini], false));
    all_ok(&uninstall(&r.env(), Some(&[Codex])));
    assert_eq!(connected(&r.env()), vec![ClaudeCode, Gemini]);
    assert!(!has_instructions(&r.env(), Codex));
    assert!(has_instructions(&r.env(), Gemini));
    // Gemini still needs the shared folder; the sleep stays.
    assert!(r.skill(".agents/skills").is_file());
    assert!(r.sleep_timer().is_file() || !cfg!(target_os = "linux"));
    all_ok(&uninstall(&r.env(), Some(&[Gemini])));
    assert!(!r.skill(".agents/skills").exists());
    assert!(r.skill(".claude/skills").is_file());
}

#[test]
fn uninstall_leaves_a_foreign_server_alone() {
    let r = rig();
    let config = r.home.join(".gemini/config/mcp_config.json");
    let text = "{\"mcpServers\": {\"clio-memory\": {\"command\": \"otra\", \"args\": []}}}\n";
    std::fs::write(&config, text).unwrap();
    all_ok(&uninstall(&r.env(), Some(&[Antigravity])));
    assert_eq!(std::fs::read_to_string(&config).unwrap(), text);
}

#[test]
fn the_report_says_each_agents_state() {
    let r = rig();
    all_ok(&r.apply(&[ClaudeCode, Codex], false));
    // OpenCode points at an old binary; Antigravity sees a skill that is not ours.
    let old = r.home.join(".cargo/bin/clio-memory");
    std::fs::write(
        r.home.join(".config/opencode/opencode.json"),
        json!({"mcp": {"clio-memory": {"type": "local", "command": [old, "mcp"]}}}).to_string(),
    )
    .unwrap();
    std::fs::write(
        r.home.join(".gemini/config/mcp_config.json"),
        json!({"mcpServers": {"clio-memory": {"command": r.exe, "args": ["mcp"]}}}).to_string(),
    )
    .unwrap();
    let foreign = r.skill(".gemini/config/skills");
    std::fs::create_dir_all(foreign.parent().unwrap()).unwrap();
    std::fs::write(&foreign, "otra\n").unwrap();
    let report: BTreeMap<AgentId, (bool, String)> = agent_report(&r.env())
        .into_iter()
        .map(|(a, ok, text)| (a, (ok, text)))
        .collect();
    assert_eq!(report.len(), 5, "{report:?}");
    assert_eq!(report[&ClaudeCode], (true, "conectado".to_owned()));
    assert_eq!(report[&Codex], (true, "conectado".to_owned()));
    assert_eq!(report[&Gemini], (true, "no conectado".to_owned()));
    assert_eq!(
        report[&OpenCode],
        (false, format!("el MCP apunta a {}", old.display()))
    );
    assert_eq!(
        report[&Antigravity],
        (false, format!("skill ajena en {}", foreign.display()))
    );
    // Only the agents found on this machine.
    std::fs::remove_file(r.bin.join("gemini")).unwrap();
    assert!(agent_report(&r.env()).iter().all(|(a, _, _)| *a != Gemini));
}

#[cfg(target_os = "linux")]
#[test]
fn an_old_clio_memory_sleep_is_repointed_and_clios_is_left_alone() {
    use clio_memory::setup::{install_sleep, sleep_installed};
    let r = rig();
    let service = r
        .home
        .join(".config/systemd/user/clio-memory-sleep.service");
    // An older install's sleep: ours, re-pointed.
    let old = r.home.join(".cargo/bin/clio-memory");
    install_sleep(&r.home, &old, &["sleep"], &no_service).unwrap();
    let steps = r.apply(&[OpenCode], false);
    let sleep = steps.last().unwrap();
    assert!(
        sleep
            .outcome
            .as_ref()
            .unwrap()
            .starts_with("sueño nocturno instalado"),
        "{steps:#?}"
    );
    assert!(sleep_installed(&r.home, &r.exe, &["sleep"]));
    // Clio's: neither replaced nor uninstalled.
    install_sleep(
        &r.home,
        Path::new("/opt/bin/clio"),
        &["memory", "sleep"],
        &no_service,
    )
    .unwrap();
    let before = std::fs::read_to_string(&service).unwrap();
    let steps = r.apply(&[OpenCode], false);
    assert!(
        steps
            .last()
            .unwrap()
            .outcome
            .as_ref()
            .unwrap()
            .contains("otro"),
        "{steps:#?}"
    );
    let steps = uninstall(&r.env(), None);
    all_ok(&steps);
    assert_eq!(std::fs::read_to_string(&service).unwrap(), before);
}

/// A foreign `clio-memory` server in Claude Code, as `claude mcp get` prints it.
fn foreign_claude_server(r: &Rig) {
    use common::fakes::{CLAUDE_USER_SCOPE, claude_get_output, claude_registry};
    std::fs::write(
        claude_registry(&r.bin),
        claude_get_output(CLAUDE_USER_SCOPE, "user", "clio", "memory mcp"),
    )
    .unwrap();
}

/// Our current skill where Claude Code (and OpenCode) read it.
fn our_claude_skill(r: &Rig) -> PathBuf {
    let skill = r.skill(".claude/skills");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(&skill, clio_memory::setup::SKILL).unwrap();
    skill
}

#[test]
fn a_declined_agent_keeps_our_skill() {
    let r = rig();
    foreign_claude_server(&r);
    let skill = our_claude_skill(&r);
    let steps = apply(
        &r.env(),
        &Selection {
            agents: vec![ClaudeCode],
            instructions: true,
            replace: false,
            exclusive: false,
        },
        &|_| false,
    );
    assert!(skill.is_file(), "{steps:#?}");
    assert_eq!(
        std::fs::read_to_string(&skill).unwrap(),
        clio_memory::setup::SKILL
    );
    assert_eq!(steps.len(), 1, "only the declined step: {steps:#?}");
    assert!(steps[0].outcome.is_err());
    assert!(!r.sleep_timer().exists(), "nothing half-installed");
}

#[test]
fn a_declined_setup_claude_code_keeps_our_skill() {
    let r = rig();
    foreign_claude_server(&r);
    let skill = our_claude_skill(&r);
    let e = clio_memory::setup::ClaudeCode {
        home: r.home.clone(),
        claude: r.bin.join("claude"),
        exe: r.exe.clone(),
        replace: false,
        claude_md: Some(false),
        ask: &|_| false,
        service: &no_service,
    }
    .run()
    .unwrap_err();
    assert!(e.contains("--replace"), "{e}");
    assert_eq!(
        std::fs::read_to_string(&skill).unwrap(),
        clio_memory::setup::SKILL
    );
}

#[test]
fn flags_only_add_and_never_take_a_skill_away() {
    let r = rig();
    // Claude Code is not connected, but our skill is in its folder (an earlier install).
    let skill = our_claude_skill(&r);
    let steps = r.apply(&[Codex], false);
    all_ok(&steps);
    assert!(skill.is_file(), "{steps:#?}");
    assert!(r.skill(".agents/skills").is_file());
    assert!(
        steps
            .iter()
            .all(|s| !s.outcome.as_ref().unwrap().contains("retirada")),
        "{steps:#?}"
    );
}

#[test]
fn uninstall_keeps_the_binary_when_something_could_not_be_undone() {
    use std::os::unix::fs::PermissionsExt;
    let mut r = rig();
    r.exe = r.home.join(".local/bin/clio-memory");
    std::fs::create_dir_all(r.exe.parent().unwrap()).unwrap();
    std::fs::write(&r.exe, "binario").unwrap();
    all_ok(&r.apply(&[ClaudeCode, Codex], false));
    // Codex answers, but cannot remove.
    let registry = common::fakes::codex_registry(&r.codex_home);
    std::fs::write(
        r.bin.join("codex"),
        format!(
            "#!/bin/sh\ncase \"$1 $2\" in\n  \"mcp get\") cat '{}';;\n  *) echo 'cannot write config.toml' >&2; exit 1;;\nesac\n",
            registry.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(r.bin.join("codex"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let steps = uninstall(&r.env(), None);
    assert!(r.exe.is_file(), "the binary stays: {steps:#?}");
    let last = steps.last().unwrap();
    assert_eq!(last.what, clio_memory::setup::plan::BINARY);
    let why = last.outcome.as_ref().unwrap_err();
    assert!(why.contains(&r.exe.display().to_string()), "{why}");
    // The rest was still done.
    assert_eq!(connected(&r.env()), vec![Codex]);
}

#[test]
fn uninstall_deletes_the_binary_from_clio_memory_bin_dir() {
    let mut r = rig();
    let dir = r.t.path().join("mis binarios");
    r.exe = dir.join("clio-memory");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&r.exe, "binario").unwrap();
    all_ok(&uninstall(&r.env(), None));
    assert!(r.exe.is_file(), "not where the installer put it");
    r.bin_dir = Some(dir);
    all_ok(&uninstall(&r.env(), None));
    assert!(!r.exe.exists());
}
