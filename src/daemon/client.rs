//! Talking to the daemon: one request per connection, and starting it when nobody answers.

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{VERSION, proto};

/// Room for the path of a Unix socket, with margin under the system's (104 bytes on macOS).
const MAX_SOCKET_PATH: usize = 100;

/// How an error of [`request`] starts when no daemon took the request (and, with a launch, none
/// started): the daemon is not running, as opposed to something else going wrong.
pub const NO_DAEMON: &str = "no hay demonio";
/// How an error of [`request`] starts when the request is too long for any daemon.
pub const TOO_LONG: &str = "la petición es demasiado larga";

/// How to start the daemon: this binary, and what to add to its environment.
#[derive(Clone)]
pub struct Launch {
    pub exe: PathBuf,
    pub env: Vec<(String, String)>,
}

/// Where the daemon of `data` listens: `CLIO_MEMORY_SOCK` (given as `sock_env`) or
/// `<data>/daemon.sock`.
pub fn socket_path(data: &Path, sock_env: Option<&OsStr>) -> Result<PathBuf, String> {
    let path = match sock_env.filter(|s| !s.is_empty()) {
        Some(s) if Path::new(s).is_absolute() => PathBuf::from(s),
        Some(_) => return Err("CLIO_MEMORY_SOCK debe ser una ruta absoluta".into()),
        None => data.join("daemon.sock"),
    };
    let len = path.as_os_str().len();
    if len > MAX_SOCKET_PATH {
        return Err(format!(
            "la ruta del socket es demasiado larga ({len} bytes): pon CLIO_MEMORY_SOCK"
        ));
    }
    Ok(path)
}

/// Sends `body` (with the protocol, this binary's version and the data folder added) to the
/// daemon of `data` and returns its answer. With `launch`, a daemon that is not there is started,
/// and one older than this binary (it answers `stale` and leaves) is replaced.
pub fn request(data: &Path, launch: Option<&Launch>, body: &Value) -> Result<Value, String> {
    // Absolute: the daemon has its own working folder, and checks the folder is its own.
    let data = &std::path::absolute(data).map_err(|e| format!("{}: {e}", data.display()))?;
    let sock = socket_path(data, std::env::var_os("CLIO_MEMORY_SOCK").as_deref())?;
    let mut body = body.clone();
    body["v"] = proto::PROTOCOL.into();
    body["bin"] = VERSION.into();
    body["data"] = data.to_string_lossy().into();
    let line = body.to_string();
    // The daemon would refuse it and close: never worth launching one for.
    if line.len() > proto::MAX_LINE {
        return Err(format!(
            "{TOO_LONG} ({} bytes; el máximo es {})",
            line.len(),
            proto::MAX_LINE
        ));
    }
    let Some(launch) = launch else {
        return ask(&sock, &line).map_err(|e| no_daemon(&sock, &e));
    };
    let answer = match ask(&sock, &line) {
        Err(e) if nobody(&e) => start_and_ask(launch, data, &sock, &line, !not_there(&e))?,
        other => other.map_err(|e| no_daemon(&sock, &e))?,
    };
    if answer["error"] != "stale" {
        return Ok(answer);
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while sock.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    start_and_ask(launch, data, &sock, &line, false)
}

/// Starts the daemon and asks until it answers, for up to 3 seconds. It is started again every
/// 250 ms: a daemon that is leaving may still hold the lock, and one launched then steps aside.
/// A request that reached a daemon and got no answer (`dropped`: it already happened once) is
/// sent again only once: that daemon may have done part of it.
fn start_and_ask(
    launch: &Launch,
    data: &Path,
    sock: &Path,
    line: &str,
    mut dropped: bool,
) -> Result<Value, String> {
    crate::agent::memory::private_dir(data)?;
    let start = Instant::now();
    let mut next = start;
    loop {
        if Instant::now() >= next {
            spawn(launch, data)?;
            next += Duration::from_millis(250);
        }
        match ask(sock, line) {
            Err(e)
                if nobody(&e)
                    && (not_there(&e) || !dropped)
                    && start.elapsed() < Duration::from_secs(3) =>
            {
                dropped |= !not_there(&e);
                std::thread::sleep(Duration::from_millis(25));
            }
            other => return other.map_err(|e| no_daemon(sock, &e)),
        }
    }
}

/// No daemon took the request: no socket file, one nobody accepts on, or a daemon that was
/// leaving and closed the connection without reading it.
fn nobody(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        ErrorKind::NotFound
            | ErrorKind::ConnectionRefused
            | ErrorKind::BrokenPipe
            | ErrorKind::ConnectionReset
            | ErrorKind::UnexpectedEof
    )
}

/// Nothing listens there: the request never left.
fn not_there(e: &std::io::Error) -> bool {
    matches!(e.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused)
}

fn no_daemon(sock: &Path, e: &std::io::Error) -> String {
    if nobody(e) {
        format!("{NO_DAEMON} en {} ({e})", sock.display())
    } else {
        format!(
            "no se pudo hablar con el demonio en {}: {e}",
            sock.display()
        )
    }
}

/// One request line out, one answer line back.
fn ask(sock: &Path, line: &str) -> std::io::Result<Value> {
    let mut stream = UnixStream::connect(sock).map_err(|e| {
        // A leftover file that is not a socket: Linux says ECONNREFUSED, macOS ENOTSOCK.
        use std::os::unix::fs::FileTypeExt;
        match std::fs::symlink_metadata(sock) {
            Ok(m) if !m.file_type().is_socket() => ErrorKind::ConnectionRefused.into(),
            _ => e,
        }
    })?;
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\n")?;
    let mut answer = String::new();
    if BufReader::new(stream).read_line(&mut answer)? == 0 {
        return Err(ErrorKind::UnexpectedEof.into());
    }
    serde_json::from_str(&answer).map_err(std::io::Error::other)
}

/// Starts the daemon detached: `sh` backgrounds it and leaves, so init adopts it (no zombie under
/// a long-lived client), and its own process group keeps an agent that kills its children's
/// group from taking it down.
fn spawn(launch: &Launch, data: &Path) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(r#""$0" --db "$1" daemon </dev/null >/dev/null 2>>"$2" &"#)
        .arg(&launch.exe)
        .arg(data.join(crate::agent::memory::DB_FILE))
        .arg(data.join("daemon.log"))
        .envs(launch.env.iter().map(|(k, v)| (k, v)))
        .process_group(0)
        // It outlives its client: it must not keep that client's folder (a mount) busy.
        .current_dir("/")
        .status()
        .map_err(|e| format!("no se pudo lanzar el demonio: {e}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "no se pudo lanzar el demonio".to_owned())
}

/// Vectors from the daemon of a data folder, for a host program that would rather not load its
/// own copy of the model.
pub struct RemoteEmbedder {
    data: PathBuf,
    launch: Launch,
    model: String,
    dim: usize,
}

impl RemoteEmbedder {
    /// Asks the daemon (starting it if needed) which model it has. An error says why not, in words
    /// for the person: the host keeps its own model then.
    pub fn connect(data: &Path, launch: Launch) -> Result<Self, String> {
        let answer = request(data, Some(&launch), &json!({"op": "model"}))?;
        if answer["error"] == "no-model" {
            let why = answer["text"].as_str().unwrap_or_default();
            return Err(format!("el demonio no tiene modelo de vectores: {why}"));
        }
        let (Some(model), Some(dim)) = (answer["model"].as_str(), answer["dim"].as_u64()) else {
            return Err(format!("respuesta inesperada del demonio: {answer}"));
        };
        Ok(Self {
            data: data.to_owned(),
            launch,
            model: model.to_owned(),
            dim: dim as usize,
        })
    }

    fn ask_embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        let answer = request(
            &self.data,
            Some(&self.launch),
            &json!({"op": "embed", "texts": texts}),
        )?;
        if answer["ok"] != true {
            return Err(answer["text"].as_str().unwrap_or("sin motivo").to_owned());
        }
        // A daemon replaced by another version may have another model: never mix vectors.
        if answer["model"] != self.model || answer["dim"] != self.dim {
            return Err(format!(
                "el demonio ahora usa «{}» de {} números, no «{}» de {}",
                answer["model"].as_str().unwrap_or("?"),
                answer["dim"],
                self.model,
                self.dim
            ));
        }
        let vectors: Vec<Vec<f32>> = answer["vectors"]
            .as_array()
            .map(|all| {
                all.iter()
                    .map(|v| {
                        let v = v.as_array()?;
                        v.iter().map(|x| x.as_f64().map(|x| x as f32)).collect()
                    })
                    .collect::<Option<_>>()
            })
            .unwrap_or_default()
            .ok_or("los vectores no son números")?;
        if vectors.len() != texts.len() {
            return Err(format!(
                "se pidieron {} textos, {} vectores",
                texts.len(),
                vectors.len()
            ));
        }
        match vectors.iter().find(|v| v.len() != self.dim) {
            Some(v) => Err(format!(
                "un vector de {} números, no de {}",
                v.len(),
                self.dim
            )),
            None => Ok(vectors),
        }
    }
}

impl crate::Embedder for RemoteEmbedder {
    fn model_id(&self) -> &str {
        &self.model
    }
    fn dim(&self) -> usize {
        self.dim
    }
    fn embed(&self, texts: &[&str]) -> crate::Result<Vec<Vec<f32>>> {
        self.ask_embed(texts)
            .map_err(|why| crate::Error::Embedding(format!("demonio: {why}")))
    }
}
