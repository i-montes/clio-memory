#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! The instructions line in each agent's file, under a temporary home. Nothing here runs a
//! program: the files are all there is.

use std::path::{Path, PathBuf};

use clio_memory::setup::INSTRUCTIONS_LINE;
use clio_memory::setup::agents::{AgentId, Env};
use clio_memory::setup::instructions::{
    add_instructions, has_instructions, instructions_file, remove_instructions,
};
use serde_json::{Value, json};

struct Rig {
    _t: tempfile::TempDir,
    home: PathBuf,
    codex_home: Option<PathBuf>,
}

fn no_service(_: &[&str]) -> Result<(), String> {
    Ok(())
}

fn rig() -> Rig {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("my home");
    std::fs::create_dir_all(&home).unwrap();
    Rig {
        _t: t,
        home,
        codex_home: None,
    }
}

impl Rig {
    fn env(&self) -> Env<'static> {
        Env {
            home: self.home.clone(),
            exe: PathBuf::from("/opt/clio-memory"),
            path: vec![],
            codex_home: self.codex_home.clone(),
            claude: None,
            service: &no_service,
        }
    }

    fn opencode_dir(&self) -> PathBuf {
        self.home.join(".config/opencode")
    }

    fn opencode_json(&self) -> PathBuf {
        self.opencode_dir().join("opencode.json")
    }

    fn opencode_md(&self) -> PathBuf {
        self.opencode_dir().join("clio-memory.md")
    }
}

fn count_line(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|l| l.trim() == INSTRUCTIONS_LINE)
        .count()
}

fn put(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn files_per_agent() {
    let mut r = rig();
    let h = r.home.clone();
    let env = r.env();
    assert_eq!(
        instructions_file(&env, AgentId::ClaudeCode),
        h.join(".claude/CLAUDE.md")
    );
    assert_eq!(
        instructions_file(&env, AgentId::Gemini),
        h.join(".gemini/GEMINI.md")
    );
    assert_eq!(
        instructions_file(&env, AgentId::Antigravity),
        h.join(".gemini/GEMINI.md")
    );
    assert_eq!(
        instructions_file(&env, AgentId::OpenCode),
        h.join(".config/opencode/clio-memory.md")
    );
    // Codex: the default folder, then `CODEX_HOME`.
    assert_eq!(
        instructions_file(&env, AgentId::Codex),
        h.join(".codex/AGENTS.md")
    );
    r.codex_home = Some(h.join("elsewhere"));
    assert_eq!(
        instructions_file(&r.env(), AgentId::Codex),
        h.join("elsewhere/AGENTS.md")
    );
}

#[test]
fn add_is_idempotent() {
    let r = rig();
    let env = r.env();
    for agent in AgentId::ALL {
        // Antigravity shares Gemini's file, which has the line by now.
        let shared = agent == AgentId::Antigravity;
        assert_eq!(has_instructions(&env, agent), shared, "{agent:?}");
        let first = add_instructions(&env, agent).unwrap();
        assert_eq!(first.is_some(), !shared, "{agent:?}");
        assert!(has_instructions(&env, agent), "{agent:?}");
        assert_eq!(add_instructions(&env, agent).unwrap(), None, "{agent:?}");
        assert_eq!(count_line(&instructions_file(&env, agent)), 1, "{agent:?}");
    }
    // OpenCode: the file's absolute path is in `instructions`, once.
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(r.opencode_json()).unwrap()).unwrap();
    assert_eq!(
        config["instructions"],
        json!([r.opencode_md().to_str().unwrap()])
    );
}

#[test]
fn opencode_never_creates_agents_md() {
    let r = rig();
    add_instructions(&r.env(), AgentId::OpenCode).unwrap();
    assert!(r.opencode_md().is_file());
    assert!(!r.opencode_dir().join("AGENTS.md").exists());
}

#[test]
fn gemini_md_is_shared() {
    let r = rig();
    let env = r.env();
    add_instructions(&env, AgentId::Gemini).unwrap();
    add_instructions(&env, AgentId::Antigravity).unwrap();
    let md = r.home.join(".gemini/GEMINI.md");

    // Gemini goes, Antigravity stays: the line stays.
    assert_eq!(
        remove_instructions(&env, AgentId::Gemini, &[AgentId::Antigravity]).unwrap(),
        None
    );
    assert_eq!(count_line(&md), 1);
    // Antigravity goes with nobody left: the line goes.
    assert_eq!(
        remove_instructions(&env, AgentId::Antigravity, &[]).unwrap(),
        Some(md.clone())
    );
    assert_eq!(count_line(&md), 0);

    // And the other way round.
    add_instructions(&env, AgentId::Gemini).unwrap();
    assert_eq!(
        remove_instructions(&env, AgentId::Antigravity, &[AgentId::Gemini]).unwrap(),
        None
    );
    assert_eq!(count_line(&md), 1);
    // An agent that is not Gemini's does not keep the line either.
    assert!(
        remove_instructions(&env, AgentId::Gemini, &[AgentId::ClaudeCode])
            .unwrap()
            .is_some()
    );
    assert_eq!(count_line(&md), 0);
}

#[test]
fn remove_keeps_other_lines() {
    let r = rig();
    let env = r.env();
    let md = r.home.join(".claude/CLAUDE.md");
    put(
        &md,
        &format!("# Mis reglas\n- corto\n{INSTRUCTIONS_LINE}\n"),
    );
    assert!(has_instructions(&env, AgentId::ClaudeCode));
    assert_eq!(
        remove_instructions(&env, AgentId::ClaudeCode, &[]).unwrap(),
        Some(md.clone())
    );
    assert_eq!(
        std::fs::read_to_string(&md).unwrap(),
        "# Mis reglas\n- corto\n"
    );

    // In the middle, with indentation and no final newline: foreign bytes are as they were.
    put(&md, &format!("a\r\n  {INSTRUCTIONS_LINE}  \n\n  - mía\nz"));
    remove_instructions(&env, AgentId::ClaudeCode, &[]).unwrap();
    assert_eq!(std::fs::read_to_string(&md).unwrap(), "a\r\n\n  - mía\nz");

    // Nothing to remove: no change, and a missing file is not an error.
    assert_eq!(
        remove_instructions(&env, AgentId::ClaudeCode, &[]).unwrap(),
        None
    );
    std::fs::remove_file(&md).unwrap();
    assert_eq!(
        remove_instructions(&env, AgentId::ClaudeCode, &[]).unwrap(),
        None
    );
    assert!(!md.exists());
}

#[test]
fn add_keeps_the_text_before_it() {
    let r = rig();
    let env = r.env();
    let md = r.home.join(".claude/CLAUDE.md");
    put(&md, "# Mis reglas\n- corto");
    add_instructions(&env, AgentId::ClaudeCode).unwrap();
    assert_eq!(
        std::fs::read_to_string(&md).unwrap(),
        format!("# Mis reglas\n- corto\n{INSTRUCTIONS_LINE}\n")
    );
    // A file that ends in a newline gets no blank line, an empty one gets just the line.
    put(&md, "# Mis reglas\n");
    add_instructions(&env, AgentId::ClaudeCode).unwrap();
    assert_eq!(
        std::fs::read_to_string(&md).unwrap(),
        format!("# Mis reglas\n{INSTRUCTIONS_LINE}\n")
    );
    put(&md, "");
    add_instructions(&env, AgentId::ClaudeCode).unwrap();
    assert_eq!(
        std::fs::read_to_string(&md).unwrap(),
        format!("{INSTRUCTIONS_LINE}\n")
    );
}

#[test]
fn codex_override_wins() {
    let mut r = rig();
    let dir = r.home.join("codex home");
    r.codex_home = Some(dir.clone());
    let env = r.env();
    // An empty override does not count: AGENTS.md.
    put(&dir.join("AGENTS.override.md"), "");
    assert_eq!(
        instructions_file(&env, AgentId::Codex),
        dir.join("AGENTS.md")
    );
    // A non-empty one wins.
    put(&dir.join("AGENTS.override.md"), "# Solo aquí\n");
    put(&dir.join("AGENTS.md"), "# Base\n");
    assert_eq!(
        add_instructions(&env, AgentId::Codex).unwrap(),
        Some(dir.join("AGENTS.override.md"))
    );
    assert_eq!(count_line(&dir.join("AGENTS.override.md")), 1);
    assert_eq!(
        std::fs::read_to_string(dir.join("AGENTS.md")).unwrap(),
        "# Base\n"
    );
}

#[test]
fn opencode_keeps_the_config_it_does_not_own() {
    let r = rig();
    let env = r.env();
    let md = r.opencode_md();
    // Foreign keys, and foreign `instructions` entries on both sides of where ours will go.
    let before = r#"{
  "theme": "tokyo",
  "instructions": [
    "CONTRIBUTING.md",
    "docs/*.md"
  ],
  "mcp": {
    "other": {
      "type": "local",
      "command": [
        "x"
      ]
    }
  },
  "model": "m"
}
"#;
    put(&r.opencode_json(), before);
    add_instructions(&env, AgentId::OpenCode).unwrap();
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(r.opencode_json()).unwrap()).unwrap();
    assert_eq!(
        config["instructions"],
        json!(["CONTRIBUTING.md", "docs/*.md", md.to_str().unwrap()])
    );
    let keys: Vec<_> = config.as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["theme", "instructions", "mcp", "model"]);
    assert_eq!(
        config["mcp"],
        json!({"other": {"type": "local", "command": ["x"]}})
    );

    // Removing puts the file back byte for byte.
    assert_eq!(
        remove_instructions(&env, AgentId::OpenCode, &[]).unwrap(),
        Some(md.clone())
    );
    assert!(!md.exists());
    assert_eq!(std::fs::read_to_string(r.opencode_json()).unwrap(), before);
    assert!(!has_instructions(&env, AgentId::OpenCode));
}

#[test]
fn opencode_empty_instructions_goes_and_siblings_keep_their_order() {
    let r = rig();
    let env = r.env();
    // Ours is the only entry and `instructions` is not the last key: removing the key must not
    // move the last key into its place.
    let config = json!({"a": 1, "instructions": [r.opencode_md().to_str().unwrap()], "b": 2, "c": 3, "d": 4});
    put(&r.opencode_json(), &config.to_string());
    put(&r.opencode_md(), &format!("{INSTRUCTIONS_LINE}\n"));
    remove_instructions(&env, AgentId::OpenCode, &[]).unwrap();
    let after: Value =
        serde_json::from_str(&std::fs::read_to_string(r.opencode_json()).unwrap()).unwrap();
    let keys: Vec<_> = after.as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["a", "b", "c", "d"]);

    // Ours between two foreign entries: they stay, in order, and so does the key.
    let config = json!({"a": 1, "instructions": ["x.md", r.opencode_md().to_str().unwrap(), "y.md", "z.md"], "b": 2});
    put(&r.opencode_json(), &config.to_string());
    remove_instructions(&env, AgentId::OpenCode, &[]).unwrap();
    let after: Value =
        serde_json::from_str(&std::fs::read_to_string(r.opencode_json()).unwrap()).unwrap();
    assert_eq!(after["instructions"], json!(["x.md", "y.md", "z.md"]));
    let keys: Vec<_> = after.as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["a", "instructions", "b"]);
}

#[test]
fn opencode_remove_without_config_creates_nothing() {
    let r = rig();
    let env = r.env();
    assert_eq!(
        remove_instructions(&env, AgentId::OpenCode, &[]).unwrap(),
        None
    );
    assert!(!r.opencode_json().exists());
    assert!(!r.opencode_dir().exists());
}

#[test]
fn opencode_config_that_is_not_ours_to_read_is_an_error() {
    let r = rig();
    let env = r.env();
    for bad in [
        "[]",
        "3",
        r#"{"instructions": "CONTRIBUTING.md"}"#,
        r#"{"instructions": {"a": 1}}"#,
        "{ // comment\n}",
    ] {
        put(&r.opencode_json(), bad);
        for result in [
            add_instructions(&env, AgentId::OpenCode),
            remove_instructions(&env, AgentId::OpenCode, &[]),
        ] {
            let e = result.unwrap_err();
            assert!(e.contains("opencode.json"), "{bad}: {e}");
        }
        // Nothing was written or created.
        assert_eq!(std::fs::read_to_string(r.opencode_json()).unwrap(), bad);
        assert!(!r.opencode_md().exists(), "{bad}");
    }
}

#[test]
fn opencode_has_instructions_needs_both_halves() {
    let r = rig();
    let env = r.env();
    add_instructions(&env, AgentId::OpenCode).unwrap();
    // The file without its entry (a run cut in half): not there, and adding finishes the job.
    std::fs::write(r.opencode_json(), "{}\n").unwrap();
    assert!(!has_instructions(&env, AgentId::OpenCode));
    assert!(add_instructions(&env, AgentId::OpenCode).unwrap().is_some());
    assert!(has_instructions(&env, AgentId::OpenCode));
    // The entry without its file.
    std::fs::remove_file(r.opencode_md()).unwrap();
    assert!(!has_instructions(&env, AgentId::OpenCode));
    add_instructions(&env, AgentId::OpenCode).unwrap();
    assert_eq!(count_line(&r.opencode_md()), 1);
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(r.opencode_json()).unwrap()).unwrap();
    assert_eq!(config["instructions"].as_array().unwrap().len(), 1);
}

#[test]
fn opencode_entry_must_be_ours_and_a_stale_file_is_rewritten() {
    let r = rig();
    let env = r.env();
    let md = r.opencode_md();
    // The file is ours and right, but `instructions` only has other entries.
    put(&md, &format!("{INSTRUCTIONS_LINE}\n"));
    put(&r.opencode_json(), r#"{"instructions": ["x.md"]}"#);
    assert!(!has_instructions(&env, AgentId::OpenCode));
    add_instructions(&env, AgentId::OpenCode).unwrap();
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(r.opencode_json()).unwrap()).unwrap();
    assert_eq!(
        config["instructions"],
        json!(["x.md", md.to_str().unwrap()])
    );

    // The entry is there, but the file has something else in it: it is ours, so it is redone.
    std::fs::write(&md, "viejo\n").unwrap();
    assert!(!has_instructions(&env, AgentId::OpenCode));
    add_instructions(&env, AgentId::OpenCode).unwrap();
    assert_eq!(
        std::fs::read_to_string(&md).unwrap(),
        format!("{INSTRUCTIONS_LINE}\n")
    );
    assert!(has_instructions(&env, AgentId::OpenCode));
}

#[test]
fn unreadable_instructions_file_is_an_error_not_overwritten() {
    let r = rig();
    let env = r.env();
    let md = r.home.join(".claude/CLAUDE.md");
    // Not UTF-8: it cannot be read as text, so it must not be replaced by a guess.
    put(&md, "");
    std::fs::write(&md, [0xff, 0xfe, b'\n']).unwrap();
    let e = add_instructions(&env, AgentId::ClaudeCode).unwrap_err();
    assert!(e.contains("CLAUDE.md"), "{e}");
    assert_eq!(std::fs::read(&md).unwrap(), [0xff, 0xfe, b'\n']);
    let e = remove_instructions(&env, AgentId::ClaudeCode, &[]).unwrap_err();
    assert!(e.contains("CLAUDE.md"), "{e}");
    assert_eq!(std::fs::read(&md).unwrap(), [0xff, 0xfe, b'\n']);
}

#[test]
fn writing_the_line_keeps_the_files_mode() {
    use std::os::unix::fs::PermissionsExt;
    let r = rig();
    let env = r.env();
    let md = r.home.join(".claude/CLAUDE.md");
    put(&md, "# Mis reglas\n");
    std::fs::set_permissions(&md, std::fs::Permissions::from_mode(0o600)).unwrap();
    add_instructions(&env, AgentId::ClaudeCode).unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&md), 0o600, "adding");
    std::fs::set_permissions(&md, std::fs::Permissions::from_mode(0o640)).unwrap();
    remove_instructions(&env, AgentId::ClaudeCode, &[]).unwrap();
    assert_eq!(mode(&md), 0o640, "removing");
}

#[test]
fn codex_removal_takes_the_line_from_both_files() {
    let mut r = rig();
    let dir = r.home.join("codex home");
    r.codex_home = Some(dir.clone());
    let env = r.env();
    // The line went into AGENTS.md; the person later wrote an override.
    put(
        &dir.join("AGENTS.md"),
        &format!("# Base\n{INSTRUCTIONS_LINE}\n"),
    );
    put(
        &dir.join("AGENTS.override.md"),
        &format!("# Solo aquí\n{INSTRUCTIONS_LINE}\n"),
    );
    assert!(
        remove_instructions(&env, AgentId::Codex, &[])
            .unwrap()
            .is_some()
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("AGENTS.md")).unwrap(),
        "# Base\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("AGENTS.override.md")).unwrap(),
        "# Solo aquí\n"
    );
    // Only in AGENTS.md, behind a non-empty override that lacks it.
    put(&dir.join("AGENTS.md"), &format!("{INSTRUCTIONS_LINE}\n"));
    assert_eq!(
        remove_instructions(&env, AgentId::Codex, &[]).unwrap(),
        Some(dir.join("AGENTS.md"))
    );
    assert_eq!(std::fs::read_to_string(dir.join("AGENTS.md")).unwrap(), "");
}
