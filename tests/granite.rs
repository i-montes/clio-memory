#![cfg(feature = "granite")]
#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! Granite 311M through ONNX Runtime. These need the model files (`CLIO_MEMORY_GRANITE_DIR`:
//! `tokenizer.json` and `model.onnx` and/or `model_quint8_avx2.onnx`) and an ONNX Runtime
//! library (installed, or `ORT_DYLIB_PATH`); without them they say so and pass.

use std::path::{Path, PathBuf};

use clio_memory::{Embedder, Granite};

fn dir() -> Option<PathBuf> {
    match std::env::var_os("CLIO_MEMORY_GRANITE_DIR") {
        Some(d) => Some(PathBuf::from(d)),
        None => {
            eprintln!("skipped: CLIO_MEMORY_GRANITE_DIR is not set");
            None
        }
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[test]
fn the_same_text_gives_the_same_vector_as_sentence_transformers() {
    let Some(dir) = dir() else { return };
    if !dir.join("model.onnx").is_file() {
        eprintln!("skipped: no model.onnx (fp32)");
        return;
    }
    let g = Granite::load_file(&dir, "model.onnx").unwrap();
    assert_eq!(
        (g.model_id(), g.dim()),
        ("granite-embedding-311m-multilingual-r2@384", 384)
    );
    let refs: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/granite_reference.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let texts: Vec<&str> = refs
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["text"].as_str().unwrap())
        .collect();
    let got = g.embed(&texts).unwrap();
    for (r, v) in refs.as_array().unwrap().iter().zip(&got) {
        let want: Vec<f32> = r["vector"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap() as f32)
            .collect();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-3, "normalized: {norm}");
        let c = cosine(&want, v);
        assert!(c >= 0.999, "{}: cosine {c}", r["text"]);
    }
}

fn jsonl(name: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("bench/embeddings")
            .join(name),
    )
    .unwrap()
    .lines()
    .filter(|l| !l.trim().is_empty())
    .map(|l| serde_json::from_str(l).unwrap())
    .collect()
}

#[test]
fn the_benchmark_ranks_as_in_python() {
    let Some(dir) = dir() else { return };
    let g = Granite::load(&dir).unwrap();
    let mems = jsonl("memories.jsonl");
    let queries = jsonl("queries.jsonl");
    let texts: Vec<&str> = mems.iter().map(|m| m["text"].as_str().unwrap()).collect();
    let docs = g.embed(&texts).unwrap();
    let qs: Vec<&str> = queries.iter().map(|q| q["q"].as_str().unwrap()).collect();
    let qv = g.embed(&qs).unwrap();
    let mut hits = 0;
    for (q, v) in queries.iter().zip(&qv) {
        let best = docs
            .iter()
            .enumerate()
            .max_by(|a, b| cosine(a.1, v).total_cmp(&cosine(b.1, v)))
            .unwrap()
            .0;
        let gold: Vec<i64> = q["gold"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_i64().unwrap())
            .collect();
        hits += usize::from(gold.contains(&mems[best]["id"].as_i64().unwrap()));
    }
    let hit1 = hits as f64 / queries.len() as f64;
    eprintln!("device {}: hit@1 {hit1:.2}", g.device());
    assert!(hit1 >= 0.85, "hit@1 {hit1:.2}");
}

#[test]
fn empty_and_long_texts_do_not_panic() {
    let Some(dir) = dir() else { return };
    let g = Granite::load(&dir).unwrap();
    let long = "palabra ".repeat(2000);
    let v = g.embed(&["", &long]).unwrap();
    assert_eq!(v.len(), 2);
    assert!(
        v.iter()
            .all(|x| x.len() == 384 && x.iter().all(|f| f.is_finite()))
    );
}

#[test]
fn a_missing_model_is_a_clear_error() {
    let t = tempfile::tempdir().unwrap();
    let e = Granite::load(t.path()).err().unwrap().to_string();
    assert!(e.contains("tokenizer.json") || e.contains("model"), "{e}");
}
