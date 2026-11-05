use std::process::ExitCode;
use std::sync::Arc;

use clio_memory::{
    ContextOpts, Engine, Error, Format, MemoryRow, OpenMode, RecallOpts, ScopeId, State,
    SystemClock,
};

use clio_memory::setup::agents::{Env, shadowing, shadowing_note};
use clio_memory::setup::plan::{Health, agent_report};

use crate::args::{Cli, Command};
use crate::output::{clean, clean_multiline, line};

const JOB_LIMIT: usize = 200;

/// Runs a command that reads; `machine` is where `doctor` looks for the agents (`None`: no
/// agent lines, as without a home).
pub fn run(cli: &Cli, machine: Option<&Env>) -> Result<ExitCode, String> {
    if cli.command.writes() {
        return crate::write::run(cli);
    }
    if matches!(cli.command, Command::Doctor) {
        return doctor(cli, machine);
    }
    let engine = open_read(cli)?;
    let fail = |e: Error| e.to_string();
    match &cli.command {
        Command::Scopes => {
            let scopes = engine.scopes().map_err(fail)?;
            if cli.json {
                return json(&scopes);
            }
            for s in scopes {
                let keep = s.retention_days.map_or("never forgotten".to_string(), |d| {
                    format!("forgets after {d} days")
                });
                line(&format!(
                    "{} (id {}) · language {} · {keep}",
                    clean(&s.name),
                    s.id.0,
                    clean(&s.language)
                ));
            }
        }
        Command::Stats => {
            let s = engine.stats().map_err(fail)?;
            if cli.json {
                return json(&s);
            }
            line(&format!(
                "file: {} bytes (wal {})",
                s.file_bytes, s.wal_bytes
            ));
            line(&format!(
                "operations: {} · decisions: {} · vectors: {} · pending embeddings: {}",
                s.operations, s.decisions, s.vectors, s.pending_embeddings
            ));
            let age = s
                .queue
                .oldest_pending_age_secs
                .map_or(String::new(), |a| format!(" · oldest pending {a}s"));
            line(&format!(
                "queue: pending {} · running {} · failed {}{age}",
                s.queue.pending, s.queue.running, s.queue.failed
            ));
            for sc in s.scopes {
                line(&format!(
                    "scope {}: active {} · quarantine {} · archived {} · replaced {}",
                    clean(&sc.name),
                    sc.active,
                    sc.quarantine,
                    sc.archived,
                    sc.replaced
                ));
            }
        }
        // Handled before the file is opened: it may not exist yet.
        Command::Doctor => return doctor(cli, machine),
        Command::List { scope, state } => {
            let rows = engine
                .list(scope_id(&engine, scope)?, *state, None)
                .map_err(fail)?;
            return memories(cli, &rows);
        }
        Command::Review { scope } => {
            let rows = engine
                .list(scope_id(&engine, scope)?, Some(State::Quarantine), None)
                .map_err(fail)?;
            return memories(cli, &rows);
        }
        Command::Get { id } => {
            let m = match engine.get(*id) {
                Ok(m) => m,
                Err(Error::NotFound) => {
                    // a purged memory shows its tombstone and nothing of its content
                    let Some(t) = engine.tombstone(*id).map_err(fail)? else {
                        return Err(Error::NotFound.to_string());
                    };
                    if cli.json {
                        return json(&t);
                    }
                    line(&format!("id: m{}", id.0));
                    line("state: Purged");
                    line(&format!(
                        "purged: op {} · by {} · at {} · {}",
                        t.op.0,
                        clean(&actor_name(&engine, t.actor.0)),
                        t.at,
                        clean(&t.reason)
                    ));
                    return Ok(ExitCode::SUCCESS);
                }
                Err(e) => return Err(fail(e)),
            };
            if cli.json {
                return json(&m);
            }
            let who = actor_name(&engine, m.actor.0);
            let scope = engine
                .scopes()
                .map_err(fail)?
                .into_iter()
                .find(|s| s.id == m.scope)
                .map_or(m.scope.0.to_string(), |s| clean(&s.name));
            line(&format!("id: m{}", m.id.0));
            line(&format!("scope: {scope}"));
            line(&format!("state: {:?}", m.state));
            line(&format!("kind: {:?}", m.kind));
            line(&format!("origin: {:?}", m.origin));
            line(&format!("actor: {}", clean(&who)));
            if let Some(t) = &m.topic_key {
                line(&format!("topic: {}", clean(t)));
            }
            if let Some(s) = &m.source {
                line(&format!("source: {}", clean(s)));
            }
            let to = m.valid_to.map_or("open".to_string(), |t| t.to_string());
            line(&format!("valid: {} .. {to}", m.valid_from));
            line(&format!("known since: {}", m.known_since));
            line(&format!("content: {}", clean_multiline(&m.content)));
        }
        Command::History { id } => {
            let ops = engine.history(*id).map_err(fail)?;
            if cli.json {
                return json(&ops);
            }
            for op in ops {
                line(&format!(
                    "op {} · {:?} · by {} · at {} · {}",
                    op.id.0,
                    op.kind,
                    clean(&actor_name(&engine, op.actor.0)),
                    op.at,
                    clean(&op.reason)
                ));
            }
        }
        Command::Jobs { state } => {
            let jobs = engine.jobs(*state, JOB_LIMIT).map_err(fail)?;
            if cli.json {
                return json(&jobs);
            }
            for j in jobs {
                let error = j
                    .error
                    .as_deref()
                    .map_or(String::new(), |e| format!(" · error: {}", clean(e)));
                line(&format!(
                    "job {} · {:?} · {:?} · attempts {}/{} · run after {}{error}",
                    j.id, j.kind, j.state, j.attempts, j.max_attempts, j.run_after
                ));
            }
        }
        Command::Recall { scope, text } => {
            let found = engine
                .recall(scope_id(&engine, scope)?, text, &RecallOpts::default())
                .map_err(fail)?;
            if cli.json {
                let hits: Vec<_> = found
                    .hits
                    .iter()
                    .map(|h| {
                        serde_json::json!({
                            "memory": h.memory, "score": h.score, "confidence": h.confidence,
                        })
                    })
                    .collect();
                return json(&hits);
            }
            if found.hits.is_empty() {
                line("nothing relevant");
            }
            for h in &found.hits {
                line(&format!(
                    "{} · score {:.4}",
                    memory_line(&h.memory),
                    h.score
                ));
            }
        }
        Command::Export { path } => {
            refuse_overwrite(path)?;
            let s = engine.export_to_file(path).map_err(|e| match e {
                Error::Io(io) if io.kind() == std::io::ErrorKind::AlreadyExists => overwrite(path),
                other => other.to_string(),
            })?;
            line(&format!(
                "exported to {}: {} scopes · {} actors · {} operations · {} decisions · {} blocks · {} usage · {} feedback",
                path.display(),
                s.scopes,
                s.actors,
                s.operations,
                s.decisions,
                s.blocks,
                s.usage,
                s.feedback
            ));
        }
        Command::Context { scope, text } => {
            let opts = ContextOpts {
                query: Some(text.clone()),
                format: if cli.json {
                    Format::Json
                } else {
                    Format::Compact
                },
                ..ContextOpts::default()
            };
            let out = engine
                .context(scope_id(&engine, scope)?, &opts)
                .map_err(fail)?;
            if cli.json {
                line(&out);
            } else {
                for l in out.lines() {
                    line(&clean(l));
                }
            }
        }
        // the write commands never get here (`Command::writes` routes them to `write::run`)
        _ => return Err("not a read command".into()),
    }
    Ok(ExitCode::SUCCESS)
}

/// When the memory last slept, whether the night was complete, and its alarms.
fn slept(db: &std::path::Path, home: Option<&std::path::Path>) -> String {
    let data = db.parent().unwrap_or(std::path::Path::new("."));
    match clio_memory::sleep::last_report(data) {
        // Any sleep counts (Clio's too): it only has not run yet.
        None if home.is_some_and(clio_memory::setup::sleep_present) => {
            "info sueño: todavía no durmió (el timer ya está instalado)".into()
        }
        None => "info sueño: todavía no durmió (clio-memory setup sleep instala el timer)".into(),
        Some(r) => {
            let mut s = format!(
                "info sueño: última noche {} ({})",
                clio_memory::sleep::night_of(r.started),
                if r.complete { "completa" } else { "incompleta" }
            );
            if !r.alarms.is_empty() {
                s.push_str(&format!(" · alarmas: {}", clean(&r.alarms.join("; "))));
            }
            s
        }
    }
}

/// `doctor`: the file's checks (or, before the first memory, that there is none yet: not a
/// failure), whether another `clio-memory` runs first on the `PATH`, and one line per agent found.
/// Only a FAIL makes the exit code 1.
fn doctor(cli: &Cli, machine: Option<&Env>) -> Result<ExitCode, String> {
    let report = if cli.db.try_exists().unwrap_or(true) {
        Some(open_read(cli)?.doctor().map_err(|e| e.to_string())?)
    } else {
        None
    };
    let checks_ok = report.as_ref().is_none_or(|r| r.all_ok());
    let agents = machine.map(agent_report).unwrap_or_default();
    let agents_ok = agents.iter().all(|(_, h, _)| *h != Health::Fail);
    let shadow = machine.and_then(|env| shadowing(env).map(|o| shadowing_note(env, &o)));
    if cli.json {
        let agents: Vec<_> = agents
            .iter()
            .map(|(a, health, detail)| {
                serde_json::json!({ "agent": a.key(), "ok": *health != Health::Fail, "detail": detail })
            })
            .collect();
        let mut out = serde_json::json!({
            "ok": checks_ok && agents_ok,
            "checks": report.as_ref().map(|r| &r.checks),
            "agents": agents,
        });
        let notes: Vec<&str> = report
            .is_none()
            .then_some(NO_MEMORY_YET)
            .into_iter()
            .chain(shadow.as_deref())
            .collect();
        if !notes.is_empty() {
            out["notes"] = serde_json::json!(notes);
        }
        json(&out)?;
    } else {
        match &report {
            Some(report) => {
                for c in &report.checks {
                    let mark = if c.ok { "ok  " } else { "FAIL" };
                    line(&format!("{mark} {}: {}", c.name, clean(&c.detail)));
                }
                line(&slept(&cli.db, machine.map(|env| env.home.as_path())));
            }
            None => line(&format!("--   {NO_MEMORY_YET}")),
        }
        if let Some(note) = &shadow {
            line(&format!("--   {}", clean(note)));
        }
        for (agent, health, detail) in &agents {
            let mark = match health {
                Health::Fail => "FAIL",
                Health::Off => "--  ",
                Health::Ok => "ok  ",
            };
            line(&format!("{mark} agente {}: {}", agent.key(), clean(detail)));
        }
    }
    Ok(if checks_ok && agents_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// What `doctor` says when the memory's file is not there yet.
const NO_MEMORY_YET: &str = "todavía no hay memoria: se crea la primera vez que un agente la usa";

fn open_read(cli: &Cli) -> Result<Engine, String> {
    let engine =
        Engine::open(&cli.db, OpenMode::ReadOnly, Arc::new(SystemClock)).map_err(|e| match e {
            Error::NotFound => format!("database not found: {}", cli.db.display()),
            other => other.to_string(),
        })?;
    with_model(engine, cli)
}

/// `--model DIR`: Granite (ONNX files) or the static model, whichever the folder holds.
pub fn with_model(engine: Engine, cli: &Cli) -> Result<Engine, String> {
    match &cli.model {
        Some(dir) => match clio_memory::agent::EmbedModel::new(Some(dir.clone())).get()? {
            Some(model) => Ok(engine.with_embedder(model)),
            None => Ok(engine),
        },
        None => Ok(engine),
    }
}

fn json<T: serde::Serialize>(value: &T) -> Result<ExitCode, String> {
    line(&serde_json::to_string(value).map_err(|e| e.to_string())?);
    Ok(ExitCode::SUCCESS)
}

pub fn scope_id(engine: &Engine, name: &str) -> Result<ScopeId, String> {
    let scopes = engine.scopes().map_err(|e| e.to_string())?;
    match scopes.iter().find(|s| s.name == name) {
        Some(s) => Ok(s.id),
        None => {
            let known: Vec<String> = scopes.iter().map(|s| clean(&s.name)).collect();
            Err(format!(
                "unknown scope {:?} (known: {})",
                clean(name),
                known.join(", ")
            ))
        }
    }
}

fn actor_name(engine: &Engine, id: i64) -> String {
    engine
        .actors()
        .ok()
        .and_then(|a| a.into_iter().find(|a| a.id.0 == id))
        .map_or(format!("actor {id}"), |a| a.name)
}

fn memory_line(m: &MemoryRow) -> String {
    let topic = m
        .topic_key
        .as_deref()
        .map_or(String::new(), |t| format!(" · topic {}", clean(t)));
    format!(
        "m{} · {:?} · {:?} · {}{topic}",
        m.id.0,
        m.state,
        m.kind,
        clean(&m.content)
    )
}

fn memories(cli: &Cli, rows: &[MemoryRow]) -> Result<ExitCode, String> {
    if cli.json {
        return json(&rows);
    }
    for m in rows {
        line(&memory_line(m));
    }
    Ok(ExitCode::SUCCESS)
}

pub fn overwrite(path: &std::path::Path) -> String {
    format!("refusing to overwrite {}", path.display())
}

/// Checked up front so a refused write leaves nothing behind (no failed job, no empty file).
pub fn refuse_overwrite(path: &std::path::Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Err(overwrite(path)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot check {}: {e}", path.display())),
    }
}
