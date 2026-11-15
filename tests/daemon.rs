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
use clio_memory::daemon::{Launch, NO_DAEMON, RemoteEmbedder, TOO_LONG, request, socket_path};
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
    assert_eq!(first["resident"], true, "it keeps its model: {first}");
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
        said.contains("demonio: activo")
            && said.contains(&format!("pid {pid}"))
            && !said.contains("versión anterior"),
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
    let f = folder("30");
    // An older daemon answers every request of a newer binary with `stale`, then goes.
    let stale = json!({"ok": false, "error": "stale", "text": "vieja"});
    let older = fake_daemon(&f, vec![stale.clone(), stale]);
    assert!(f.cli_out(&["--status"]).contains("demonio: inactivo"));
    let started = Instant::now();
    assert!(f.cli_out(&["--stop"]).contains("demonio detenido"));
    // It waited for the socket to go, not for its 2 s.
    assert!(started.elapsed() < Duration::from_millis(1500));
    assert!(!f.data.join("daemon.sock").exists());
    older.join().unwrap();
}

#[test]
fn with_the_daemon_turned_off_status_and_doctor_say_so() {
    let f = folder("30");
    let out = f
        .cli(&["--status"])
        .env("CLIO_MEMORY_NO_DAEMON", "1")
        .output()
        .unwrap();
    let said = String::from_utf8(out.stdout).unwrap();
    assert!(
        said.contains("demonio: desactivado (CLIO_MEMORY_NO_DAEMON=1)"),
        "{said}"
    );
    let out = Command::new(EXE)
        .arg("--db")
        .arg(f.data.join("memory.db"))
        .arg("doctor")
        .env("HOME", &f.home)
        .env("CLIO_MEMORY_NO_DAEMON", "1")
        .output()
        .unwrap();
    let said = String::from_utf8(out.stdout).unwrap();
    assert!(
        said.contains("info demonio: desactivado (CLIO_MEMORY_NO_DAEMON=1)"),
        "{said}"
    );
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
    let n = answers.len();
    let answers = std::sync::Mutex::new(answers.into_iter());
    fake_daemon_with(f, n, move |_| answers.lock().unwrap().next().unwrap())
}

/// A daemon that answers `n` requests with `answer(request)`, then goes (its socket too). Fewer
/// requests in 10 s fail the test instead of leaving it waiting.
fn fake_daemon_with(
    f: &Folder,
    n: usize,
    answer: impl Fn(&Value) -> Value + Send + 'static,
) -> std::thread::JoinHandle<()> {
    use std::io::{BufRead, BufReader, ErrorKind};
    use std::os::unix::net::UnixListener;
    std::fs::create_dir(&f.data).unwrap();
    let sock = f.data.join("daemon.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    listener.set_nonblocking(true).unwrap();
    std::thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(10);
        for i in 0..n {
            let conn = loop {
                match listener.accept() {
                    Ok((conn, _)) => break conn,
                    Err(e) if e.kind() == ErrorKind::WouldBlock && Instant::now() < until => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("request {} of {n} never came: {e}", i + 1),
                }
            };
            // macOS hands the listener's non-blocking mode to the connection.
            conn.set_nonblocking(false).unwrap();
            let mut line = String::new();
            BufReader::new(&conn).read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            writeln!(&conn, "{}", answer(&request)).unwrap();
        }
        std::fs::remove_file(&sock).unwrap();
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
            Some("demonio: la respuesta no trae vectores"),
        ),
        (
            vectors("m", 2, json!([[1.0, "x"], [0.0, 1.0]])),
            Some("demonio: los vectores no son números"),
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
fn a_host_is_told_the_daemons_reason_when_it_refuses() {
    let f = folder("30");
    let why = "el demonio falló al atender la petición (mira daemon.log)";
    let daemon = fake_daemon(
        &f,
        vec![json!({"ok": false, "error": "internal", "text": why})],
    );
    let err = RemoteEmbedder::connect(&f.data, f.launch.clone()).err();
    daemon.join().unwrap();
    assert_eq!(
        err.as_deref(),
        Some(format!("el demonio no dijo su modelo: {why}").as_str())
    );
}

#[test]
fn a_host_batch_too_long_for_one_request_is_sent_in_parts() {
    let f = folder("30");
    let model = json!({"ok": true, "model": "m", "dim": 2});
    // Each vector is its text's length: the order is seen.
    let daemon = fake_daemon_with(&f, 3, move |req| {
        let Some(texts) = req["texts"].as_array() else {
            return model.clone();
        };
        let vectors: Vec<Value> = texts
            .iter()
            .map(|t| json!([t.as_str().unwrap().len(), 0]))
            .collect();
        json!({"ok": true, "model": "m", "dim": 2, "vectors": vectors})
    });
    let remote = RemoteEmbedder::connect(&f.data, f.launch.clone()).unwrap();
    // Two fit in one request, three do not.
    let texts: Vec<String> = (0..3).map(|i| "a".repeat(MAX_LINE * 2 / 5 + i)).collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let got = remote.embed(&refs);
    daemon.join().unwrap();
    let want: Vec<Vec<f32>> = texts.iter().map(|t| vec![t.len() as f32, 0.0]).collect();
    assert_eq!(got.unwrap(), want);
    // One text that does not fit by itself is said, and nothing is sent.
    let one = "a".repeat(MAX_LINE);
    let err = remote.embed(&[&one]).unwrap_err().to_string();
    assert!(err.contains(TOO_LONG), "{err}");
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

#[test]
fn a_daemon_that_dies_mid_request_is_replaced_and_the_request_answered() {
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;
    let f = folder("30");
    std::fs::create_dir(&f.data).unwrap();
    let listener = UnixListener::bind(f.data.join("daemon.sock")).unwrap();
    // It reads the request and dies without answering: its socket file stays.
    let dying = std::thread::spawn(move || {
        let (conn, _) = listener.accept().unwrap();
        BufReader::new(&conn).read_line(&mut String::new()).unwrap();
        drop(listener);
    });
    let a = request(&f.data, Some(&f.launch), &json!({"op": "status"})).unwrap();
    dying.join().unwrap();
    assert_eq!(a["ok"], true, "{a}");
    assert!(alive(a["pid"].as_u64().unwrap()));
}

#[test]
fn a_client_waits_for_a_stale_daemon_to_go_and_starts_its_own() {
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;
    let f = folder("30");
    std::fs::create_dir(&f.data).unwrap();
    let sock = f.data.join("daemon.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    listener.set_nonblocking(true).unwrap();
    // An older daemon that is leaving: it answers `stale` to whatever reaches it until it is gone.
    let older = std::thread::spawn(move || {
        let until = Instant::now() + Duration::from_millis(400);
        while Instant::now() < until {
            match listener.accept() {
                Ok((conn, _)) => {
                    conn.set_nonblocking(false).unwrap();
                    BufReader::new(&conn).read_line(&mut String::new()).unwrap();
                    writeln!(&conn, r#"{{"ok":false,"error":"stale","text":"vieja"}}"#).unwrap();
                }
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        let _ = std::fs::remove_file(&sock);
    });
    let a = request(&f.data, Some(&f.launch), &json!({"op": "status"})).unwrap();
    older.join().unwrap();
    assert_eq!(a["ok"], true, "{a}");
}

#[test]
fn a_request_dropped_after_it_was_sent_is_sent_again_only_once() {
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::{UnixListener, UnixStream};
    for late in [false, true] {
        let f = folder("30");
        std::fs::create_dir(&f.data).unwrap();
        let sock = f.data.join("daemon.sock");
        // There from the start (the first send is dropped), or once the client waits for one.
        let bound = (!late).then(|| UnixListener::bind(&sock).unwrap());
        let at = sock.clone();
        // It takes every request and closes without answering, until told to stop.
        let taker = std::thread::spawn(move || {
            let listener = bound.unwrap_or_else(|| {
                std::thread::sleep(Duration::from_millis(100));
                UnixListener::bind(&at).unwrap()
            });
            let mut taken = 0;
            for conn in listener.incoming() {
                let mut line = String::new();
                BufReader::new(conn.unwrap()).read_line(&mut line).unwrap();
                if line.trim() == "fin" {
                    break;
                }
                taken += 1;
            }
            taken
        });
        // A launch that starts nothing: only the one on the socket ever takes the request.
        let launch = Launch {
            exe: "true".into(),
            env: vec![],
        };
        let err = request(&f.data, Some(&launch), &json!({"op": "status"})).unwrap_err();
        writeln!(UnixStream::connect(&sock).unwrap(), "fin").unwrap();
        assert_eq!(
            taker.join().unwrap(),
            2,
            "late {late}: sent once and again once"
        );
        assert!(err.starts_with(NO_DAEMON), "{err}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn the_daemon_keeps_no_clients_folder_busy() {
    let f = folder("30");
    let pid = f.status()["pid"].as_u64().unwrap();
    let cwd = std::fs::read_link(format!("/proc/{pid}/cwd")).unwrap();
    assert_eq!(cwd, Path::new("/"));
}

#[test]
fn status_and_stop_never_touch_another_folders_daemon() {
    let (a, b) = (folder("30"), folder("30"));
    let sock = a.home.parent().unwrap().join("shared.sock");
    // A's daemon, on a socket B is told to use too (it serves only a folder that is there).
    std::fs::create_dir(&a.data).unwrap();
    let mut daemon_a = a
        .cli(&[])
        .env("CLIO_MEMORY_SOCK", &sock)
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while !sock.exists() && started.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(25));
    }
    for flag in ["--status", "--stop"] {
        let out = b
            .cli(&[flag])
            .env("CLIO_MEMORY_SOCK", &sock)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{flag}: {out:?}");
        let err = String::from_utf8(out.stderr).unwrap();
        assert!(err.contains("su propio CLIO_MEMORY_SOCK"), "{flag}: {err}");
    }
    assert!(
        daemon_a.try_wait().unwrap().is_none(),
        "A's daemon was stopped"
    );
    let out = a
        .cli(&["--stop"])
        .env("CLIO_MEMORY_SOCK", &sock)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("demonio detenido"),
        "{out:?}"
    );
    assert!(daemon_a.wait().unwrap().success());
}

#[test]
fn a_bare_db_file_name_is_the_current_folder() {
    let dir = common::short_dir();
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let out = Command::new(EXE)
        .args(["--db", "memory.db", "daemon", "--status"])
        .current_dir(dir.path())
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let said = String::from_utf8(out.stdout).unwrap();
    assert!(said.contains("inactivo"), "{said}");
    // A daemon in the foreground serves this folder too.
    let mut daemon = Command::new(EXE)
        .args(["--db", "memory.db", "daemon"])
        .current_dir(dir.path())
        .env("HOME", &home)
        .env("CLIO_MEMORY_DAEMON_IDLE", "30")
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while !dir.path().join("daemon.sock").exists() && started.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(25));
    }
    let bye = request(dir.path(), None, &json!({"op": "shutdown"}));
    let _ = daemon.kill();
    daemon.wait().unwrap();
    assert_eq!(bye.unwrap()["ok"], true);
}

#[test]
fn a_request_for_an_empty_folder_is_for_this_one() {
    let err = request(Path::new(""), None, &json!({"op": "status"})).unwrap_err();
    assert!(err.starts_with(NO_DAEMON), "{err}");
}

#[test]
fn a_relative_exe_is_found_from_the_clients_folder() {
    let f = folder("30");
    // Relative to this test's folder (the crate), where cargo puts its target.
    let cwd = std::env::current_dir().unwrap();
    // A target folder outside the crate (CARGO_TARGET_DIR) has no such path: nothing to check.
    let Ok(rel) = Path::new(EXE).strip_prefix(&cwd) else {
        return;
    };
    let launch = Launch {
        exe: rel.into(),
        env: f.launch.env.clone(),
    };
    let a = request(&f.data, Some(&launch), &json!({"op": "status"})).unwrap();
    assert_eq!(a["ok"], true, "{a}");
}

#[test]
fn a_daemon_that_cannot_start_points_at_its_log() {
    let f = folder("30");
    let launch = Launch {
        exe: "/nonexistent/clio-memory".into(),
        env: vec![],
    };
    let err = request(&f.data, Some(&launch), &json!({"op": "status"})).unwrap_err();
    let log = f.data.join("daemon.log");
    assert!(err.contains(&format!("mira {}", log.display())), "{err}");
}

#[test]
fn the_socket_is_private_from_the_start_and_leaves_nothing_behind() {
    use std::os::unix::fs::PermissionsExt;
    let f = folder("30");
    std::fs::create_dir(&f.data).unwrap();
    let place = common::short_dir();
    let sock = place.path().join("s.sock");
    let mut daemon = f
        .cli(&[])
        .env("CLIO_MEMORY_SOCK", &sock)
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while !sock.exists() && started.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(25));
    }
    let mode = std::fs::symlink_metadata(&sock)
        .unwrap()
        .permissions()
        .mode();
    let there: Vec<_> = std::fs::read_dir(place.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let out = f
        .cli(&["--stop"])
        .env("CLIO_MEMORY_SOCK", &sock)
        .output()
        .unwrap();
    assert!(daemon.wait().unwrap().success(), "{out:?}");
    assert_eq!(mode & 0o777, 0o600, "{mode:o}");
    assert_eq!(there, vec![OsStr::new("s.sock")]);
}

#[test]
fn a_daemon_for_a_missing_folder_leaves_without_making_it() {
    let f = folder("30");
    let mut daemon = f
        .cli(&[])
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    let code = loop {
        if let Some(code) = daemon.try_wait().unwrap() {
            break code;
        }
        if started.elapsed() > Duration::from_secs(3) {
            daemon.kill().unwrap();
            daemon.wait().unwrap();
            panic!("the daemon of a missing folder is still running");
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    assert!(code.success(), "{code}");
    assert!(!f.data.exists());
}

#[test]
fn status_says_when_the_daemon_is_older_and_leaves_with_the_next_call() {
    let f = folder("30");
    let older = json!({"ok": true, "bin": "0.0.1", "pid": 1, "loaded": false, "sessions": 0});
    let daemon = fake_daemon(&f, vec![older.clone(), older]);
    let said = f.cli_out(&["--status"]);
    assert!(
        said.contains("v0.0.1")
            && said.contains(" — versión anterior: sale con la próxima llamada"),
        "{said}"
    );
    let out = Command::new(EXE)
        .arg("--db")
        .arg(f.data.join("memory.db"))
        .arg("doctor")
        .env("HOME", &f.home)
        .output()
        .unwrap();
    let said = String::from_utf8(out.stdout).unwrap();
    assert!(
        said.contains(
            "info demonio: activo (pid 1, v0.0.1) — versión anterior: sale con la próxima llamada"
        ),
        "{said}"
    );
    daemon.join().unwrap();
}
