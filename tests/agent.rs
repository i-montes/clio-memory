#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! The agent layer on the file Clio shares: the same places and actors, every write active at once
//! and signed by the agent's own actor, a global read, forgetting to quarantine.

use std::path::Path;
use std::sync::Arc;

use clio_memory::agent::{Draft, EmbedModel, Memory, Place};
use clio_memory::{
    ActorKind, Engine, MemoryId, MemoryType, OpenMode, Origin, Quota, State, SystemClock,
};

fn write(data: &Path, project: Option<&str>, actor: &str) -> Memory {
    Memory::open_write(data, &EmbedModel::none(), project, actor).unwrap()
}

fn plain(text: &str) -> Draft {
    Draft::plain(text, MemoryType::Semantic)
}

#[test]
fn a_file_made_by_clio_opens_and_keeps_its_names() {
    let t = tempfile::tempdir().unwrap();
    let path = Memory::db_path(t.path());
    {
        // What Clio leaves in a new file: its scopes and actors.
        let e = Engine::open(&path, OpenMode::ReadWrite, Arc::new(SystemClock)).unwrap();
        e.create_scope("clio", "es").unwrap();
        e.create_scope("lecciones", "es").unwrap();
        let q = Quota {
            max: 20,
            window_secs: 3600,
        };
        e.register_actor("persona", ActorKind::Person, q).unwrap();
        e.register_actor("agente", ActorKind::Agent, q).unwrap();
    }
    let m = write(t.path(), None, "mcp:claude-code");
    let names: Vec<String> = m
        .engine()
        .scopes()
        .unwrap()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(
        names.iter().filter(|n| *n == "clio").count(),
        1,
        "{names:?}"
    );
    let actors = m.engine().actors().unwrap();
    assert_eq!(actors.iter().filter(|a| a.name == "agente").count(), 1);
    assert!(
        actors
            .iter()
            .any(|a| a.name == "mcp:claude-code" && a.kind == ActorKind::Agent)
    );
}

#[test]
fn a_save_is_active_and_a_session_reads_every_project() {
    let t = tempfile::tempdir().unwrap();
    let alfa = write(t.path(), Some("alfa"), "mcp:claude-code");
    let o = alfa
        .save(&plain("usa Postgres 16"), "s1", Place::Project, false)
        .unwrap();
    let row = alfa.get(o.memory).unwrap();
    assert_eq!((row.state, row.origin), (State::Active, Origin::Agent));
    assert_eq!(row.actor, alfa.actors().own);
    drop(alfa);
    let beta = Memory::open_read(t.path(), &EmbedModel::none(), Some("beta"), "agente")
        .unwrap()
        .unwrap();
    let (found, _) = beta.search_all("Postgres", 5).unwrap();
    assert_eq!(found[0].id, o.memory);
    assert_eq!(beta.place_label(&found[0]), "proyecto alfa");
}

#[test]
fn update_replaces_in_the_old_ones_project() {
    let t = tempfile::tempdir().unwrap();
    let old = write(t.path(), Some("alfa"), "agente")
        .save(&plain("usa Redis"), "s1", Place::Project, false)
        .unwrap()
        .memory;
    let beta = write(t.path(), Some("beta"), "mcp:claude-code");
    let o = beta.update(old, &plain("usa Valkey"), "s2", false).unwrap();
    assert_eq!(beta.get(old).unwrap_err(), "no existe ese recuerdo");
    assert_eq!(beta.engine().get(old).unwrap().state, State::Replaced);
    let new = beta.get(o.memory).unwrap();
    assert_eq!(beta.place_label(&new), "proyecto alfa");
}

#[test]
fn forget_then_revert_own_and_never_anothers_change() {
    let t = tempfile::tempdir().unwrap();
    let m = write(t.path(), None, "mcp:claude-code");
    let id = m
        .save(
            &plain("el servidor usa compose"),
            "s",
            Place::Personal,
            true,
        )
        .unwrap()
        .memory;
    let op = m.forget(id).unwrap();
    assert!(m.search_all("servidor compose", 5).unwrap().0.is_empty());
    assert_eq!(m.forgotten().unwrap()[0].id, id);
    m.revert_own(op).unwrap();
    assert_eq!(m.get(id).unwrap().state, State::Active);
    // Clio's agent's change is not this client's to undo.
    drop(m);
    let clio = write(t.path(), None, "agente");
    let theirs = clio
        .save(&plain("dato de Clio"), "s", Place::Personal, false)
        .unwrap();
    drop(clio);
    let m = write(t.path(), None, "mcp:claude-code");
    assert!(m.revert_own(theirs.op).is_err());
    assert_eq!(m.get(theirs.memory).unwrap().state, State::Active);
}

#[test]
fn each_client_is_its_own_actor_at_500_an_hour() {
    let t = tempfile::tempdir().unwrap();
    let m = write(t.path(), None, "mcp:claude-code");
    for i in 0..500 {
        m.save(&plain(&format!("dato {i}")), "s", Place::Personal, false)
            .unwrap_or_else(|e| panic!("{i}: {e}"));
    }
    let e = m
        .save(&plain("uno de más"), "s", Place::Personal, false)
        .unwrap_err();
    assert!(e.contains("cuota"), "{e}");
    // Another client has its own hour.
    drop(m);
    write(t.path(), None, "mcp:cursor")
        .save(&plain("de otro cliente"), "s", Place::Personal, false)
        .unwrap();
}

#[test]
fn a_name_that_is_not_an_agents_does_not_sign() {
    let t = tempfile::tempdir().unwrap();
    drop(write(t.path(), None, "agente"));
    for name in ["persona", "sesion", "importador"] {
        let e = Memory::open_write(t.path(), &EmbedModel::none(), None, name)
            .err()
            .unwrap();
        assert!(e.contains("no es de un agente"), "{name}: {e}");
    }
}

#[test]
fn the_context_starts_with_the_projects_summary_then_the_pins() {
    let t = tempfile::tempdir().unwrap();
    let m = write(t.path(), Some("alfa"), "mcp:claude-code");
    let pinned = m
        .save(&plain("la rama es main"), "s", Place::Personal, false)
        .unwrap()
        .memory;
    m.pin(pinned).unwrap();
    let summary = m
        .save(
            &Draft::plain("Objetivo: migrar · Hecho: esquema 2", MemoryType::Episodic),
            "s",
            Place::Project,
            false,
        )
        .unwrap()
        .memory;
    let ids: Vec<MemoryId> = m.context().unwrap().iter().map(|r| r.id).collect();
    assert_eq!(ids[..2], [summary, pinned]);
}
