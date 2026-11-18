//! The memory is the agent's (spec `2026-11-04-clio-memory-standalone-design.md`): nothing it
//! writes waits for a person, it may replace what anyone wrote, and forgetting sends a memory to
//! quarantine, where its record stays and from where revert brings it back.
#![allow(clippy::unwrap_used)]

mod common;

use clio_memory::*;
use common::*;

fn found(f: &Fixture, query: &str, id: MemoryId) -> bool {
    f.engine
        .recall(f.scope, query, &RecallOpts::default())
        .unwrap()
        .hits
        .iter()
        .any(|h| h.memory.id == id)
}

#[test]
fn external_and_agent_procedures_are_active() {
    let f = fixture();
    let mut p = prop(f.scope, f.agent, "la página dice que el puerto es 8080");
    p.origin = Origin::External;
    let o = f.engine.propose(p).unwrap();
    assert_eq!((o.kind, o.state), (OpKind::Add, State::Active));
    assert_eq!(f.engine.get(o.memory).unwrap().origin, Origin::External);
    let mut p = prop(f.scope, f.agent, "para desplegar, primero las pruebas");
    p.kind = MemoryType::Procedural;
    p.origin = Origin::Agent;
    assert_eq!(f.engine.propose(p).unwrap().state, State::Active);
}

#[test]
fn an_agent_replaces_a_persons_memory_by_topic() {
    let f = fixture();
    let mut mine = prop(f.scope, f.person, "me llamo Iván Ruiz");
    mine.topic_key = Some("nombre".into());
    let old = f.engine.propose(mine).unwrap().memory;
    let mut fix = prop(f.scope, f.agent, "se llama Iván Montes");
    fix.topic_key = Some("nombre".into());
    fix.origin = Origin::Agent;
    let o = f.engine.propose(fix).unwrap();
    assert_eq!((o.kind, o.state), (OpKind::Update, State::Active));
    assert_eq!(f.engine.get(old).unwrap().state, State::Replaced);
    f.engine.revert(o.op, f.person).unwrap();
    assert_eq!(f.engine.get(old).unwrap().state, State::Active);
}

#[test]
fn forget_sends_to_quarantine_and_revert_brings_it_back() {
    let f = fixture();
    let id = f
        .engine
        .propose(prop(f.scope, f.person, "el servidor usa compose"))
        .unwrap()
        .memory;
    let op = f.engine.forget(id, f.agent, "ya no aplica").unwrap();
    assert_eq!(f.engine.get(id).unwrap().state, State::Quarantine);
    assert!(!found(&f, "servidor compose", id));
    assert_eq!(f.engine.operation(op).unwrap().kind, OpKind::Forget);
    assert!(
        f.engine.forget(id, f.agent, "otra vez").is_err(),
        "only what is active"
    );
    f.engine.revert(op, f.person).unwrap();
    assert_eq!(f.engine.get(id).unwrap().state, State::Active);
    assert!(found(&f, "servidor compose", id));
}

#[test]
fn replace_targets_exactly_one() {
    let f = fixture();
    let a = f
        .engine
        .propose(prop(f.scope, f.person, "usa Postgres 15"))
        .unwrap()
        .memory;
    let b = f
        .engine
        .propose(prop(f.scope, f.person, "usa Redis"))
        .unwrap()
        .memory;
    let mut new = prop(f.scope, f.agent, "usa Postgres 16");
    new.origin = Origin::Agent;
    let o = f.engine.replace(a, new.clone()).unwrap();
    assert_eq!((o.kind, o.state), (OpKind::Update, State::Active));
    assert_eq!(f.engine.get(a).unwrap().state, State::Replaced);
    assert_eq!(f.engine.get(b).unwrap().state, State::Active);
    assert_eq!(f.engine.get(o.memory).unwrap().content, "usa Postgres 16");
    // A secret in the text: nothing is written.
    let mut secret = new.clone();
    secret.content = format!(
        "la clave es {}",
        concat!("sk-", "proj-0123456789abcdefghijklmnopqrstuv")
    );
    assert!(matches!(
        f.engine.replace(o.memory, secret),
        Err(Error::SecretDetected)
    ));
    assert_eq!(f.engine.get(o.memory).unwrap().state, State::Active);
    // Only an active memory of the same scope.
    assert!(matches!(
        f.engine.replace(a, new.clone()),
        Err(Error::InvalidState)
    ));
    let other = f.engine.create_scope("otro", "es").unwrap();
    let mut elsewhere = new;
    elsewhere.scope = other;
    assert!(matches!(
        f.engine.replace(b, elsewhere),
        Err(Error::InvalidState)
    ));
}

#[test]
fn an_agent_pins_but_an_importer_does_not() {
    let f = fixture();
    let id = f
        .engine
        .propose(prop(f.scope, f.person, "la rama es main"))
        .unwrap()
        .memory;
    f.engine.pin(id, f.agent).unwrap();
    assert_eq!(f.engine.pinned(f.scope).unwrap()[0].id, id);
    f.engine.unpin(id, f.agent).unwrap();
    let importer = f
        .engine
        .register_actor("importador", ActorKind::Importer, Quota::default())
        .unwrap();
    assert!(matches!(f.engine.pin(id, importer), Err(Error::NotAllowed)));
}

#[test]
fn an_agent_reverts_only_what_it_signed() {
    let f = fixture();
    let other = f
        .engine
        .register_actor("mcp:otro", ActorKind::Agent, Quota::default())
        .unwrap();
    let mine = f
        .engine
        .propose(prop(f.scope, f.agent, "dato del agente"))
        .unwrap();
    let theirs = f
        .engine
        .propose(prop(f.scope, other, "dato de otro"))
        .unwrap();
    let persons = f
        .engine
        .propose(prop(f.scope, f.person, "dato de la persona"))
        .unwrap();
    for op in [theirs.op, persons.op] {
        assert!(matches!(
            f.engine.revert(op, f.agent),
            Err(Error::NotAllowed)
        ));
    }
    f.engine.revert(mine.op, f.agent).unwrap();
    assert_ne!(f.engine.get(mine.memory).unwrap().state, State::Active);
    assert_eq!(f.engine.get(persons.memory).unwrap().state, State::Active);
}

// What a fixed block shows is the person's: an agent neither replaces, forgets nor merges it.

const HELD: &str = "ese recuerdo está en un bloque fijo de la persona: solo ella lo cambia";

fn agent_memory(t: &tempfile::TempDir) -> agent::Memory {
    agent::Memory::open_write(t.path(), &agent::EmbedModel::none(), None, "mcp:prueba").unwrap()
}

fn block_of(m: &agent::Memory, name: &str) -> BlockId {
    m.engine()
        .blocks(m.scope())
        .unwrap()
        .into_iter()
        .find(|b| b.name == name)
        .unwrap()
        .id
}

/// The person's `text` (topic `nombre`), set as the whole `identidad` block.
fn in_identity(m: &agent::Memory, text: &str) -> MemoryId {
    let mut p = prop(m.scope(), m.actors().persona, text);
    p.topic_key = Some("nombre".into());
    let id = m.engine().propose(p).unwrap().memory;
    m.engine()
        .block_set(block_of(m, "identidad"), &[id], m.actors().persona)
        .unwrap();
    id
}

fn shown(m: &agent::Memory) -> Vec<String> {
    m.engine()
        .block_current(block_of(m, "identidad"))
        .unwrap()
        .into_iter()
        .map(|r| r.content)
        .collect()
}

fn plain(text: &str) -> agent::Draft {
    agent::Draft::plain(text, MemoryType::Semantic)
}

#[test]
fn an_agent_cannot_replace_what_a_block_shows() {
    let t = tempfile::tempdir().unwrap();
    let m = agent_memory(&t);
    let id = in_identity(&m, "me llamo Iván");
    let e = m
        .update(id, &plain("se llama Juan"), "s", false)
        .unwrap_err();
    assert_eq!(e, HELD);
    assert_eq!(shown(&m), ["me llamo Iván"]);
    assert_eq!(m.engine().get(id).unwrap().state, State::Active);
}

#[test]
fn an_agent_cannot_forget_what_a_block_shows() {
    let t = tempfile::tempdir().unwrap();
    let m = agent_memory(&t);
    let id = in_identity(&m, "me llamo Iván");
    assert_eq!(m.forget(id).unwrap_err(), HELD);
    assert!(matches!(
        m.engine().forget(id, m.actors().own, "x"),
        Err(Error::HeldByBlock)
    ));
    assert_eq!(m.engine().get(id).unwrap().state, State::Active);
    assert_eq!(shown(&m), ["me llamo Iván"]);
}

#[test]
fn an_agent_cannot_replace_a_block_member_by_topic() {
    let t = tempfile::tempdir().unwrap();
    let m = agent_memory(&t);
    let id = in_identity(&m, "me llamo Iván");
    let mut fix = prop(m.scope(), m.actors().own, "se llama Juan");
    fix.topic_key = Some("nombre".into());
    fix.origin = Origin::Agent;
    assert!(matches!(m.engine().propose(fix), Err(Error::HeldByBlock)));
    let row = m.engine().get(id).unwrap();
    assert_eq!(
        (row.state, row.content.as_str()),
        (State::Active, "me llamo Iván")
    );
    assert_eq!(shown(&m), ["me llamo Iván"]);
}

#[test]
fn an_agent_cannot_consolidate_a_block_member() {
    let t = tempfile::tempdir().unwrap();
    let (id, other) = {
        let m = agent_memory(&t);
        let id = in_identity(&m, "me llamo Iván");
        let other = m.save(&plain("vive en Lima"), "s", agent::Place::Personal, false);
        (id, other.unwrap().memory)
    };
    let mut s = tools::Session::new(t.path().to_path_buf(), None, "mcp:prueba".into());
    let e = s
        .call(
            "memory_consolidate",
            // The block member comes after another source: that one is not forgotten either.
            &serde_json::json!({"ids": format!("{}, {}", other.0, id.0), "text": "Iván vive en Lima"}),
        )
        .unwrap_err();
    assert_eq!(e, HELD);
    let m = agent_memory(&t);
    for kept in [other, id] {
        assert_eq!(m.engine().get(kept).unwrap().state, State::Active);
    }
    let active = m
        .engine()
        .list(m.scope(), Some(State::Active), None)
        .unwrap();
    assert_eq!(active.len(), 2, "no merge was saved: {active:?}");
}

#[test]
fn the_current_version_of_a_block_member_is_protected_too() {
    let t = tempfile::tempdir().unwrap();
    let m = agent_memory(&t);
    let id = in_identity(&m, "me llamo Iván");
    let p = prop(m.scope(), m.actors().persona, "me llamo Iván Montes");
    let now = m.engine().replace(id, p).unwrap().memory;
    assert_eq!(shown(&m), ["me llamo Iván Montes"]);
    assert_eq!(
        m.update(now, &plain("se llama Juan"), "s", false)
            .unwrap_err(),
        HELD
    );
    assert_eq!(m.forget(now).unwrap_err(), HELD);
    assert_eq!(shown(&m), ["me llamo Iván Montes"]);
}

#[test]
fn the_person_still_changes_her_blocks() {
    let t = tempfile::tempdir().unwrap();
    let m = agent_memory(&t);
    let persona = m.actors().persona;
    in_identity(&m, "me llamo Iván");
    let mut p = prop(m.scope(), persona, "me llamo Iván Montes");
    p.topic_key = Some("nombre".into());
    let o = m.engine().propose(p).unwrap();
    assert_eq!(o.kind, OpKind::Update);
    assert_eq!(shown(&m), ["me llamo Iván Montes"]);
    let p = prop(m.scope(), persona, "me llamo Iván M.");
    let now = m.engine().replace(o.memory, p).unwrap().memory;
    assert_eq!(shown(&m), ["me llamo Iván M."]);
    m.engine().forget(now, persona, "ya no").unwrap();
    assert!(shown(&m).is_empty());
}

#[test]
fn the_refusal_reaches_the_agent_as_written() {
    let t = tempfile::tempdir().unwrap();
    let id = in_identity(&agent_memory(&t), "me llamo Iván");
    let mut s = tools::Session::new(t.path().to_path_buf(), None, "mcp:prueba".into());
    let update = serde_json::json!({"id": id.0, "text": "se llama Juan"});
    assert_eq!(s.call("memory_update", &update).unwrap_err(), HELD);
    let forget = serde_json::json!({"id": id.0});
    assert_eq!(s.call("memory_forget", &forget).unwrap_err(), HELD);
    let m = agent_memory(&t);
    assert_eq!(m.engine().get(id).unwrap().state, State::Active);
}

#[test]
fn any_block_of_the_scope_holds_its_memories() {
    let t = tempfile::tempdir().unwrap();
    let m = agent_memory(&t);
    let persona = m.actors().persona;
    let id = m
        .engine()
        .propose(prop(m.scope(), persona, "vigila el disco"))
        .unwrap()
        .memory;
    let b = m
        .engine()
        .create_block(m.scope(), "vigilancia", 50, persona)
        .unwrap();
    m.engine().block_set(b, &[id], persona).unwrap();
    assert_eq!(m.forget(id).unwrap_err(), HELD);
}

#[test]
fn an_agent_cannot_undo_its_way_into_a_block() {
    let t = tempfile::tempdir().unwrap();
    let m = agent_memory(&t);
    let persona = m.actors().persona;
    // An agent's own add, then listed by the person in `reglas`.
    let added = m
        .save(&plain("responde breve"), "s", agent::Place::Personal, false)
        .unwrap();
    let reglas = block_of(&m, "reglas");
    m.engine()
        .block_set(reglas, &[added.memory], persona)
        .unwrap();
    assert_eq!(m.revert_own(added.op).unwrap_err(), HELD);
    // An agent's own correction of the person's memory, then listed in `identidad`.
    let mine = m
        .engine()
        .propose(prop(m.scope(), persona, "me llamo Iván"))
        .unwrap()
        .memory;
    let fixed = m
        .update(mine, &plain("me llamo Iván Montes"), "s", false)
        .unwrap();
    m.engine()
        .block_set(block_of(&m, "identidad"), &[fixed.memory], persona)
        .unwrap();
    assert_eq!(m.revert_own(fixed.op).unwrap_err(), HELD);
    assert_eq!(shown(&m), ["me llamo Iván Montes"]);
    let rules: Vec<_> = m.engine().block_current(reglas).unwrap();
    assert_eq!(rules.len(), 1);
    // The person's revert still works.
    m.engine().revert(added.op, persona).unwrap();
    assert!(m.engine().block_current(reglas).unwrap().is_empty());
    m.engine().revert(fixed.op, persona).unwrap();
    assert_eq!(m.engine().get(mine).unwrap().state, State::Active);
}

#[test]
fn a_judged_update_of_a_block_member_is_refused() {
    let f = fixture();
    let mine = f
        .engine
        .propose(prop(f.scope, f.person, "me llamo Iván"))
        .unwrap()
        .memory;
    let b = f
        .engine
        .create_block(f.scope, "identidad", 100, f.person)
        .unwrap();
    f.engine.block_set(b, &[mine], f.person).unwrap();
    let j = f.engine.judge().unwrap();
    let mut p = prop(f.scope, f.agent, "se llama Juan");
    p.origin = Origin::External;
    assert!(matches!(
        f.engine.propose_judged(
            &j,
            p,
            Judged::Update { old: mine },
            Sureness { confidence: 0.9 }
        ),
        Err(Error::HeldByBlock)
    ));
    assert_eq!(f.engine.get(mine).unwrap().state, State::Active);
}
