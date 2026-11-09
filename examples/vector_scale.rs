//! How the flat vector search scales (task 305 of Clio's list): N synthetic memories with
//! 384-dimension vectors (Granite's size) from a fake model, then the cost of opening the file,
//! of the first `recall` (it loads the vectors into memory), of a warm `recall`, and the memory
//! the process holds. `cargo run --release --example vector_scale -- 5000 50000 500000`
//!
//! The vectors are random, so the ranking means nothing: this measures time and space only.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clio_memory::{
    ActorKind, Embedder, Engine, MemoryType, OpenMode, Origin, Proposal, Quota, RecallOpts,
    SearchOpts, SystemClock,
};

const DIM: usize = 384;
const WORDS: [&str; 16] = [
    "base",
    "datos",
    "pedido",
    "envío",
    "estación",
    "lectura",
    "cita",
    "pago",
    "correo",
    "prueba",
    "despliegue",
    "caché",
    "índice",
    "cola",
    "servidor",
    "cliente",
];

/// A unit vector made from the text's hash: the same text, the same vector.
struct Fake;

impl Embedder for Fake {
    fn model_id(&self) -> &str {
        "fake-384"
    }
    fn dim(&self) -> usize {
        DIM
    }
    fn embed(&self, texts: &[&str]) -> clio_memory::Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| vector(t)).collect())
    }
}

fn vector(text: &str) -> Vec<f32> {
    // FNV-1a seed, then xorshift.
    let mut x = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    }) | 1;
    let mut v: Vec<f32> = (0..DIM)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 40) as f32 / (1u64 << 24) as f32 - 0.5
        })
        .collect();
    let n = v.iter().map(|a| a * a).sum::<f32>().sqrt();
    v.iter_mut().for_each(|a| *a /= n);
    v
}

fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
        })
        .map_or(f64::NAN, |kb| kb / 1024.0)
}

fn text(i: usize) -> String {
    let w = |k: usize| WORDS[(i / k) % WORDS.len()];
    format!(
        "Recuerdo sintético {i}: la {} del {} usa la {} número {}",
        w(1),
        w(7),
        w(13),
        i % 997
    )
}

fn build(path: &Path, n: usize) -> Duration {
    let start = Instant::now();
    let e = Engine::open(path, OpenMode::ReadWrite, Arc::new(SystemClock))
        .expect("the measurement cannot go on")
        .with_embedder(Arc::new(Fake));
    let scope = e
        .create_scope("clio", "es")
        .expect("the measurement cannot go on");
    let person = e
        .register_actor(
            "persona",
            ActorKind::Person,
            Quota {
                max: u32::MAX,
                window_secs: 1,
            },
        )
        .expect("the measurement cannot go on");
    for i in 0..n {
        e.propose(Proposal {
            scope,
            actor: person,
            kind: MemoryType::Semantic,
            content: text(i),
            topic_key: None,
            origin: Origin::User,
            session: None,
            episode: None,
            source: None,
            valid_from: None,
            valid_to: None,
            title: None,
            category: None,
        })
        .expect("the measurement cannot go on");
        if i % 5000 == 4999 {
            while e.embed_pending(1000).expect("the measurement cannot go on") > 0 {}
        }
    }
    while e.embed_pending(1000).expect("the measurement cannot go on") > 0 {}
    start.elapsed()
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn main() {
    let sizes: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let sizes = if sizes.is_empty() { vec![5000] } else { sizes };
    println!(
        "| memories | build (s) | file (MB) | open (ms) | 1st recall (ms) | warm recall p50 / p95 (ms) | same, text only p50 (ms) | RSS after (MB) | RSS added by the vectors (MB) |"
    );
    println!("|---|---|---|---|---|---|---|---|---|");
    for n in sizes {
        let dir = tempfile::tempdir().expect("the measurement cannot go on");
        let path = dir.path().join("memory.db");
        let built = build(&path, n);
        let file_mb = ["", "-wal"]
            .iter()
            .filter_map(|s| std::fs::metadata(format!("{}{s}", path.display())).ok())
            .map(|m| m.len() as f64)
            .sum::<f64>()
            / 1_048_576.0;

        let before = rss_mb();
        let t = Instant::now();
        let e = Engine::open(&path, OpenMode::ReadOnly, Arc::new(SystemClock))
            .expect("the measurement cannot go on")
            .with_embedder(Arc::new(Fake));
        let open = t.elapsed();
        let scope = e.scopes().expect("the measurement cannot go on")[0].id;
        let opts = RecallOpts {
            search: SearchOpts {
                limit: 5,
                ..SearchOpts::default()
            },
            ..RecallOpts::default()
        };
        let t = Instant::now();
        e.recall(scope, "¿qué base de datos usa el pedido?", &opts)
            .expect("the measurement cannot go on");
        let first = t.elapsed();
        let warm = |e: &Engine| {
            let mut v: Vec<Duration> = (0..50)
                .map(|i| {
                    let q = format!(
                        "¿qué {} usa el {} número {i}?",
                        WORDS[i % 16],
                        WORDS[(i * 5) % 16]
                    );
                    let t = Instant::now();
                    e.recall(scope, &q, &opts)
                        .expect("the measurement cannot go on");
                    t.elapsed()
                })
                .collect();
            v.sort();
            v
        };
        let with_vectors = warm(&e);
        let after = rss_mb();
        // The same questions with no model: what the text search costs on its own.
        let text_only = warm(
            &Engine::open(&path, OpenMode::ReadOnly, Arc::new(SystemClock))
                .expect("the measurement cannot go on"),
        );
        println!(
            "| {n} | {:.1} | {file_mb:.0} | {:.1} | {:.1} | {:.1} / {:.1} | {:.1} | {after:.0} | {:.0} |",
            built.as_secs_f64(),
            ms(open),
            ms(first),
            ms(with_vectors[with_vectors.len() / 2]),
            ms(with_vectors[with_vectors.len() * 95 / 100]),
            ms(text_only[text_only.len() / 2]),
            after - before
        );
    }
}
