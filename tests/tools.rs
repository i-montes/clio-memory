#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! The tools an agent calls: what they answer, what they refuse, and their caps.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use clio_memory::agent::memory::embed_pending;
use clio_memory::agent::{Draft, EmbedModel, Memory, Place};
use clio_memory::tools::{Session, TOOLS};
use clio_memory::{Embedder, Engine, FixedClock, JobKind, MemoryType, OpenMode, SystemClock};
use common::HashEmbedder;
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
fn a_read_marks_the_memory_as_used() {
    let t = tempfile::tempdir().unwrap();
    let mut s = session(&t);
    assert_eq!(clio_memory::sleep::last_used(t.path()), 0);
    // Whatever it answers on an empty memory, it was a use.
    let _ = s.call("memory_search", &json!({"query": "servidor de pruebas"}));
    assert!(clio_memory::sleep::last_used(t.path()) > 0);
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
fn the_read_budget_refills_after_half_an_hour() {
    let t = tempfile::tempdir().unwrap();
    let clock = Arc::new(FixedClock::new(10_000));
    let mut s = session(&t).with_clock(clock.clone());
    for _ in 0..20 {
        s.call("memory_context", &json!({})).unwrap();
    }
    let why = s.call("memory_context", &json!({})).unwrap_err();
    assert!(why.contains("refills in 30 minutes"), "{why}");
    clock.set(10_000 + 1799);
    let why = s.call("memory_context", &json!({})).unwrap_err();
    assert!(why.ends_with("; it refills in 1 minute"), "{why}");
    clock.set(10_000 + 1800);
    assert!(s.call("memory_context", &json!({})).is_ok());
}

#[test]
fn the_save_budget_refills_too() {
    let t = tempfile::tempdir().unwrap();
    let clock = Arc::new(FixedClock::new(10_000));
    let mut s = session(&t).with_clock(clock.clone());
    for i in 0..30 {
        save(&mut s, &format!("dato número {i}")).unwrap_or_else(|e| panic!("{i}: {e}"));
    }
    let why = save(&mut s, "uno de más").unwrap_err();
    assert!(why.contains("refills in"), "{why}");
    clock.set(10_000 + 1800);
    save(&mut s, "uno de más").unwrap();
}

#[test]
fn the_change_budget_refills_too() {
    let t = tempfile::tempdir().unwrap();
    let clock = Arc::new(FixedClock::new(10_000));
    let mut s = session(&t).with_clock(clock.clone());
    save(&mut s, "uno").unwrap();
    for _ in 0..10 {
        s.call("memory_pin", &json!({"id": 1})).unwrap();
    }
    let why = s.call("memory_pin", &json!({"id": 1})).unwrap_err();
    assert_eq!(
        why,
        "this session made its memory changes (10); it refills in 30 minutes (the person can \
         still make changes on the command line)"
    );
    clock.set(10_000 + 1800);
    s.call("memory_pin", &json!({"id": 1})).unwrap();
}

#[test]
fn the_text_budget_refills_too() {
    let t = tempfile::tempdir().unwrap();
    let clock = Arc::new(FixedClock::new(10_000));
    let mut s = session(&t).with_clock(clock.clone());
    for i in 0..8 {
        save(&mut s, &format!("{} {i}", "relleno largo ".repeat(80))).unwrap();
    }
    let ask = json!({"query": "relleno", "limit": 8});
    let calls = (0..20)
        .take_while(|_| s.call("memory_search", &ask).is_ok())
        .count();
    assert!(
        calls < 20,
        "the 24 KiB cap should stop it before the 20 calls"
    );
    clock.set(10_000 + 1800);
    s.call("memory_search", &ask).unwrap();
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

/// Thirty valid saves still go in: the refusals before them spent nothing.
fn thirty_saves_go_in(s: &mut Session) {
    for i in 0..30 {
        save(s, &format!("dato número {i}")).unwrap_or_else(|e| panic!("{i}: {e}"));
    }
}

#[test]
fn a_summary_too_long_with_its_labels_is_refused_with_the_numbers_and_spends_nothing() {
    let t = tempfile::tempdir().unwrap();
    let mut s = session(&t);
    // "Objetivo: " + 600 + " · " + "Hecho: " + 600 = 1220 characters
    let args = json!({"goal": "a".repeat(600), "accomplished": "b".repeat(600)});
    for _ in 0..31 {
        let e = s.call("memory_session_summary", &args).unwrap_err();
        assert!(e.contains("1220") && e.contains("1200"), "{e}");
    }
    thirty_saves_go_in(&mut s);
}

#[test]
fn a_summary_part_over_600_is_refused_and_spends_nothing() {
    let t = tempfile::tempdir().unwrap();
    let mut s = session(&t);
    let args = json!({"goal": "a".repeat(601), "accomplished": "hecho"});
    for _ in 0..31 {
        let e = s.call("memory_session_summary", &args).unwrap_err();
        assert!(
            e.contains("goal") && e.contains("601") && e.contains("600"),
            "{e}"
        );
    }
    thirty_saves_go_in(&mut s);
}

#[test]
fn a_summary_that_fits_is_saved() {
    let t = tempfile::tempdir().unwrap();
    let mut s = session(&t);
    // 10 + 590 + 3 + 7 + 590 = 1200: right at the limit
    let r = s
        .call(
            "memory_session_summary",
            &json!({"goal": "a".repeat(590), "accomplished": "b".repeat(590)}),
        )
        .unwrap();
    assert!(r.starts_with("saved"), "{r}");
}

#[test]
fn a_text_too_long_or_a_bad_argument_spends_no_save() {
    let t = tempfile::tempdir().unwrap();
    let mut s = session(&t);
    for _ in 0..31 {
        let e = save(&mut s, &"x".repeat(1201)).unwrap_err();
        assert!(e.contains("1201") && e.contains("1200"), "{e}");
        let e = s
            .call("memory_save", &json!({"text": "dato", "kind": "otro"}))
            .unwrap_err();
        assert!(e.contains("kind"), "{e}");
        let e = s
            .call(
                "memory_save",
                &json!({"text": "dato", "kind": "semantic", "title": "t".repeat(121)}),
            )
            .unwrap_err();
        assert!(e.contains("title"), "{e}");
        let e = s
            .call("memory_update", &json!({"id": 0, "text": "dato"}))
            .unwrap_err();
        assert!(e.contains("id"), "{e}");
    }
    thirty_saves_go_in(&mut s);
}

#[test]
fn a_secret_spends_a_save() {
    let t = tempfile::tempdir().unwrap();
    let mut s = session(&t);
    let key = concat!("sk-", "proj-0123456789abcdefghijklmnopqrstuv");
    for _ in 0..30 {
        let e = save(&mut s, &format!("la clave es {key}")).unwrap_err();
        assert!(e.contains("secret"), "{e}");
    }
    let e = save(&mut s, "uno de más").unwrap_err();
    assert!(e.contains("30"), "{e}");
}

#[test]
fn a_secret_in_a_summary_is_refused_and_spends_a_save() {
    let t = tempfile::tempdir().unwrap();
    let mut s = session(&t);
    let key = concat!("sk-", "proj-0123456789abcdefghijklmnopqrstuv");
    for _ in 0..30 {
        let e = s
            .call(
                "memory_session_summary",
                &json!({"goal": "probar", "accomplished": format!("usé {key}")}),
            )
            .unwrap_err();
        assert!(e.contains("secret") && !e.contains(key), "{e}");
    }
    let e = save(&mut s, "uno de más").unwrap_err();
    assert!(e.contains("30"), "{e}");
}

#[test]
fn a_text_measured_once_clean_fits_though_it_arrived_longer() {
    let t = tempfile::tempdir().unwrap();
    let mut s = session(&t);
    // 300 words of 3 letters with 5 spaces between: 2395 typed, 1199 once clean
    let typed = vec!["abc"; 300].join("     ");
    assert!(typed.chars().count() > 1200);
    let r = save(&mut s, &typed).unwrap();
    assert!(r.starts_with("saved"), "{r}");
}

#[test]
fn the_schemas_say_the_lengths() {
    use clio_memory::tools::schema;
    let summary = schema("memory_session_summary").unwrap();
    for part in [
        "goal",
        "instructions",
        "discoveries",
        "accomplished",
        "next_steps",
        "files",
    ] {
        assert_eq!(summary["properties"][part]["maxLength"], 600, "{part}");
    }
    let save = schema("memory_save").unwrap();
    assert_eq!(save["properties"]["text"]["maxLength"], 1200);
    assert_eq!(save["properties"]["title"]["maxLength"], 120);
    let (_, about) = TOOLS
        .iter()
        .find(|(n, _)| *n == "memory_session_summary")
        .unwrap();
    assert!(about.contains("1200"), "{about}");
}

#[test]
fn a_bad_id_spends_no_change() {
    let t = tempfile::tempdir().unwrap();
    let mut s = session(&t);
    save(&mut s, "uno").unwrap();
    for _ in 0..11 {
        for tool in ["memory_forget", "memory_pin"] {
            let e = s.call(tool, &json!({"id": 0})).unwrap_err();
            assert!(e.contains("id"), "{tool}: {e}");
        }
        let e = s.call("memory_revert", &json!({"change": -1})).unwrap_err();
        assert!(e.contains("change"), "{e}");
        let e = s
            .call("memory_consolidate", &json!({"ids": "1", "text": "x"}))
            .unwrap_err();
        assert!(e.contains("ids"), "{e}");
    }
    let r = s.call("memory_pin", &json!({"id": 1})).unwrap();
    assert!(!r.contains("10"), "{r}");
}

/// Inside `embed`, tries to take the writer: `held` says it once could not (someone held it while
/// the model ran), `calls` how many times the model ran.
struct ProbeEmbedder {
    inner: HashEmbedder,
    db: PathBuf,
    held: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
}
impl Embedder for ProbeEmbedder {
    fn model_id(&self) -> &str {
        self.inner.model_id()
    }
    fn dim(&self) -> usize {
        self.inner.dim()
    }
    fn embed(&self, texts: &[&str]) -> clio_memory::Result<Vec<Vec<f32>>> {
        if Engine::open(&self.db, OpenMode::ReadWrite, Arc::new(SystemClock)).is_err() {
            self.held.store(true, Ordering::SeqCst);
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.embed(texts)
    }
}

/// A probe over `t`'s file: the model, whether it ever ran under the writer, how often it ran.
fn probe(t: &tempfile::TempDir) -> (Arc<EmbedModel>, Arc<AtomicBool>, Arc<AtomicUsize>) {
    let held = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let model = EmbedModel::from_embedder(Arc::new(ProbeEmbedder {
        inner: HashEmbedder::new("hash", 16),
        db: t.path().join("memory.db"),
        held: held.clone(),
        calls: calls.clone(),
    }));
    (model, held, calls)
}

fn stats(t: &tempfile::TempDir) -> (u64, usize) {
    let m = Memory::open_read(t.path(), &EmbedModel::none(), None, "mcp:prueba")
        .unwrap()
        .unwrap();
    let e = m.engine();
    (
        e.stats().unwrap().vectors,
        e.ready_jobs(JobKind::Embed).unwrap(),
    )
}

#[test]
fn a_save_does_not_hold_the_writer_while_it_embeds() {
    let t = tempfile::tempdir().unwrap();
    let (model, held, calls) = probe(&t);
    let mut s = Session::new(t.path().to_path_buf(), None, "mcp:prueba".into()).with_model(model);
    let said = s
        .call(
            "memory_save",
            &json!({"text": "Qué: una prueba", "kind": "semantic"}),
        )
        .unwrap();
    assert!(said.starts_with("saved"), "{said}");
    assert!(calls.load(Ordering::SeqCst) > 0);
    assert!(
        !held.load(Ordering::SeqCst),
        "the writer was held while the model ran"
    );
    assert_eq!(
        stats(&t),
        (1, 0),
        "the vector is stored all the same, and its queued job closed"
    );
}

#[test]
fn a_merge_does_not_hold_the_writer_while_it_embeds() {
    let t = tempfile::tempdir().unwrap();
    let (model, held, calls) = probe(&t);
    let mut s = Session::new(t.path().to_path_buf(), None, "mcp:prueba".into()).with_model(model);
    for text in [
        "Qué: el servidor de pruebas es TITAN-9",
        "Qué: las copias se guardan en el disco B los lunes",
    ] {
        let said = s
            .call("memory_save", &json!({"text": text, "kind": "semantic"}))
            .unwrap();
        assert!(said.starts_with("saved"), "{said}");
    }
    held.store(false, Ordering::SeqCst);
    let before = calls.load(Ordering::SeqCst);
    let said = s
        .call(
            "memory_consolidate",
            &json!({"ids": "1, 2", "text": "Qué: TITAN-9 es el servidor de pruebas; copias en B los lunes"}),
        )
        .unwrap();
    assert!(said.starts_with("merged into m3"), "{said}");
    assert!(
        calls.load(Ordering::SeqCst) > before,
        "the merge was not embedded"
    );
    assert!(
        !held.load(Ordering::SeqCst),
        "the writer was held while the model ran"
    );
}

#[test]
fn what_is_written_while_the_vectors_are_stored_is_embedded_without_the_writer() {
    let t = tempfile::tempdir().unwrap();
    drop(Memory::open_write(t.path(), &EmbedModel::none(), None, "agente").unwrap());
    let (model, held, _) = probe(&t);
    let (ready, go) = std::sync::mpsc::channel();
    // Another process holds the writer, and writes a memory (and its queued job) once the read
    // part found nothing to compute: when the last step takes the writer, something waits.
    let other = {
        let data = t.path().to_path_buf();
        std::thread::spawn(move || {
            let hash = EmbedModel::from_embedder(Arc::new(HashEmbedder::new("hash", 16)));
            let w = Memory::open_write(&data, &hash, None, "agente").unwrap();
            ready.send(()).unwrap();
            // Timing: the read part takes far less. If it ever took longer, it would find this
            // memory itself and the test would pass without reaching the closing step.
            std::thread::sleep(std::time::Duration::from_millis(300));
            w.save(
                &Draft::plain(
                    "Qué: llegó entre la lectura y el cierre",
                    MemoryType::Semantic,
                ),
                "agente",
                Place::Personal,
                false,
            )
            .unwrap();
        })
    };
    go.recv().unwrap();
    embed_pending(t.path(), &model).unwrap();
    other.join().unwrap();
    assert!(
        !held.load(Ordering::SeqCst),
        "the model ran under the writer"
    );
    assert_eq!(stats(&t), (1, 0), "it was embedded and its job closed");
}

// The person's fixed blocks come first in the context, each under its header.

/// The memory with `identidad` and `reglas` set by the person to `a` and `b` (empty: left
/// empty), and `recent` memories of the agent saved after.
fn with_blocks(t: &tempfile::TempDir, a: &str, b: &str, recent: &[String]) {
    let m = Memory::open_write(t.path(), &EmbedModel::none(), None, "mcp:prueba").unwrap();
    let persona = m.actors().persona;
    for (name, text) in [("identidad", a), ("reglas", b)] {
        if text.is_empty() {
            continue;
        }
        let id = m
            .engine()
            .propose(common::prop(m.scope(), persona, text))
            .unwrap()
            .memory;
        let block = m
            .engine()
            .blocks(m.scope())
            .unwrap()
            .into_iter()
            .find(|b| b.name == name)
            .unwrap();
        m.engine().block_set(block.id, &[id], persona).unwrap();
    }
    for text in recent {
        m.save(
            &Draft::plain(text, MemoryType::Semantic),
            "s",
            Place::Personal,
            false,
        )
        .unwrap();
    }
}

fn ten_recent() -> Vec<String> {
    (0..10)
        .map(|i| format!("dato reciente número {i}"))
        .collect()
}

#[test]
fn the_context_starts_with_the_fixed_blocks() {
    let t = tempfile::tempdir().unwrap();
    with_blocks(
        &t,
        "Eres Clio, la asistente",
        "Responde en español",
        &ten_recent(),
    );
    let out = session(&t).call("memory_context", &json!({})).unwrap();
    let at = |s: &str| out.find(s).unwrap_or_else(|| panic!("{s} missing: {out}"));
    // The most recent eight are shown, newest first.
    let first_recent = (0..10)
        .filter_map(|i| out.find(&format!("dato reciente número {i}")))
        .min()
        .unwrap();
    let order = [
        at("\n## identidad\n"),
        at("Eres Clio, la asistente"),
        at("\n## reglas\n"),
        at("Responde en español"),
        first_recent,
    ];
    assert!(order.is_sorted(), "{order:?}: {out}");
    assert!(!out.contains("## preferencias"), "{out}");
}

#[test]
fn an_empty_block_prints_nothing() {
    let t = tempfile::tempdir().unwrap();
    with_blocks(&t, "", "", &ten_recent());
    let out = session(&t).call("memory_context", &json!({})).unwrap();
    assert!(out.contains("dato reciente"), "{out}");
    assert!(!out.contains("## "), "{out}");
}

#[test]
fn only_the_three_blocks_are_headers() {
    let t = tempfile::tempdir().unwrap();
    {
        let m = Memory::open_write(t.path(), &EmbedModel::none(), None, "mcp:prueba").unwrap();
        let persona = m.actors().persona;
        let id = m
            .engine()
            .propose(common::prop(m.scope(), persona, "vigila el disco"))
            .unwrap()
            .memory;
        let b = m
            .engine()
            .create_block(m.scope(), "vigilancia", 50, persona)
            .unwrap();
        m.engine().block_set(b, &[id], persona).unwrap();
    }
    let out = session(&t).call("memory_context", &json!({})).unwrap();
    assert!(out.contains("vigila el disco"), "{out}");
    assert!(!out.contains("## "), "{out}");
}

#[test]
fn the_blocks_survive_the_answer_cap() {
    let t = tempfile::tempdir().unwrap();
    let a = format!("Eres Clio, la asistente {}", "de la persona ".repeat(20));
    let b = format!("Responde en español {}", "y con calma ".repeat(20));
    let long: Vec<String> = (0..10)
        .map(|i| format!("recuerdo largo {i} {}", "con mucho texto ".repeat(70)))
        .collect();
    with_blocks(&t, a.trim(), b.trim(), &long);
    let out = session(&t).call("memory_context", &json!({})).unwrap();
    assert!(out.len() <= 4096, "{}", out.len());
    assert!(
        out.contains("more memories left out"),
        "the cap was reached: {out}"
    );
    assert!(out.contains(a.trim()) && out.contains(b.trim()), "{out}");
}
