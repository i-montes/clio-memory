#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! The recall evaluation harness (`bench/recall/`): a made-up memory of 120 entries in Spanish
//! (a person, three projects, lessons), 159 questions with known answers or none, and 28
//! proposals that repeat, paraphrase or only resemble a memory. It measures what the agent's
//! `memory_search` would return (`Memory::search_all`, top 5), whether recall abstains when
//! nothing answers, what saving a repeat does, and how the thresholds of `RecallOpts` and the
//! near-duplicate floor trade answers for silence.
//!
//! Without a model (the CI) it runs by text only and checks the harness and a floor. With one:
//!
//! ```sh
//! CLIO_MEMORY_MODEL_DIR=~/.cache/clio-models/granite-embedding-311m-multilingual-r2 \
//!   cargo test --release --features granite --test recall_eval -- --ignored --nocapture
//! ```
//!
//! `CLIO_MEMORY_EVAL_OUT=report.json` also writes the report as JSON.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use clio_memory::agent::memory::clean_text;
use clio_memory::agent::{Draft, EmbedModel, Memory, Place};
use clio_memory::{
    Category, Embedder, MemoryId, MemoryType, OpKind, RecallOpts, SearchOpts, State,
};
use serde_json::{Value, json};

/// What `memory_search` returns when the model does not ask for a number (`tools.rs`).
const TOP: usize = 5;
const ACTOR: &str = "mcp:eval";

fn bench_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("bench/recall")
}

fn lines(name: &str) -> Vec<Value> {
    let text = std::fs::read_to_string(bench_dir().join(name)).unwrap();
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or_default()
}

fn project(v: &Value) -> Option<String> {
    v["project"].as_str().map(str::to_owned)
}

fn place(v: &Value) -> Place {
    match s(v, "place") {
        "personal" => Place::Personal,
        "leccion" => Place::Lessons,
        _ => Place::Project,
    }
}

fn draft(v: &Value) -> Draft {
    Draft {
        text: s(v, "text").to_owned(),
        title: Some(s(v, "title").to_owned()),
        category: Category::ALL
            .into_iter()
            .find(|c| c.label() == s(v, "category")),
        kind: MemoryType::Semantic,
    }
}

/// The model, with every vector it computed kept by text: the threshold sweep asks the same
/// questions many times, and a query vector costs Granite ~100 ms on a CPU.
struct Cached {
    inner: Arc<dyn Embedder>,
    seen: Mutex<HashMap<String, Vec<f32>>>,
}

impl Embedder for Cached {
    fn model_id(&self) -> &str {
        self.inner.model_id()
    }
    fn dim(&self) -> usize {
        self.inner.dim()
    }
    fn embed(&self, texts: &[&str]) -> clio_memory::Result<Vec<Vec<f32>>> {
        let mut seen = self.seen.lock().unwrap();
        let missing: Vec<&str> = texts
            .iter()
            .copied()
            .filter(|t| !seen.contains_key(*t))
            .collect();
        if !missing.is_empty() {
            for (t, v) in missing.iter().zip(self.inner.embed(&missing)?) {
                seen.insert((*t).to_owned(), v);
            }
        }
        Ok(texts.iter().map(|t| seen[*t].clone()).collect())
    }
}

fn model_from_env() -> Arc<EmbedModel> {
    let dir = std::env::var("CLIO_MEMORY_MODEL_DIR").expect("CLIO_MEMORY_MODEL_DIR");
    let dir = dir.replacen('~', &std::env::var("HOME").unwrap_or_default(), 1);
    let inner = EmbedModel::with_dirs(vec![PathBuf::from(dir)])
        .get()
        .unwrap()
        .expect("a model folder");
    EmbedModel::from_embedder(Arc::new(Cached {
        inner,
        seen: Mutex::new(HashMap::new()),
    }))
}

/// The bench's memory, written as an agent would (`memory_save`): grouped by the project the
/// session was in, then every vector computed. Bench id to memory id.
struct Bench {
    _dir: tempfile::TempDir,
    data: PathBuf,
    model: Arc<EmbedModel>,
    ids: HashMap<i64, MemoryId>,
    saved: usize,
    not_active: usize,
}

impl Bench {
    fn load(model: Arc<EmbedModel>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let mut by_project: Vec<(Option<String>, Vec<Value>)> = Vec::new();
        for m in lines("memories.jsonl") {
            let p = project(&m);
            match by_project.iter_mut().find(|(q, _)| *q == p) {
                Some((_, list)) => list.push(m),
                None => by_project.push((p, vec![m])),
            }
        }
        let mut ids = HashMap::new();
        let (mut saved, mut not_active) = (0, 0);
        for (p, list) in by_project {
            let w = Memory::open_write(&data, &model, p.as_deref(), ACTOR).unwrap();
            for m in list {
                let o = w
                    .save(&draft(&m), ACTOR, place(&m), false)
                    .unwrap_or_else(|e| panic!("memory {}: {e}", m["id"]));
                assert_eq!(o.kind, OpKind::Add, "memory {} is not new", m["id"]);
                saved += 1;
                not_active += usize::from(o.state != State::Active);
                ids.insert(m["id"].as_i64().unwrap(), o.memory);
            }
            w.embed_queue().unwrap();
        }
        Self {
            _dir: dir,
            data,
            model,
            ids,
            saved,
            not_active,
        }
    }

    fn reader(&self, project: Option<&str>) -> Memory {
        Memory::open_read(&self.data, &self.model, project, ACTOR)
            .unwrap()
            .unwrap()
    }

    fn bench_id(&self, id: MemoryId) -> i64 {
        self.ids
            .iter()
            .find(|(_, m)| **m == id)
            .map_or(-1, |(b, _)| *b)
    }
}

/// One question's answer: the bench ids returned, best first, and whether recall abstained.
struct Answer {
    got: Vec<i64>,
    abstained: bool,
}

fn ask(
    b: &Bench,
    readers: &HashMap<Option<String>, Memory>,
    q: &Value,
    opts: &RecallOpts,
) -> Answer {
    let m = &readers[&project(q)];
    let r = m
        .engine()
        .recall_many(&m.reading_scopes(), &clean_text(s(q, "q")), opts)
        .unwrap();
    Answer {
        got: r.hits.iter().map(|h| b.bench_id(h.memory.id)).collect(),
        abstained: r.abstained,
    }
}

fn opts(min_vector_score: f32, min_text_coverage: f64) -> RecallOpts {
    RecallOpts {
        search: SearchOpts {
            limit: TOP,
            ..SearchOpts::default()
        },
        min_vector_score,
        // The sweep sets the floor itself.
        model_floor: false,
        min_text_coverage,
        token_budget: None,
    }
}

/// What `memory_search` uses: the engine's defaults, the model's own floor included.
fn default_opts() -> RecallOpts {
    RecallOpts {
        search: SearchOpts {
            limit: TOP,
            ..SearchOpts::default()
        },
        ..RecallOpts::default()
    }
}

/// Totals over a group of questions.
#[derive(Default, Clone, Copy)]
struct Score {
    asked: usize,
    first: usize,
    any_in_top: usize,
    gold_found: f64,
    rr: f64,
    abstained: usize,
}

impl Score {
    fn add(&mut self, gold: &[i64], a: &Answer) {
        self.asked += 1;
        self.abstained += usize::from(a.abstained);
        if gold.is_empty() {
            return;
        }
        let rank = a.got.iter().position(|g| gold.contains(g));
        self.first += usize::from(rank == Some(0));
        self.any_in_top += usize::from(rank.is_some());
        self.rr += rank.map_or(0.0, |r| 1.0 / (r as f64 + 1.0));
        let found = gold.iter().filter(|g| a.got.contains(g)).count();
        self.gold_found += found as f64 / gold.len() as f64;
    }

    fn pct(n: usize, of: usize) -> f64 {
        if of == 0 {
            0.0
        } else {
            100.0 * n as f64 / of as f64
        }
    }

    fn json(&self) -> Value {
        let n = self.asked.max(1) as f64;
        json!({
            "asked": self.asked,
            "hit_at_1": Self::pct(self.first, self.asked),
            "success_at_5": Self::pct(self.any_in_top, self.asked),
            "recall_at_5": 100.0 * self.gold_found / n,
            "mrr_at_5": self.rr / n,
            "abstained": Self::pct(self.abstained, self.asked),
        })
    }
}

fn answerable(t: &str) -> bool {
    !matches!(t, "sin-respuesta" | "fuera")
}

/// Every question under `opts`: the score per type, the answerable ones together and the ones
/// without an answer together.
fn score_all(
    b: &Bench,
    readers: &HashMap<Option<String>, Memory>,
    qs: &[Value],
    o: &RecallOpts,
) -> (Vec<(String, Score)>, Score, Score) {
    let mut by_type: Vec<(String, Score)> = Vec::new();
    let (mut yes, mut no) = (Score::default(), Score::default());
    for q in qs {
        let gold: Vec<i64> = q["gold"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g.as_i64().unwrap())
            .collect();
        let a = ask(b, readers, q, o);
        let t = s(q, "type");
        match by_type.iter_mut().find(|(k, _)| k == t) {
            Some((_, sc)) => sc.add(&gold, &a),
            None => {
                let mut sc = Score::default();
                sc.add(&gold, &a);
                by_type.push((t.to_owned(), sc));
            }
        }
        if answerable(t) { &mut yes } else { &mut no }.add(&gold, &a);
    }
    (by_type, yes, no)
}

fn readers(b: &Bench, qs: &[Value]) -> HashMap<Option<String>, Memory> {
    let mut out = HashMap::new();
    for q in qs {
        out.entry(project(q))
            .or_insert_with(|| b.reader(project(q).as_deref()));
    }
    out
}

/// What saving a repeat does: the outcome, and the nearest other memory with its similarity.
struct Probe {
    kind: String,
    twin: i64,
    noop: bool,
    nearest: Option<(i64, f32)>,
}

fn probe_duplicates(b: &mut Bench) -> Vec<Probe> {
    let mut out = Vec::new();
    for d in lines("duplicates.jsonl") {
        let w = Memory::open_write(&b.data, &b.model, project(&d).as_deref(), ACTOR).unwrap();
        let o = w.save(&draft(&d), ACTOR, place(&d), false).unwrap();
        b.saved += 1;
        b.not_active += usize::from(o.state != State::Active);
        w.embed_queue().unwrap();
        let nearest = w
            .engine()
            .neighbors(o.memory, &w.reading_scopes(), 3)
            .unwrap_or_default()
            .into_iter()
            .find(|(r, _)| r.id != o.memory)
            .map(|(r, sim)| (b.bench_id(r.id), sim));
        out.push(Probe {
            kind: s(&d, "type").to_owned(),
            twin: d["twin"].as_i64().unwrap(),
            noop: o.kind == OpKind::Noop,
            nearest,
        });
        if o.kind == OpKind::Add {
            // A probe must not be what the next probe finds: it goes, as an agent would forget it.
            let id = o.memory;
            w.forget(id).unwrap();
        }
    }
    out
}

/// At `floor`, how many paraphrases are told their twin and how many unrelated saves are told
/// a twin they do not have.
fn near_duplicate_at(probes: &[Probe], floor: f32) -> (usize, usize, usize, usize) {
    let (mut para, mut para_ok, mut rel, mut rel_flagged) = (0, 0, 0, 0);
    for p in probes {
        let flagged = p.nearest.filter(|(_, sim)| *sim >= floor);
        match p.kind.as_str() {
            "parafrasis" => {
                para += 1;
                para_ok += usize::from(flagged.is_some_and(|(id, _)| id == p.twin));
            }
            "relacionada" => {
                rel += 1;
                rel_flagged += usize::from(flagged.is_some());
            }
            _ => {}
        }
    }
    (para, para_ok, rel, rel_flagged)
}

fn row(name: &str, sc: &Score) -> String {
    let j = sc.json();
    format!(
        "| {name} | {} | {:.0} % | {:.0} % | {:.0} % | {:.3} | {:.0} % |",
        sc.asked,
        j["hit_at_1"].as_f64().unwrap(),
        j["success_at_5"].as_f64().unwrap(),
        j["recall_at_5"].as_f64().unwrap(),
        j["mrr_at_5"].as_f64().unwrap(),
        j["abstained"].as_f64().unwrap(),
    )
}

/// Runs everything and prints the tables (markdown); returns the report.
fn evaluate(model: Arc<EmbedModel>, sweep: bool) -> Value {
    let mut b = Bench::load(model);
    let qs = lines("queries.jsonl");
    let rs = readers(&b, &qs);
    let with_vectors = rs.values().next().is_some_and(Memory::has_embeddings);
    let model_id = rs
        .values()
        .next()
        .and_then(|m| m.engine().model_id().map(str::to_owned))
        .unwrap_or_else(|| "none".into());

    let d = default_opts();
    let vector_floor = clio_memory::relevance_floor(&model_id).unwrap_or(d.min_vector_score);
    let (by_type, yes, no) = score_all(&b, &rs, &qs, &d);
    println!(
        "\n## Recall with the default thresholds (vector {vector_floor}, text {}) · model {model_id}\n",
        d.min_text_coverage
    );
    println!("| Questions | n | 1st | in top 5 | recall@5 | MRR@5 | abstained |");
    println!("|---|---|---|---|---|---|---|");
    for (t, sc) in &by_type {
        println!("{}", row(t, sc));
    }
    println!("{}", row("**with an answer**", &yes));
    println!("{}", row("**without one**", &no));

    let mut grid = Vec::new();
    let mut cosines = json!(null);
    if sweep && with_vectors {
        // The best cosine of each question, by vectors alone: where an answer and no answer part.
        let raw = opts(-1.0, 2.0);
        let (mut yes_cos, mut no_cos) = (Vec::new(), Vec::new());
        for q in &qs {
            let m = &rs[&project(q)];
            let r = m
                .engine()
                .recall_many(&m.reading_scopes(), &clean_text(s(q, "q")), &raw)
                .unwrap();
            let best = r
                .hits
                .iter()
                .filter_map(|h| h.vector)
                .fold(f32::MIN, f32::max);
            if answerable(s(q, "type")) {
                &mut yes_cos
            } else {
                &mut no_cos
            }
            .push(best);
        }
        yes_cos.sort_by(f32::total_cmp);
        no_cos.sort_by(f32::total_cmp);
        let at = |v: &[f32], p: usize| v[(v.len() - 1) * p / 100];
        println!("\n## Best cosine of each question (vectors only)\n");
        println!("| Questions | min | p10 | p50 | p90 | max |");
        println!("|---|---|---|---|---|---|");
        for (name, v) in [("with an answer", &yes_cos), ("without one", &no_cos)] {
            println!(
                "| {name} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |",
                v[0],
                at(v, 10),
                at(v, 50),
                at(v, 90),
                v[v.len() - 1]
            );
        }
        cosines = json!({"with_an_answer": yes_cos, "without_one": no_cos});
    }
    if sweep {
        println!("\n## Threshold sweep (top 5)\n");
        println!(
            "| vector ≥ | text ≥ | answered (in top 5) | wrongly silent | silent when nothing answers | balanced |"
        );
        println!("|---|---|---|---|---|---|");
        // Granite puts unrelated Spanish text at 0.8-0.9: the fine steps are up there.
        let vectors: Vec<f32> = if with_vectors {
            [0.3, 0.5, 0.7, 0.75]
                .into_iter()
                .chain((80..=95).map(|i| i as f32 / 100.0))
                .collect()
        } else {
            vec![d.min_vector_score]
        };
        for v in vectors {
            for t in [0.34, 0.5, 0.67, 1.0] {
                let (_, yes, no) = score_all(&b, &rs, &qs, &opts(v, t));
                let answered = Score::pct(yes.any_in_top, yes.asked);
                let wrongly_silent = Score::pct(yes.abstained, yes.asked);
                let silent = Score::pct(no.abstained, no.asked);
                let balanced = (answered + silent) / 2.0;
                println!(
                    "| {v:.2} | {t:.2} | {answered:.0} % | {wrongly_silent:.0} % | {silent:.0} % | {balanced:.1} |"
                );
                grid.push(json!({"min_vector_score": v, "min_text_coverage": t,
                    "answered": answered, "wrongly_silent": wrongly_silent,
                    "silent_when_nothing_answers": silent, "balanced": balanced}));
            }
        }
    }

    let probes = probe_duplicates(&mut b);
    let exact: Vec<&Probe> = probes.iter().filter(|p| p.kind == "exacta").collect();
    let exact_noop = exact.iter().filter(|p| p.noop).count();
    let para_noop = probes
        .iter()
        .filter(|p| p.kind != "exacta" && p.noop)
        .count();
    println!(
        "\n## Saving what is already there ({} probes)\n",
        probes.len()
    );
    println!(
        "- exact repeats answered NOOP: {exact_noop} of {}; other probes answered NOOP: {para_noop}",
        exact.len()
    );
    println!(
        "- saves that did not stay active (the review rate): {} of {}",
        b.not_active, b.saved
    );
    let mut floors = Vec::new();
    if with_vectors {
        println!(
            "\n| near-duplicate floor | paraphrases told their twin | related saves wrongly told |"
        );
        println!("|---|---|---|");
        for i in 0..=8 {
            let floor = 0.80 + i as f32 * 0.02;
            let (para, ok, rel, bad) = near_duplicate_at(&probes, floor);
            println!("| {floor:.2} | {ok} of {para} | {bad} of {rel} |");
            floors.push(
                json!({"floor": floor, "paraphrases_told": ok, "paraphrases": para,
                "related_wrongly_told": bad, "related": rel}),
            );
        }
        println!("\nNearest memory of each probe:\n");
        for p in &probes {
            if let Some((id, sim)) = p.nearest {
                println!("- {} (twin m{}): nearest m{id} at {sim:.3}", p.kind, p.twin);
            }
        }
    }

    json!({
        "model": model_id,
        "memories": b.ids.len(),
        "questions": qs.len(),
        "defaults": {"min_vector_score": vector_floor, "min_text_coverage": d.min_text_coverage},
        "by_type": by_type.iter().map(|(t, sc)| (t.clone(), sc.json())).collect::<serde_json::Map<_, _>>(),
        "with_an_answer": yes.json(),
        "without_one": no.json(),
        "sweep": grid,
        "best_cosines": cosines,
        "duplicates": {
            "exact": exact.len(), "exact_noop": exact_noop, "other_noop": para_noop,
            "saved": b.saved, "not_active": b.not_active, "floors": floors,
        },
    })
}

/// Text only, as in the CI: the harness runs end to end, every save is active, a repeat is a
/// NOOP, and the text path keeps a floor (measured 2026-11-09; see `bench/recall/results.md`).
#[test]
fn the_recall_bench_runs_by_text_and_keeps_its_floor() {
    let r = evaluate(EmbedModel::none(), false);
    assert_eq!(r["memories"], 120);
    assert_eq!(r["duplicates"]["not_active"], 0, "{r}");
    assert_eq!(
        r["duplicates"]["exact_noop"], r["duplicates"]["exact"],
        "{r}"
    );
    let yes = &r["with_an_answer"];
    let no = &r["without_one"];
    assert!(
        yes["success_at_5"].as_f64().unwrap() >= TEXT_FLOOR_ANSWERED,
        "{r}"
    );
    assert!(
        no["abstained"].as_f64().unwrap() >= TEXT_FLOOR_SILENT,
        "{r}"
    );
}

const TEXT_FLOOR_ANSWERED: f64 = 55.0;
const TEXT_FLOOR_SILENT: f64 = 65.0;

/// With the model in `CLIO_MEMORY_MODEL_DIR`: the whole report, the sweep and the floors.
#[test]
#[ignore = "needs a model folder in CLIO_MEMORY_MODEL_DIR; prints the report"]
fn the_recall_bench_with_a_model() {
    let r = evaluate(model_from_env(), true);
    if let Ok(out) = std::env::var("CLIO_MEMORY_EVAL_OUT") {
        std::fs::write(out, serde_json::to_string_pretty(&r).unwrap()).unwrap();
    }
}
