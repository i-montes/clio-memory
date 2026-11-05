#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! The five agents: where each one reads skills from, how many copies of the skill that takes,
//! and how an old or foreign copy is treated. Everything runs under a temporary home.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use clio_memory::setup::SKILL;
use clio_memory::setup::agents::{AgentId, Env, Found, detect};
use clio_memory::setup::skill::{SkillDir, SkillFile, place_skill, placement, skill_state};

use AgentId::{Antigravity, ClaudeCode, Codex, Gemini, OpenCode};

fn fake_program(dir: &Path, name: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let bin = dir.join(name);
    std::fs::write(&bin, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

fn skill_at(home: &Path, dir: &str) -> PathBuf {
    home.join(dir).join("clio-memory/SKILL.md")
}

#[test]
fn placement_cases() {
    let cases: Vec<(Vec<AgentId>, Vec<SkillDir>)> = vec![
        (vec![ClaudeCode], vec![SkillDir::Claude]),
        (vec![OpenCode], vec![SkillDir::OpenCode]),
        (vec![ClaudeCode, OpenCode], vec![SkillDir::Claude]),
        (vec![Codex, Gemini], vec![SkillDir::Agents]),
        (
            vec![ClaudeCode, OpenCode, Codex],
            vec![SkillDir::Claude, SkillDir::Agents],
        ),
        (
            vec![Gemini, Antigravity],
            vec![SkillDir::Agents, SkillDir::Antigravity],
        ),
        (
            AgentId::ALL.to_vec(),
            vec![SkillDir::Claude, SkillDir::Agents, SkillDir::Antigravity],
        ),
        (vec![], vec![]),
    ];
    for (agents, want) in cases {
        assert_eq!(placement(&agents), want, "agents: {agents:?}");
    }
}

#[test]
fn every_reader_sees_the_skill() {
    for mask in 1u32..32 {
        let agents: Vec<AgentId> = AgentId::ALL
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, a)| *a)
            .collect();
        let dirs = placement(&agents);
        for a in &agents {
            assert!(
                dirs.iter().any(|d| d.readers().contains(a)),
                "{a:?} sees no copy of the skill in {dirs:?} (agents: {agents:?})"
            );
        }
    }
}

#[test]
fn agent_keys_and_labels_round_trip() {
    let keys: Vec<&str> = AgentId::ALL.iter().map(|a| a.key()).collect();
    assert_eq!(
        keys,
        ["claude-code", "opencode", "codex", "gemini", "antigravity"]
    );
    let labels: Vec<&str> = AgentId::ALL.iter().map(|a| a.label()).collect();
    assert_eq!(
        labels,
        [
            "Claude Code",
            "OpenCode",
            "Codex",
            "Gemini CLI",
            "Antigravity"
        ]
    );
    for a in AgentId::ALL {
        assert_eq!(AgentId::from_key(a.key()), Some(a));
    }
    assert_eq!(AgentId::from_key("cursor"), None);
}

#[test]
fn skill_paths_are_under_the_home() {
    let home = Path::new("/h");
    assert_eq!(
        SkillDir::Claude.path(home),
        Path::new("/h/.claude/skills/clio-memory/SKILL.md")
    );
    assert_eq!(
        SkillDir::OpenCode.path(home),
        Path::new("/h/.config/opencode/skills/clio-memory/SKILL.md")
    );
    assert_eq!(
        SkillDir::Agents.path(home),
        Path::new("/h/.agents/skills/clio-memory/SKILL.md")
    );
    assert_eq!(
        SkillDir::Antigravity.path(home),
        Path::new("/h/.gemini/config/skills/clio-memory/SKILL.md")
    );
}

#[test]
fn old_published_skill_is_ours() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("SKILL.md");
    assert_eq!(skill_state(&p), SkillFile::Absent);

    // v0.2.0 is what many people have installed today.
    std::fs::copy("tests/fixtures/skill-v0.2.0.md", &p).unwrap();
    assert_eq!(skill_state(&p), SkillFile::Old);

    std::fs::write(&p, SKILL).unwrap();
    assert_eq!(skill_state(&p), SkillFile::Current);

    std::fs::write(&p, "my own notes about memory\n").unwrap();
    assert_eq!(skill_state(&p), SkillFile::Foreign);
}

#[test]
fn place_skill_moves_between_dirs() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path();
    place_skill(home, &[ClaudeCode], false).unwrap();
    assert_eq!(
        std::fs::read_to_string(skill_at(home, ".claude/skills")).unwrap(),
        SKILL
    );

    place_skill(home, &[Codex], false).unwrap();
    assert!(!home.join(".claude/skills/clio-memory").exists());
    assert_eq!(
        std::fs::read_to_string(skill_at(home, ".agents/skills")).unwrap(),
        SKILL
    );
}

#[test]
fn place_skill_replaces_the_old_published_copy_and_removes_it_elsewhere() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path();
    let old = skill_at(home, ".claude/skills");
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    std::fs::copy("tests/fixtures/skill-v0.2.0.md", &old).unwrap();

    // Same folder: updated without --replace.
    place_skill(home, &[ClaudeCode], false).unwrap();
    assert_eq!(std::fs::read_to_string(&old).unwrap(), SKILL);

    // Another folder: the old copy is ours too, so it is removed.
    std::fs::copy("tests/fixtures/skill-v0.2.0.md", &old).unwrap();
    place_skill(home, &[Codex], false).unwrap();
    assert!(!home.join(".claude/skills/clio-memory").exists());
}

#[test]
fn foreign_skill_blocks_without_replace() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path();
    let foreign = skill_at(home, ".agents/skills");
    std::fs::create_dir_all(foreign.parent().unwrap()).unwrap();
    std::fs::write(&foreign, "someone else's skill").unwrap();

    // Claude would be written first: nothing may be written when a later folder blocks.
    let err = place_skill(home, &[ClaudeCode, Codex], false).unwrap_err();
    assert!(err.contains(&foreign.display().to_string()), "{err}");
    assert!(!home.join(".claude").exists());
    assert_eq!(
        std::fs::read_to_string(&foreign).unwrap(),
        "someone else's skill"
    );

    place_skill(home, &[ClaudeCode, Codex], true).unwrap();
    assert_eq!(std::fs::read_to_string(&foreign).unwrap(), SKILL);
    assert!(skill_at(home, ".claude/skills").exists());
}

#[test]
fn a_foreign_skill_in_an_unused_dir_is_left_alone() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path();
    let foreign = skill_at(home, ".claude/skills");
    std::fs::create_dir_all(foreign.parent().unwrap()).unwrap();
    std::fs::write(&foreign, "someone else's skill").unwrap();

    place_skill(home, &[Codex], false).unwrap();
    assert_eq!(
        std::fs::read_to_string(&foreign).unwrap(),
        "someone else's skill"
    );
}

#[test]
fn detect_finds_by_program_and_by_folder() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let bin = t.path().join("bin");
    let codex = fake_program(&bin, "codex");
    fake_program(&bin, "gemini");
    // A file that is not executable is not a program.
    std::fs::write(bin.join("opencode"), "").unwrap();
    std::fs::create_dir_all(home.join(".gemini/config")).unwrap();

    let svc = |_: &[&str]| -> Result<(), String> { Ok(()) };
    let env = Env {
        home: home.clone(),
        exe: PathBuf::from("/x/clio-memory"),
        path: vec![t.path().join("nowhere"), bin.clone()],
        codex_home: None,
        claude: None,
        bin_dir: None,
        service: &svc,
    };
    assert_eq!(
        detect(&env),
        vec![
            Found {
                agent: Codex,
                at: codex
            },
            Found {
                agent: Gemini,
                at: bin.join("gemini")
            },
            Found {
                agent: Antigravity,
                at: home.join(".gemini/config")
            },
        ]
    );
}

#[test]
fn detect_prefers_the_override_and_the_program_over_the_folder() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let bin = t.path().join("bin");
    let agy = fake_program(&bin, "agy");
    let claude = fake_program(&t.path().join("elsewhere"), "claude");
    std::fs::create_dir_all(home.join(".config/opencode")).unwrap();
    std::fs::create_dir_all(home.join(".gemini/config")).unwrap();

    let svc = |_: &[&str]| -> Result<(), String> { Ok(()) };
    let env = Env {
        home: home.clone(),
        exe: PathBuf::from("/x/clio-memory"),
        path: vec![bin],
        codex_home: None,
        claude: Some(claude.clone()),
        bin_dir: None,
        service: &svc,
    };
    assert_eq!(
        detect(&env),
        vec![
            Found {
                agent: ClaudeCode,
                at: claude
            },
            Found {
                agent: OpenCode,
                at: home.join(".config/opencode")
            },
            Found {
                agent: Antigravity,
                at: agy
            },
        ]
    );
}

#[test]
fn detect_finds_nothing_in_an_empty_machine() {
    let t = tempfile::tempdir().unwrap();
    let svc = |_: &[&str]| -> Result<(), String> { Ok(()) };
    let env = Env {
        home: t.path().to_path_buf(),
        exe: PathBuf::from("/x/clio-memory"),
        path: vec![],
        codex_home: None,
        claude: None,
        bin_dir: None,
        service: &svc,
    };
    assert_eq!(detect(&env), vec![]);
}

#[test]
fn place_skill_says_what_it_did_and_leaves_a_current_copy_alone() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path();
    let first = place_skill(home, &[ClaudeCode], false).unwrap();
    assert!(first[0].contains("instalada"), "{first:?}");

    let again = place_skill(home, &[ClaudeCode], false).unwrap();
    assert!(again[0].contains("al día"), "{again:?}");

    let old = skill_at(home, ".claude/skills");
    std::fs::copy("tests/fixtures/skill-v0.2.0.md", &old).unwrap();
    let updated = place_skill(home, &[ClaudeCode], false).unwrap();
    assert!(updated[0].contains("actualizada"), "{updated:?}");
}

#[test]
fn what_cannot_be_read_as_a_skill_is_foreign() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("SKILL.md");
    std::fs::create_dir(&p).unwrap();
    assert_eq!(skill_state(&p), SkillFile::Foreign);
}

fn env_with<'a>(
    home: &Path,
    path: Vec<PathBuf>,
    claude: Option<PathBuf>,
    svc: &'a dyn Fn(&[&str]) -> Result<(), String>,
) -> Env<'a> {
    Env {
        home: home.to_path_buf(),
        exe: PathBuf::from("/x/clio-memory"),
        path,
        codex_home: None,
        claude,
        bin_dir: None,
        service: svc,
    }
}

#[test]
fn the_claude_override_beats_the_one_on_the_path() {
    let t = tempfile::tempdir().unwrap();
    let on_path = fake_program(&t.path().join("bin"), "claude");
    let chosen = fake_program(&t.path().join("mine"), "claude");
    let svc = |_: &[&str]| -> Result<(), String> { Ok(()) };

    let env = env_with(
        t.path(),
        vec![t.path().join("bin")],
        Some(chosen.clone()),
        &svc,
    );
    assert_eq!(
        detect(&env),
        vec![Found {
            agent: ClaudeCode,
            at: chosen
        }]
    );
    let env = env_with(t.path(), vec![t.path().join("bin")], None, &svc);
    assert_eq!(
        detect(&env),
        vec![Found {
            agent: ClaudeCode,
            at: on_path
        }]
    );
}

#[test]
fn antigravity_is_agy_then_antigravity() {
    let t = tempfile::tempdir().unwrap();
    let svc = |_: &[&str]| -> Result<(), String> { Ok(()) };
    let long = fake_program(&t.path().join("a"), "antigravity");
    let env = env_with(t.path(), vec![t.path().join("a")], None, &svc);
    assert_eq!(
        detect(&env),
        vec![Found {
            agent: Antigravity,
            at: long
        }]
    );

    let short = fake_program(&t.path().join("b"), "agy");
    let env = env_with(
        t.path(),
        vec![t.path().join("a"), t.path().join("b")],
        None,
        &svc,
    );
    assert_eq!(
        detect(&env),
        vec![Found {
            agent: Antigravity,
            at: short
        }]
    );
}

#[test]
fn a_directory_called_like_a_program_is_not_one() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(t.path().join("bin/codex")).unwrap();
    let svc = |_: &[&str]| -> Result<(), String> { Ok(()) };
    let env = env_with(t.path(), vec![t.path().join("bin")], None, &svc);
    assert_eq!(env.program("codex"), None);
    assert_eq!(detect(&env), vec![]);
}

// An old `cargo install` first on the `PATH` runs instead of this binary: it is named; this binary
// itself, or a link to it, is not.
#[test]
fn another_clio_memory_first_on_the_path_is_found() {
    use clio_memory::setup::agents::shadowing;
    let t = tempfile::tempdir().unwrap();
    let ours = fake_program(&t.path().join("local/bin"), "clio-memory");
    let cargo = fake_program(&t.path().join("cargo/bin"), "clio-memory");
    let links = t.path().join("links");
    std::fs::create_dir_all(&links).unwrap();
    std::os::unix::fs::symlink(&ours, links.join("clio-memory")).unwrap();
    let service = |_: &[&str]| Ok(());
    let env = |path: Vec<PathBuf>| Env {
        home: t.path().join("home"),
        exe: ours.clone(),
        path,
        codex_home: None,
        claude: None,
        bin_dir: None,
        service: &service,
    };
    let local = t.path().join("local/bin");
    let cargo_dir = t.path().join("cargo/bin");
    assert_eq!(
        shadowing(&env(vec![cargo_dir.clone(), local.clone()])),
        Some(cargo.clone())
    );
    assert_eq!(
        shadowing(&env(vec![local.clone(), cargo_dir.clone()])),
        None
    );
    assert_eq!(shadowing(&env(vec![links, cargo_dir])), None);
    assert_eq!(shadowing(&env(vec![])), None);
}
