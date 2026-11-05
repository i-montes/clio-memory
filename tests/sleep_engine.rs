#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! What the sleep may do to the memory, and the rules the engine enforces on it: it never touches
//! what is the person's, it purges only what was discarded long ago, and everything else it does
//! can be undone.

mod common;

use clio_memory::*;
use common::*;

const DAY: i64 = 86_400;

fn agent_memory(f: &Fixture, text: &str) -> MemoryId {
    f.engine
        .propose(prop(f.scope, f.agent, text))
        .unwrap()
        .memory
}

fn person_memory(f: &Fixture, text: &str) -> MemoryId {
    f.engine
        .propose(prop(f.scope, f.person, text))
        .unwrap()
        .memory
}

/// Sends to review (quarantine) every proposal whose text says «revisión».
struct Reviewer;

impl Decider for Reviewer {
    fn name(&self) -> &str {
        "revisor"
    }
    fn decide(&self, q: &Question) -> Result<Option<Decision>> {
        Ok(q.candidate
            .content
            .contains("revisión")
            .then_some(Decision {
                choice: Choice::Review,
                confidence: 1.0,
                reason: None,
            }))
    }
}

fn reviewing(mut f: Fixture) -> Fixture {
    f.engine = f
        .engine
        .with_decider(std::sync::Arc::new(Reviewer), DeciderMode::Active, 0.5);
    f
}

/// A candidate waiting for the person's review.
fn waiting(engine: &Engine, f: &Fixture, text: &str) -> Outcome {
    engine
        .propose(prop(f.scope, f.agent, &format!("{text} (revisión)")))
        .unwrap()
}

fn state(f: &Fixture, id: MemoryId) -> State {
    f.engine.get(id).unwrap().state
}

#[test]
fn sleep_quarantine_refuses_the_persons_and_pinned() {
    let f = fixture();
    let mine = person_memory(&f, "prefiero respuestas cortas");
    let pinned = agent_memory(&f, "el repositorio usa cargo deny");
    f.engine.pin(pinned, f.person).unwrap();
    let plain = agent_memory(&f, "la prueba lenta tarda dos minutos");

    assert!(matches!(
        f.engine.sleep_quarantine(mine, "sueño: secreto"),
        Err(Error::NotAllowed)
    ));
    assert!(matches!(
        f.engine.sleep_quarantine(pinned, "sueño: secreto"),
        Err(Error::NotAllowed)
    ));
    // Only reasons of the sleep.
    assert!(matches!(
        f.engine.sleep_quarantine(plain, "otro motivo"),
        Err(Error::NotAllowed)
    ));
    let op = f.engine.sleep_quarantine(plain, "sueño: secreto").unwrap();
    assert_eq!(state(&f, plain), State::Quarantine);
    let done = f.engine.operation(op).unwrap();
    assert_eq!(done.actor, f.engine.system_actor().unwrap());
    assert!(done.reason.starts_with("sueño:"));
    assert_eq!(state(&f, mine), State::Active);
    assert_eq!(state(&f, pinned), State::Active);
    // Not active any more: nothing to do.
    assert!(matches!(
        f.engine.sleep_quarantine(plain, "sueño: secreto"),
        Err(Error::InvalidState)
    ));
}

#[test]
fn sleep_purge_only_purges_what_was_discarded_long_ago() {
    let f = reviewing(fixture());
    let forgotten = agent_memory(&f, "un dato que el agente olvidó");
    let waiting = waiting(&f.engine, &f, "un texto que espera").memory;
    assert_eq!(state(&f, waiting), State::Quarantine);
    let persons_by_agent = person_memory(&f, "mi editor es helix");
    let persons_by_person = person_memory(&f, "mi color es el verde");
    f.engine.forget(forgotten, f.agent, "ya no vale").unwrap();
    f.engine
        .forget(persons_by_agent, f.agent, "ya no vale")
        .unwrap();
    f.engine
        .forget(persons_by_person, f.person, "ya no vale")
        .unwrap();

    f.clock.set(1000 + 29 * DAY);
    assert!(matches!(
        f.engine.sleep_purge(forgotten, "sueño: purga"),
        Err(Error::NotAllowed)
    ));
    f.clock.set(1000 + 30 * DAY);
    f.engine.sleep_purge(forgotten, "sueño: purga").unwrap();
    assert!(f.engine.tombstone(forgotten).unwrap().is_some());

    f.clock.set(1000 + 90 * DAY);
    assert!(matches!(
        f.engine.sleep_purge(waiting, "sueño: purga"),
        Err(Error::NotAllowed)
    ));
    assert!(matches!(
        f.engine.sleep_purge(persons_by_agent, "sueño: purga"),
        Err(Error::NotAllowed)
    ));
    f.engine
        .sleep_purge(persons_by_person, "sueño: purga")
        .unwrap();
    assert!(f.engine.tombstone(persons_by_person).unwrap().is_some());
    assert!(f.engine.tombstone(waiting).unwrap().is_none());
    assert!(f.engine.tombstone(persons_by_agent).unwrap().is_none());
}

#[test]
fn a_secret_quarantined_by_the_sleep_is_purged_after_seven_days() {
    let f = fixture();
    let secret = agent_memory(&f, "algo que el sueño vio como secreto");
    let attack = agent_memory(&f, "algo que el sueño vio como ataque");
    f.engine.sleep_quarantine(secret, "sueño: secreto").unwrap();
    f.engine
        .sleep_quarantine(attack, "sueño: posible ataque (Injection, Exfil)")
        .unwrap();

    f.clock.set(1000 + 6 * DAY);
    assert!(matches!(
        f.engine.sleep_purge(secret, "sueño: purga"),
        Err(Error::NotAllowed)
    ));
    f.clock.set(1000 + 7 * DAY);
    f.engine.sleep_purge(secret, "sueño: purga").unwrap();
    // The short wait is for secrets only.
    assert!(matches!(
        f.engine.sleep_purge(attack, "sueño: purga"),
        Err(Error::NotAllowed)
    ));
}

#[test]
fn sleep_merge_replaces_the_sources_and_reverts_whole() {
    let f = fixture();
    let a = agent_memory(&f, "el build usa cargo");
    let b = agent_memory(&f, "el build corre con cargo");
    let c = agent_memory(&f, "para compilar se usa cargo build");
    let other = agent_memory(&f, "las pruebas usan tempfile");
    let rival = agent_memory(&f, "el build usa make");
    for s in [a, b, c] {
        f.engine
            .relate(s, other, Relation::Related, "jev", 0.9)
            .unwrap();
    }
    f.engine
        .relate(a, rival, Relation::ConflictsWith, "jev", 0.95)
        .unwrap();
    // A relation between two sources does not survive as one to a replaced memory.
    f.engine
        .relate(a, b, Relation::Related, "jev", 0.9)
        .unwrap();

    let (new, op) = f
        .engine
        .sleep_merge(&[a, b, c], "el build usa cargo build", Some("build"))
        .unwrap();
    for s in [a, b, c] {
        let row = f.engine.get(s).unwrap();
        assert_eq!(row.state, State::Replaced);
        assert_eq!(row.replaced_by, Some(new));
    }
    let merged = f.engine.get(new).unwrap();
    assert_eq!(merged.state, State::Active);
    assert_eq!(merged.content, "el build usa cargo build");
    let done = f.engine.operation(op).unwrap();
    assert_eq!(done.kind, OpKind::Update);
    assert_eq!(done.actor, f.engine.system_actor().unwrap());
    let others: Vec<(MemoryId, Relation)> = f
        .engine
        .relations(new)
        .unwrap()
        .into_iter()
        .map(|r| {
            (
                if r.source == new { r.target } else { r.source },
                r.relation,
            )
        })
        .collect();
    assert_eq!(others, vec![(other, Relation::Related)]);

    f.engine.revert(op, f.person).unwrap();
    for s in [a, b, c] {
        assert_eq!(state(&f, s), State::Active);
    }
    assert_ne!(state(&f, new), State::Active);
}

#[test]
fn sleep_merge_refuses_mixed_trust_scope_or_the_persons() {
    let f = fixture();
    let judge = f.engine.judge().unwrap();
    let outside = {
        let mut p = prop(f.scope, f.agent, "el build usa cargo según la web");
        p.origin = Origin::External;
        f.engine
            .propose_judged(&judge, p, Judged::Add, Sureness { confidence: 0.9 })
            .unwrap()
            .memory
    };
    assert_eq!(state(&f, outside), State::Active);
    let a = agent_memory(&f, "el build usa cargo");
    let b = agent_memory(&f, "el build corre con cargo");
    let elsewhere = {
        let s = f.engine.create_scope("otro", "es").unwrap();
        f.engine
            .propose(prop(s, f.agent, "el build usa cargo allá"))
            .unwrap()
            .memory
    };
    let mine = person_memory(&f, "el build lo corro con cargo");

    for group in [[a, outside], [a, elsewhere], [a, mine]] {
        assert!(
            f.engine
                .sleep_merge(&group, "el build usa cargo", None)
                .is_err()
        );
    }
    // One source is not a merge.
    assert!(
        f.engine
            .sleep_merge(&[a], "el build usa cargo", None)
            .is_err()
    );
    assert!(
        f.engine
            .sleep_merge(&[a, a], "el build usa cargo", None)
            .is_err()
    );
    for s in [a, b, outside, elsewhere, mine] {
        assert_eq!(state(&f, s), State::Active);
    }
}

#[test]
fn the_sleeps_quarantines_do_not_fill_the_authors_limit() {
    let mut f = reviewing(fixture());
    f.engine = f.engine.with_limits(Limits {
        max_quarantine_per_actor: 2,
        ..Limits::default()
    });
    for i in 0..3 {
        let id = agent_memory(&f, &format!("dato número {i} del agente"));
        f.engine.sleep_quarantine(id, "sueño: secreto").unwrap();
    }
    assert_eq!(waiting(&f.engine, &f, "uno").state, State::Quarantine);
    assert_eq!(waiting(&f.engine, &f, "dos").state, State::Quarantine);
    // The author's own two fill it: the limit still holds.
    assert!(matches!(
        f.engine.propose(prop(f.scope, f.agent, "tres (revisión)")),
        Err(Error::QuarantineFull)
    ));
}

#[test]
fn system_still_cannot_forget_or_purge_directly() {
    let f = fixture();
    let id = agent_memory(&f, "un dato cualquiera");
    let system = f.engine.system_actor().unwrap();
    assert!(matches!(
        f.engine.forget(id, system, "no"),
        Err(Error::NotAllowed)
    ));
    assert!(matches!(
        f.engine.purge(id, system, "no"),
        Err(Error::NotAllowed)
    ));
}

#[test]
fn operations_after_and_last_activity() {
    let f = fixture();
    f.clock.set(2000);
    let a = agent_memory(&f, "uno");
    f.clock.set(3000);
    agent_memory(&f, "dos");
    let first = f.engine.history(a).unwrap()[0].id;
    let after = f.engine.operations_after(first, 10).unwrap();
    assert_eq!(after.len(), 1);
    assert!(after[0].id > first);
    assert_eq!(f.engine.operations_after(OpId(0), 1).unwrap().len(), 1);

    let system = f.engine.system_actor().unwrap();
    assert_eq!(f.engine.last_activity(system).unwrap(), 3000);
    f.clock.set(5000);
    f.engine.sleep_quarantine(a, "sueño: secreto").unwrap();
    // What the sleep itself did is not activity.
    assert_eq!(f.engine.last_activity(system).unwrap(), 3000);
}
