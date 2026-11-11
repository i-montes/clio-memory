//! The agent's own memory: the places of the file, the actors, and what an agent may do with it.
//!
//! The file is `<data>/memory.db` (0600, folder 0700), the same one Clio uses: scope `clio` is the
//! person's place, `lecciones` the lessons, `proyecto:<name>` each project. The memory is global: a
//! memory reads every place, its own project first, each row marked with its place. Actors:
//! `persona` (the person), `agente` (Clio's agent), `sesion`, `importador`, and one per client of
//! the MCP server (`mcp:<client>`), each with its own quota; an agent writes 500 times an hour.
//!
//! What an agent does ([`Memory::save`], [`Memory::update`], [`Memory::forget`], [`Memory::pin`],
//! [`Memory::revert_own`]) is active at once and signed by its own actor; forgetting sends to
//! quarantine, where the record stays. Every change is in the operation log. The person's own
//! powers (promote, archive, purge, revert of anyone's change) are not here: they are the
//! person's command line. The embedding model is optional: without it, search is by text.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::{
    ActorId, ActorInfo, ActorKind, Category, Engine, Error, MemoryId, MemoryRow, MemoryType, OpId,
    OpKind, OpenMode, Origin, Outcome, Proposal, Quota, RecallOpts, Relation, ScopeId, SearchOpts,
    State, SystemClock,
};

/// The similarity from which a new memory is called nearly the same as an old one, per model:
/// measured on the relations benchmark, so its three contradictions are above it (the full model
/// puts them at 0.880 to 0.937, the quantized one at 0.863 to 0.939; the pairs above are memories
/// worth seeing together, 10 and 12 of them). `None`: not measured, no warning.
pub fn near_duplicate_floor(model: &str) -> Option<f32> {
    match model {
        crate::GRANITE_ID => Some(0.87),
        crate::GRANITE_Q8_ID => Some(0.86),
        _ => None,
    }
}

/// How long a neighbor's preview is.
const PREVIEW_CHARS: usize = 60;

/// A memory in a few words, to name it beside another: its title, else the start of its text,
/// cleaned like everything returned and on one line.
pub fn preview(row: &MemoryRow) -> String {
    let text = clean_text(row.title.as_deref().unwrap_or(&row.content));
    if text.chars().count() <= PREVIEW_CHARS {
        return text;
    }
    let mut out: String = text.chars().take(PREVIEW_CHARS - 1).collect();
    out.push('…');
    out
}

/// How many relation notes a memory's line carries.
const MAX_NOTES: usize = 4;

pub const SCOPE: &str = "clio";
pub const DB_FILE: &str = "memory.db";
/// Where the local embedding model lives, relative to HOME.
pub const MODEL_REL: &str = ".cache/clio-models/potion-multilingual-128M";
const BLOCKS: [(&str, usize); 3] = [("identidad", 150), ("reglas", 200), ("preferencias", 150)];

pub type Res<T> = Result<T, String>;

/// A refusal in the words of a person.
pub fn explain(e: &Error) -> String {
    match e {
        Error::SecretDetected => "rechazado: parece una clave o un secreto".into(),
        Error::QuotaExceeded => "rechazado: cuota de propuestas agotada para este actor".into(),
        Error::TooLong => "rechazado: demasiado largo".into(),
        Error::QuarantineFull => {
            "rechazado: hay demasiados recuerdos de este actor en cuarentena (`clio-memory review SCOPE`)"
                .into()
        }
        Error::EmptyContent => "rechazado: vacío".into(),
        Error::WriterLocked => "otro proceso está escribiendo la memoria; reintenta".into(),
        Error::NotAllowed => "no permitido para este actor".into(),
        Error::InvalidState => "el recuerdo no está en un estado que lo permita".into(),
        Error::NotFound => "no existe ese recuerdo".into(),
        other => other.to_string(),
    }
}

/// Control characters, bidi marks and zero-width characters become spaces: a memory is one
/// short line of plain text.
pub fn clean_text(s: &str) -> String {
    let swapped: String = s
        .chars()
        .map(|c| {
            if c.is_control()
                || matches!(
                    c,
                    '\u{200b}'..='\u{200f}'
                        | '\u{202a}'..='\u{202e}'
                        | '\u{2066}'..='\u{2069}'
                        | '\u{feff}'
                )
            {
                ' '
            } else {
                c
            }
        })
        .collect();
    swapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let head: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

#[derive(Debug, Clone, Copy)]
pub struct Actors {
    pub persona: ActorId,
    pub agente: ActorId,
    pub sesion: ActorId,
    pub importador: ActorId,
    /// Who signs this memory's writes: `agente` (Clio's agent) or `mcp:<client>`.
    pub own: ActorId,
}

/// The folder of the shared memory file: `$CLIO_MEMORY_DATA`, else `~/.local/share/clio` (Clio's);
/// `None` with neither (never a path relative to wherever the program was started).
pub fn default_data() -> Option<PathBuf> {
    let set = |k| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    set("CLIO_MEMORY_DATA")
        .or_else(|| set("HOME").map(|h| h.join(".local/share/clio")))
        .filter(|p| p.is_absolute())
}

/// The scope of the lessons: what was learned in one project and is worth knowing in any.
pub const LESSONS: &str = "lecciones";
/// A project's scope is this and the project's name.
pub const PROJECT_PREFIX: &str = "proyecto:";
/// The longest text of an agent's memory (`Qué: … · Por qué: … · Dónde: … · Aprendido: …`).
pub const AGENT_TEXT_CHARS: usize = 1200;
/// The longest title.
pub const TITLE_CHARS: usize = 120;

/// Where a memory lives (spec `2026-11-04-memory-engram-design.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// This project's: decisions, architecture, settings.
    Project,
    /// The person's, in every project: who they are, what they prefer.
    Personal,
    /// Learned somewhere, worth knowing anywhere: errors, root causes, patterns, traps.
    Lessons,
}

impl Place {
    pub fn label(self) -> &'static str {
        match self {
            Place::Project => "proyecto",
            Place::Personal => "personal",
            Place::Lessons => "leccion",
        }
    }

    pub fn parse(s: &str) -> Option<Place> {
        [Place::Project, Place::Personal, Place::Lessons]
            .into_iter()
            .find(|p| p.label() == s.trim())
    }

    /// Where a memory goes when nobody (Jev) says: who the person is and what they prefer are the
    /// person's, an error or a
    /// pattern is a lesson, the rest is the project's (or the person's when there is no project).
    pub fn by_rule(category: Option<Category>, has_project: bool) -> Place {
        match category {
            Some(Category::Profile | Category::Preference) => Place::Personal,
            Some(Category::Bugfix | Category::Pattern | Category::Learning) => Place::Lessons,
            _ if has_project => Place::Project,
            _ => Place::Personal,
        }
    }
}

/// What the agent proposes: the text, and optionally a title and a category.
#[derive(Debug, Clone)]
pub struct Draft {
    pub text: String,
    pub title: Option<String>,
    pub category: Option<Category>,
    pub kind: MemoryType,
}

impl Draft {
    pub fn plain(text: &str, kind: MemoryType) -> Self {
        Self {
            text: text.to_owned(),
            title: None,
            category: None,
            kind,
        }
    }
}

pub struct Memory {
    engine: Engine,
    /// The person's scope (`clio`).
    scope: ScopeId,
    /// The project of the session and its scope, when there is one (and, reading, when it exists).
    project: Option<(String, Option<ScopeId>)>,
    lessons: Option<ScopeId>,
    /// Every other project's scope and its name: the memory is global, a session reads them too.
    others: Vec<(ScopeId, String)>,
    actors: Actors,
    writable: bool,
    embeddings: bool,
    embed_error: Option<String>,
}

fn private_dir(dir: &Path) -> Res<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("{}: {e}", dir.display()))
}

/// The model in `dir`: Granite (ONNX files) or the static model (`model.safetensors`).
fn load_embedder(dir: &Path) -> Result<Arc<dyn crate::Embedder>, String> {
    let onnx = ["model.onnx", "model_quint8_avx2.onnx"]
        .iter()
        .any(|f| dir.join(f).is_file());
    if onnx {
        #[cfg(feature = "granite")]
        return crate::Granite::load(dir)
            .map(|e| Arc::new(e) as Arc<dyn crate::Embedder>)
            .map_err(|e| e.to_string());
        #[cfg(not(feature = "granite"))]
        return Err("clio-memory se compiló sin la característica granite".into());
    }
    #[cfg(feature = "model2vec")]
    return crate::Model2VecEmbedder::from_dir(dir)
        .map(|e| Arc::new(e) as Arc<dyn crate::Embedder>)
        .map_err(|e| e.to_string());
    #[cfg(not(feature = "model2vec"))]
    Err("clio-memory se compiló sin la característica model2vec".into())
}

/// The agent's writes an hour: a brake on a loop, not a review (the memory is the agent's).
const AGENT_QUOTA: u32 = 500;

fn quota(max: u32) -> Quota {
    Quota {
        max,
        window_secs: 3600,
    }
}

/// The embedding model: loaded at most once per process (reading it takes seconds, and a run
/// opens the memory twice), and only when something asks for it. Share it with an `Arc`. Several
/// folders are tried in order: the first that loads is the model.
pub struct EmbedModel {
    dirs: Vec<PathBuf>,
    cell: OnceLock<Result<Arc<dyn crate::Embedder>, String>>,
}

impl EmbedModel {
    /// A model in `dir`, or none (`None`): memory then works by text only.
    pub fn new(dir: Option<PathBuf>) -> Arc<Self> {
        Self::with_dirs(dir.into_iter().collect())
    }

    /// The first of `dirs` that loads.
    pub fn with_dirs(dirs: Vec<PathBuf>) -> Arc<Self> {
        Arc::new(Self {
            dirs,
            cell: OnceLock::new(),
        })
    }

    /// An embedder given by the caller (a host with its own model, a test).
    pub fn from_embedder(embedder: Arc<dyn crate::Embedder>) -> Arc<Self> {
        let model = Self {
            dirs: vec![PathBuf::from("<embedder>")],
            cell: OnceLock::new(),
        };
        let _ = model.cell.set(Ok(embedder));
        Arc::new(model)
    }

    /// No model, never loads anything.
    pub fn none() -> Arc<Self> {
        Self::new(None)
    }

    /// The models installed in `home`, best first: Granite, then the static model.
    pub fn of_home(home: &Path) -> Arc<Self> {
        Self::with_dirs(
            [crate::setup::GRANITE.dir, crate::setup::POTION.dir]
                .iter()
                .map(|d| home.join(d))
                .filter(|d| d.is_dir())
                .collect(),
        )
    }

    /// The folders it tries, in order.
    pub fn dirs(&self) -> &[PathBuf] {
        &self.dirs
    }

    pub fn is_present(&self) -> bool {
        !self.dirs.is_empty()
    }

    /// Whether `get` was called already (it loaded, or failed to).
    pub fn is_loaded(&self) -> bool {
        self.cell.get().is_some()
    }

    /// The model, loaded the first time: the first folder that loads; `None` with no folder; the
    /// reasons of every folder when none loads.
    pub fn get(&self) -> Result<Option<Arc<dyn crate::Embedder>>, String> {
        if self.dirs.is_empty() {
            return Ok(None);
        }
        self.cell
            .get_or_init(|| {
                let mut why = Vec::new();
                for dir in &self.dirs {
                    match load_embedder(dir) {
                        Ok(e) => return Ok(e),
                        Err(e) => why.push(format!("{}: {e}", dir.display())),
                    }
                }
                Err(why.join("; "))
            })
            .clone()
            .map(Some)
    }
}

/// Attaches the embedder when there is a model and it loads.
fn with_model(engine: Engine, model: &EmbedModel) -> (Engine, bool, Option<String>) {
    match model.get() {
        Ok(Some(emb)) => (engine.with_embedder(emb), true, None),
        Ok(None) => (engine, false, None),
        Err(e) => (engine, false, Some(e)),
    }
}

impl Memory {
    pub fn db_path(data: &Path) -> PathBuf {
        data.join(DB_FILE)
    }

    pub fn model_dir(home: &Path) -> PathBuf {
        home.join(MODEL_REL)
    }

    /// Opens (creating on first use) `<data>/memory.db` for writing, with the places, the actors
    /// and Clio's fixed blocks in place, in `project` (its scope is made if it does not exist yet),
    /// signing as `actor` (`agente`, or `mcp:<client>`: an agent's name, never another kind's).
    /// Waits a little for another writer (Clio, or another client).
    pub fn open_write(
        data: &Path,
        model: &EmbedModel,
        project: Option<&str>,
        actor: &str,
    ) -> Res<Self> {
        let path = Self::db_path(data);
        if !data.is_dir() {
            private_dir(data)?;
        }
        let mut engine = None;
        for _ in 0..8 {
            match Engine::open(&path, OpenMode::ReadWrite, Arc::new(SystemClock)) {
                Ok(e) => {
                    engine = Some(e);
                    break;
                }
                Err(Error::WriterLocked) => std::thread::sleep(Duration::from_millis(250)),
                Err(e) => return Err(format!("{}: {}", path.display(), explain(&e))),
            }
        }
        let engine = engine.ok_or_else(|| explain(&Error::WriterLocked))?;
        let (engine, embeddings, embed_error) = with_model(engine, model);
        let m = |e: Error| explain(&e);
        let scopes = engine.scopes().map_err(m)?;
        let find_or_make = |name: &str| -> Res<ScopeId> {
            match scopes.iter().find(|s| s.name == name) {
                Some(s) => Ok(s.id),
                None => engine.create_scope(name, "es").map_err(m),
            }
        };
        let scope = find_or_make(SCOPE)?;
        let lessons = Some(find_or_make(LESSONS)?);
        let project = match project {
            Some(p) => Some((
                p.to_owned(),
                Some(find_or_make(&format!("{PROJECT_PREFIX}{p}"))?),
            )),
            None => None,
        };
        let have = engine.actors().map_err(m)?;
        let get = |name: &str, kind: ActorKind, max: u32| -> Res<ActorId> {
            match have.iter().find(|a: &&ActorInfo| a.name == name) {
                Some(a) => Ok(a.id),
                None => engine.register_actor(name, kind, quota(max)).map_err(m),
            }
        };
        // Only an agent's name signs as an agent: the file's own names (and the judge's) are
        // refused before anything is registered, so a refusal never leaves a wrong actor behind.
        if ["persona", "sesion", "importador", "jev"].contains(&actor)
            || have
                .iter()
                .any(|a| a.name == actor && a.kind != ActorKind::Agent)
        {
            return Err(format!("{actor}: ese nombre no es de un agente"));
        }
        let agente = get("agente", ActorKind::Agent, AGENT_QUOTA)?;
        let own = if actor == "agente" {
            agente
        } else {
            get(actor, ActorKind::Agent, AGENT_QUOTA)?
        };
        let actors = Actors {
            persona: get("persona", ActorKind::Person, 200)?,
            agente,
            sesion: get("sesion", ActorKind::Session, 30)?,
            importador: get("importador", ActorKind::Importer, 100)?,
            own,
        };
        // A file made when the agent had 20 an hour gets the quota of now.
        for a in [agente, own] {
            engine.set_quota(a, quota(AGENT_QUOTA)).map_err(m)?;
        }
        let blocks = engine.blocks(scope).map_err(m)?;
        for (name, budget) in BLOCKS {
            if !blocks.iter().any(|b| b.name == name) {
                engine
                    .create_block(scope, name, budget, actors.persona)
                    .map_err(m)?;
            }
        }
        Ok(Self {
            engine,
            scope,
            project,
            lessons,
            others: Vec::new(),
            actors,
            writable: true,
            embeddings,
            embed_error,
        }
        .with_others())
    }

    /// Opens for reading when the file exists and was set up; `None` otherwise. Never creates or
    /// changes anything, and does not wait for the writer. In `project` (whose scope may not exist
    /// yet: then nothing of it is found), as `actor` (`agente` when it never wrote).
    pub fn open_read(
        data: &Path,
        model: &EmbedModel,
        project: Option<&str>,
        actor: &str,
    ) -> Res<Option<Self>> {
        let path = Self::db_path(data);
        if !path.is_file() {
            return Ok(None);
        }
        let open = || Engine::open(&path, OpenMode::ReadOnly, Arc::new(SystemClock));
        let engine = match open() {
            // A file of an older schema: a reader never changes it, so it is opened once for
            // writing (which upgrades it, in one transaction) and then read. If another writer
            // holds it, that writer has already upgraded it or will: the read is tried again.
            Err(Error::SchemaTooOld(_)) => {
                drop(Self::open_write(data, &EmbedModel::none(), None, "agente")?);
                open()
            }
            other => other,
        }
        .map_err(|e| format!("{}: {}", path.display(), explain(&e)))?;
        let (engine, embeddings, embed_error) = with_model(engine, model);
        let m = |e: Error| explain(&e);
        let scopes = engine.scopes().map_err(m)?;
        let find = |name: &str| scopes.iter().find(|s| s.name == name).map(|s| s.id);
        let Some(scope) = find(SCOPE) else {
            return Ok(None);
        };
        let lessons = find(LESSONS);
        let project = project.map(|p| (p.to_owned(), find(&format!("{PROJECT_PREFIX}{p}"))));
        let have = engine.actors().map_err(m)?;
        let id = |name: &str| have.iter().find(|a| a.name == name).map(|a| a.id);
        let (Some(persona), Some(agente), Some(sesion), Some(importador)) =
            (id("persona"), id("agente"), id("sesion"), id("importador"))
        else {
            return Ok(None);
        };
        Ok(Some(
            Self {
                engine,
                scope,
                project,
                lessons,
                others: Vec::new(),
                actors: Actors {
                    persona,
                    agente,
                    sesion,
                    importador,
                    own: id(actor).unwrap_or(agente),
                },
                writable: false,
                embeddings,
                embed_error,
            }
            .with_others(),
        ))
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn scope(&self) -> ScopeId {
        self.scope
    }

    pub fn actors(&self) -> Actors {
        self.actors
    }

    pub fn has_embeddings(&self) -> bool {
        self.embeddings
    }

    /// A discreet line about the embedding model, `None` when it is in use.
    pub fn note(&self) -> Option<String> {
        if self.embeddings {
            return None;
        }
        Some(if self.embed_error.is_some() {
            "sin modelo de embeddings (no se pudo cargar): la memoria busca solo por texto".into()
        } else {
            "sin modelo de embeddings: la memoria busca solo por texto".into()
        })
    }

    fn need_write(&self) -> Res<()> {
        if self.writable {
            Ok(())
        } else {
            Err("la memoria está abierta solo para leer".into())
        }
    }

    /// What the agent saves during a session (the `memory_save` tool): active at once, the agent's,
    /// with origin `externo` when the session read outside content (`tainted`) and `agente`
    /// otherwise. There is no argument for the actor or the scope beyond the place: they are not
    /// the caller's to say (S-61). The engine's secret check and the agent's quota still apply. A
    /// place this link has no scope for (a project when there is none) is the person's.
    pub fn save(&self, draft: &Draft, session: &str, place: Place, tainted: bool) -> Res<Outcome> {
        self.need_write()?;
        self.engine
            .propose(self.agent_proposal(draft, session, place, tainted))
            .map_err(|e| explain(&e))
    }

    /// The same, in the scope of `beside` (another project's too): what a merge of that place's
    /// memories saves.
    pub fn save_beside(
        &self,
        draft: &Draft,
        session: &str,
        beside: &MemoryRow,
        tainted: bool,
    ) -> Res<Outcome> {
        self.need_write()?;
        let place = self
            .place_of(beside.scope)
            .ok_or("esa memoria no es de este lugar")?;
        let mut p = self.agent_proposal(draft, session, place, tainted);
        p.scope = beside.scope;
        self.engine.propose(p).map_err(|e| explain(&e))
    }

    /// The agent corrects `old` (the `memory_update` tool): the new text replaces exactly that
    /// memory, whoever wrote it, in its place and of its kind. The old one stays in the log as
    /// replaced and the person's revert brings it back.
    pub fn update(
        &self,
        old: MemoryId,
        draft: &Draft,
        session: &str,
        tainted: bool,
    ) -> Res<Outcome> {
        self.need_write()?;
        let row = self.get(old)?;
        let place = self
            .place_of(row.scope)
            .ok_or("esa memoria no es de este lugar")?;
        let mut p = self.agent_proposal(draft, session, place, tainted);
        // In the old one's own scope (another project's, too) and of its kind.
        p.scope = row.scope;
        p.kind = row.kind;
        let o = self.engine.replace(old, p).map_err(|e| explain(&e))?;
        if o.kind == OpKind::Update {
            // Best effort: the correction is made; a relation that does not follow is a note lost.
            let _ = self.engine.copy_relations(old, o.memory);
        }
        Ok(o)
    }

    /// Copies the judged relations of `from` to `to` (a merge's originals to the merge).
    pub fn copy_relations(&self, from: MemoryId, to: MemoryId) -> Res<usize> {
        self.need_write()?;
        self.engine
            .copy_relations(from, to)
            .map_err(|e| explain(&e))
    }

    /// The memory this link may read that is nearest to `id`, when its similarity is at least
    /// `floor`. Only by meaning: a memory without a vector has none.
    pub fn near_duplicate_at(&self, id: MemoryId, floor: f32) -> Option<MemoryRow> {
        if !self.engine.has_vector(id).unwrap_or(false) {
            return None;
        }
        self.engine
            .neighbors(id, &self.reading_scopes(), 3)
            .ok()?
            .into_iter()
            .find(|(r, _)| self.get(r.id).is_ok())
            .filter(|(_, s)| *s >= floor)
            .map(|(r, _)| r)
    }

    /// `near_duplicate_at` with the floor of the model in use; `None` without one.
    pub fn near_duplicate(&self, id: MemoryId) -> Option<MemoryRow> {
        let floor = near_duplicate_floor(self.engine.model_id()?)?;
        self.near_duplicate_at(id, floor)
    }

    /// The memories nearest to `row` that this link may read, best first (at most `k`): what the
    /// model can walk to from it. Empty when they cannot be found.
    pub fn neighbors(&self, row: &MemoryRow, k: usize) -> Vec<MemoryRow> {
        self.engine
            .neighbors(row.id, &self.reading_scopes(), k * 2)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(r, _)| self.get(r.id).ok())
            .take(k)
            .collect()
    }

    /// The agent pins `id` (the `memory_pin` tool): it comes first in the context.
    pub fn pin(&self, id: MemoryId) -> Res<()> {
        self.need_write()?;
        self.get(id)?; // only a memory of this link's places
        self.engine
            .pin(id, self.actors.own)
            .map_err(|e| explain(&e))
    }

    /// The agent undoes `op` (the `memory_revert` tool): only a change it signed itself, in a
    /// place of this link.
    pub fn revert_own(&self, op: OpId) -> Res<OpId> {
        self.need_write()?;
        let done = self.engine.operation(op).map_err(|e| explain(&e))?;
        if done.actor != self.actors.own || self.place_of(done.scope).is_none() {
            return Err("ese cambio no lo hizo el agente: no se puede deshacer por aquí".into());
        }
        self.engine
            .revert(op, self.actors.own)
            .map_err(|e| explain(&e))
    }

    /// The agent forgets `id` (the `memory_forget` tool): to quarantine, out of every search, its
    /// record kept; the person's revert brings it back.
    pub fn forget(&self, id: MemoryId) -> Res<OpId> {
        self.need_write()?;
        self.get(id)?; // only a memory of this link's places
        self.engine
            .forget(id, self.actors.own, "olvidado por el agente")
            .map_err(|e| explain(&e))
    }

    /// The session of an agent's proposal as it is stored.
    pub fn agent_session(session: &str) -> String {
        clip(&clean_text(session), 200)
    }

    /// The text of an agent's proposal as it is stored: clean, one line.
    pub fn agent_text(text: &str) -> String {
        clip(&clean_text(text), AGENT_TEXT_CHARS)
    }

    fn agent_proposal(
        &self,
        draft: &Draft,
        session: &str,
        place: Place,
        tainted: bool,
    ) -> Proposal {
        let session = Self::agent_session(session);
        // Where it was learned: the project, when there is one (what a lesson shows elsewhere).
        let source = match self.project_name() {
            Some(p) => format!("{PROJECT_PREFIX}{p}"),
            None => session.clone(),
        };
        Proposal {
            scope: self.scope_of(place).unwrap_or(self.scope),
            actor: self.actors.own,
            kind: draft.kind,
            content: Self::agent_text(&draft.text),
            topic_key: None,
            origin: if tainted {
                Origin::External
            } else {
                Origin::Agent
            },
            session: Some(session.clone()),
            // The session is the episode: the timeline groups by it.
            episode: Some(session),
            source: Some(source),
            valid_from: None,
            valid_to: None,
            title: draft
                .title
                .as_deref()
                .map(|t| clip(&clean_text(t), TITLE_CHARS))
                .filter(|t| !t.is_empty()),
            category: draft.category,
        }
    }

    /// What the relations of `row` say, for the model and the person: only those with a memory
    /// this link can read and that is active (a note pointing at nothing readable says nothing),
    /// the newest four.
    pub fn relation_notes(&self, row: &MemoryRow) -> Vec<String> {
        let Ok(rels) = self.engine.relations(row.id) else {
            return Vec::new();
        };
        rels.into_iter()
            .filter_map(|r| {
                let mine = r.source == row.id;
                let other = if mine { r.target } else { r.source };
                let readable = self.get(other).is_ok_and(|o| o.state == State::Active);
                if !readable {
                    return None;
                }
                let n = other.0;
                Some(match (r.relation, mine) {
                    (Relation::ConflictsWith, _) => format!("contradice a m{n}"),
                    (Relation::Compatible, _) => format!("compatible con m{n}"),
                    (Relation::Related, _) => format!("relacionada con m{n}"),
                    (Relation::Scoped, true) => format!("caso concreto de m{n}"),
                    (Relation::Scoped, false) => format!("m{n} es un caso concreto"),
                    (Relation::NotConflict, _) => return None,
                })
            })
            .take(MAX_NOTES)
            .collect()
    }

    // ---------------------------------------------------------------- reading

    /// An active memory of any place of the file. Missing, forgotten, archived, replaced or of a
    /// scope that is not a place: the same answer, so which one it is does not leak.
    pub fn get(&self, id: MemoryId) -> Res<MemoryRow> {
        match self.engine.get(id) {
            Ok(row) if row.state == State::Active && self.place_of(row.scope).is_some() => Ok(row),
            _ => Err("no existe ese recuerdo".into()),
        }
    }

    /// Every other project, read after the session's own places (the memory is global; the
    /// project is a mark).
    fn with_others(mut self) -> Self {
        let own = self.scope_of(Place::Project);
        self.others = self
            .engine
            .scopes()
            .unwrap_or_default()
            .into_iter()
            .filter(|s| Some(s.id) != own)
            .filter_map(|s| {
                let name = s.name.strip_prefix(PROJECT_PREFIX)?.to_owned();
                Some((s.id, name))
            })
            .collect();
        self
    }

    /// Is this active memory past its review cycle? (Only its place is looked at.)
    pub fn is_due(&self, row: &MemoryRow) -> bool {
        self.engine.due(row).unwrap_or(false)
    }

    /// Is it pinned (first in its place's context)?
    pub fn is_pinned(&self, row: &MemoryRow) -> bool {
        self.engine
            .pinned(row.scope)
            .is_ok_and(|v| v.iter().any(|r| r.id == row.id))
    }

    /// The memories around `id` in its session (or its place), as this view can read them.
    pub fn timeline(&self, id: MemoryId, before: usize, after: usize) -> Res<Vec<MemoryRow>> {
        if self.get(id)?.state != State::Active {
            return Err("solo los recuerdos activos tienen línea de tiempo".into());
        }
        Ok(self
            .engine
            .timeline(id, before, after)
            .map_err(|e| explain(&e))?
            .into_iter()
            .filter(|r| self.place_of(r.scope).is_some())
            .collect())
    }

    /// The project of this link, if it has one.
    pub fn project_name(&self) -> Option<&str> {
        self.project.as_ref().map(|(p, _)| p.as_str())
    }

    /// Which of this link's places a scope is (`None`: another project's, or unknown).
    pub fn place_of(&self, scope: ScopeId) -> Option<Place> {
        if scope == self.scope {
            Some(Place::Personal)
        } else if Some(scope) == self.lessons {
            Some(Place::Lessons)
        } else if self.project.as_ref().and_then(|(_, s)| *s) == Some(scope)
            || self.others.iter().any(|(id, _)| *id == scope)
        {
            Some(Place::Project)
        } else {
            None
        }
    }

    /// The scope of a place, if it exists.
    pub fn scope_of(&self, place: Place) -> Option<ScopeId> {
        match place {
            Place::Personal => Some(self.scope),
            Place::Lessons => self.lessons,
            Place::Project => self.project.as_ref().and_then(|(_, s)| *s),
        }
    }

    /// What a session reads: its project first, then the person's, the lessons and every other
    /// project (the memory is global).
    pub fn reading_scopes(&self) -> Vec<ScopeId> {
        [Place::Project, Place::Personal, Place::Lessons]
            .into_iter()
            .filter_map(|p| self.scope_of(p))
            .chain(self.others.iter().map(|(id, _)| *id))
            .collect()
    }

    /// `search` over every place the session reads.
    pub fn search_all(&self, query: &str, limit: usize) -> Res<(Vec<MemoryRow>, bool)> {
        let opts = RecallOpts {
            search: SearchOpts {
                limit,
                ..SearchOpts::default()
            },
            ..RecallOpts::default()
        };
        let r = self
            .engine
            .recall_many(&self.reading_scopes(), &clean_text(query), &opts)
            .map_err(|e| explain(&e))?;
        Ok((r.hits.into_iter().map(|h| h.memory).collect(), r.abstained))
    }

    /// `search` in one place (where a proposal would go: its neighbors).
    pub fn search_in(&self, place: Place, query: &str, limit: usize) -> Res<Vec<MemoryRow>> {
        let Some(scope) = self.scope_of(place) else {
            return Ok(Vec::new());
        };
        let opts = RecallOpts {
            search: SearchOpts {
                limit,
                ..SearchOpts::default()
            },
            ..RecallOpts::default()
        };
        self.engine
            .recall(scope, &clean_text(query), &opts)
            .map(|r| r.hits.into_iter().map(|h| h.memory).collect())
            .map_err(|e| explain(&e))
    }

    /// A label for where a memory lives, for the model: `proyecto`, `personal`, `lección de X`.
    pub fn place_label(&self, row: &MemoryRow) -> String {
        match self.place_of(row.scope) {
            Some(Place::Project) => match self.others.iter().find(|(id, _)| *id == row.scope) {
                Some((_, name)) => format!("proyecto {name}"),
                None => "proyecto".into(),
            },
            Some(Place::Personal) => "personal".into(),
            Some(Place::Lessons) => match Self::learned_in(row) {
                Some(p) => format!("lección de {p}"),
                None => "lección".into(),
            },
            None => String::new(),
        }
    }

    /// The project a lesson was learned in (its `source`), if it says.
    pub fn learned_in(row: &MemoryRow) -> Option<&str> {
        row.source.as_deref()?.strip_prefix(PROJECT_PREFIX)
    }

    /// What a model needs on starting or after a compaction: the last session's summary of the
    /// project (or the person's place), what is pinned in any place, the most recent of the project,
    /// and a few past their review cycle.
    pub fn context(&self) -> Res<Vec<MemoryRow>> {
        let fail = |_| "la memoria no se pudo leer".to_owned();
        let home = self.scope_of(Place::Project).unwrap_or(self.scope);
        let recent = self.engine.recent(home, 30).map_err(fail)?;
        let mut out: Vec<MemoryRow> = Vec::new();
        let push = |r: MemoryRow, out: &mut Vec<MemoryRow>| {
            if !out.iter().any(|o| o.id == r.id) {
                out.push(r);
            }
        };
        if let Some(summary) = recent.iter().find(|r| r.kind == MemoryType::Episodic) {
            push(summary.clone(), &mut out);
        }
        for scope in self.reading_scopes() {
            for r in self.engine.pinned(scope).unwrap_or_default() {
                push(r, &mut out);
            }
        }
        for r in recent.into_iter().take(8) {
            push(r, &mut out);
        }
        let mut due = Vec::new();
        for scope in self.reading_scopes() {
            due.extend(self.engine.review_due(scope).unwrap_or_default());
        }
        for r in due.into_iter().take(3) {
            push(r, &mut out);
        }
        Ok(out)
    }

    /// Computes the vectors of everything waiting for one, the whole queue and not only what this
    /// memory wrote: the queued job, then any memory still without a vector (one written by a
    /// program with no model). How many vectors were stored; 0 with no model.
    pub fn embed_queue(&self) -> Res<usize> {
        if !self.embeddings {
            return Ok(0);
        }
        self.need_write()?;
        let m = |e: Error| explain(&e);
        let before = self.engine.stats().map_err(m)?.vectors;
        self.engine
            .run_jobs(&crate::RunOpts {
                max_jobs: 8,
                allow_heavy: false,
                only: Some(crate::JobKind::Embed),
            })
            .map_err(m)?;
        while self.engine.embed_pending(256).map_err(m)? > 0 {}
        let after = self.engine.stats().map_err(m)?.vectors;
        Ok(usize::try_from(after.saturating_sub(before)).unwrap_or(usize::MAX))
    }

    /// What was forgotten (quarantine) in every place this memory reads.
    pub fn forgotten(&self) -> Res<Vec<MemoryRow>> {
        let mut out = Vec::new();
        for scope in self.reading_scopes() {
            out.extend(
                self.engine
                    .list(scope, Some(State::Quarantine), None)
                    .map_err(|e| explain(&e))?,
            );
        }
        Ok(out)
    }
}

/// The Spanish word for a kind.
pub fn kind_label(k: MemoryType) -> &'static str {
    match k {
        MemoryType::Working => "trabajo",
        MemoryType::Episodic => "episódico",
        MemoryType::Semantic => "semántico",
        MemoryType::Procedural => "procedimental",
    }
}

pub fn state_label(s: State) -> &'static str {
    match s {
        State::Active => "activo",
        State::Quarantine => "cuarentena",
        State::Archived => "archivado",
        State::Replaced => "reemplazado",
    }
}

pub fn origin_label(o: Origin) -> &'static str {
    match o {
        Origin::User => "persona",
        Origin::Agent => "agente",
        Origin::External => "externo",
    }
}
