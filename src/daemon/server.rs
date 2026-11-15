//! The daemon's process: the lock that makes it one per data folder, the socket, and leaving.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use super::{After, Daemon, proto, socket_path};
use crate::SystemClock;
use crate::agent::memory::{EmbedModel, embed_pending};

/// The log is cut back to nothing at start when it grows past this.
const MAX_LOG: u64 = 1 << 20;
/// A client that sends nothing, or reads nothing, is let go after this.
const IO_TIMEOUT: Duration = Duration::from_secs(10);

/// Serves the data folder `data` until it is idle for `idle`, is asked to leave (`shutdown`, or
/// a newer client: `stale`), or its folder is gone. Returns at once if another daemon has it.
pub fn serve(data: &Path, home: &Path, idle: Duration) -> Result<(), String> {
    // Its client made it: a daemon that arrives after it was deleted must not bring it back.
    if !data.is_dir() {
        eprintln!(
            "clio-memory daemon: {} no existe: no hay nada que atender",
            data.display()
        );
        return Ok(());
    }
    let lock = crate::platform::try_lock(&data.join("daemon.lock"))
        .map_err(|e| format!("{}: {e}", data.join("daemon.lock").display()))?;
    let Some(lock) = lock else {
        return Ok(());
    };
    tidy_log(&data.join("daemon.log"));
    let sock = socket_path(data, std::env::var_os("CLIO_MEMORY_SOCK").as_deref())?;
    let listener = bind_private(&sock).map_err(|e| format!("{}: {e}", sock.display()))?;
    let model = daemon_model(home);
    let daemon = Daemon::new(
        data.to_path_buf(),
        home.to_path_buf(),
        Arc::new(SystemClock),
        model.clone(),
    );
    let leaving = AtomicBool::new(false);
    // Set when the accept loop is over: the watcher's job is done.
    let done = AtomicBool::new(false);
    let in_flight = AtomicUsize::new(0);
    let idle = i64::try_from(idle.as_secs()).unwrap_or(i64::MAX);
    std::thread::scope(|s| {
        s.spawn(|| {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                if done.load(Ordering::SeqCst) {
                    return;
                }
                // Its folder deleted (a test's, or `uninstall --data`): nothing to serve or save.
                if !data.is_dir() {
                    std::process::exit(0);
                }
                if daemon.idle_secs() >= idle || !sock.exists() {
                    leaving.store(true, Ordering::SeqCst);
                }
                // Wakes the `accept` below so it sees `leaving`. When nothing can reach it (its
                // socket file is gone), the requests in progress finish and the process ends
                // here: the lock goes with it, and the next daemon computes what is pending.
                if leaving.load(Ordering::SeqCst)
                    && UnixStream::connect(&sock).is_err()
                    && !done.load(Ordering::SeqCst)
                {
                    while in_flight.load(Ordering::SeqCst) > 0 {
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    if let Err(why) = embed_pending(data, &model) {
                        eprintln!("al salir, los vectores pendientes quedaron sin calcular: {why}");
                    }
                    std::process::exit(0);
                }
            }
        });
        for conn in listener.incoming() {
            // Served even when leaving: its client already connected, and the next one launches
            // a new daemon.
            match conn {
                Ok(conn) => {
                    in_flight.fetch_add(1, Ordering::SeqCst);
                    s.spawn(|| {
                        let _counted = Counted(&in_flight);
                        if answer(&daemon, conn, &leaving) == After::Exit {
                            drop(UnixStream::connect(&sock));
                        }
                    });
                }
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
            if leaving.load(Ordering::SeqCst) {
                break;
            }
        }
        // Before the listener closes: a wake refused from here on is not a daemon nobody reaches.
        done.store(true, Ordering::SeqCst);
        // Those still waiting in the backlog are cut (their clients launch a new daemon); the
        // requests in progress finish.
        drop(listener);
    });
    // The socket and the lock go first, so a new daemon can start while this one computes the
    // vectors left pending (which may load the model).
    let _ = std::fs::remove_file(&sock);
    drop(lock);
    if let Err(why) = embed_pending(data, &model) {
        eprintln!("al salir, los vectores pendientes quedaron sin calcular: {why}");
    }
    Ok(())
}

/// Listens on `sock`, never open to others: bound in a fresh private folder beside it, made 0600
/// and moved into place (over a leftover file).
/// shortcut: the temporary path is up to 10 bytes longer than `sock` for a short file name; a
/// path near the limit with one may not bind, then name the socket longer.
fn bind_private(sock: &Path) -> std::io::Result<UnixListener> {
    use std::os::unix::fs::DirBuilderExt;
    let dir = sock
        .parent()
        .unwrap_or(Path::new("/"))
        .join(format!(".s{}", std::process::id()));
    // Ours, left by a crash of a process with this pid; another's is not removed, and then
    // `create` fails.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    let bound = (|| {
        let tmp = dir.join("s");
        let listener = UnixListener::bind(&tmp)?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        std::fs::rename(&tmp, sock)?;
        Ok(listener)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    bound
}

/// A request in progress, counted until its thread ends (also when it panics).
struct Counted<'a>(&'a AtomicUsize);

impl Drop for Counted<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Reads one request line from `conn`, answers it and closes. On `After::Exit` it marks
/// `leaving` before answering, so the client's next request already finds the daemon gone.
fn answer(daemon: &Daemon, conn: UnixStream, leaving: &AtomicBool) -> After {
    let _ = conn.set_read_timeout(Some(IO_TIMEOUT));
    let _ = conn.set_write_timeout(Some(IO_TIMEOUT));
    let mut line = Vec::new();
    let mut reader = BufReader::new(&conn).take(proto::MAX_LINE as u64 + 1);
    if reader.read_until(b'\n', &mut line).is_err() || line.is_empty() {
        return After::Continue;
    }
    if line.ends_with(b"\n") {
        line.pop();
    }
    let (text, after) = daemon.handle_line(&String::from_utf8_lossy(&line));
    if after == After::Exit {
        leaving.store(true, Ordering::SeqCst);
    }
    let mut out = &conn;
    let _ = out
        .write_all(text.as_bytes())
        .and_then(|()| out.write_all(b"\n"));
    after
}

/// The daemon keeps its model for its whole life: its exit, not an idle release, gives the memory
/// back (a reload peaks near 1 GB).
fn daemon_model(home: &Path) -> Arc<EmbedModel> {
    EmbedModel::of_home_resident(home)
}

/// The log is the daemon's stderr, opened by whoever launched it: only its owner reads it, and
/// it does not grow without end. Its writer appends, so cutting it in place is safe.
fn tidy_log(log: &Path) {
    let Ok(file) = std::fs::OpenOptions::new().write(true).open(log) else {
        return;
    };
    let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
    if file.metadata().is_ok_and(|m| m.len() > MAX_LOG) {
        let _ = file.set_len(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_daemon_never_releases_its_model() {
        assert_eq!(daemon_model(Path::new("/nonexistent")).idle(), None);
    }
}
