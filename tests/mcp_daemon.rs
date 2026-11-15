#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! `clio-memory mcp` as a thin client: it answers MCP itself and sends every tool call to the
//! daemon of its data folder, and serves in its own process when it cannot.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use clio_memory::daemon::request;
use serde_json::{Value, json};
use tempfile::TempDir;

const EXE: &str = env!("CARGO_BIN_EXE_clio-memory");

/// A data folder with its own home; its daemon is shut down when it is dropped.
struct Folder {
    _dir: TempDir,
    data: PathBuf,
    home: PathBuf,
}

fn folder() -> Folder {
    let dir = common::short_dir();
    let data = dir.path().join("data");
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    Folder {
        _dir: dir,
        data,
        home,
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = request(&self.data, None, &json!({"op": "shutdown"}));
    }
}

impl Folder {
    /// The pid of the daemon that runs (it must run).
    fn daemon_pid(&self) -> u64 {
        let a = request(&self.data, None, &json!({"op": "status"})).unwrap();
        a["pid"].as_u64().unwrap()
    }
}

/// `clio-memory mcp` on `f`'s folder, with pipes; `extra_env` is added to its environment.
struct Mcp {
    child: Child,
    stdin: ChildStdin,
    out: BufReader<ChildStdout>,
}

fn mcp(f: &Folder, extra_env: &[(&str, &str)]) -> Mcp {
    let mut cmd = Command::new(EXE);
    cmd.arg("--db")
        .arg(f.data.join("memory.db"))
        .arg("mcp")
        .env("HOME", &f.home)
        .env("CLIO_MEMORY_DAEMON_IDLE", "30")
        .envs(extra_env.iter().copied())
        .current_dir(&f.home);
    mcp_of(cmd)
}

/// `cmd` (an `mcp`) started with pipes and initialized.
fn mcp_of(mut cmd: Command) -> Mcp {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let out = BufReader::new(child.stdout.take().unwrap());
    let mut m = Mcp { child, stdin, out };
    let init = m.ask(json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "clientInfo": {"name": "prueba", "version": "1"}}}));
    assert_eq!(
        init["result"]["serverInfo"]["name"], "clio-memory",
        "{init}"
    );
    m
}

impl Mcp {
    fn ask(&mut self, msg: Value) -> Value {
        writeln!(self.stdin, "{msg}").unwrap();
        let mut line = String::new();
        self.out.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    /// The text of a tool call's answer, and whether it is an error.
    fn call(&mut self, tool: &str, args: Value) -> (String, bool) {
        let a = self.ask(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": tool, "arguments": args}}));
        let text = a["result"]["content"][0]["text"].as_str();
        (
            text.unwrap_or_else(|| panic!("{a}")).to_owned(),
            a["result"]["isError"] == true,
        )
    }

    /// Ends its input and waits for it: what it wrote to stderr.
    fn end(self) -> String {
        let Mcp {
            mut child, stdin, ..
        } = self;
        drop(stdin);
        let mut err = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut err)
            .unwrap();
        assert!(child.wait().unwrap().success(), "{err}");
        err
    }
}

#[cfg(target_os = "linux")]
#[test]
fn the_mcp_process_never_opens_the_database() {
    let f = folder();
    let mut m = mcp(&f, &[]);
    let (saved, _) = m.call(
        "memory_save",
        json!({"text": "Qué: el servidor delgado no abre la base", "kind": "semantic"}),
    );
    assert!(saved.starts_with("saved"), "{saved}");
    let open: Vec<String> = std::fs::read_dir(format!("/proc/{}/fd", m.child.id()))
        .unwrap()
        .flatten()
        .filter_map(|e| std::fs::read_link(e.path()).ok())
        .map(|p| p.display().to_string())
        .collect();
    // Mapped, too: the database's `-shm`, or ONNX Runtime loaded for a model of its own.
    let maps = std::fs::read_to_string(format!("/proc/{}/maps", m.child.id())).unwrap();
    m.end();
    assert!(!open.iter().any(|p| p.contains("memory.db")), "{open:?}");
    assert!(
        !maps.contains("memory.db") && !maps.contains("onnxruntime"),
        "{maps}"
    );
    assert!(f.data.join("daemon.sock").exists());
}

#[test]
fn five_sessions_saving_at_once_are_all_saved() {
    let f = folder();
    let answers: Vec<String> = std::thread::scope(|s| {
        let runs: Vec<_> = (0..5)
            .map(|p| {
                let f = &f;
                s.spawn(move || {
                    let mut m = mcp(f, &[]);
                    let said: Vec<String> = (0..10)
                        .map(|i| {
                            let text = format!(
                                "Qué: hecho {p}-{i} de la sesión {p}, guardado número {i} de {}",
                                ["rojo", "verde", "azul", "ocre", "gris"][p]
                            );
                            m.call("memory_save", json!({"text": text, "kind": "semantic"}))
                                .0
                        })
                        .collect();
                    m.end();
                    said
                })
            })
            .collect();
        runs.into_iter().flat_map(|r| r.join().unwrap()).collect()
    });
    assert_eq!(answers.len(), 50);
    for a in &answers {
        assert!(a.starts_with("saved"), "{a}");
        assert!(!a.contains("otro proceso"), "{a}");
    }
}

#[test]
fn the_budget_lives_in_the_daemon() {
    let f = folder();
    let mut m = mcp(&f, &[]);
    for i in 0..20 {
        let (text, is_error) = m.call("memory_context", json!({}));
        assert!(!is_error, "{i}: {text}");
    }
    let (text, is_error) = m.call("memory_context", json!({}));
    assert!(is_error && text.contains("refills in"), "{text}");
    m.end();
    assert!(f.data.join("daemon.sock").exists());
}

#[test]
fn without_a_daemon_it_serves_in_its_own_process() {
    let f = folder();
    let mut m = mcp(&f, &[("CLIO_MEMORY_NO_DAEMON", "1")]);
    let (saved, _) = m.call(
        "memory_save",
        json!({"text": "Qué: sin demonio también se guarda", "kind": "semantic"}),
    );
    m.end();
    assert!(saved.starts_with("saved"), "{saved}");
    assert!(!f.data.join("daemon.sock").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn when_the_daemon_dies_the_call_is_still_answered() {
    let f = folder();
    let mut m = mcp(&f, &[]);
    let (saved, _) = m.call(
        "memory_save",
        json!({"text": "Qué: la jirafa morada sobrevive al demonio", "kind": "semantic"}),
    );
    assert!(saved.starts_with("saved"), "{saved}");
    let pid = f.daemon_pid();
    let killed = Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    let (found, is_error) = m.call("memory_search", json!({"query": "jirafa morada"}));
    m.end();
    assert!(!is_error && found.contains("jirafa morada"), "{found}");
}

#[test]
fn a_socket_path_too_long_falls_back() {
    let f = folder();
    let sock = format!("/tmp/{}", "s".repeat(115));
    assert_eq!(sock.len(), 120);
    let mut m = mcp(&f, &[("CLIO_MEMORY_SOCK", &sock)]);
    let (saved, _) = m.call(
        "memory_save",
        json!({"text": "Qué: una ruta larga no deja sin memoria", "kind": "semantic"}),
    );
    let err = m.end();
    assert!(saved.starts_with("saved"), "{saved}");
    assert!(err.contains("sin demonio"), "{err}");
}

#[test]
fn a_daemon_that_refuses_the_request_is_not_an_answer() {
    use std::os::unix::net::UnixListener;
    let f = folder();
    std::fs::create_dir(&f.data).unwrap();
    // Something on the socket that refuses every request, as a daemon of another protocol would.
    let listener = UnixListener::bind(f.data.join("daemon.sock")).unwrap();
    let refuser = std::thread::spawn(move || {
        let (conn, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(&conn).read_line(&mut line).unwrap();
        writeln!(
            &conn,
            r#"{{"ok":false,"error":"protocol","text":"otro protocolo"}}"#
        )
        .unwrap();
    });
    let mut m = mcp(&f, &[]);
    let (saved, is_error) = m.call(
        "memory_save",
        json!({"text": "Qué: un demonio que rechaza no deja sin memoria", "kind": "semantic"}),
    );
    let err = m.end();
    refuser.join().unwrap();
    assert!(!is_error && saved.starts_with("saved"), "{saved}");
    assert!(err.contains("sin demonio (otro protocolo)"), "{err}");
}

#[test]
fn a_daemon_that_dies_mid_call_is_replaced_and_the_call_answered() {
    use std::os::unix::net::UnixListener;
    let f = folder();
    std::fs::create_dir(&f.data).unwrap();
    let listener = UnixListener::bind(f.data.join("daemon.sock")).unwrap();
    // It reads the call and dies without answering: its socket file stays.
    let dying = std::thread::spawn(move || {
        let (conn, _) = listener.accept().unwrap();
        BufReader::new(&conn).read_line(&mut String::new()).unwrap();
        drop(listener);
    });
    let mut m = mcp(&f, &[]);
    let (saved, is_error) = m.call(
        "memory_save",
        json!({"text": "Qué: un demonio que muere a mitad de la llamada", "kind": "semantic"}),
    );
    let err = m.end();
    dying.join().unwrap();
    assert!(!is_error && saved.starts_with("saved"), "{saved}");
    assert!(!err.contains("sin demonio"), "{err}");
    f.daemon_pid();
}

#[test]
fn two_folders_on_one_socket_never_write_into_each_other() {
    let (a, b) = (folder(), folder());
    let sock = a.home.parent().unwrap().join("shared.sock");
    let sock = sock.to_str().unwrap();
    let mut ma = mcp(&a, &[("CLIO_MEMORY_SOCK", sock)]);
    let (saved, _) = ma.call(
        "memory_save",
        json!({"text": "Qué: la carpeta A tiene su demonio", "kind": "semantic"}),
    );
    assert!(saved.starts_with("saved"), "{saved}");
    let mut mb = mcp(&b, &[("CLIO_MEMORY_SOCK", sock)]);
    let (saved, _) = mb.call(
        "memory_save",
        json!({"text": "Qué: el pingüino violeta vive en la carpeta B", "kind": "semantic"}),
    );
    let err_b = mb.end();
    assert!(saved.starts_with("saved"), "{saved}");
    assert!(
        err_b.contains("sin demonio") && err_b.contains("CLIO_MEMORY_SOCK"),
        "{err_b}"
    );
    let (found, _) = ma.call("memory_search", json!({"query": "pingüino violeta"}));
    ma.end();
    let stop = Command::new(EXE)
        .arg("--db")
        .arg(a.data.join("memory.db"))
        .args(["daemon", "--stop"])
        .env("HOME", &a.home)
        .env("CLIO_MEMORY_SOCK", sock)
        .output()
        .unwrap();
    assert!(stop.status.success(), "{stop:?}");
    assert!(
        !found.contains("pingüino"),
        "B's memory went into A's: {found}"
    );
}

#[test]
fn a_call_too_long_for_the_daemon_is_the_tools_error_and_it_stays_on_the_daemon() {
    let f = folder();
    let mut m = mcp(&f, &[]);
    let (saved, _) = m.call(
        "memory_save",
        json!({"text": "Qué: antes de la llamada larga", "kind": "semantic"}),
    );
    assert!(saved.starts_with("saved"), "{saved}");
    // It fits in an MCP line, not in a request to the daemon (which adds its own fields).
    let long = "x".repeat(clio_memory::daemon::proto::MAX_LINE - 130);
    let (text, is_error) = m.call("memory_save", json!({"text": long, "kind": "semantic"}));
    assert!(is_error && text.contains("demasiado larga"), "{text}");
    let (saved, _) = m.call(
        "memory_save",
        json!({"text": "Qué: después de la llamada larga", "kind": "semantic"}),
    );
    let err = m.end();
    assert!(saved.starts_with("saved"), "{saved}");
    assert!(!err.contains("sin demonio"), "{err}");
}

#[test]
fn a_relative_data_folder_reaches_its_daemon() {
    let f = folder();
    let mut cmd = Command::new(EXE);
    cmd.args(["--db", "data/memory.db", "mcp"])
        .env("HOME", &f.home)
        .env("CLIO_MEMORY_DAEMON_IDLE", "30")
        .current_dir(f.home.parent().unwrap());
    let mut m = mcp_of(cmd);
    let (saved, _) = m.call(
        "memory_save",
        json!({"text": "Qué: una carpeta relativa también tiene demonio", "kind": "semantic"}),
    );
    let err = m.end();
    assert!(saved.starts_with("saved"), "{saved}");
    assert!(!err.contains("sin demonio"), "{err}");
    f.daemon_pid();
}

#[test]
fn a_daemon_that_hangs_is_not_waited_for() {
    use std::os::unix::net::UnixListener;
    use std::time::{Duration, Instant};
    let f = folder();
    std::fs::create_dir(&f.data).unwrap();
    let listener = UnixListener::bind(f.data.join("daemon.sock")).unwrap();
    let (bye, wait) = std::sync::mpsc::channel::<()>();
    // It takes the call and never answers, nor dies.
    let hung = std::thread::spawn(move || {
        let (conn, _) = listener.accept().unwrap();
        BufReader::new(&conn).read_line(&mut String::new()).unwrap();
        let _ = wait.recv();
    });
    let mut m = mcp(&f, &[("CLIO_MEMORY_DAEMON_TIMEOUT", "1")]);
    let started = Instant::now();
    let (saved, is_error) = m.call(
        "memory_save",
        json!({"text": "Qué: un demonio colgado no deja esperando", "kind": "semantic"}),
    );
    let took = started.elapsed();
    let err = m.end();
    bye.send(()).unwrap();
    hung.join().unwrap();
    assert!(!is_error && saved.starts_with("saved"), "{saved}");
    assert!(took < Duration::from_secs(5), "{took:?}");
    assert!(err.contains("sin demonio"), "{err}");
}

#[cfg(target_os = "linux")]
#[test]
fn a_folder_whose_name_is_not_utf8_is_served_here() {
    use std::os::unix::ffi::OsStrExt;
    let f = folder();
    let odd = f.home.join(std::ffi::OsStr::from_bytes(b"caf\xe9"));
    std::fs::create_dir(&odd).unwrap();
    let mut cmd = Command::new(EXE);
    cmd.arg("--db")
        .arg(f.data.join("memory.db"))
        .arg("mcp")
        .env("HOME", &f.home)
        .env("CLIO_MEMORY_DAEMON_IDLE", "30")
        .current_dir(&odd);
    let mut m = mcp_of(cmd);
    let (saved, is_error) = m.call(
        "memory_save",
        json!({"text": "Qué: una carpeta con un nombre raro", "kind": "semantic"}),
    );
    let err = m.end();
    assert!(!is_error && saved.starts_with("saved"), "{saved}");
    assert!(!f.data.join("daemon.sock").exists(), "{err}");
    assert!(!err.contains("sin demonio"), "{err}");
}

#[test]
fn a_note_on_the_project_reaches_the_clients_log() {
    let f = folder();
    let dir = f.home.join("nuevo");
    std::fs::create_dir(&dir).unwrap();
    std::fs::create_dir(&f.data).unwrap();
    let key = dir.canonicalize().unwrap().display().to_string();
    std::fs::write(
        f.data.join("projects.json"),
        json!({ key: "viejo" }).to_string(),
    )
    .unwrap();
    let mut cmd = Command::new(EXE);
    cmd.arg("--db")
        .arg(f.data.join("memory.db"))
        .arg("mcp")
        .env("HOME", &f.home)
        .env("CLIO_MEMORY_DAEMON_IDLE", "30")
        .current_dir(&dir);
    let mut m = mcp_of(cmd);
    let (project, _) = m.call("memory_current_project", json!({}));
    let err = m.end();
    assert_eq!(project, "nuevo");
    assert!(
        err.contains("clio-memory mcp: esta carpeta era del proyecto viejo"),
        "{err}"
    );
    f.daemon_pid();
}
