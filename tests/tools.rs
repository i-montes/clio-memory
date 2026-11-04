#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! The tools an agent calls: what they answer, what they refuse, and their caps.

use clio_memory::tools::{Session, TOOLS};
use serde_json::json;

fn session(t: &tempfile::TempDir) -> Session {
    Session::new(t.path().to_path_buf(), None, "mcp:prueba".into())
}

fn save(s: &mut Session, text: &str) -> Result<String, String> {
    s.call(
        "memory_save",
        &json!({"text": text, "kind": "semantic", "title": "dato", "category": "discovery"}),
    )
}

#[test]
fn a_save_then_a_search_returns_it_wrapped_as_data() {
    let t = tempfile::tempdir().unwrap();
    let mut s = session(&t);
    let r = save(&mut s, "el servidor de pruebas es TITAN-9").unwrap();
    assert!(r.starts_with("saved: it is memory now (m1, change "), "{r}");
    let found = s
        .call("memory_search", &json!({"query": "servidor de pruebas"}))
        .unwrap();
    assert!(found.starts_with("<<<UNTRUSTED-DATA memory "), "{found}");
    assert!(found.contains("[m1] semántico · agente · "), "{found}");
    assert!(found.contains("TITAN-9"), "{found}");
    assert_eq!(TOOLS.len(), 12);
}

#[test]
fn memory_propose_still_saves() {
    let t = tempfile::tempdir().unwrap();
    let mut s = session(&t);
    let r = s
        .call(
            "memory_propose",
            &json!({"text": "dato viejo", "kind": "semantic"}),
        )
        .unwrap();
    assert!(r.starts_with("saved"), "{r}");
}

#[test]
fn promote_purge_archive_and_extra_arguments_are_refused() {
    let t = tempfile::tempdir().unwrap();
    let mut s = session(&t);
    save(&mut s, "uno").unwrap();
    for tool in ["memory_promote", "memory_purge", "memory_archive", "said"] {
        let e = s.call(tool, &json!({"id": 1})).unwrap_err();
        assert!(e.contains("unknown tool"), "{tool}: {e}");
    }
    for extra in ["origin", "actor", "scope", "state"] {
        let e = s
            .call(
                "memory_save",
                &json!({"text": "x", "kind": "semantic", extra: "persona"}),
            )
            .unwrap_err();
        assert!(e.contains(extra), "{extra}: {e}");
    }
}

#[test]
fn the_read_budget_is_20_calls() {
    let t = tempfile::tempdir().unwrap();
    let mut s = session(&t);
    save(&mut s, "uno").unwrap();
    for i in 0..20 {
        s.call("memory_search", &json!({"query": "uno"}))
            .unwrap_or_else(|e| panic!("{i}: {e}"));
    }
    let e = s.call("memory_get", &json!({"id": 1})).unwrap_err();
    assert!(e.contains("budget"), "{e}");
}

#[test]
fn the_thirty_first_save_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let mut s = session(&t);
    for i in 0..30 {
        save(&mut s, &format!("dato número {i}")).unwrap_or_else(|e| panic!("{i}: {e}"));
    }
    let e = save(&mut s, "uno de más").unwrap_err();
    assert!(e.contains("30"), "{e}");
}

#[test]
fn a_text_with_a_secret_is_not_saved_and_not_echoed() {
    let t = tempfile::tempdir().unwrap();
    let mut s = session(&t);
    let key = concat!("sk-", "proj-0123456789abcdefghijklmnopqrstuv");
    let e = save(&mut s, &format!("la clave es {key}")).unwrap_err();
    assert!(!e.contains(key), "{e}");
    let found = s.call("memory_search", &json!({"query": "clave"})).unwrap();
    assert_eq!(found, "nothing relevant in memory");
}

#[test]
fn forget_pin_revert_answer_in_fixed_words() {
    let t = tempfile::tempdir().unwrap();
    let mut s = session(&t);
    save(&mut s, "uno").unwrap();
    let saved = save(&mut s, "dos").unwrap();
    let change: i64 = saved
        .split("change ")
        .nth(1)
        .and_then(|r| r.split(':').next())
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        s.call("memory_pin", &json!({"id": 1})).unwrap(),
        "pinned: it comes first in the context"
    );
    assert!(
        s.call("memory_forget", &json!({"id": 1}))
            .unwrap()
            .starts_with("forgotten: it is in quarantine")
    );
    assert_eq!(
        s.call("memory_revert", &json!({"change": change})).unwrap(),
        "undone: the change was reverted"
    );
}

#[test]
fn a_missing_memory_and_a_forgotten_one_give_the_same_error() {
    let t = tempfile::tempdir().unwrap();
    let mut s = session(&t);
    save(&mut s, "uno").unwrap();
    s.call("memory_forget", &json!({"id": 1})).unwrap();
    let forgotten = s.call("memory_get", &json!({"id": 1})).unwrap_err();
    let missing = s.call("memory_get", &json!({"id": 999})).unwrap_err();
    assert_eq!(forgotten, missing);
    assert_eq!(missing, "ese recuerdo no existe o no se puede leer");
}

#[test]
fn a_merge_stays_in_the_memories_own_project_and_never_mixes_two() {
    let t = tempfile::tempdir().unwrap();
    let in_project =
        |p: &str| Session::new(t.path().to_path_buf(), Some(p.into()), "mcp:prueba".into());
    let decision = |s: &mut Session, text: &str| {
        s.call(
            "memory_save",
            &json!({"text": text, "kind": "semantic", "category": "decision"}),
        )
        .unwrap()
    };
    let mut alfa = in_project("alfa");
    decision(&mut alfa, "alfa usa Postgres");
    decision(&mut alfa, "alfa usa la versión 16");
    let mut beta = in_project("beta");
    decision(&mut beta, "beta usa Redis");
    let mut nowhere = session(&t);
    let e = nowhere
        .call(
            "memory_consolidate",
            &json!({"ids": "1, 3", "text": "mezcla"}),
        )
        .unwrap_err();
    assert_eq!(e, "solo se unen memorias del mismo sitio");
    let r = nowhere
        .call(
            "memory_consolidate",
            &json!({"ids": "1, 2", "text": "alfa usa Postgres 16"}),
        )
        .unwrap();
    assert!(r.starts_with("merged into m4"), "{r}");
    let read = nowhere.call("memory_get", &json!({"id": 4})).unwrap();
    assert!(read.contains("proyecto alfa"), "{read}");
}
