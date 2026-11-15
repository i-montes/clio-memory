//! `clio-memory`: inspect and maintain a memory file without writing code.
//! The commands that only read open the file read-only, so they work while another program
//! has it open and never change it; the ones that change something open it read-write and
//! fail with a clear message if another program holds its write lock.

mod args;
mod commands;
mod output;
mod palette;
mod setup_flow;
mod tui;
mod write;

use std::process::ExitCode;

use clio_memory::setup::agents::{AgentId, Env};

fn main() -> ExitCode {
    let _ = clio_memory::private_runtime();
    match args::parse(std::env::args().skip(1)) {
        Ok(args::Parsed::Help) => {
            output::line(args::USAGE);
            ExitCode::SUCCESS
        }
        Ok(args::Parsed::Version) => {
            output::line(&format!("clio-memory {}", env!("CARGO_PKG_VERSION")));
            ExitCode::SUCCESS
        }
        Ok(args::Parsed::Run(args::Cli {
            command:
                args::Command::Setup {
                    yes,
                    from_installer,
                    agents,
                    all,
                    model,
                    instructions,
                    replace,
                },
            db,
            ..
        })) => {
            let env = match machine() {
                Ok(env) => env,
                Err(message) => {
                    output::error(&format!("error: {message}"));
                    return ExitCode::from(1);
                }
            };
            let terminal = std::io::IsTerminal::is_terminal(&std::io::stderr());
            // In a terminal (and no `--yes`) the TUI; else the plain lines.
            let mut tui;
            let mut plain = setup_flow::Plain;
            let screen: &mut dyn setup_flow::Screen = if setup_flow::interactive(yes, terminal) {
                let mode = palette::color_mode(
                    std::env::var("COLORTERM").ok().as_deref(),
                    std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()),
                    terminal,
                );
                tui = tui::Tui::new(mode, env.home.clone());
                &mut tui
            } else {
                &mut plain
            };
            setup_flow::run(
                screen,
                &env,
                setup_flow::SetupArgs {
                    yes,
                    from_installer,
                    agents,
                    all,
                    model,
                    instructions,
                    replace,
                    terminal,
                    curl: "curl".into(),
                    data: db.parent().map(std::path::Path::to_path_buf),
                },
            )
        }
        Ok(args::Parsed::Run(args::Cli {
            command: args::Command::SetupClaudeCode { replace, claude_md },
            ..
        })) => done(setup_claude_code(replace, claude_md)),
        Ok(args::Parsed::Run(args::Cli {
            command: args::Command::SetupModel { potion },
            db,
            ..
        })) => {
            let said = setup_model(potion);
            // The daemon keeps the models it found at start; also after a failure part way, which
            // may have left a new model or runtime.
            let stopped = stop_daemon(db.parent().unwrap_or(std::path::Path::new(".")));
            done(said.map(|mut said| {
                match stopped {
                    Ok(true) => {
                        said.push("demonio detenido: el próximo uso carga el modelo nuevo".into())
                    }
                    Ok(false) => {}
                    Err(e) => said.push(format!("Ojo: no se pudo detener el demonio: {e}.")),
                }
                said
            }))
        }
        Ok(args::Parsed::Run(args::Cli {
            command: args::Command::SetupSleep { remove },
            ..
        })) => done(setup_sleep(remove)),
        Ok(args::Parsed::Run(args::Cli {
            command: args::Command::Update,
            ..
        })) => done(clio_memory::setup::update(clio_memory::setup::INSTALLER_URL).map(|()| vec![])),
        Ok(args::Parsed::Run(args::Cli {
            command:
                args::Command::Sleep {
                    now,
                    dry_run,
                    report,
                },
            db,
            model,
            json,
            ..
        })) => done(sleep(&db, model, now, dry_run, report, json)),
        Ok(args::Parsed::Run(args::Cli {
            command:
                args::Command::Uninstall {
                    agents,
                    models,
                    data,
                    yes,
                },
            db,
            ..
        })) => uninstall(&db, agents.as_deref(), models, data, yes),
        Ok(args::Parsed::Run(args::Cli {
            command: args::Command::Daemon { status, stop },
            db,
            ..
        })) => done(daemon(&db, status, stop)),
        Ok(args::Parsed::Run(cli)) if matches!(cli.command, args::Command::Mcp) => {
            match mcp(&cli.db) {
                Ok(()) => ExitCode::SUCCESS,
                Err(message) => {
                    output::error(&format!("error: {message}"));
                    ExitCode::from(1)
                }
            }
        }
        Ok(args::Parsed::Run(cli)) => match commands::run(&cli, machine().ok().as_ref()) {
            Ok(code) => code,
            Err(message) => {
                output::error(&format!("error: {message}"));
                ExitCode::from(1)
            }
        },
        Err(args::UsageError(message)) => {
            // The full text is ~14 KB; a caller (often an agent) pays for it on every typo.
            output::error(&format!(
                "error: {message}\nhelp: `clio-memory --help` prints the usage"
            ));
            ExitCode::from(2)
        }
    }
}

/// `clio-memory mcp`: the file must be called `memory.db` (the agent layer's name for it).
fn mcp(db: &std::path::Path) -> Result<(), String> {
    use clio_memory::agent::memory::DB_FILE;
    use clio_memory::daemon::Launch;
    use clio_memory::mcp::{Backend, serve_with};
    if db.file_name().is_none_or(|n| n != DB_FILE) {
        return Err(format!(
            "mcp uses <folder>/{DB_FILE}: give --db a file called {DB_FILE}, or set CLIO_MEMORY_DATA"
        ));
    }
    let data = db.parent().unwrap_or(std::path::Path::new("."));
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let backend = if std::env::var_os("CLIO_MEMORY_NO_DAEMON").is_some_and(|v| v == "1") {
        Backend::Local
    } else {
        Backend::Daemon(Launch {
            exe: std::env::current_exe().map_err(|e| e.to_string())?,
            env: vec![],
        })
    };
    serve_with(
        std::io::stdin().lock(),
        std::io::stdout().lock(),
        data,
        &cwd,
        &home,
        backend,
    )
}

/// `clio-memory daemon`: serve `<folder>/memory.db`'s folder in the foreground, or ask the one
/// that runs.
fn daemon(db: &std::path::Path, status: bool, stop: bool) -> Result<Vec<String>, String> {
    use clio_memory::agent::memory::DB_FILE;
    use clio_memory::daemon::serve;
    if db.file_name().is_none_or(|n| n != DB_FILE) {
        return Err(format!(
            "daemon uses <folder>/{DB_FILE}: give --db a file called {DB_FILE}, or set CLIO_MEMORY_DATA"
        ));
    }
    let data = db.parent().unwrap_or(std::path::Path::new("."));
    if stop {
        return Ok(vec![match daemon_answer(data, "shutdown")? {
            Some(_) => "demonio detenido".into(),
            None => "no había demonio".into(),
        }]);
    }
    if status {
        return Ok(vec![match daemon_answer(data, "status")? {
            Some(a) => format!(
                "demonio: activo (pid {}, v{}, modelo {}, {} sesiones)",
                a["pid"],
                a["bin"].as_str().unwrap_or_default(),
                if a["loaded"] == true {
                    "cargado"
                } else {
                    "sin cargar"
                },
                a["sessions"]
            ),
            None => "demonio: inactivo (se inicia solo con la primera llamada)".into(),
        }]);
    }
    let idle = std::env::var("CLIO_MEMORY_DAEMON_IDLE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(600);
    serve(data, &home()?, std::time::Duration::from_secs(idle)).map(|()| vec![])
}

/// Asks the daemon of `data` without launching one. `None`: it is not running, or it is an older
/// one that answered `stale` and is leaving. Any other failure is said as it is.
fn daemon_answer(data: &std::path::Path, op: &str) -> Result<Option<serde_json::Value>, String> {
    use clio_memory::daemon::{NO_DAEMON, request};
    match request(data, None, &serde_json::json!({ "op": op })) {
        Ok(a) if a["ok"] == true => Ok(Some(a)),
        Ok(a) if a["error"] == "stale" => Ok(None),
        Ok(a) => Err(format!(
            "el demonio contestó: {}",
            a["text"].as_str().unwrap_or_default()
        )),
        Err(e) if e.starts_with(NO_DAEMON) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Stops the daemon of `data` and waits (up to 2 s) for its socket to go. Whether there was one.
fn stop_daemon(data: &std::path::Path) -> Result<bool, String> {
    let was = daemon_answer(data, "shutdown")?.is_some();
    let sock =
        clio_memory::daemon::socket_path(data, std::env::var_os("CLIO_MEMORY_SOCK").as_deref())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while sock.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    Ok(was)
}

fn done(r: Result<Vec<String>, String>) -> ExitCode {
    match r {
        Ok(lines) => {
            for l in lines {
                output::line(&l);
            }
            ExitCode::SUCCESS
        }
        Err(message) => {
            output::error(&format!("error: {message}"));
            ExitCode::from(1)
        }
    }
}

fn home() -> Result<std::path::PathBuf, String> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(std::path::PathBuf::from)
        .filter(|h| h.is_absolute())
        .ok_or_else(|| "HOME is not set".to_owned())
}

/// A yes/no question on the terminal; no terminal is a no.
fn ask(question: &str) -> bool {
    use std::io::{BufRead, IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        return false;
    }
    eprint!("{question} [s/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    let _ = std::io::stdin().lock().read_line(&mut answer);
    matches!(
        answer.trim().to_lowercase().as_str(),
        "s" | "si" | "sí" | "y" | "yes"
    )
}

/// This machine, as setup sees it: the home, this binary, the `PATH`, and the overrides of
/// where Codex and Claude Code are and of where the installer puts the binary.
fn machine() -> Result<Env<'static>, String> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(std::path::PathBuf::from)
    };
    Ok(Env {
        home: home()?,
        exe: std::env::current_exe().map_err(|e| e.to_string())?,
        path: std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default(),
        codex_home: var("CODEX_HOME"),
        claude: var("CLIO_MEMORY_CLAUDE"),
        bin_dir: var("CLIO_MEMORY_BIN_DIR"),
        service: &clio_memory::setup::service_command,
    })
}

/// `clio-memory uninstall`: every step says how it went; any that failed makes the exit code 1.
fn uninstall(
    db: &std::path::Path,
    agents: Option<&[AgentId]>,
    models: bool,
    data: bool,
    yes: bool,
) -> ExitCode {
    use clio_memory::setup::plan::{self, Step};
    let fail = |message: String, code: u8| {
        output::error(&format!("error: {message}"));
        ExitCode::from(code)
    };
    let env = match machine() {
        Ok(env) => env,
        Err(e) => return fail(e, 1),
    };
    // The memory goes only with a yes, asked before anything else: a no changes nothing.
    let memory = match data.then(|| memory_files(db)).transpose() {
        Ok(memory) => memory,
        Err(e) => return fail(e, 2),
    };
    if let Some(memory) = memory.as_ref().filter(|_| !yes) {
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            return fail(
                format!(
                    "--data borra para siempre {}: sin terminal, confírmalo con --yes",
                    memory.what()
                ),
                2,
            );
        }
        if !ask(&format!("¿Borro {}? No se puede deshacer.", memory.what())) {
            return fail("no se cambió nada".into(), 1);
        }
    }
    // The models and the memory go before the binary, which goes last and stays if either fails.
    let gone = || {
        let mut gone = Vec::new();
        if models {
            let dir = env.home.join(".cache/clio-models");
            gone.push(Step {
                agent: None,
                what: "modelos".into(),
                outcome: remove_folder(&dir).map(|()| {
                    format!(
                        "modelos borrados: {} (también eran de Clio; `clio-memory setup model` los vuelve a bajar)",
                        dir.display()
                    )
                }),
            });
        }
        if let Some(memory) = &memory {
            gone.push(Step {
                agent: None,
                what: "datos".into(),
                outcome: memory
                    .remove()
                    .map(|()| format!("memoria borrada: {}", memory.what())),
            });
        }
        gone
    };
    // Best effort: a daemon that cannot be reached leaves by itself when its folder is gone.
    let _ = stop_daemon(db.parent().unwrap_or(std::path::Path::new(".")));
    let steps = plan::uninstall_with(&env, agents, &gone);
    if steps.is_empty() {
        output::line("nada que quitar");
    }
    for step in &steps {
        output::line(&step.line());
    }
    if steps.iter().any(|s| s.outcome.is_err()) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// What `--data` deletes: the memory file with what SQLite and the locks keep beside it, and
/// the sleep's folder (its state, reports and nightly copies of the file). Nothing else of the
/// folder they are in: Clio keeps its own data there (its vault among it).
struct MemoryFiles {
    db: std::path::PathBuf,
    files: Vec<std::path::PathBuf>,
    sleep: std::path::PathBuf,
}

impl MemoryFiles {
    /// For the person: the file and the sleep's copies.
    fn what(&self) -> String {
        format!(
            "la memoria: {} y las copias del sueño en {}",
            self.db.display(),
            self.sleep.display()
        )
    }

    /// Deletes them; one that is not there is already gone.
    fn remove(&self) -> Result<(), String> {
        for file in &self.files {
            match std::fs::remove_file(file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    return Err(format!("{}: {e}", file.display()));
                }
                _ => {}
            }
        }
        remove_folder(&self.sleep)
    }
}

/// The memory's files for `--db`, which must be a file called `memory.db` (the sleep's folder is
/// only that file's beside it).
fn memory_files(db: &std::path::Path) -> Result<MemoryFiles, String> {
    use clio_memory::agent::memory::DB_FILE;
    let data = db
        .parent()
        .filter(|_| db.file_name().is_some_and(|n| n == DB_FILE))
        .ok_or_else(|| {
            format!(
                "--data borra {DB_FILE} y lo que lo acompaña, y {} no es un {DB_FILE}",
                db.display()
            )
        })?;
    // SQLite's `-wal` and `-shm`, the writer's lock and the sleep's lock, and the daemon's own.
    let files = ["", "-wal", "-shm", ".lock", ".sleep.lock"]
        .into_iter()
        .map(|suffix| {
            let mut name = db.as_os_str().to_owned();
            name.push(suffix);
            std::path::PathBuf::from(name)
        })
        .chain(["daemon.sock", "daemon.lock", "daemon.log"].map(|f| data.join(f)))
        .collect();
    Ok(MemoryFiles {
        db: db.to_path_buf(),
        files,
        sleep: clio_memory::sleep::sleep_dir(data),
    })
}

/// Deletes a folder and everything in it; one that is not there is already gone.
fn remove_folder(dir: &std::path::Path) -> Result<(), String> {
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("{}: {e}", dir.display()))
        }
        _ => Ok(()),
    }
}

fn setup_claude_code(replace: bool, claude_md: Option<bool>) -> Result<Vec<String>, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let claude = std::env::var_os("CLIO_MEMORY_CLAUDE")
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "claude".into());
    clio_memory::setup::ClaudeCode {
        home: home()?,
        claude: claude.into(),
        exe,
        replace,
        claude_md,
        ask: &ask,
        service: &clio_memory::setup::service_command,
    }
    .run()
}

fn setup_sleep(remove: bool) -> Result<Vec<String>, String> {
    use clio_memory::setup::{install_sleep, remove_sleep, service_command};
    let home = home()?;
    if remove {
        remove_sleep(&home, &service_command)?;
        return Ok(vec!["sueño quitado".into()]);
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let at = install_sleep(&home, &exe, &["sleep"], &service_command)?;
    Ok(vec![format!("sueño instalado: {}", at.display())])
}

/// `clio-memory sleep`: one night, or with `--report` the last night's report.
fn sleep(
    db: &std::path::Path,
    model: Option<std::path::PathBuf>,
    now: bool,
    dry_run: bool,
    report: bool,
    json: bool,
) -> Result<Vec<String>, String> {
    use clio_memory::agent::memory::{DB_FILE, EmbedModel};
    use clio_memory::sleep::{Options, Slept, last_report, morning_line, run};
    if db.file_name().is_none_or(|n| n != DB_FILE) {
        return Err(format!(
            "sleep uses <folder>/{DB_FILE}: give --db a file called {DB_FILE}"
        ));
    }
    let data = db.parent().unwrap_or(std::path::Path::new("."));
    if report {
        return Ok(vec![match last_report(data) {
            Some(r) => serde_json::to_string_pretty(&r).map_err(|e| e.to_string())?,
            None => "sueño: todavía no durmió".into(),
        }]);
    }
    let model = match model {
        Some(dir) => EmbedModel::new(Some(dir)),
        None => EmbedModel::of_home(&home()?),
    };
    let clock: std::sync::Arc<dyn clio_memory::Clock> =
        std::sync::Arc::new(clio_memory::SystemClock);
    let out = run(
        data,
        &model,
        None,
        &Options {
            now,
            dry_run,
            clock: clock.clone(),
        },
    )?;
    let r = match out {
        Slept::Skipped(why) => return Ok(vec![format!("no durmió: {why}")]),
        Slept::Done(r) => r,
    };
    if json {
        return Ok(vec![serde_json::to_string(&r).map_err(|e| e.to_string())?]);
    }
    let mut said = Vec::new();
    if dry_run {
        said.push("solo mirar: no se cambió nada".to_owned());
    }
    said.push(morning_line(&r, clock.now()).map_or_else(
        || "nada que hacer".to_owned(),
        |l| l.replacen("Anoche (sueño)", "sueño", 1),
    ));
    if !r.complete {
        said.push("el sueño quedó incompleto: el próximo turno lo sigue".into());
    }
    Ok(said)
}

fn setup_model(potion: bool) -> Result<Vec<String>, String> {
    use clio_memory::setup::{
        GRANITE, POTION, RUNTIME_DIR, curl_fetch, curl_get, install_model, install_runtime,
        remove_obsolete, runtime_here, tar_unpack,
    };
    let spec = if potion { &POTION } else { &GRANITE };
    let home = home()?;
    let dest = home.join(spec.dir);
    let curl = std::path::PathBuf::from("curl");
    let mut said = install_model(&dest, &spec.pairs(), &curl_fetch(&curl, spec))?;
    said.push(format!("modelo listo en {}", dest.display()));
    if !potion {
        // What Granite runs on: ONNX Runtime for the CPU.
        let runtime = runtime_here();
        if runtime.is_empty() {
            said.push("no hay ONNX Runtime publicado para este sistema: se usará el modelo estático o el texto".into());
        } else {
            let root = home.join(RUNTIME_DIR);
            said.extend(install_runtime(
                &root,
                &runtime,
                &|url, to| curl_get(&curl, url, to),
                &tar_unpack,
            )?);
            said.push(format!("runtime listo en {} (procesador)", root.display()));
            said.extend(remove_obsolete(&home));
        }
    }
    Ok(said)
}
