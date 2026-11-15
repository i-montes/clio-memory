#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! The daemon as a process: started by the first client that does not find it, one per data
//! folder, gone when idle.

mod common;

use std::collections::HashSet;
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use clio_memory::Embedder;
use clio_memory::daemon::proto::MAX_LINE;
use clio_memory::daemon::{Launch, RemoteEmbedder, request, socket_path};
use serde_json::{Value, json};
use tempfile::TempDir;

const EXE: &str = env!("CARGO_BIN_EXE_clio-memory");

/// A data folder (not created yet: the client creates it) and how to launch its daemon, with
/// its own home. Its daemon is told to leave when the folder is dropped.
struct Folder {
    _dir: TempDir,
    data: PathBuf,
    home: PathBuf,
    launch: Launch,
}

fn folder(idle: &str) -> Folder {
    let dir = common::short_dir();
    let data = dir.path().join("data");
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let launch = Launch {
        exe: EXE.into(),
        env: vec![
            ("HOME".into(), home.display().to_string()),
            ("CLIO_MEMORY_DAEMON_IDLE".into(), idle.into()),
        ],
    };
    Folder {
        _dir: dir,
        data,
        home,
        launch,
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = request(&self.data, None, &json!({"op": "shutdown"}));
    }
}

impl Folder {
    fn status(&self) -> Value {
        let answer = request(&self.data, Some(&self.launch), &json!({"op": "status"})).unwrap();
        assert_eq!(answer["ok"], true, "{answer}");
        answer
    }

    /// `clio-memory --db <data>/memory.db daemon ARGS`, with this folder's home.
    fn cli(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(EXE);
        cmd.arg("--db")
            .arg(self.data.join("memory.db"))
            .arg("daemon")
            .args(args)
            .env("HOME", &self.home)
            .env("CLIO_MEMORY_DAEMON_IDLE", "30");
        cmd
    }

    fn cli_out(&self, args: &[&str]) -> String {
        let out = self.cli(args).output().unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8(out.stdout).unwrap()
    }
}

fn alive(pid: u64) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

#[test]
fn a_client_starts_the_daemon_and_the_next_one_finds_it() {
    let f = folder("30");
    let first = f.status();
    let second = f.status();
    assert!(first["pid"].as_u64().is_some(), "{first}");
    assert_eq!(first["pid"], second["pid"]);
    assert!(f.data.join("daemon.sock").exists());
}

#[test]
fn two_clients_starting_it_at_once_get_one_daemon() {
    let f = folder("30");
    let pids: HashSet<u64> = std::thread::scope(|s| {
        let asks: Vec<_> = (0..8)
            .map(|_| s.spawn(|| f.status()["pid"].as_u64().unwrap()))
            .collect();
        asks.into_iter().map(|a| a.join().unwrap()).collect()
    });
    assert_eq!(pids.len(), 1, "{pids:?}");
}

#[test]
fn it_leaves_when_idle_and_comes_back_when_asked() {
    let f = folder("1");
    let a = f.status()["pid"].as_u64().unwrap();
    std::thread::sleep(Duration::from_millis(3500));
    assert!(!f.data.join("daemon.sock").exists());
    let b = f.status()["pid"].as_u64().unwrap();
    assert_ne!(a, b);
}

#[test]
fn clients_asking_while_it_leaves_are_all_answered() {
    let f = folder("30");
    f.status();
    let stopped = AtomicBool::new(false);
    std::thread::scope(|s| {
        for _ in 0..6 {
            s.spawn(|| {
                // Until 30 more requests after the shutdown: some meet the daemon leaving.
                let mut after = 0;
                while after < 30 {
                    f.status();
                    after += usize::from(stopped.load(Ordering::SeqCst));
                }
            });
        }
        std::thread::sleep(Duration::from_millis(50));
        let bye = request(&f.data, None, &json!({"op": "shutdown"})).unwrap();
        assert_eq!(bye["ok"], true, "{bye}");
        stopped.store(true, Ordering::SeqCst);
    });
}

#[test]
fn a_second_daemon_for_the_same_folder_steps_aside() {
    let f = folder("30");
    let first = f.status()["pid"].clone();
    let started = Instant::now();
    let mut second = f.cli(&[]).spawn().unwrap();
    let code = loop {
        if let Some(code) = second.try_wait().unwrap() {
            break code;
        }
        if started.elapsed() > Duration::from_secs(2) {
            second.kill().unwrap();
            second.wait().unwrap();
            panic!("the second daemon did not step aside");
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    assert!(code.success(), "{code}");
    assert_eq!(f.status()["pid"], first);
}

#[test]
fn a_dead_socket_file_is_replaced() {
    let f = folder("30");
    std::fs::create_dir(&f.data).unwrap();
    std::fs::write(f.data.join("daemon.sock"), b"").unwrap();
    f.status();
}

#[test]
fn a_socket_path_that_is_too_long_is_said() {
    let long = Path::new("/tmp").join("x".repeat(115));
    let err = socket_path(&long, None).unwrap_err();
    assert!(err.contains("demasiado larga"), "{err}");
    let short = OsStr::new("/tmp/clio.sock");
    assert_eq!(socket_path(&long, Some(short)), Ok(PathBuf::from(short)));
}

#[test]
fn stop_and_status_from_the_command_line() {
    let f = folder("30");
    let pid = f.status()["pid"].as_u64().unwrap();
    let said = f.cli_out(&["--status"]);
    assert!(
        said.contains("demonio: activo") && said.contains(&format!("pid {pid}")),
        "{said}"
    );
    assert!(f.cli_out(&["--stop"]).contains("demonio detenido"));
    let said = f.cli_out(&["--status"]);
    assert!(said.contains("demonio: inactivo"), "{said}");
    assert!(f.cli_out(&["--stop"]).contains("no había demonio"));
}

#[test]
fn it_leaves_when_its_data_folder_is_gone() {
    let f = folder("600");
    let pid = f.status()["pid"].as_u64().unwrap();
    std::fs::remove_dir_all(&f.data).unwrap();
    let started = Instant::now();
    while alive(pid) && started.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !alive(pid),
        "the daemon of a deleted folder is still running"
    );
}

#[test]
fn a_daemon_whose_socket_file_is_deleted_leaves_and_another_takes_over() {
    let f = folder("30");
    let a = f.status()["pid"].as_u64().unwrap();
    std::fs::remove_file(f.data.join("daemon.sock")).unwrap();
    let started = Instant::now();
    while alive(a) && started.elapsed() < Duration::from_secs(4) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!alive(a), "a daemon nobody can reach is still running");
    let b = f.status()["pid"].as_u64().unwrap();
    assert_ne!(a, b);
}

#[test]
fn a_request_too_long_is_refused_without_launching() {
    let f = folder("30");
    let started = Instant::now();
    let big = json!({"op": "embed", "texts": ["x".repeat(MAX_LINE)]});
    let err = request(&f.data, Some(&f.launch), &big).unwrap_err();
    assert!(err.contains("demasiado larga"), "{err}");
    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(!f.data.join("daemon.sock").exists());
}

#[test]
fn status_and_stop_say_a_bad_socket_path_as_an_error() {
    let f = folder("30");
    for flag in ["--status", "--stop"] {
        let out = f
            .cli(&[flag])
            .env("CLIO_MEMORY_SOCK", "relative.sock")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{flag}: {out:?}");
        let err = String::from_utf8(out.stderr).unwrap();
        assert!(err.contains("ruta absoluta"), "{flag}: {err}");
    }
}

#[test]
fn a_daemon_that_answers_stale_is_one_that_is_leaving() {
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;
    let f = folder("30");
    std::fs::create_dir(&f.data).unwrap();
    // An older daemon answers every request of a newer binary with `stale`, then goes.
    let listener = UnixListener::bind(f.data.join("daemon.sock")).unwrap();
    let older = std::thread::spawn(move || {
        for _ in 0..2 {
            let (conn, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&conn).read_line(&mut line).unwrap();
            writeln!(&conn, r#"{{"ok":false,"error":"stale","text":"vieja"}}"#).unwrap();
        }
    });
    assert!(f.cli_out(&["--status"]).contains("demonio: inactivo"));
    assert!(f.cli_out(&["--stop"]).contains("no había demonio"));
    older.join().unwrap();
}

#[test]
fn a_host_without_a_model_in_the_daemon_is_told() {
    let f = folder("30");
    let err = RemoteEmbedder::connect(&f.data, f.launch.clone()).err();
    let err = err.expect("no model, no embedder");
    assert!(err.contains("no tiene modelo"), "{err}");
}

/// A daemon that answers each request with the next of `answers`, then goes.
fn fake_daemon(f: &Folder, answers: Vec<Value>) -> std::thread::JoinHandle<()> {
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;
    std::fs::create_dir(&f.data).unwrap();
    let listener = UnixListener::bind(f.data.join("daemon.sock")).unwrap();
    std::thread::spawn(move || {
        for answer in answers {
            let (conn, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&conn).read_line(&mut line).unwrap();
            writeln!(&conn, "{answer}").unwrap();
        }
    })
}

#[test]
fn a_host_gets_the_daemons_vectors_and_only_if_they_are_right() {
    let model = json!({"ok": true, "model": "m", "dim": 2});
    let vectors = |model: &str, dim: u64, vectors: Value| json!({"ok": true, "model": model, "dim": dim, "vectors": vectors});
    let cases = [
        (vectors("m", 2, json!([[1.0, 0.0], [0.0, 1.0]])), None),
        (
            vectors("m", 2, json!([[1.0, 0.0]])),
            Some("2 textos, 1 vectores"),
        ),
        (
            vectors("m", 2, json!([[1.0, 0.0], [0.0]])),
            Some("de 1 números"),
        ),
        (
            vectors("m", 3, json!([[1.0, 0.0], [0.0, 1.0]])),
            Some("de 3 números"),
        ),
        (
            vectors("otro", 2, json!([[1.0, 0.0], [0.0, 1.0]])),
            Some("«otro»"),
        ),
        (
            json!({"ok": false, "text": "no se pudo"}),
            Some("no se pudo"),
        ),
        (
            json!({"ok": true, "model": "m", "dim": 2}),
            Some("vectores"),
        ),
    ];
    for (answer, wrong) in cases {
        let f = folder("30");
        let daemon = fake_daemon(&f, vec![model.clone(), answer]);
        let remote = RemoteEmbedder::connect(&f.data, f.launch.clone()).unwrap();
        assert_eq!((remote.model_id(), remote.dim()), ("m", 2));
        let got = remote.embed(&["a", "b"]);
        match wrong {
            None => assert_eq!(got.unwrap(), vec![vec![1.0, 0.0], vec![0.0, 1.0]]),
            Some(why) => {
                let err = got.unwrap_err().to_string();
                assert!(err.contains(why), "{why}: {err}");
            }
        }
        daemon.join().unwrap();
    }
}

#[test]
fn a_host_whose_daemon_left_gets_a_new_one() {
    let f = folder("30");
    let daemon = fake_daemon(&f, vec![json!({"ok": true, "model": "m", "dim": 2})]);
    let remote = RemoteEmbedder::connect(&f.data, f.launch.clone()).unwrap();
    daemon.join().unwrap();
    // The one that answers now is a real daemon, started for this request: it has no model.
    let err = remote.embed(&["a"]).unwrap_err().to_string();
    assert!(err.contains("no hay un modelo de vectores"), "{err}");
}
