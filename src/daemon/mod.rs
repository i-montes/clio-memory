//! The daemon: one process per data folder that answers every agent's tool calls with one vector
//! model. This is its core, without sockets: one request line in, one answer line out
//! ([`proto`]). Tool calls are served one at a time, each in the session of its client. [`serve`]
//! puts it behind a Unix socket; [`request`] is the client's side.

mod client;
pub mod proto;
mod server;

pub use client::{Launch, NO_DAEMON, RemoteEmbedder, TOO_LONG, request, socket_path};
pub use server::serve;

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde_json::{Value, json};

use crate::agent::EmbedModel;
use crate::agent::memory::embed_pending;
use crate::clock::Clock;
use crate::mcp::{actor_of, project_in};
use crate::tools::Session;

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// A session nobody used for this long is dropped.
const SESSION_TTL: i64 = 86_400;

/// What the daemon does after answering.
#[derive(Debug, PartialEq, Eq)]
pub enum After {
    Continue,
    Exit,
}

type Sessions = HashMap<(String, String), (Session, i64)>;

pub struct Daemon {
    data: PathBuf,
    home: PathBuf,
    clock: Arc<dyn Clock>,
    model: Arc<EmbedModel>,
    /// By (client as received, session id): the session and when it was last used.
    sessions: Mutex<Sessions>,
    /// When the last request (or the backlog's work) ended.
    last: Arc<AtomicI64>,
    /// Requests in progress.
    busy: AtomicUsize,
    /// Whether the backlog's thread runs.
    embedding: Arc<AtomicBool>,
    /// Hands the backlog to that thread (each session's writes call it).
    backlog: Arc<dyn Fn() + Send + Sync>,
}

/// Marks the end of a request, also when it panics: the daemon must not look busy forever.
struct Done<'a>(&'a Daemon);

impl Drop for Done<'_> {
    fn drop(&mut self) {
        self.0.last.store(self.0.clock.now(), Ordering::SeqCst);
        self.0.busy.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Daemon {
    pub fn new(
        data: PathBuf,
        home: PathBuf,
        clock: Arc<dyn Clock>,
        model: Arc<EmbedModel>,
    ) -> Self {
        let last = Arc::new(AtomicI64::new(clock.now()));
        let embedding = Arc::new(AtomicBool::new(false));
        let backlog = backlog(&data, &model, &clock, &last, &embedding);
        Self {
            data,
            home,
            clock,
            model,
            sessions: Mutex::new(HashMap::new()),
            last,
            busy: AtomicUsize::new(0),
            embedding,
            backlog,
        }
    }

    /// The answer to one request line, as one line without a newline, and what to do next.
    pub fn handle_line(&self, line: &str) -> (String, After) {
        // What `status` reports: the idle time before this request, which then counts as one.
        let idle = self.idle_secs();
        self.busy.fetch_add(1, Ordering::SeqCst);
        let _done = Done(self);
        // A request that panics (its message is in the log) must not take the daemon down.
        let (answer, after) = catch_unwind(AssertUnwindSafe(|| self.answer(line, idle)))
            .unwrap_or_else(|_| {
                let text = "el demonio falló al atender la petición (mira daemon.log)";
                (refused("internal", text), After::Continue)
            });
        (answer.to_string(), after)
    }

    /// Seconds since the last request ended; 0 while one is in progress, or the backlog is being
    /// embedded (or the clock went back).
    pub fn idle_secs(&self) -> i64 {
        if self.busy.load(Ordering::SeqCst) > 0 || self.embedding.load(Ordering::SeqCst) {
            0
        } else {
            (self.clock.now() - self.last.load(Ordering::SeqCst)).max(0)
        }
    }

    fn answer(&self, line: &str, idle: i64) -> (Value, After) {
        if line.len() > proto::MAX_LINE {
            return bad("la petición es demasiado larga");
        }
        let Ok(req) = serde_json::from_str::<Value>(line) else {
            return bad("la petición no es JSON");
        };
        let Some(bin) = req["bin"].as_str() else {
            return bad("falta la versión del programa (bin)");
        };
        // The newest binary wins: this one leaves so the client can start its own. Before the
        // folder check, so a newer client of any folder replaces it (leaving writes into no other
        // folder). `status` only reads: it is answered, and changes nothing.
        if proto::newer(bin, VERSION) && req["op"] != "status" {
            let text =
                format!("este demonio ({VERSION}) es más viejo que el cliente ({bin}): sale");
            return (refused("stale", &text), After::Exit);
        }
        if req["v"].as_u64() != Some(proto::PROTOCOL.into()) {
            let text = format!("este demonio habla el protocolo {}", proto::PROTOCOL);
            return (refused("protocol", &text), After::Continue);
        }
        // Two folders may share one socket by mistake (one CLIO_MEMORY_SOCK for both): a request
        // for another folder must never touch this one.
        let Some(theirs) = req["data"].as_str() else {
            return bad("falta la carpeta de datos (data)");
        };
        if !same_folder(Path::new(theirs), &self.data) {
            let text = format!(
                "este demonio es de la carpeta {}, no de {theirs}: cada carpeta de datos necesita su propio CLIO_MEMORY_SOCK",
                self.data.display()
            );
            return (refused("folder", &text), After::Continue);
        }
        let answer = match req["op"].as_str().unwrap_or_default() {
            "call" => self.call(&req),
            "embed" => self.embed(&req),
            "model" => match self.embedder() {
                Ok(e) => json!({"ok": true, "model": e.model_id(), "dim": e.dim()}),
                Err(no) => no,
            },
            "status" => json!({
                "ok": true,
                "bin": VERSION,
                "pid": std::process::id(),
                "loaded": self.model.is_loaded(),
                "resident": self.model.is_resident(),
                "sessions": self.sessions().len(),
                "idle_secs": idle,
            }),
            "shutdown" => return (json!({"ok": true}), After::Exit),
            _ => return bad("operación desconocida"),
        };
        (answer, After::Continue)
    }

    /// The sessions, without those unused for a day; held for the whole call.
    fn sessions(&self) -> MutexGuard<'_, Sessions> {
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        let now = self.clock.now();
        sessions.retain(|_, (_, used)| now - *used < SESSION_TTL);
        sessions
    }

    fn call(&self, req: &Value) -> Value {
        let cwd = req["cwd"].as_str().map(Path::new);
        let (Some(tool), Some(cwd)) = (req["tool"].as_str(), cwd.filter(|c| c.is_absolute()))
        else {
            return bad("call: faltan tool o cwd (una ruta absoluta)").0;
        };
        let named = |k: &str| req[k].as_str().filter(|v| !v.is_empty());
        // Never one session shared by whoever leaves them out.
        let (Some(client), Some(id)) = (named("client"), named("session")) else {
            return bad("call: faltan client o session").0;
        };
        let now = self.clock.now();
        let mut note = None;
        let mut sessions = self.sessions();
        let key = (client.to_owned(), id.to_owned());
        let (session, used) = sessions.entry(key).or_insert_with(|| {
            // Fixed for the session's life, as for one `mcp` process.
            let env = req["project_env"].as_str();
            let project;
            (project, note) = project_in(&self.data, cwd, &self.home, env);
            let session = Session::new(self.data.clone(), project, actor_of(client))
                .with_model(self.model.clone())
                .with_clock(self.clock.clone())
                .with_backlog(self.backlog.clone());
            (session, now)
        });
        *used = now;
        let mut answer = match session.call(tool, &req["args"]) {
            Ok(text) => json!({"ok": true, "text": text}),
            Err(why) => json!({"ok": false, "text": why}),
        };
        if let Some(n) = note {
            answer["note"] = n.into();
        }
        answer
    }

    fn embed(&self, req: &Value) -> Value {
        let texts: Option<Vec<&str>> = req["texts"]
            .as_array()
            .and_then(|t| t.iter().map(Value::as_str).collect());
        let Some(texts) = texts else {
            return bad("embed: texts debe ser una lista de textos").0;
        };
        let e = match self.embedder() {
            Ok(e) => e,
            Err(no) => return no,
        };
        match e.embed(&texts) {
            Ok(vectors) => {
                json!({"ok": true, "model": e.model_id(), "dim": e.dim(), "vectors": vectors})
            }
            Err(why) => {
                json!({"ok": false, "text": format!("no se pudieron calcular los vectores: {why}")})
            }
        }
    }

    fn embedder(&self) -> Result<Arc<dyn crate::Embedder>, Value> {
        match self.model.get() {
            Ok(Some(e)) => Ok(e),
            Ok(None) => Err(refused(
                "no-model",
                "no hay un modelo de vectores instalado",
            )),
            Err(why) => Err(refused(
                "no-model",
                &format!("el modelo de vectores no se pudo cargar: {why}"),
            )),
        }
    }
}

/// What a session calls after a write: the vectors still waiting are computed in one thread, off
/// the sessions' lock, one such thread at a time; a call while it runs asks it for another round.
/// Its end counts as a request's (`last`).
fn backlog(
    data: &Path,
    model: &Arc<EmbedModel>,
    clock: &Arc<dyn Clock>,
    last: &Arc<AtomicI64>,
    running: &Arc<AtomicBool>,
) -> Arc<dyn Fn() + Send + Sync> {
    let wanted = Arc::new(AtomicBool::new(false));
    let (data, model, clock, last, running) = (
        data.to_path_buf(),
        model.clone(),
        clock.clone(),
        last.clone(),
        running.clone(),
    );
    Arc::new(move || {
        wanted.store(true, Ordering::SeqCst);
        if running.swap(true, Ordering::SeqCst) {
            return;
        }
        let (data, model, clock, last) = (data.clone(), model.clone(), clock.clone(), last.clone());
        let (wanted, still) = (wanted.clone(), running.clone());
        let spawned = std::thread::Builder::new().spawn(move || {
            loop {
                while wanted.swap(false, Ordering::SeqCst) {
                    // A panic is in the log already: the thread must still end as it should.
                    let done = catch_unwind(AssertUnwindSafe(|| embed_pending(&data, &model)));
                    if let Ok(Err(why)) = done {
                        eprintln!("clio-memory daemon: vectores pendientes: {why}");
                    }
                }
                last.store(clock.now(), Ordering::SeqCst);
                still.store(false, Ordering::SeqCst);
                // Asked again after its last round, while it still looked busy.
                if !wanted.load(Ordering::SeqCst) || still.swap(true, Ordering::SeqCst) {
                    return;
                }
            }
        });
        if spawned.is_err() {
            running.store(false, Ordering::SeqCst);
        }
    })
}

fn same_folder(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(a), Ok(b)) if a == b
        )
}

fn refused(error: &str, text: &str) -> Value {
    json!({"ok": false, "error": error, "text": text})
}

fn bad(text: &str) -> (Value, After) {
    (refused("bad-request", text), After::Continue)
}
