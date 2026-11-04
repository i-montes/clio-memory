//! The most similar pairs of the relations benchmark with Granite as clio-memory runs it, to set
//! the near-duplicate floor: `CLIO_MEMORY_GRANITE_DIR=… cargo run --release --features granite
//! --example relations_floor`.
use clio_memory::{Embedder, Granite};

fn texts(path: &str) -> Vec<(i64, String)> {
    std::fs::read_to_string(path)
        .expect("bench file")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).expect("json line");
            (
                v["id"].as_i64().expect("id"),
                v["text"].as_str().expect("text").to_owned(),
            )
        })
        .collect()
}

fn main() {
    let dir = std::env::var("CLIO_MEMORY_GRANITE_DIR").expect("CLIO_MEMORY_GRANITE_DIR");
    let g = Granite::load(std::path::Path::new(&dir)).expect("load");
    let mut mems = texts("bench/embeddings/memories.jsonl");
    mems.extend(texts("bench/relations/extra.jsonl"));
    let refs: Vec<&str> = mems.iter().map(|(_, t)| t.as_str()).collect();
    let v = g.embed(&refs).expect("embed");
    let mut pairs = Vec::new();
    for i in 0..v.len() {
        for j in i + 1..v.len() {
            let s: f32 = v[i].iter().zip(&v[j]).map(|(a, b)| a * b).sum();
            pairs.push((s, mems[i].0, mems[j].0));
        }
    }
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (s, a, b) in pairs.iter().take(10) {
        println!("{s:.4}  m{a}–m{b}");
    }
}
