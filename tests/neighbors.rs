#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! A memory's neighbors (what the model can walk to from it) and the relations that follow a
//! memory when it is corrected.

mod common;

use std::sync::Arc;

use clio_memory::*;
use common::*;

fn save(f: &Fixture, scope: ScopeId, text: &str) -> MemoryId {
    f.engine
        .propose(prop(scope, f.person, text))
        .unwrap()
        .memory
}

fn ids(n: &[(MemoryRow, f32)]) -> Vec<MemoryId> {
    n.iter().map(|(r, _)| r.id).collect()
}

#[test]
fn neighbors_are_the_nearest_active_memories_of_the_scopes_read_and_never_itself() {
    let f = fixture_with(Arc::new(HashEmbedder::new("m", 256)));
    let other = f.engine.create_scope("otro", "es").unwrap();
    let m1 = save(&f, f.scope, "respaldo restic de la base cada noche");
    let m2 = save(&f, f.scope, "restaurar la base desde el respaldo restic");
    let m3 = save(&f, f.scope, "la base guarda los pedidos");
    let m4 = save(&f, f.scope, "el perro se llama Toby");
    let far = save(
        &f,
        other,
        "respaldo restic de la base cada noche también aquí",
    );
    f.engine.embed_pending(100).unwrap();

    let n = f.engine.neighbors(m1, &[f.scope], 5).unwrap();
    let got = ids(&n);
    assert_eq!(got.len(), 3, "{got:?}");
    assert!(!got.contains(&m1) && !got.contains(&far), "{got:?}");
    assert_eq!(got[0], m2, "the closest first: {got:?}");
    assert!(got.contains(&m3) && got.contains(&m4));

    f.engine.forget(m2, f.person, "ya no").unwrap();
    let got = ids(&f.engine.neighbors(m1, &[f.scope], 5).unwrap());
    assert!(
        !got.contains(&m2),
        "a forgotten memory is no neighbor: {got:?}"
    );
}

#[test]
fn a_memory_without_a_vector_gets_its_neighbors_from_text_search() {
    let f = fixture();
    let a = save(&f, f.scope, "los respaldos se hacen con restic");
    let b = save(
        &f,
        f.scope,
        "para restaurar se usa el último respaldo de restic",
    );
    save(&f, f.scope, "el perro se llama Toby");
    assert_eq!(ids(&f.engine.neighbors(a, &[f.scope], 1).unwrap()), [b]);
    assert_eq!(ids(&f.engine.neighbors(b, &[f.scope], 1).unwrap()), [a]);
}

#[test]
fn a_lone_memory_has_no_neighbors() {
    let f = fixture_with(Arc::new(HashEmbedder::new("m", 64)));
    let a = save(&f, f.scope, "una sola memoria");
    f.engine.embed_pending(10).unwrap();
    assert!(f.engine.neighbors(a, &[f.scope], 5).unwrap().is_empty());
}

#[test]
fn relations_are_copied_except_contradictions_and_self() {
    let f = fixture();
    let [m1, m2, m3, m4] = ["uno", "dos", "tres", "cuatro"].map(|t| save(&f, f.scope, t));
    f.engine
        .relate(m1, m2, Relation::Related, "jev", 0.8)
        .unwrap();
    f.engine
        .relate(m3, m1, Relation::ConflictsWith, "jev", 0.9)
        .unwrap();
    f.engine
        .relate(m1, m4, Relation::Related, "jev", 0.7)
        .unwrap();

    assert_eq!(f.engine.copy_relations(m1, m4).unwrap(), 1);
    let of4 = f.engine.relations(m4).unwrap();
    let copied = of4
        .iter()
        .find(|r| r.source == m4 && r.target == m2)
        .expect("m4 is related to m2 now");
    assert_eq!(
        (copied.relation, copied.by.as_str(), copied.confidence),
        (Relation::Related, "jev", 0.8)
    );
    assert!(
        !of4.iter()
            .any(|r| r.relation == Relation::ConflictsWith || r.source == r.target)
    );
    assert_eq!(f.engine.relations(m1).unwrap().len(), 3, "m1 keeps its own");
    assert_eq!(
        f.engine.copy_relations(m1, m4).unwrap(),
        0,
        "twice adds nothing"
    );
}
