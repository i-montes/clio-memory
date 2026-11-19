#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! `clio-memory m <verb>`: the memory tools as commands an agent runs from its shell.

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use clio_memory::*;
use tempfile::TempDir;

const EXE: &str = env!("CARGO_BIN_EXE_clio-memory");

/// A temporary home and data folder. With the daemon, it is stopped when they are dropped.
struct Dirs {
    _t: TempDir,
    home: PathBuf,
    data: PathBuf,
    daemon: bool,
}

/// What one invocation did.
#[derive(Debug)]
struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Dirs {
    fn new() -> Self {
        // Short: the daemon's socket path must stay under 100 bytes (macOS).
        let t = common::short_dir();
        let home = t.path().join("home");
        let data = t.path().join("data");
        std::fs::create_dir(&home).unwrap();
        Dirs {
            _t: t,
            home,
            data,
            daemon: false,
        }
    }

    fn with_daemon() -> Self {
        let mut t = Self::new();
        t.daemon = true;
        t
    }

    fn command(&self, dir: &Path, env: &[(&str, &str)]) -> Command {
        let mut cmd = Command::new(EXE);
        cmd.arg("--db")
            .arg(self.data.join("memory.db"))
            .env("HOME", &self.home)
            .env("CLIO_MEMORY_DAEMON_IDLE", "30")
            // Whatever runs these tests (Claude Code among them) is not the caller here.
            .env_remove("CLAUDECODE")
            .env_remove("CLAUDE_CODE_SESSION_ID")
            .env_remove("CLIO_MEMORY_AGENT")
            .env_remove("CLIO_PROJECT")
            .env_remove("CLIO_MEMORY_NO_DAEMON")
            .current_dir(dir);
        if !self.daemon {
            cmd.env("CLIO_MEMORY_NO_DAEMON", "1");
        }
        cmd.envs(env.iter().copied());
        cmd
    }

    /// `clio-memory m ARGS` in `dir`, with `env` added and `input` on its stdin.
    fn run(&self, dir: &Path, args: &[&str], env: &[(&str, &str)], input: &str) -> Out {
        let mut child = self
            .command(dir, env)
            .arg("m")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(input.as_bytes()).unwrap();
        drop(stdin);
        let o = child.wait_with_output().unwrap();
        Out {
            code: o.status.code().unwrap(),
            stdout: String::from_utf8(o.stdout).unwrap(),
            stderr: String::from_utf8(o.stderr).unwrap(),
        }
    }

    fn m(&self, args: &[&str]) -> Out {
        self.run(&self.home, args, &[], "")
    }

    fn m_stdin(&self, args: &[&str], input: &str) -> Out {
        self.run(&self.home, args, &[], input)
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        if self.daemon {
            let _ = self
                .command(&self.home, &[])
                .args(["daemon", "--stop"])
                .output();
        }
    }
}

#[test]
fn text_from_stdin_is_saved_as_typed() {
    let t = Dirs::new();
    let text = "Qué: usar `cargo test -- --ignored` y $HOME sin \"comillas\" raras";
    let out = t.m_stdin(&["save", "--title", "Órdenes de prueba", "-"], text);
    assert!(out.stdout.starts_with("saved"), "{out:?}");
    let got = t.m(&["search", "cargo", "test"]);
    assert_eq!(got.code, 0);
    assert!(
        got.stdout.contains("cargo test -- --ignored") && got.stdout.contains("$HOME"),
        "{}",
        got.stdout
    );
}

#[test]
fn a_refusal_is_exit_1_with_its_words_on_stdout() {
    let t = Dirs::new();
    let out = t.m(&["get", "999"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stdout
            .contains("ese recuerdo no existe o no se puede leer"),
        "{out:?}"
    );
}

#[test]
fn a_typo_is_exit_2_and_changes_nothing() {
    let t = Dirs::new();
    let out = t.m(&["sav", "hola"]);
    assert_eq!(out.code, 2, "{out:?}");
    assert!(
        out.stderr.contains("unknown memory verb \"sav\""),
        "{out:?}"
    );
    assert!(!t.data.join("memory.db").exists());
}

#[test]
fn the_global_options_are_refused_before_m() {
    let t = Dirs::new();
    for opt in [&["--actor", "x"][..], &["--model", "/m"], &["--json"]] {
        let o = t
            .command(&t.home, &[])
            .args(opt)
            .args(["m", "save", "Qué: x"])
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(2), "{opt:?}: {err}");
        assert!(
            err.contains("do not apply to m; use m --agent"),
            "{opt:?}: {err}"
        );
    }
    assert!(!t.data.join("memory.db").exists());
}

#[test]
fn the_agent_signs_as_before() {
    let t = Dirs::new();
    let out = t.m(&["--agent", "codex", "save", "Qué: x"]);
    assert!(out.stdout.starts_with("saved"), "{out:?}");
    let e = Engine::open(
        &t.data.join("memory.db"),
        OpenMode::ReadOnly,
        Arc::new(SystemClock),
    )
    .unwrap();
    let names: Vec<String> = e.actors().unwrap().into_iter().map(|a| a.name).collect();
    assert!(names.contains(&"mcp:codex".to_owned()), "{names:?}");
}

#[test]
fn the_project_is_the_folder_it_runs_in() {
    let t = Dirs::new();
    let (a, b) = (t.home.join("w/alfa"), t.home.join("w/beta"));
    for (dir, name) in [(&a, "alfa"), (&b, "beta")] {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(".clio-project"), name).unwrap();
    }
    for (dir, name) in [(&a, "alfa"), (&b, "beta")] {
        let out = t.run(dir, &["project"], &[], "");
        assert_eq!((out.code, out.stdout.trim()), (0, name), "{out:?}");
    }
}

#[test]
fn it_goes_through_the_daemon_when_there_is_one() {
    let t = Dirs::with_daemon();
    let out = t.m(&["save", "Qué:", "el", "demonio", "guarda", "esto"]);
    assert!(out.stdout.starts_with("saved"), "{out:?}");
    assert!(t.data.join("daemon.sock").exists(), "{out:?}");
    assert!(!out.stderr.contains("sin demonio"), "{out:?}");
    let got = t.m(&["search", "demonio"]);
    assert_eq!(got.code, 0, "{got:?}");
    assert!(got.stdout.contains("el demonio guarda esto"), "{got:?}");
}
