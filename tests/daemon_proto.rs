#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! The daemon's protocol and request handler, driven in process one line at a time.

mod common;

use std::path::Path;
use std::sync::Arc;

use clio_memory::agent::memory::EmbedModel;
use clio_memory::daemon::proto::MAX_LINE;
use clio_memory::daemon::{After, Daemon};
use clio_memory::{Engine, FixedClock, OpenMode, SystemClock};
use common::HashEmbedder;
use serde_json::{Value, json};

const BIN: &str = env!("CARGO_PKG_VERSION");

/// A daemon over `data`, with the person's home in its own folder (above no project).
fn daemon(data: &Path) -> Daemon {
    let home = data.join("home");
    std::fs::create_dir_all(&home).unwrap();
    Daemon::new(
        data.to_path_buf(),
        home,
        Arc::new(FixedClock::new(1_800_000_000)),
        EmbedModel::from_embedder(Arc::new(HashEmbedder::new("hash", 16))),
    )
}

/// `req` sent by a client of the data folder `data`, as `request` sends it.
fn ask(d: &Daemon, data: &Path, req: &Value) -> (Value, After) {
    let mut req = req.clone();
    req["data"] = json!(data);
    let (line, after) = d.handle_line(&req.to_string());
    assert!(!line.contains('\n'), "{line}");
    (serde_json::from_str(&line).unwrap(), after)
}

fn call(session: &str, cwd: &Path, tool: &str, args: Value) -> Value {
    json!({"v": 1, "bin": BIN, "op": "call", "client": "claude-code", "session": session,
        "cwd": cwd, "tool": tool, "args": args})
}

fn op(name: &str) -> Value {
    json!({"v": 1, "bin": BIN, "op": name})
}

fn project_dir(name: &str) -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    std::fs::write(t.path().join(".clio-project"), name).unwrap();
    t
}

#[test]
fn a_call_answers_with_the_tools_text() {
    let t = tempfile::tempdir().unwrap();
    let d = daemon(t.path());
    let (a, _) = ask(
        &d,
        t.path(),
        &call(
            "s",
            t.path(),
            "memory_save",
            json!({"text": "el puerto del demonio es un socket unix", "kind": "semantic"}),
        ),
    );
    assert_eq!(a["ok"], true, "{a}");
    let (a, after) = ask(
        &d,
        t.path(),
        &call("s", t.path(), "memory_search", json!({"query": "socket"})),
    );
    assert_eq!(a["ok"], true, "{a}");
    let text = a["text"].as_str().unwrap();
    assert!(text.contains("UNTRUSTED-DATA"), "{text}");
    assert!(
        text.contains("el puerto del demonio es un socket unix"),
        "{text}"
    );
    assert!(matches!(after, After::Continue));
}

#[test]
fn a_tool_refusal_is_ok_false_with_its_words() {
    let t = tempfile::tempdir().unwrap();
    let d = daemon(t.path());
    let (a, _) = ask(
        &d,
        t.path(),
        &call("s", t.path(), "memory_get", json!({"id": 999})),
    );
    assert_eq!(a["ok"], false, "{a}");
    assert_eq!(a["text"], "ese recuerdo no existe o no se puede leer");
    assert!(a.get("error").is_none(), "{a}");
}

#[test]
fn each_session_has_its_own_budget() {
    let t = tempfile::tempdir().unwrap();
    let d = daemon(t.path());
    for _ in 0..20 {
        let (a, _) = ask(
            &d,
            t.path(),
            &call("a", t.path(), "memory_context", json!({})),
        );
        assert_eq!(a["ok"], true, "{a}");
    }
    let (a, _) = ask(
        &d,
        t.path(),
        &call("a", t.path(), "memory_context", json!({})),
    );
    assert_eq!(a["ok"], false, "{a}");
    let (b, _) = ask(
        &d,
        t.path(),
        &call("b", t.path(), "memory_context", json!({})),
    );
    assert_eq!(b["ok"], true, "{b}");
}

#[test]
fn the_project_comes_from_the_folder_sent() {
    let t = tempfile::tempdir().unwrap();
    let d = daemon(t.path());
    let (alfa, beta) = (project_dir("alfa"), project_dir("beta"));
    let (a, _) = ask(
        &d,
        t.path(),
        &call("1", alfa.path(), "memory_current_project", json!({})),
    );
    let (b, _) = ask(
        &d,
        t.path(),
        &call("2", beta.path(), "memory_current_project", json!({})),
    );
    assert_eq!(a["text"], "alfa", "{a}");
    assert_eq!(b["text"], "beta", "{b}");
}

#[test]
fn the_clients_project_variable_names_the_project_wherever_it_is() {
    let t = tempfile::tempdir().unwrap();
    let d = daemon(t.path());
    let alfa = project_dir("alfa");
    let mut req = call("1", alfa.path(), "memory_current_project", json!({}));
    req["project_env"] = json!("gamma");
    let (a, _) = ask(&d, t.path(), &req);
    assert_eq!(a["text"], "gamma", "{a}");
    // In the person's home there is no project, whatever the variable says (as `mcp` does).
    let mut req = call(
        "2",
        &t.path().join("home"),
        "memory_current_project",
        json!({}),
    );
    req["project_env"] = json!("gamma");
    let (a, _) = ask(&d, t.path(), &req);
    assert_eq!(a["text"], "(ninguno: lo personal)", "{a}");
}

#[test]
fn the_actor_is_the_clients() {
    let t = tempfile::tempdir().unwrap();
    let d = daemon(t.path());
    let mut req = call(
        "s",
        t.path(),
        "memory_save",
        json!({"text": "uno", "kind": "semantic"}),
    );
    req["client"] = json!("Claude Code");
    let (a, _) = ask(&d, t.path(), &req);
    assert_eq!(a["ok"], true, "{a}");
    let e = Engine::open(
        &t.path().join("memory.db"),
        OpenMode::ReadOnly,
        Arc::new(SystemClock),
    )
    .unwrap();
    let names: Vec<String> = e.actors().unwrap().into_iter().map(|a| a.name).collect();
    assert!(names.contains(&"mcp:claude-code".to_owned()), "{names:?}");
}

#[test]
fn a_newer_client_makes_it_leave() {
    let t = tempfile::tempdir().unwrap();
    let d = daemon(t.path());
    let (a, after) = ask(
        &d,
        t.path(),
        &json!({"v": 1, "bin": "99.0.0", "op": "model"}),
    );
    assert_eq!(a["ok"], false, "{a}");
    assert_eq!(a["error"], "stale", "{a}");
    assert!(matches!(after, After::Exit));
}

#[test]
fn a_newer_clients_status_is_answered_and_it_stays() {
    let t = tempfile::tempdir().unwrap();
    let d = daemon(t.path());
    let (a, after) = ask(
        &d,
        t.path(),
        &json!({"v": 1, "bin": "99.0.0", "op": "status"}),
    );
    assert_eq!(a["ok"], true, "{a}");
    assert_eq!(a["bin"], BIN, "{a}");
    assert!(matches!(after, After::Continue));
}

#[test]
fn an_older_client_is_served() {
    let t = tempfile::tempdir().unwrap();
    let d = daemon(t.path());
    let (a, after) = ask(
        &d,
        t.path(),
        &json!({"v": 1, "bin": "0.0.1", "op": "status"}),
    );
    assert_eq!(a["ok"], true, "{a}");
    assert_eq!(a["bin"], BIN);
    assert_eq!(a["pid"], std::process::id());
    assert_eq!(
        a["loaded"], true,
        "the test's embedder is there from the start"
    );
    assert_eq!(a["sessions"], 0);
    assert!(matches!(after, After::Continue));
}

#[test]
fn a_newer_protocol_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let d = daemon(t.path());
    let (a, after) = ask(&d, t.path(), &json!({"v": 2, "bin": BIN, "op": "status"}));
    assert_eq!(a["ok"], false, "{a}");
    assert_eq!(a["error"], "protocol", "{a}");
    assert!(matches!(after, After::Continue));
}

#[test]
fn garbage_gets_an_answer_and_it_goes_on() {
    let t = tempfile::tempdir().unwrap();
    let d = daemon(t.path());
    // A good request, one byte too long.
    let status = op("status").to_string();
    let long = format!("{status}{}", " ".repeat(MAX_LINE + 1 - status.len()));
    let mut dance = op("baila");
    dance["data"] = json!(t.path());
    let dance = dance.to_string();
    for line in ["no es json", long.as_str(), dance.as_str()] {
        let (answer, after) = d.handle_line(line);
        let a: Value = serde_json::from_str(&answer).unwrap();
        assert_eq!(a["ok"], false, "{a}");
        assert_eq!(a["error"], "bad-request", "{a}");
        assert!(matches!(after, After::Continue));
    }
}

#[test]
fn embed_returns_one_vector_per_text() {
    let t = tempfile::tempdir().unwrap();
    let d = daemon(t.path());
    let mut req = op("embed");
    req["texts"] = json!(["a", "b"]);
    let (a, _) = ask(&d, t.path(), &req);
    assert_eq!(a["ok"], true, "{a}");
    assert_eq!(a["model"], "hash");
    assert_eq!(a["dim"], 16);
    let vectors = a["vectors"].as_array().unwrap();
    assert_eq!(vectors.len(), 2);
    assert!(vectors.iter().all(|v| v.as_array().unwrap().len() == 16));
    let (m, _) = ask(&d, t.path(), &op("model"));
    assert_eq!(m, json!({"ok": true, "model": "hash", "dim": 16}));
}

#[test]
fn with_no_model_embed_and_model_say_so() {
    let t = tempfile::tempdir().unwrap();
    let d = Daemon::new(
        t.path().to_path_buf(),
        t.path().join("home"),
        Arc::new(FixedClock::new(1_800_000_000)),
        EmbedModel::none(),
    );
    let mut req = op("embed");
    req["texts"] = json!(["a"]);
    for req in [req, op("model")] {
        let (a, after) = ask(&d, t.path(), &req);
        assert_eq!(a["ok"], false, "{a}");
        assert_eq!(a["error"], "no-model", "{a}");
        assert!(matches!(after, After::Continue));
    }
}

#[test]
fn shutdown_says_ok_and_leaves() {
    let t = tempfile::tempdir().unwrap();
    let d = daemon(t.path());
    let (a, after) = ask(&d, t.path(), &op("shutdown"));
    assert_eq!(a, json!({"ok": true}));
    assert!(matches!(after, After::Exit));
}

#[test]
fn idle_counts_from_the_last_request_and_old_sessions_go() {
    let t = tempfile::tempdir().unwrap();
    let clock = Arc::new(FixedClock::new(1_800_000_000));
    let d = Daemon::new(
        t.path().to_path_buf(),
        t.path().join("home"),
        clock.clone(),
        EmbedModel::none(),
    );
    ask(
        &d,
        t.path(),
        &call("a", t.path(), "memory_current_project", json!({})),
    );
    clock.set(1_800_000_000 + 40);
    assert_eq!(d.idle_secs(), 40);
    // A day later, session `a` is gone: only `b` is left.
    clock.set(1_800_000_000 + 86_400);
    ask(
        &d,
        t.path(),
        &call("b", t.path(), "memory_current_project", json!({})),
    );
    let (s, _) = ask(&d, t.path(), &op("status"));
    assert_eq!(s["sessions"], 1, "{s}");
}

#[test]
fn status_tells_the_idle_time_before_it_and_then_counts_as_a_request() {
    let t = tempfile::tempdir().unwrap();
    let clock = Arc::new(FixedClock::new(1_800_000_000));
    let d = Daemon::new(
        t.path().to_path_buf(),
        t.path().join("home"),
        clock.clone(),
        EmbedModel::none(),
    );
    ask(&d, t.path(), &op("model"));
    clock.set(1_800_000_000 + 30);
    let (s, _) = ask(&d, t.path(), &op("status"));
    assert_eq!(s["idle_secs"], 30, "{s}");
    assert_eq!(d.idle_secs(), 0, "the status itself was a request");
    // A clock that goes back never makes it negative.
    clock.set(1_800_000_000 - 100);
    assert_eq!(d.idle_secs(), 0);
}

/// An embedder that, inside `embed`, says it is there and waits to be let go.
struct Gate {
    entered: std::sync::Mutex<std::sync::mpsc::Sender<()>>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl clio_memory::Embedder for Gate {
    fn model_id(&self) -> &str {
        "gate"
    }
    fn dim(&self) -> usize {
        1
    }
    fn embed(&self, texts: &[&str]) -> clio_memory::Result<Vec<Vec<f32>>> {
        self.entered.lock().unwrap().send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        Ok(texts.iter().map(|_| vec![1.0]).collect())
    }
}

#[test]
fn a_request_in_progress_is_never_idle() {
    let t = tempfile::tempdir().unwrap();
    let clock = Arc::new(FixedClock::new(1_800_000_000));
    let (entered_tx, entered) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let gate = Gate {
        entered: std::sync::Mutex::new(entered_tx),
        release: std::sync::Mutex::new(release_rx),
    };
    let d = Daemon::new(
        t.path().to_path_buf(),
        t.path().join("home"),
        clock.clone(),
        EmbedModel::from_embedder(Arc::new(gate)),
    );
    let mut req = op("embed");
    req["texts"] = json!(["a"]);
    std::thread::scope(|s| {
        let asking = s.spawn(|| ask(&d, t.path(), &req));
        entered.recv().unwrap();
        clock.set(1_800_000_000 + 500);
        let idle = d.idle_secs();
        release.send(()).unwrap();
        assert_eq!(idle, 0, "an embed is in progress");
        let (a, _) = asking.join().unwrap();
        assert_eq!(a["ok"], true, "{a}");
    });
    assert_eq!(d.idle_secs(), 0, "it just ended");
}

#[test]
fn a_request_for_another_folder_is_refused_and_it_goes_on() {
    let t = tempfile::tempdir().unwrap();
    let d = daemon(t.path());
    let other = tempfile::tempdir().unwrap();
    let save = call(
        "s",
        t.path(),
        "memory_save",
        json!({"text": "de la otra carpeta", "kind": "semantic"}),
    );
    for req in [save, op("shutdown"), op("status")] {
        let (a, after) = ask(&d, other.path(), &req);
        assert_eq!(a["ok"], false, "{a}");
        assert_eq!(a["error"], "folder", "{a}");
        assert!(
            a["text"].as_str().unwrap().contains("CLIO_MEMORY_SOCK"),
            "{a}"
        );
        assert!(matches!(after, After::Continue));
    }
    assert!(!t.path().join("memory.db").exists(), "nothing was written");
    // Without a folder: refused too.
    let (line, _) = d.handle_line(&op("status").to_string());
    let a: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(a["error"], "bad-request", "{a}");
    // Its own folder, spelled another way, is its own.
    let (a, _) = ask(&d, &t.path().join("home/.."), &op("status"));
    assert_eq!(a["ok"], true, "{a}");
}
