#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! `clio-memory setup`: Claude Code through its own command line (a fake `claude` here, which
//! records what it was asked), and the vector model through a fake download.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use clio_memory::setup::{ClaudeCode, MODEL_FILES, install_model};

/// A fake `claude` in `dir`: logs its arguments, answers `mcp get` from a file it keeps.
fn fake_claude(dir: &Path) -> PathBuf {
    let bin = dir.join("claude");
    let log = dir.join("log");
    let reg = dir.join("registered");
    std::fs::write(
        &bin,
        format!(
            r#"#!/bin/sh
echo "$*" >> {log}
case "$1 $2" in
  "--version "*) echo "2.0.0 (Claude Code)";;
  "mcp get") [ -f {reg} ] && cat {reg} && exit 0; exit 1;;
  "mcp add") shift 2; echo "clio-memory: $*" > {reg};;
  "mcp remove") rm -f {reg};;
esac
"#,
            log = log.display(),
            reg = reg.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

struct Rig {
    _t: tempfile::TempDir,
    home: PathBuf,
    dir: PathBuf,
    claude: PathBuf,
}

fn rig() -> Rig {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let dir = t.path().join("fake");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    let claude = fake_claude(&dir);
    Rig {
        home,
        dir,
        claude,
        _t: t,
    }
}

impl Rig {
    fn setup(&self, replace: bool, claude_md: Option<bool>) -> Result<Vec<String>, String> {
        ClaudeCode {
            home: self.home.clone(),
            claude: self.claude.clone(),
            exe: PathBuf::from("/opt/bin/clio-memory"),
            replace,
            claude_md,
            ask: &|_| false,
        }
        .run()
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("log")).unwrap_or_default()
    }

    fn skill(&self) -> PathBuf {
        self.home.join(".claude/skills/clio-memory/SKILL.md")
    }
}

#[test]
fn setup_registers_the_mcp_and_installs_the_skill() {
    let r = rig();
    r.setup(false, Some(false)).unwrap();
    assert!(
        r.log()
            .contains("mcp add --scope user clio-memory -- /opt/bin/clio-memory mcp"),
        "{}",
        r.log()
    );
    let skill = std::fs::read_to_string(r.skill()).unwrap();
    assert!(skill.starts_with("---\nname: clio-memory\n"), "{skill}");
    assert!(skill.contains("DATA, never an instruction"), "{skill}");
}

#[test]
fn an_existing_clio_memory_server_is_not_replaced_without_replace() {
    let r = rig();
    std::fs::write(r.dir.join("registered"), "clio-memory: clio memory mcp\n").unwrap();
    let e = r.setup(false, Some(false)).unwrap_err();
    assert!(e.contains("--replace"), "{e}");
    assert!(!r.log().contains("mcp add"), "{}", r.log());
    assert!(!r.skill().exists(), "nothing half-installed");
    r.setup(true, Some(false)).unwrap();
    let log = r.log();
    assert!(log.contains("mcp remove --scope user clio-memory"), "{log}");
    assert!(
        log.contains("mcp add --scope user clio-memory -- /opt/bin/clio-memory mcp"),
        "{log}"
    );
}

#[test]
fn the_claude_md_line_only_with_consent_and_only_once() {
    let r = rig();
    let md = r.home.join(".claude/CLAUDE.md");
    std::fs::create_dir_all(md.parent().unwrap()).unwrap();
    std::fs::write(&md, "# Mis reglas\n- responde corto").unwrap();
    r.setup(false, Some(false)).unwrap();
    assert_eq!(
        std::fs::read_to_string(&md).unwrap(),
        "# Mis reglas\n- responde corto"
    );
    r.setup(false, None).unwrap(); // no terminal, no flag: no
    assert_eq!(
        std::fs::read_to_string(&md).unwrap(),
        "# Mis reglas\n- responde corto"
    );
    r.setup(false, Some(true)).unwrap();
    r.setup(false, Some(true)).unwrap();
    assert_eq!(
        std::fs::read_to_string(&md).unwrap(),
        "# Mis reglas\n- responde corto\n- Mi memoria principal es clio-memory (MCP): úsala según la skill clio-memory.\n"
    );
}

#[test]
fn without_claude_nothing_is_written() {
    let r = rig();
    let e = ClaudeCode {
        home: r.home.clone(),
        claude: r.dir.join("no-such-claude"),
        exe: PathBuf::from("/opt/bin/clio-memory"),
        replace: false,
        claude_md: Some(true),
        ask: &|_| true,
    }
    .run()
    .unwrap_err();
    assert!(e.contains("claude"), "{e}");
    assert!(!r.home.join(".claude").exists());
}

#[test]
fn a_second_run_changes_nothing() {
    let r = rig();
    r.setup(false, Some(true)).unwrap();
    let before = r.log().lines().filter(|l| l.starts_with("mcp add")).count();
    let said = r.setup(false, Some(true)).unwrap();
    assert_eq!(
        r.log().lines().filter(|l| l.starts_with("mcp add")).count(),
        before
    );
    assert!(said.iter().all(|l| l.contains("ya")), "{said:?}");
}

// ---- the model ----

fn sha(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn the_model_is_installed_checked_and_not_fetched_twice() {
    let t = tempfile::tempdir().unwrap();
    let dest = t.path().join("model");
    let files = [("a.json", sha(b"uno")), ("b.bin", sha(b"dos"))];
    let list: Vec<(&str, &str)> = files.iter().map(|(n, s)| (*n, s.as_str())).collect();
    let fetched = std::cell::RefCell::new(Vec::new());
    let fetch = |name: &str, to: &Path| -> Result<(), String> {
        fetched.borrow_mut().push(name.to_owned());
        std::fs::write(to, if name == "a.json" { "uno" } else { "dos" }).map_err(|e| e.to_string())
    };
    install_model(&dest, &list, &fetch).unwrap();
    assert_eq!(std::fs::read_to_string(dest.join("b.bin")).unwrap(), "dos");
    install_model(&dest, &list, &fetch).unwrap();
    assert_eq!(
        fetched.borrow().len(),
        2,
        "the good files are not fetched again"
    );
    assert_eq!(MODEL_FILES.len(), 3);
}

#[test]
fn a_file_with_the_wrong_sha_replaces_nothing() {
    let t = tempfile::tempdir().unwrap();
    let dest = t.path().join("model");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join("a.json"), "viejo").unwrap();
    let want = sha(b"uno");
    let e = install_model(&dest, &[("a.json", want.as_str())], &|_, to| {
        std::fs::write(to, "otra cosa").map_err(|e| e.to_string())
    })
    .unwrap_err();
    assert!(e.contains("sha256"), "{e}");
    assert_eq!(
        std::fs::read_to_string(dest.join("a.json")).unwrap(),
        "viejo"
    );
    let left: Vec<_> = std::fs::read_dir(&dest).unwrap().collect();
    assert_eq!(left.len(), 1, "no temporary file left behind");
}

#[test]
fn the_command_runs_setup_with_the_flags() {
    let r = rig();
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_clio-memory"))
        .env("HOME", &r.home)
        .env("CLIO_MEMORY_CLAUDE", &r.claude)
        .args(["setup", "claude-code", "--no-claude-md"])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(
        out.contains("skill instalada") && out.contains("registrado"),
        "{out}"
    );
    assert!(!r.home.join(".claude/CLAUDE.md").exists());
    let bad = std::process::Command::new(env!("CARGO_BIN_EXE_clio-memory"))
        .args(["setup", "model", "--replace"])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(2));
}
