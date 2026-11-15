#![cfg(feature = "granite")]
#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! Granite 311M through ONNX Runtime. These need the model files (`CLIO_MEMORY_GRANITE_DIR`:
//! `tokenizer.json` and `model_quint8_avx2.onnx` and/or `model.onnx`) and an ONNX Runtime
//! library (the one `setup model` puts beside the model, `ORT_DYLIB_PATH`, or the system's);
//! without them they say so and pass.

use std::path::{Path, PathBuf};

use clio_memory::{Embedder, GRANITE_ID, GRANITE_Q8_ID, Granite};

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
    assert_eq!((g.model_id(), g.dim()), (GRANITE_ID, 384));
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

/// Of the benchmark's questions, the share whose memory comes first, and in the first five.
fn hits(g: &Granite) -> (f64, f64) {
    let mems = jsonl("memories.jsonl");
    let queries = jsonl("queries.jsonl");
    let texts: Vec<&str> = mems.iter().map(|m| m["text"].as_str().unwrap()).collect();
    let docs = g.embed(&texts).unwrap();
    let qs: Vec<&str> = queries.iter().map(|q| q["q"].as_str().unwrap()).collect();
    let qv = g.embed(&qs).unwrap();
    let (mut first, mut five) = (0, 0);
    for (q, v) in queries.iter().zip(&qv) {
        let mut order: Vec<usize> = (0..docs.len()).collect();
        order.sort_by(|&a, &b| cosine(&docs[b], v).total_cmp(&cosine(&docs[a], v)));
        let gold: Vec<i64> = q["gold"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_i64().unwrap())
            .collect();
        let rank = order
            .iter()
            .position(|&i| gold.contains(&mems[i]["id"].as_i64().unwrap()))
            .unwrap();
        first += usize::from(rank == 0);
        five += usize::from(rank < 5);
    }
    let n = queries.len() as f64;
    (first as f64 / n, five as f64 / n)
}

#[test]
fn the_benchmark_ranks_as_in_python() {
    let Some(dir) = dir() else { return };
    if !dir.join("model.onnx").is_file() {
        eprintln!("skipped: no model.onnx (fp32)");
        return;
    }
    let (first, five) = hits(&Granite::load_file(&dir, "model.onnx").unwrap());
    eprintln!("full: hit@1 {first:.3}, hit@5 {five:.3}");
    assert!(
        first >= 0.875 && five >= 0.95,
        "hit@1 {first:.3}, hit@5 {five:.3}"
    );
}

// The model `setup model` installs: a few first hits fewer than the full one, the same first five.
#[test]
fn the_quantized_model_finds_the_same_first_five() {
    let Some(dir) = dir() else { return };
    if !dir.join("model_quint8_avx2.onnx").is_file() {
        eprintln!("skipped: no model_quint8_avx2.onnx");
        return;
    }
    let g = Granite::load_file(&dir, "model_quint8_avx2.onnx").unwrap();
    assert_eq!(g.model_id(), GRANITE_Q8_ID);
    let (first, five) = hits(&g);
    eprintln!("quantized: hit@1 {first:.3}, hit@5 {five:.3}");
    assert!(
        first >= 0.825 && five >= 0.95,
        "hit@1 {first:.3}, hit@5 {five:.3}"
    );
}

// The quantized one is taken when both are there.
#[test]
fn the_quantized_model_is_preferred() {
    let Some(dir) = dir() else { return };
    if !dir.join("model_quint8_avx2.onnx").is_file() {
        eprintln!("skipped: no model_quint8_avx2.onnx");
        return;
    }
    assert_eq!(Granite::load(&dir).unwrap().model_id(), GRANITE_Q8_ID);
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
    // Files are checked when it is made, though the model is only read on the first text.
    let t = tempfile::tempdir().unwrap();
    std::fs::write(t.path().join("model_quint8_avx2.onnx"), "x").unwrap();
    let e = Granite::load(t.path()).err().unwrap().to_string();
    assert!(e.contains("tokenizer.json"), "{e}");
    // A file that is not a model, though its name is right.
    std::fs::write(t.path().join("tokenizer.json"), "{}").unwrap();
    let e = Granite::load(t.path()).err().unwrap().to_string();
    assert!(e.contains("not an ONNX model"), "{e}");
    let e = Granite::load_file(t.path(), "otro.onnx")
        .err()
        .unwrap()
        .to_string();
    assert!(e.contains("otro.onnx"), "{e}");
}

#[test]
fn a_dir_without_a_model_is_a_clear_error() {
    let t = tempfile::tempdir().unwrap();
    let e = Granite::load(t.path()).err().unwrap().to_string();
    assert!(e.contains("tokenizer.json") || e.contains("model"), "{e}");
}

/// A session that embedded with the runtime: the process is the one `private_runtime` started, so
/// ONNX Runtime never opened its telemetry database.
#[cfg(target_os = "linux")]
#[test]
fn a_session_that_embedded_holds_no_telemetry_database() {
    use std::io::{BufRead, BufReader, Write};
    let Some(dir) = dir() else { return };
    let Some(models) = dir.parent() else { return };
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".cache")).unwrap();
    std::os::unix::fs::symlink(models, home.path().join(".cache/clio-models")).unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_clio-memory"))
        .arg("mcp")
        .env_remove("ORT_DISABLE_TELEMETRY")
        .env("HOME", home.path())
        .env("CLIO_MEMORY_DATA", home.path().join("data"))
        .current_dir(home.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut ask = |request: &str| {
        writeln!(stdin, "{request}").unwrap();
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        line
    };
    ask(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
    );
    let saved = ask(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"memory_save","arguments":{"text":"Qué: prueba de vectores","kind":"semantic"}}}"#,
    );
    let open: Vec<String> = std::fs::read_dir(format!("/proc/{}/fd", child.id()))
        .unwrap()
        .flatten()
        .filter_map(|e| std::fs::read_link(e.path()).ok())
        .map(|p| p.display().to_string())
        .collect();
    drop(stdin);
    child.wait().unwrap();
    assert!(!saved.contains("sin modelo de vectores"), "{saved}");
    assert!(
        !open.iter().any(|p| p.contains("onnxruntime.db")),
        "{open:?}"
    );
}
