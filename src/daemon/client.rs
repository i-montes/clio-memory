//! Talking to the daemon: one request per connection, and starting it when nobody answers.

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{VERSION, proto};

/// Room for the path of a Unix socket, with margin under the system's (104 bytes on macOS).
const MAX_SOCKET_PATH: usize = 100;

/// How an error of [`request`] starts when no daemon took the request (and, with a launch, none
/// started): the daemon is not running, as opposed to something else going wrong.
pub const NO_DAEMON: &str = "no hay demonio";

/// How to start the daemon: this binary, and what to add to its environment.
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

/// Sends `body` (with the protocol and this binary's version added) to the daemon of `data` and
/// returns its answer. With `launch`, a daemon that is not there is started, and one older than
/// this binary (it answers `stale` and leaves) is replaced.
pub fn request(data: &Path, launch: Option<&Launch>, body: &Value) -> Result<Value, String> {
    let sock = socket_path(data, std::env::var_os("CLIO_MEMORY_SOCK").as_deref())?;
    let mut body = body.clone();
    body["v"] = proto::PROTOCOL.into();
    body["bin"] = VERSION.into();
    let line = body.to_string();
    // The daemon would refuse it and close: never worth launching one for.
    if line.len() > proto::MAX_LINE {
        return Err(format!(
            "la petición es demasiado larga ({} bytes; el máximo es {})",
            line.len(),
            proto::MAX_LINE
        ));
    }
    let Some(launch) = launch else {
        return ask(&sock, &line).map_err(|e| no_daemon(&sock, &e));
    };
    let answer = match ask(&sock, &line) {
        Err(e) if nobody(&e) => start_and_ask(launch, data, &sock, &line)?,
        other => other.map_err(|e| no_daemon(&sock, &e))?,
    };
    if answer["error"] != "stale" {
        return Ok(answer);
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while sock.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    start_and_ask(launch, data, &sock, &line)
}

/// Starts the daemon and asks until it answers, for up to 3 seconds. It is started again every
/// 250 ms: a daemon that is leaving may still hold the lock, and one launched then steps aside.
fn start_and_ask(launch: &Launch, data: &Path, sock: &Path, line: &str) -> Result<Value, String> {
    crate::agent::memory::private_dir(data)?;
    let start = Instant::now();
    let mut next = start;
    loop {
        if Instant::now() >= next {
            spawn(launch, data)?;
            next += Duration::from_millis(250);
        }
        match ask(sock, line) {
            Err(e) if nobody(&e) && start.elapsed() < Duration::from_secs(3) => {
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
    let mut stream = UnixStream::connect(sock)?;
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
        .status()
        .map_err(|e| format!("no se pudo lanzar el demonio: {e}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "no se pudo lanzar el demonio".to_owned())
}
