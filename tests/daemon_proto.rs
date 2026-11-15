#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! The daemon's protocol and request handler, driven in process one line at a time.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use clio_memory::agent::memory::EmbedModel;
use clio_memory::agent::{Draft, Memory, Place};
use clio_memory::daemon::proto::MAX_LINE;
use clio_memory::daemon::{After, Daemon};
use clio_memory::{Engine, FixedClock, MemoryType, OpenMode, SystemClock};
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

/// Like `HashEmbedder`, but a batch of nothing but the backlog («viejo») waits until `open`
/// (10 s at most): `waiting` says one is waiting.
struct Backlog {
    inner: HashEmbedder,
    open: Arc<AtomicBool>,
    waiting: Arc<AtomicBool>,
}

impl clio_memory::Embedder for Backlog {
    fn model_id(&self) -> &str {
        self.inner.model_id()
    }
    fn dim(&self) -> usize {
        self.inner.dim()
    }
    fn embed(&self, texts: &[&str]) -> clio_memory::Result<Vec<Vec<f32>>> {
        if texts.iter().all(|t| t.contains("viejo")) {
            self.waiting.store(true, Ordering::SeqCst);
            let start = Instant::now();
            while !self.open.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(10) {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        self.inner.embed(texts)
    }
}

/// Polls `f` for up to 10 s.
fn eventually(f: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while !f() {
        if start.elapsed() > Duration::from_secs(10) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    true
}

fn vectors(data: &Path) -> u64 {
    let m = Memory::open_read(data, &EmbedModel::none(), None, "agente")
        .unwrap()
        .unwrap();
    m.engine().stats().unwrap().vectors
}

#[test]
fn a_backlog_is_embedded_in_the_background_and_holds_up_no_session() {
    let t = tempfile::tempdir().unwrap();
    // 40 memories written by a program with no model: none has a vector.
    let w = Memory::open_write(t.path(), &EmbedModel::none(), None, "agente").unwrap();
    let words = [
        "alfa", "bravo", "charlie", "delta", "eco", "foxtrot", "golf", "hotel",
    ];
    for i in 0..40 {
        let text = format!(
            "Qué: viejo {} {} {i}",
            words[i % 8],
            words[(i / 8) % 8].repeat(i % 3 + 1)
        );
        w.save(
            &Draft::plain(&text, MemoryType::Semantic),
            "agente",
            Place::Personal,
            false,
        )
        .unwrap();
    }
    drop(w);
    let (open, waiting) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let clock = Arc::new(FixedClock::new(1_800_000_000));
    let home = t.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let d = Daemon::new(
        t.path().to_path_buf(),
        home,
        clock.clone(),
        EmbedModel::from_embedder(Arc::new(Backlog {
            inner: HashEmbedder::new("hash", 16),
            open: open.clone(),
            waiting: waiting.clone(),
        })),
    );
    std::thread::scope(|s| {
        let (said, saved) = std::sync::mpsc::channel();
        let (d, t) = (&d, &t);
        s.spawn(move || {
            let save = call(
                "a",
                t.path(),
                "memory_save",
                json!({"text": "Qué: el dato nuevo de la sesión a", "kind": "semantic"}),
            );
            said.send(ask(d, t.path(), &save).0).unwrap();
        });
        let saved = saved.recv_timeout(Duration::from_secs(5));
        let in_background = eventually(|| waiting.load(Ordering::SeqCst));
        let pending = vectors(t.path()) < 41;
        clock.set(1_800_000_000 + 500);
        let idle = d.idle_secs();
        let (found, _) = ask(
            d,
            t.path(),
            &call(
                "b",
                t.path(),
                "memory_search",
                json!({"query": "dato nuevo"}),
            ),
        );
        // The background ends after this: its end is what the idle time counts from.
        clock.set(1_800_000_000 + 550);
        open.store(true, Ordering::SeqCst);
        let saved = saved.expect("the save waited for the backlog");
        assert!(
            saved["text"].as_str().unwrap().starts_with("saved"),
            "{saved}"
        );
        assert!(
            in_background && pending,
            "the backlog was not left to the background"
        );
        assert_eq!(idle, 0, "the background work counts as busy");
        assert_eq!(found["ok"], true, "{found}");
        assert!(
            found["text"].as_str().unwrap().contains("dato nuevo"),
            "{found}"
        );
    });
    assert!(
        eventually(|| vectors(t.path()) == 41 && ready_embed_jobs(t.path()) == 0),
        "the backlog was not embedded"
    );
    // Its last step after closing the jobs is marking its end, with the clock's time then: 550,
    // or later if it is slow to get there. The clock moves on until it is seen ended; the idle
    // time then counts from 550 at most, never from the last request (500).
    let (mut now, mut idle) = (1_800_000_550, 0);
    while idle == 0 && now < 1_800_000_550 + 1000 {
        now += 1;
        clock.set(now);
        std::thread::sleep(Duration::from_millis(10));
        idle = d.idle_secs();
    }
    assert!(
        idle > 0 && idle <= now - 1_800_000_550,
        "the idle time counts from the background's end: {idle} s at {now}"
    );
}

#[test]
fn a_save_with_nothing_else_waiting_closes_its_own_job() {
    let t = tempfile::tempdir().unwrap();
    let d = daemon(t.path());
    let save = call(
        "a",
        t.path(),
        "memory_save",
        json!({"text": "Qué: el único dato", "kind": "semantic"}),
    );
    let (saved, _) = ask(&d, t.path(), &save);
    assert!(
        saved["text"].as_str().unwrap().starts_with("saved"),
        "{saved}"
    );
    // Its own job is closed by the save itself, under the writer that stored its vector.
    assert_eq!(vectors(t.path()), 1);
    assert_eq!(ready_embed_jobs(t.path()), 0, "the save left its job open");
}

fn ready_embed_jobs(data: &Path) -> usize {
    let m = Memory::open_read(data, &EmbedModel::none(), None, "agente")
        .unwrap()
        .unwrap();
    m.engine().ready_jobs(clio_memory::JobKind::Embed).unwrap()
}

#[test]
fn a_session_made_with_a_note_on_its_project_says_it_in_its_first_answer() {
    let t = tempfile::tempdir().unwrap();
    let d = daemon(t.path());
    let dir = t.path().join("nuevo");
    std::fs::create_dir(&dir).unwrap();
    let key = dir.canonicalize().unwrap().display().to_string();
    std::fs::write(
        t.path().join("projects.json"),
        json!({ key: "viejo" }).to_string(),
    )
    .unwrap();
    let req = call("s", &dir, "memory_current_project", json!({}));
    let (a, _) = ask(&d, t.path(), &req);
    assert_eq!(a["text"], "nuevo", "{a}");
    let note = a["note"].as_str().unwrap_or_default();
    assert!(note.contains("era del proyecto viejo"), "{a}");
    let (a, _) = ask(&d, t.path(), &req);
    assert!(
        a.get("note").is_none(),
        "only when the session is made: {a}"
    );
}

#[test]
fn a_call_without_its_client_or_session_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let d = daemon(t.path());
    for (field, value) in [
        ("client", None),
        ("session", None),
        ("client", Some("")),
        ("session", Some("")),
    ] {
        let mut req = call("s", t.path(), "memory_current_project", json!({}));
        match value {
            Some(v) => req[field] = json!(v),
            None => drop(req.as_object_mut().unwrap().remove(field)),
        }
        let (a, after) = ask(&d, t.path(), &req);
        assert_eq!(a["error"], "bad-request", "{field}={value:?}: {a}");
        assert!(matches!(after, After::Continue));
    }
    let (s, _) = ask(&d, t.path(), &op("status"));
    assert_eq!(s["sessions"], 0, "{s}");
}

struct Panics;

impl clio_memory::Embedder for Panics {
    fn model_id(&self) -> &str {
        "roto"
    }
    fn dim(&self) -> usize {
        1
    }
    fn embed(&self, _: &[&str]) -> clio_memory::Result<Vec<Vec<f32>>> {
        panic!("el modelo se rompió")
    }
}

#[test]
fn a_request_that_panics_gets_an_error_and_the_daemon_goes_on() {
    let t = tempfile::tempdir().unwrap();
    let d = Daemon::new(
        t.path().to_path_buf(),
        t.path().join("home"),
        Arc::new(FixedClock::new(1_800_000_000)),
        EmbedModel::from_embedder(Arc::new(Panics)),
    );
    let mut req = op("embed");
    req["texts"] = json!(["a"]);
    let (a, after) = ask(&d, t.path(), &req);
    assert_eq!(a["ok"], false, "{a}");
    assert_eq!(a["error"], "internal", "{a}");
    assert!(matches!(after, After::Continue));
    let (s, _) = ask(&d, t.path(), &op("status"));
    assert_eq!(s["ok"], true, "{s}");
}
