//! `clio-memory`: inspect and maintain a memory file without writing code.
//! The commands that only read open the file read-only, so they work while another program
//! has it open and never change it; the ones that change something open it read-write and
//! fail with a clear message if another program holds its write lock.

mod args;
mod commands;
mod output;
mod write;

use std::process::ExitCode;

use clio_memory::setup::agents::{AgentId, Env};

fn main() -> ExitCode {
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
            command: args::Command::SetupClaudeCode { replace, claude_md },
            ..
        })) => done(setup_claude_code(replace, claude_md)),
        Ok(args::Parsed::Run(args::Cli {
            command: args::Command::SetupModel { potion },
            ..
        })) => done(setup_model(potion)),
        Ok(args::Parsed::Run(args::Cli {
            command: args::Command::SetupSleep { remove },
            ..
        })) => done(setup_sleep(remove)),
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
    clio_memory::mcp::serve(
        std::io::stdin().lock(),
        std::io::stdout().lock(),
        data,
        &cwd,
        &home,
    )
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
/// where Codex and Claude Code are.
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
    let mut steps = plan::uninstall(&env, agents);
    let mut gone = Vec::new();
    if models {
        let dir = env.home.join(".cache/clio-models");
        gone.push(Step {
            agent: None,
            what: "modelos".into(),
            outcome: remove_folder(&dir).map(|()| format!("modelos borrados: {}", dir.display())),
        });
    }
    if let Some(memory) = memory {
        gone.push(Step {
            agent: None,
            what: "datos".into(),
            outcome: memory
                .remove()
                .map(|()| format!("memoria borrada: {}", memory.what())),
        });
    }
    // Before the binary, which goes last.
    let at = steps
        .iter()
        .position(|s| s.what == plan::BINARY)
        .unwrap_or(steps.len());
    steps.splice(at..at, gone);
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
    // SQLite's `-wal` and `-shm`, the writer's lock and the sleep's lock.
    let files = ["", "-wal", "-shm", ".lock", ".sleep.lock"]
        .into_iter()
        .map(|suffix| {
            let mut name = db.as_os_str().to_owned();
            name.push(suffix);
            std::path::PathBuf::from(name)
        })
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
        return Ok(vec!["sueño nocturno quitado".into()]);
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let at = install_sleep(&home, &exe, &["sleep"], &service_command)?;
    Ok(vec![format!("sueño nocturno instalado: {}", at.display())])
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
        said.push("la noche quedó incompleta: el próximo turno la sigue".into());
    }
    Ok(said)
}

fn setup_model(potion: bool) -> Result<Vec<String>, String> {
    use clio_memory::setup::{
        GRANITE, POTION, RUNTIME_DIR, curl_fetch, curl_get, install_model, install_runtime,
        runtime_here, tar_unpack,
    };
    let spec = if potion { &POTION } else { &GRANITE };
    let home = home()?;
    let dest = home.join(spec.dir);
    let curl = std::path::PathBuf::from("curl");
    let mut said = install_model(&dest, &spec.pairs(), &curl_fetch(&curl, spec))?;
    said.push(format!("modelo listo en {}", dest.display()));
    if !potion {
        // What Granite runs on: the CPU runtime, or with an NVIDIA card the GPU one and the CUDA
        // libraries the system lacks.
        let runtime = runtime_here();
        if runtime.is_empty() {
            said.push("no hay ONNX Runtime publicado para este sistema: se usará el modelo estático o el texto".into());
        } else {
            let gpu = runtime.len() > 1 || runtime[0].url.contains("gpu");
            let root = home.join(RUNTIME_DIR);
            said.extend(install_runtime(
                &root,
                &runtime,
                &|url, to| curl_get(&curl, url, to),
                &tar_unpack,
            )?);
            said.push(format!(
                "runtime listo en {} ({})",
                root.display(),
                if gpu {
                    "GPU, y el procesador si la GPU no responde"
                } else {
                    "procesador"
                }
            ));
        }
    }
    Ok(said)
}
