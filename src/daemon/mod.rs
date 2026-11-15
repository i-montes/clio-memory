//! The daemon: one process per data folder that answers every agent's tool calls with one vector
//! model. This is its core, without sockets: one request line in, one answer line out
//! ([`proto`]). Tool calls are served one at a time, each in the session of its client. [`serve`]
//! puts it behind a Unix socket; [`request`] is the client's side.

mod client;
pub mod proto;
mod server;

pub use client::{Launch, NO_DAEMON, RemoteEmbedder, request, socket_path};
pub use server::serve;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde_json::{Value, json};

use crate::agent::EmbedModel;
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
    /// When the last request ended.
    last: AtomicI64,
    /// Requests in progress.
    busy: AtomicUsize,
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
        let now = clock.now();
        Self {
            data,
            home,
            clock,
            model,
            sessions: Mutex::new(HashMap::new()),
            last: AtomicI64::new(now),
            busy: AtomicUsize::new(0),
        }
    }

    /// The answer to one request line, as one line without a newline, and what to do next.
    pub fn handle_line(&self, line: &str) -> (String, After) {
        // What `status` reports: the idle time before this request, which then counts as one.
        let idle = self.idle_secs();
        self.busy.fetch_add(1, Ordering::SeqCst);
        let _done = Done(self);
        let (answer, after) = self.answer(line, idle);
        (answer.to_string(), after)
    }

    /// Seconds since the last request ended; 0 while one is in progress (or the clock went back).
    pub fn idle_secs(&self) -> i64 {
        if self.busy.load(Ordering::SeqCst) > 0 {
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
        // The newest binary wins: this one leaves so the client can start its own.
        if proto::newer(bin, VERSION) {
            let text =
                format!("este demonio ({VERSION}) es más viejo que el cliente ({bin}): sale");
            return (refused("stale", &text), After::Exit);
        }
        if req["v"].as_u64() != Some(proto::PROTOCOL.into()) {
            let text = format!("este demonio habla el protocolo {}", proto::PROTOCOL);
            return (refused("protocol", &text), After::Continue);
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
        let client = req["client"].as_str().unwrap_or_default();
        let key = (
            client.to_owned(),
            req["session"].as_str().unwrap_or_default().to_owned(),
        );
        let now = self.clock.now();
        let mut sessions = self.sessions();
        let (session, used) = sessions.entry(key).or_insert_with(|| {
            let env = req["project_env"].as_str();
            let project = project_in(&self.data, cwd, &self.home, env, "daemon");
            let session = Session::new(self.data.clone(), project, actor_of(client))
                .with_model(self.model.clone())
                .with_clock(self.clock.clone());
            (session, now)
        });
        *used = now;
        match session.call(tool, &req["args"]) {
            Ok(text) => json!({"ok": true, "text": text}),
            Err(why) => json!({"ok": false, "text": why}),
        }
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

fn refused(error: &str, text: &str) -> Value {
    json!({"ok": false, "error": error, "text": text})
}

fn bad(text: &str) -> (Value, After) {
    (refused("bad-request", text), After::Continue)
}
