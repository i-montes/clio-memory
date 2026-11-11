#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! Which vector model is used, and what happens when it changes.

mod common;

use std::sync::Arc;

use clio_memory::agent::{Draft, EmbedModel, Memory, Place};
use clio_memory::setup::{GRANITE, POTION};
use clio_memory::{Engine, MemoryType, OpenMode, SystemClock};
use common::HashEmbedder;

fn save(data: &std::path::Path, model: &Arc<EmbedModel>, text: &str) {
    let m = Memory::open_write(data, model, None, "agente").unwrap();
    m.save(
        &Draft::plain(text, MemoryType::Semantic),
        "s",
        Place::Personal,
        false,
    )
    .unwrap();
    m.embed_queue().unwrap();
}

#[test]
fn a_new_model_gets_vectors_for_everything_and_the_old_ones_stay_apart() {
    let t = tempfile::tempdir().unwrap();
    let old = EmbedModel::from_embedder(Arc::new(HashEmbedder::new("viejo", 32)));
    let new = EmbedModel::from_embedder(Arc::new(HashEmbedder::new("nuevo", 32)));
    save(t.path(), &old, "el servidor usa compose");
    save(t.path(), &old, "la base es Postgres 16");
    // The new model's first write fills its vectors for every memory, the old ones included.
    save(t.path(), &new, "los pagos van por Stripe");
    let e = Engine::open(
        &Memory::db_path(t.path()),
        OpenMode::ReadOnly,
        Arc::new(SystemClock),
    )
    .unwrap();
    assert_eq!(
        e.stats().unwrap().vectors,
        2 + 3,
        "the old model's two, the new one's three"
    );
    // A search with the new model compares only the new model's vectors.
    let m = Memory::open_read(t.path(), &new, None, "agente")
        .unwrap()
        .unwrap();
    let (rows, _) = m.search_all("servidor compose", 3).unwrap();
    assert_eq!(rows[0].content, "el servidor usa compose");
}

#[test]
fn the_home_prefers_granite_then_the_static_model() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path();
    assert!(
        EmbedModel::of_home(home).dirs().is_empty(),
        "nothing installed"
    );
    std::fs::create_dir_all(home.join(POTION.dir)).unwrap();
    assert_eq!(EmbedModel::of_home(home).dirs(), [home.join(POTION.dir)]);
    std::fs::create_dir_all(home.join(GRANITE.dir)).unwrap();
    assert_eq!(
        EmbedModel::of_home(home).dirs(),
        [home.join(GRANITE.dir), home.join(POTION.dir)]
    );
}

#[test]
fn a_model_that_cannot_load_gives_way_to_the_next_and_says_why() {
    let t = tempfile::tempdir().unwrap();
    // A Granite folder whose files are not a model (or no ONNX Runtime): it does not load.
    let bad = t.path().join("granite");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join("tokenizer.json"), "no").unwrap();
    std::fs::write(bad.join("model.onnx"), "no").unwrap();
    let only_bad = EmbedModel::with_dirs(vec![bad.clone()]);
    assert!(only_bad.get().is_err());
    // With the static model installed after it, that one is used.
    let potion = dirs_home().join(POTION.dir);
    if !cfg!(feature = "model2vec") || !potion.join("model.safetensors").is_file() {
        eprintln!(
            "skipped the fallback half: no model2vec build or no static model in {}",
            potion.display()
        );
        return;
    }
    let m = EmbedModel::with_dirs(vec![bad, potion]);
    let e = m.get().unwrap().unwrap();
    assert!(e.model_id().starts_with("model2vec:"), "{}", e.model_id());
}

fn dirs_home() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

#[test]
fn granite_is_pinned_by_revision_and_hash() {
    assert_eq!(
        GRANITE.repo,
        "ibm-granite/granite-embedding-311m-multilingual-r2"
    );
    assert_eq!(GRANITE.rev, "44399559930365213510b1ee2eb15ded83374f0e");
    let names: Vec<&str> = GRANITE.files.iter().map(|f| f.local).collect();
    assert_eq!(names, ["tokenizer.json", "model_quint8_avx2.onnx"]);
    assert!(GRANITE.files.iter().all(|f| f.sha256.len() == 64));
    assert_eq!(
        GRANITE.url("model_quint8_avx2.onnx").unwrap(),
        "https://huggingface.co/ibm-granite/granite-embedding-311m-multilingual-r2/resolve/44399559930365213510b1ee2eb15ded83374f0e/onnx/model_quint8_avx2.onnx"
    );
    assert_eq!(POTION.files.len(), 3);
}
