//! The sleep: once a night, while nobody is using the memory, it checks and copies the file,
//! quarantines secrets and likely attacks, folds repeats, merges near-duplicates when given a
//! model, and purges what was discarded long ago. Each night it looks only at what changed since
//! the last one (and at its nearest neighbors): what is solid is not reviewed again. See
//! docs/motor.md.

pub mod signals;
mod state;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub use state::{SleepState, last_report};

use crate::agent::memory::{DB_FILE, EmbedModel};
use crate::{
    ActorId, ActorKind, Clock, Embedder, Engine, Error, MemoryId, MemoryRow, OpId, OpenMode,
    Origin, Relation, SLEEP_REASON, SLEEP_SECRET, State,
};
use signals::{RULES_VERSION, attack_families, has_secret, looks_like_attack, validate_merge};

/// The model step, given by a host that has a language model (Clio). The memory text it gets is
/// data: it must not be able to act on it.
pub trait Merger {
    /// The model's raw answer for a group: `{"texto","titulo","fuentes"}`, checked by the sleep.
    /// `None` when there is no answer (no model, budget spent): the step ends for the night.
    fn merge(&self, group: &[MemoryRow]) -> Option<String>;
    /// Does `merged` keep everything `original` said? Only `Some(true)` lets the merge happen.
    fn preserved(&self, original: &MemoryRow, merged: &str) -> Option<bool>;
    /// Tokens spent so far.
    fn tokens(&self) -> u64 {
        0
    }
}

pub struct Options {
    /// Skip the waits: the last sleep and the quiet half hour (trying it by hand).
    pub now: bool,
    /// Look and report, change nothing: no copy, no model, no write.
    pub dry_run: bool,
    pub clock: Arc<dyn Clock>,
}

#[derive(Debug)]
pub enum Slept {
    Skipped(&'static str),
    Done(Report),
}

/// What a night did. One file a night; a night interrupted and taken up again keeps adding to it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub started: i64,
    pub ended: i64,
    pub complete: bool,
    /// Memory and the reason it went to quarantine.
    pub quarantined: Vec<(i64, String)>,
    /// The new memory and the ones it replaced.
    pub merged: Vec<(i64, Vec<i64>)>,
    pub purged: Vec<i64>,
    /// A repeat and the memory it repeated (which stays).
    pub repeated: Vec<(i64, i64)>,
    /// For the agent or the person to look at: contradictions, merges refused.
    pub review: Vec<String>,
    pub alarms: Vec<String>,
    pub tokens: u64,
}

impl Report {
    fn alarm(&mut self, text: String) {
        if !self.alarms.contains(&text) {
            self.alarms.push(text);
        }
    }

    fn review(&mut self, text: String) {
        if !self.review.contains(&text) {
            self.review.push(text);
        }
    }
}

/// A complete night less than this long ago: no other one.
const SLEPT_WITHIN: i64 = 20 * 3600;
/// The memory must have been quiet this long.
const QUIET: i64 = 30 * 60;
/// Similarity from which two memories are the same thing said twice (Granite).
const REPEAT: f32 = 0.97;
/// Similarity from which two memories may be merged by the model (Granite).
const NEAR: f32 = 0.87;
const NEIGHBORS: usize = 5;
const MAX_GROUPS: usize = 20;
const MAX_GROUP: usize = 4;
const MAX_PURGES: usize = 50;
/// Quarantines a night: at least this many, or a tenth of the active memories.
const MIN_QUARANTINES: usize = 10;
const EMBED_BATCH: usize = 32;

/// `YYYY-MM-DD` of the evening a night belongs to (UTC, twelve hours back): a sleep at 00:30 is
/// still the night that began at 23:00.
pub fn night_of(at: i64) -> String {
    crate::tools::civil_date(at - 12 * 3600)
}

/// Why a night stops before its end.
enum Stop {
    /// Someone else wrote or used the memory: they come first, the next slot carries on.
    Disturbed,
    Failed(String),
}

impl From<Error> for Stop {
    fn from(e: Error) -> Self {
        Stop::Failed(crate::agent::memory::explain(&e))
    }
}

/// One night of sleep over `<data>/memory.db`. See the module docs and docs/motor.md.
pub fn run(
    data: &Path,
    model: &Arc<EmbedModel>,
    merger: Option<&dyn Merger>,
    opts: &Options,
) -> Result<Slept, String> {
    let db = data.join(DB_FILE);
    if !db.is_file() {
        return Ok(Slept::Skipped("todavía no hay memoria"));
    }
    let Some(_lock) = sleep_lock(&db)? else {
        return Ok(Slept::Skipped("otro sueño está corriendo"));
    };
    let now = opts.clock.now();
    let state = state::load(data);
    if !opts.now && now - state.last_complete < SLEPT_WITHIN {
        return Ok(Slept::Skipped("ya durmió esta noche"));
    }
    let night = night_of(now);
    let mut report = state::load_report(data, &night)
        .filter(|r| !r.complete)
        .unwrap_or(Report {
            started: now,
            ..Report::default()
        });
    let save = |report: &mut Report| {
        report.ended = opts.clock.now();
        if !opts.dry_run {
            state::save_report(data, &night, report)?;
        }
        Ok::<_, String>(())
    };
    if let Err(why) = quick_check(&db) {
        report.alarm(format!(
            "la base no pasó el chequeo ({why}): no se tocó nada"
        ));
        report.complete = false;
        save(&mut report)?;
        return Ok(Slept::Done(report));
    }
    let embedder = match model.get() {
        Ok(e) => e,
        Err(e) => {
            report.alarm(format!("el modelo de vectores no cargó: {e}"));
            None
        }
    };
    let open = |mode| -> Result<Engine, Error> {
        let e = Engine::open(&db, mode, opts.clock.clone())?;
        Ok(match &embedder {
            Some(m) => e.with_embedder(m.clone()),
            None => e,
        })
    };
    let read = open(OpenMode::ReadOnly).map_err(|e| crate::agent::memory::explain(&e))?;
    let system = read
        .actors()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|a| a.kind == ActorKind::System)
        .map_or(ActorId(0), |a| a.id);
    let activity = read.last_activity(system).map_err(|e| e.to_string())?;
    if !opts.now && now - activity < QUIET {
        return Ok(Slept::Skipped("la memoria está en uso"));
    }
    let mut night_run = Night {
        db: db.clone(),
        read,
        open: &open,
        embedder: embedder.clone(),
        dry_run: opts.dry_run,
        system,
        seen: OpId(state.cursor),
        activity,
        report: &mut report,
    };
    let done = night_run.sleep(data, &night, &state, merger);
    let cursor = night_run.seen;
    report.tokens += merger.map_or(0, |m| m.tokens());
    match done {
        Ok(()) => {
            report.complete = true;
            if !opts.dry_run {
                state::save(
                    data,
                    &SleepState {
                        cursor: cursor.0,
                        rules_version: RULES_VERSION,
                        last_complete: opts.clock.now(),
                    },
                )?;
            }
        }
        Err(Stop::Disturbed) => report.complete = false,
        Err(Stop::Failed(why)) => {
            report.complete = false;
            report.alarm(format!("el sueño se detuvo: {why}"));
        }
    }
    save(&mut report)?;
    Ok(Slept::Done(report))
}

/// `<db>.sleep.lock`: two sleeps never overlap (two installations, a manual run). `None` when
/// another holds it.
fn sleep_lock(db: &Path) -> Result<Option<std::fs::File>, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut name = db.as_os_str().to_owned();
    name.push(".sleep.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(PathBuf::from(&name))
        .map_err(|e| format!("{}: {e}", PathBuf::from(&name).display()))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e.to_string()),
    }
}

/// `PRAGMA quick_check` on a read-only connection.
fn quick_check(db: &Path) -> Result<(), String> {
    use rusqlite::{Connection, OpenFlags};
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    let first: String = conn
        .query_row("PRAGMA quick_check(1)", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if first == "ok" { Ok(()) } else { Err(first) }
}

struct Night<'a> {
    db: PathBuf,
    /// Reads go here; it never holds the writer.
    read: Engine,
    open: &'a dyn Fn(OpenMode) -> Result<Engine, Error>,
    embedder: Option<Arc<dyn Embedder>>,
    dry_run: bool,
    system: ActorId,
    /// The last operation looked at: anything after it not signed by the sleep is someone else.
    seen: OpId,
    /// The last use or foreign write when the night began.
    activity: i64,
    report: &'a mut Report,
}

impl Night<'_> {
    fn sleep(
        &mut self,
        data: &Path,
        night: &str,
        state: &SleepState,
        merger: Option<&dyn Merger>,
    ) -> Result<(), Stop> {
        let changed = self.read.operations_after(OpId(state.cursor), usize::MAX)?;
        if let Some(last) = changed.last() {
            self.seen = last.id;
        }
        if !self.dry_run {
            self.system = self.writer()?.system_actor()?;
        }
        let copied = self.dry_run
            || match state::copy(&self.db, data, night) {
                Ok(()) => true,
                Err(e) => {
                    self.report.alarm(format!(
                        "la copia falló ({e}): esta noche no hay purga ni unión"
                    ));
                    false
                }
            };
        self.vectors()?;

        let mut fresh: Vec<MemoryId> = changed
            .iter()
            .flat_map(|op| op.changes.iter().map(|c| c.memory))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        fresh.sort();
        let mut active = Vec::new();
        for scope in self.read.scopes()? {
            active.extend(self.read.list(scope.id, Some(State::Active), None)?);
        }
        let fresh: Vec<MemoryRow> = fresh
            .into_iter()
            .filter_map(|id| active.iter().find(|r| r.id == id).cloned())
            .collect();
        // New rules look at everything once; otherwise only at what changed.
        let checked = if state.rules_version == RULES_VERSION {
            fresh.clone()
        } else {
            active.clone()
        };
        // A night taken up again counts what it already quarantined.
        let limit = MIN_QUARANTINES
            .max(active.len() / 10)
            .saturating_sub(self.report.quarantined.len());
        let mut gone: HashSet<MemoryId> = HashSet::new();
        let before = self.report.quarantined.len();
        self.quarantine(&checked, limit, &mut gone)?;
        let left = limit.saturating_sub(self.report.quarantined.len() - before);
        if self.embedder.is_some() {
            self.repeats(&fresh, left, &mut gone)?;
            if let (Some(m), true, false) = (merger, copied, self.dry_run) {
                self.merge(m, &fresh, &mut gone)?;
            }
        }
        self.contradictions(&fresh, &gone)?;
        if copied {
            self.purge()?;
        }
        if self.disturbed()? {
            return Err(Stop::Disturbed);
        }
        Ok(())
    }

    /// Did anyone else write or use the memory since the night began?
    fn disturbed(&mut self) -> Result<bool, Stop> {
        let after = self.read.operations_after(self.seen, usize::MAX)?;
        if after.iter().any(|op| op.actor != self.system) {
            return Ok(true);
        }
        if let Some(last) = after.last() {
            self.seen = last.id;
        }
        Ok(self.read.last_activity(self.system)? > self.activity)
    }

    /// The writer, for one short section: never while someone else is at it.
    fn writer(&mut self) -> Result<Engine, Stop> {
        if self.disturbed()? {
            return Err(Stop::Disturbed);
        }
        for _ in 0..8 {
            match (self.open)(OpenMode::ReadWrite) {
                Ok(e) => return Ok(e),
                Err(Error::WriterLocked) => std::thread::sleep(Duration::from_millis(250)),
                Err(e) => return Err(e.into()),
            }
        }
        Err(Stop::Disturbed)
    }

    fn vectors(&mut self) -> Result<(), Stop> {
        if self.dry_run || self.embedder.is_none() {
            return Ok(());
        }
        // shortcut: bounded so a model that never finishes cannot hold the night; the rest waits.
        for _ in 0..1000 {
            match self.writer()?.embed_pending(EMBED_BATCH) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => {
                    self.report.alarm(format!("los vectores fallaron: {e}"));
                    break;
                }
            }
        }
        Ok(())
    }

    fn kept(&self, row: &MemoryRow) -> Result<bool, Stop> {
        Ok(self.read.kept_by_person(row)?)
    }

    /// Secrets first, then likely attacks, up to `limit`.
    fn quarantine(
        &mut self,
        rows: &[MemoryRow],
        limit: usize,
        gone: &mut HashSet<MemoryId>,
    ) -> Result<(), Stop> {
        let mut secrets = Vec::new();
        let mut attacks = Vec::new();
        for r in rows {
            let text = format!("{}\n{}", r.content, r.title.as_deref().unwrap_or(""));
            let kept = self.kept(r)?;
            if has_secret(&text) {
                if kept {
                    self.report.alarm(format!(
                        "m{} es de la persona y parece tener un secreto: no se tocó",
                        r.id.0
                    ));
                } else {
                    secrets.push((r.id, SLEEP_SECRET.to_owned()));
                }
            } else if !kept && looks_like_attack(&text, r.origin == Origin::External) {
                let families: Vec<&str> = attack_families(&text, r.origin == Origin::External)
                    .into_iter()
                    .map(|f| f.label())
                    .collect();
                attacks.push((
                    r.id,
                    format!("{SLEEP_REASON} posible ataque ({})", families.join(", ")),
                ));
            }
        }
        let mut all = secrets;
        all.extend(attacks);
        if all.len() > limit {
            self.report.alarm(format!(
                "demasiados para cuarentena ({}; tope {limit}): posible ataque o regla rota",
                all.len()
            ));
            all.truncate(limit);
        }
        for (id, why) in all {
            if self.quarantine_one(id, &why)? {
                gone.insert(id);
            }
        }
        Ok(())
    }

    fn quarantine_one(&mut self, id: MemoryId, why: &str) -> Result<bool, Stop> {
        let done = self.dry_run || self.writer()?.sleep_quarantine(id, why).is_ok();
        if done {
            self.report.quarantined.push((id.0, why.to_owned()));
        }
        Ok(done)
    }

    /// The same thing said twice in a place, with the same trust: one stays (the person's, else
    /// the pinned, else the most used, else the newest) with the other's relations.
    fn repeats(
        &mut self,
        fresh: &[MemoryRow],
        mut left: usize,
        gone: &mut HashSet<MemoryId>,
    ) -> Result<(), Stop> {
        for r in fresh {
            if gone.contains(&r.id) || !self.read.has_vector(r.id)? {
                continue;
            }
            for (n, score) in self.read.neighbors(r.id, &[r.scope], NEIGHBORS)? {
                if score < REPEAT || gone.contains(&n.id) || trust(&n) != trust(r) {
                    continue;
                }
                let (keep, lose) = if self.rank(&n)? > self.rank(r)? {
                    (n, r.clone())
                } else {
                    (r.clone(), n)
                };
                if self.kept(&lose)? {
                    continue;
                }
                if left == 0 {
                    self.report
                        .alarm("demasiados para cuarentena: quedaron repetidos sin unir".into());
                    return Ok(());
                }
                let why = format!("{SLEEP_REASON} repetido de m{}", keep.id.0);
                if !self.dry_run {
                    let _ = self.writer()?.copy_relations(lose.id, keep.id);
                }
                if self.quarantine_one(lose.id, &why)? {
                    self.report.repeated.push((lose.id.0, keep.id.0));
                    gone.insert(lose.id);
                    left -= 1;
                }
                if lose.id == r.id {
                    break;
                }
            }
        }
        Ok(())
    }

    fn rank(&self, r: &MemoryRow) -> Result<(bool, bool, u32, i64), Stop> {
        let pinned = self.read.pinned(r.scope)?.iter().any(|p| p.id == r.id);
        let uses = self.read.uses(&[r.id])?.get(&r.id).copied().unwrap_or(0);
        Ok((self.read.held_by_person(r)?, pinned, uses, r.id.0))
    }

    /// Groups of new memories and their near-duplicates, merged by the model and checked before
    /// anything is written.
    fn merge(
        &mut self,
        merger: &dyn Merger,
        fresh: &[MemoryRow],
        gone: &mut HashSet<MemoryId>,
    ) -> Result<(), Stop> {
        let (mut tried, mut failed) = (0usize, 0usize);
        for seed in fresh {
            if tried >= MAX_GROUPS {
                break;
            }
            if gone.contains(&seed.id) || self.kept(seed)? || !self.read.has_vector(seed.id)? {
                continue;
            }
            let mut group = vec![seed.clone()];
            for (n, score) in self.read.neighbors(seed.id, &[seed.scope], NEIGHBORS)? {
                if group.len() >= MAX_GROUP {
                    break;
                }
                if !(NEAR..REPEAT).contains(&score)
                    || gone.contains(&n.id)
                    || n.kind != seed.kind
                    || trust(&n) != trust(seed)
                    || self.kept(&n)?
                    || self.conflicts(&n, &group)?
                {
                    continue;
                }
                group.push(n);
            }
            if group.len() < 2 {
                continue;
            }
            gone.extend(group.iter().map(|r| r.id));
            tried += 1;
            if self.disturbed()? {
                return Err(Stop::Disturbed);
            }
            let ids: Vec<MemoryId> = group.iter().map(|r| r.id).collect();
            let names = ids
                .iter()
                .map(|i| format!("m{}", i.0))
                .collect::<Vec<_>>()
                .join(" + ");
            let Some(answer) = merger.merge(&group) else {
                self.report.review(format!(
                    "{names}: el modelo no respondió; la unión quedó para otra noche"
                ));
                break;
            };
            let checked = validate_merge(&group, &answer, &|a, b| self.similarity(a, b))
                .map_err(|r| format!("{r:?}"))
                .and_then(|m| {
                    match group
                        .iter()
                        .find(|o| merger.preserved(o, &m.text) != Some(true))
                    {
                        Some(o) => Err(format!("se pierde algo de m{}", o.id.0)),
                        None => Ok(m),
                    }
                });
            match checked {
                Ok(m) => match self
                    .writer()?
                    .sleep_merge(&ids, &m.text, m.title.as_deref())
                {
                    Ok((new, _)) => self
                        .report
                        .merged
                        .push((new.0, ids.iter().map(|i| i.0).collect())),
                    Err(e) => self.report.review(format!("{names}: no se unieron ({e})")),
                },
                Err(why) => {
                    failed += 1;
                    self.report
                        .review(format!("{names}: unión rechazada ({why})"));
                    if tried >= 2 && failed * 2 > tried {
                        self.report.alarm(format!(
                            "la unión se detuvo: {failed} de {tried} grupos no pasaron la validación"
                        ));
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    fn conflicts(&self, n: &MemoryRow, group: &[MemoryRow]) -> Result<bool, Stop> {
        Ok(self.read.relations(n.id)?.iter().any(|r| {
            r.relation == Relation::ConflictsWith
                && group.iter().any(|g| g.id == r.source || g.id == r.target)
        }))
    }

    fn similarity(&self, a: &str, b: &str) -> Option<f32> {
        let v = self.embedder.as_ref()?.embed(&[a, b]).ok()?;
        let (x, y) = (v.first()?, v.get(1)?);
        let dot: f32 = x.iter().zip(y).map(|(p, q)| p * q).sum();
        let norm = |v: &[f32]| v.iter().map(|p| p * p).sum::<f32>().sqrt();
        let n = norm(x) * norm(y);
        (n > 0.0).then(|| dot / n)
    }

    /// Two active memories one contradicts: for review, never touched.
    fn contradictions(
        &mut self,
        fresh: &[MemoryRow],
        gone: &HashSet<MemoryId>,
    ) -> Result<(), Stop> {
        for r in fresh {
            if gone.contains(&r.id) {
                continue;
            }
            for rel in self.read.relations(r.id)? {
                if rel.relation != Relation::ConflictsWith {
                    continue;
                }
                let other = if rel.source == r.id {
                    rel.target
                } else {
                    rel.source
                };
                if gone.contains(&other)
                    || self
                        .read
                        .get(other)
                        .map_or(true, |o| o.state != State::Active)
                {
                    continue;
                }
                let (a, b) = (r.id.0.min(other.0), r.id.0.max(other.0));
                self.report.review(format!("m{a} contradice a m{b}"));
            }
        }
        Ok(())
    }

    fn purge(&mut self) -> Result<(), Stop> {
        let mut waiting = Vec::new();
        for scope in self.read.scopes()? {
            waiting.extend(self.read.list(scope.id, Some(State::Quarantine), None)?);
        }
        for r in waiting {
            if self.report.purged.len() >= MAX_PURGES {
                break;
            }
            if self.read.sleep_purgeable(r.id).is_err() {
                continue;
            }
            if self.dry_run
                || self
                    .writer()?
                    .sleep_purge(r.id, &format!("{SLEEP_REASON} purga"))
                    .is_ok()
            {
                self.report.purged.push(r.id.0);
            }
        }
        Ok(())
    }
}

/// Outside text and the agent's own are never mixed.
fn trust(r: &MemoryRow) -> bool {
    r.origin == Origin::External
}

/// The line the agent reads first the morning after: what the sleep did and what it left for
/// review. `None` when the night did nothing or ended more than 24 hours ago.
pub fn morning_line(r: &Report, now: i64) -> Option<String> {
    if now - r.ended > 24 * 3600 {
        return None;
    }
    let mut parts = Vec::new();
    if !r.quarantined.is_empty() {
        let which: Vec<String> = r
            .quarantined
            .iter()
            .map(|(id, why)| {
                let why = why.trim_start_matches(crate::SLEEP_REASON).trim();
                let why = why.split(" (").next().unwrap_or(why);
                format!("m{id} {why}")
            })
            .collect();
        parts.push(format!(
            "{} a cuarentena ({})",
            r.quarantined.len(),
            which.join(", ")
        ));
    }
    for (new, sources) in &r.merged {
        parts.push(format!("{} unidas en m{new}", sources.len()));
    }
    match r.purged.len() {
        0 => {}
        1 => parts.push("1 purgada".into()),
        n => parts.push(format!("{n} purgadas")),
    }
    let mut line = parts.join(", ");
    if !r.review.is_empty() {
        line.push_str(&format!(" · para revisar: {}", r.review.join("; ")));
    }
    if !r.alarms.is_empty() {
        line.push_str(&format!(" · alarmas: {}", r.alarms.join("; ")));
    }
    let line = line.trim_start_matches(" · ").to_owned();
    (!line.is_empty()).then(|| format!("Anoche (sueño): {line}"))
}
