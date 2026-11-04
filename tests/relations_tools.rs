#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! The tools show each memory's neighbors (so the model can walk to them), and the relations of a
//! memory follow it when it is corrected or merged.

mod common;

use std::path::Path;
use std::sync::Arc;

use clio_memory::agent::{Draft, EmbedModel, Memory, Place};
use clio_memory::tools::Session;
use clio_memory::{MemoryId, MemoryType, Relation};
use common::HashEmbedder;
use serde_json::json;

fn model() -> Arc<EmbedModel> {
    EmbedModel::from_embedder(Arc::new(HashEmbedder::new("m", 256)))
}

fn session(data: &Path) -> Session {
    Session::new(data.to_path_buf(), None, "mcp:prueba".into()).with_model(model())
}

/// Saves `text` (with `title`) as the agent; `outside` marks it as come from outside content.
fn put(data: &Path, title: &str, text: &str, outside: bool) -> MemoryId {
    let m = Memory::open_write(data, &model(), None, "agente").unwrap();
    let draft = Draft {
        title: Some(title.into()),
        ..Draft::plain(text, MemoryType::Semantic)
    };
    let id = m
        .save(&draft, "s", Place::Personal, outside)
        .unwrap()
        .memory;
    m.embed_queue().unwrap();
    id
}

fn get(s: &mut Session, id: MemoryId) -> String {
    s.call("memory_get", &json!({ "id": id.0 })).unwrap()
}

fn neighbors_line(answer: &str) -> &str {
    answer
        .lines()
        .find(|l| l.starts_with("Cerca: "))
        .unwrap_or_else(|| panic!("no neighbors line in {answer}"))
}

fn restic(data: &Path) -> [MemoryId; 3] {
    [
        put(
            data,
            "respaldos",
            "los respaldos de la base se hacen con restic",
            false,
        ),
        put(
            data,
            "restaurar",
            "restaurar la base con el respaldo de restic",
            false,
        ),
        put(
            data,
            "pedidos",
            "la base guarda los pedidos de la tienda",
            false,
        ),
    ]
}

#[test]
fn memory_get_ends_with_the_neighbors_line() {
    let t = tempfile::tempdir().unwrap();
    let [a, b, c] = restic(t.path());
    let answer = get(&mut session(t.path()), a);
    let line = neighbors_line(&answer);
    assert!(line.contains(&format!("m{} «", b.0)), "{line}");
    assert!(line.contains(&format!("m{} «", c.0)), "{line}");
    assert!(
        !line.contains(&format!("m{} ", a.0)),
        "never itself: {line}"
    );
}

#[test]
fn an_external_neighbor_is_marked() {
    let t = tempfile::tempdir().unwrap();
    let a = put(
        t.path(),
        "respaldos",
        "los respaldos de la base se hacen con restic",
        false,
    );
    let x = put(
        t.path(),
        "web",
        "según una web, restic respalda la base",
        true,
    );
    let answer = get(&mut session(t.path()), a);
    assert!(
        neighbors_line(&answer).contains(&format!("m{} (externo) «", x.0)),
        "{answer}"
    );
}

#[test]
fn a_neighbor_preview_is_cleaned_and_on_one_line() {
    let t = tempfile::tempdir().unwrap();
    let a = put(
        t.path(),
        "respaldos",
        "los respaldos de la base se hacen con restic",
        false,
    );
    put(
        t.path(),
        "hola\u{1b}[31m rojo\nlinea",
        "restic respalda la base",
        false,
    );
    let answer = get(&mut session(t.path()), a);
    assert!(!answer.contains('\u{1b}'), "{answer:?}");
    let line = neighbors_line(&answer);
    assert!(line.contains("rojo"), "{line}");
    assert!(!line.contains("linea\n") && answer.lines().any(|l| l == line));
}

#[test]
fn search_lines_carry_three_neighbor_ids() {
    let t = tempfile::tempdir().unwrap();
    restic(t.path());
    put(
        t.path(),
        "copias",
        "las copias de la base se guardan fuera",
        false,
    );
    put(
        t.path(),
        "noche",
        "el respaldo de la base corre cada noche",
        false,
    );
    let answer = session(t.path())
        .call("memory_search", &json!({"query": "respaldo de la base"}))
        .unwrap();
    let lines: Vec<&str> = answer.lines().filter(|l| l.starts_with("[m")).collect();
    assert!(!lines.is_empty(), "{answer}");
    for l in lines {
        let (_, ids) = l.rsplit_once(" · cerca: ").unwrap_or_else(|| panic!("{l}"));
        let n = ids.split(", ").filter(|i| i.starts_with('m')).count();
        assert!((1..=3).contains(&n), "{l}");
    }
}

#[test]
fn the_answer_cap_holds_with_neighbors() {
    let t = tempfile::tempdir().unwrap();
    let long = format!("base restic {}", "palabra larga de relleno ".repeat(46));
    let a = put(t.path(), "larga", &long, false);
    for i in 0..5 {
        put(
            t.path(),
            &format!("título larguísimo número {i} sobre la base y restic y más cosas aún"),
            &format!("base restic vecino {i}"),
            false,
        );
    }
    let answer = get(&mut session(t.path()), a);
    assert!(answer.len() <= 4096, "{}", answer.len());
    assert!(
        answer.contains("palabra larga de relleno"),
        "the memory itself is shown"
    );
}

#[test]
fn a_correction_keeps_the_relations_and_undoing_it_brings_them_back() {
    let t = tempfile::tempdir().unwrap();
    let [a, b, _] = restic(t.path());
    Memory::open_write(t.path(), &model(), None, "agente")
        .unwrap()
        .engine()
        .relate(a, b, Relation::Related, "jev", 0.8)
        .unwrap();
    let mut s = session(t.path());
    let said = s
        .call(
            "memory_update",
            &json!({"id": a.0, "text": "los respaldos de la base se hacen con restic a un disco externo"}),
        )
        .unwrap();
    let new: i64 = said
        .split("(m")
        .nth(1)
        .and_then(|r| r.split(',').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("{said}"));
    let change: i64 = said
        .split("change ")
        .nth(1)
        .and_then(|r| r.split(':').next())
        .and_then(|n| n.parse().ok())
        .unwrap();
    assert!(
        get(&mut s, MemoryId(new)).contains(&format!("relacionada con m{}", b.0)),
        "the correction keeps the relation"
    );
    s.call("memory_revert", &json!({ "change": change }))
        .unwrap();
    let back = get(&mut s, a);
    assert!(
        back.contains(&format!("relacionada con m{}", b.0)),
        "{back}"
    );
    assert!(
        s.call("memory_get", &json!({ "id": new })).is_err(),
        "the correction is undone"
    );
}

#[test]
fn consolidating_carries_the_relations_of_both() {
    let t = tempfile::tempdir().unwrap();
    let [a, b, c] = restic(t.path());
    let other = put(t.path(), "perro", "el perro se llama Toby", false);
    let e = Memory::open_write(t.path(), &model(), None, "agente").unwrap();
    e.engine()
        .relate(a, c, Relation::Related, "jev", 0.8)
        .unwrap();
    e.engine()
        .relate(other, b, Relation::Compatible, "jev", 0.7)
        .unwrap();
    drop(e);
    let mut s = session(t.path());
    let said = s
        .call(
            "memory_consolidate",
            &json!({"ids": format!("{}, {}", a.0, b.0), "text": "los respaldos con restic sirven para restaurar la base"}),
        )
        .unwrap();
    let merged: i64 = said
        .split("merged into m")
        .nth(1)
        .and_then(|r| r.split(':').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("{said}"));
    let rels = Memory::open_write(t.path(), &model(), None, "agente")
        .unwrap()
        .engine()
        .relations(MemoryId(merged))
        .unwrap();
    let others: Vec<i64> = rels
        .iter()
        .map(|r| {
            if r.source.0 == merged {
                r.target.0
            } else {
                r.source.0
            }
        })
        .collect();
    assert!(
        others.contains(&c.0) && others.contains(&other.0),
        "{rels:?}"
    );
}

/// A model that gives the same vector to texts that start with the same word: twins on purpose.
struct FirstWord(&'static str);

impl clio_memory::Embedder for FirstWord {
    fn model_id(&self) -> &str {
        self.0
    }
    fn dim(&self) -> usize {
        8
    }
    fn embed(&self, texts: &[&str]) -> clio_memory::Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.1f32; 8];
                let w = t.split_whitespace().next().unwrap_or("");
                v[w.len() % 8] = 1.0;
                v
            })
            .collect())
    }
}

fn twins_model(id: &'static str) -> Arc<EmbedModel> {
    EmbedModel::from_embedder(Arc::new(FirstWord(id)))
}

#[test]
fn near_duplicate_at_finds_the_twin_only() {
    let t = tempfile::tempdir().unwrap();
    let model = twins_model("twins");
    let m = Memory::open_write(t.path(), &model, None, "agente").unwrap();
    let put = |text: &str| {
        let id = m
            .save(
                &Draft::plain(text, MemoryType::Semantic),
                "s",
                Place::Personal,
                false,
            )
            .unwrap()
            .memory;
        m.embed_queue().unwrap();
        id
    };
    let first = put("postgres guarda los pedidos");
    let twin = put("postgres ya no; ahora mysql");
    let other = put("el perro se llama Toby");
    assert_eq!(m.near_duplicate_at(twin, 0.99).map(|r| r.id), Some(first));
    assert_eq!(m.near_duplicate_at(other, 0.99).map(|r| r.id), None);
}

fn save_twice(model: Arc<EmbedModel>) -> String {
    let t = tempfile::tempdir().unwrap();
    let mut s = Session::new(t.path().to_path_buf(), None, "mcp:prueba".into()).with_model(model);
    s.call(
        "memory_save",
        &json!({"text": "postgres guarda los pedidos", "kind": "semantic"}),
    )
    .unwrap();
    s.call(
        "memory_save",
        &json!({"text": "postgres ya no; ahora mysql", "kind": "semantic"}),
    )
    .unwrap()
}

#[test]
fn saving_a_near_duplicate_says_so() {
    let said = save_twice(twins_model(clio_memory::GRANITE_ID));
    assert!(said.contains("muy parecida a m1 «"), "{said}");
    assert!(
        said.contains("usa memory_update o memory_consolidate"),
        "{said}"
    );
}

#[test]
fn no_floor_no_warning() {
    let said = save_twice(twins_model("buckets"));
    assert!(!said.contains("muy parecida"), "{said}");
}
