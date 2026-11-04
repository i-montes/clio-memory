#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! Vectors are computed as soon as something is saved: the whole queue, not just what this call
//! wrote; and once more when the session ends. `run-jobs` is only for what is left.

mod common;

use std::sync::Arc;

use clio_memory::agent::{Draft, EmbedModel, Memory, Place};
use clio_memory::tools::Session;
use clio_memory::{Engine, MemoryType, OpenMode, SystemClock};
use common::HashEmbedder;
use serde_json::json;

fn model() -> Arc<EmbedModel> {
    EmbedModel::from_embedder(Arc::new(HashEmbedder::new("m", 64)))
}

fn vectors(data: &std::path::Path) -> u64 {
    Engine::open(
        &Memory::db_path(data),
        OpenMode::ReadOnly,
        Arc::new(SystemClock),
    )
    .unwrap()
    .stats()
    .unwrap()
    .vectors
}

/// A memory written by someone with no model (it leaves no vector behind).
fn write_without_model(data: &std::path::Path, text: &str) {
    Memory::open_write(data, &EmbedModel::none(), None, "agente")
        .unwrap()
        .save(
            &Draft::plain(text, MemoryType::Semantic),
            "s",
            Place::Personal,
            false,
        )
        .unwrap();
}

#[test]
fn a_save_computes_the_vectors_of_the_whole_queue_at_once() {
    let t = tempfile::tempdir().unwrap();
    write_without_model(t.path(), "lo guardó Clio sin modelo");
    let mut s = Session::new(t.path().to_path_buf(), None, "mcp:prueba".into()).with_model(model());
    s.call(
        "memory_save",
        &json!({"text": "lo guarda Claude Code", "kind": "semantic"}),
    )
    .unwrap();
    assert_eq!(
        vectors(t.path()),
        2,
        "this save and the one waiting in the queue"
    );
}

#[test]
fn the_end_of_a_session_computes_what_is_left() {
    let t = tempfile::tempdir().unwrap();
    let s = Session::new(t.path().to_path_buf(), None, "mcp:prueba".into()).with_model(model());
    write_without_model(t.path(), "escrito mientras la sesión vivía");
    assert_eq!(vectors(t.path()), 0);
    assert_eq!(s.finish().unwrap(), 1);
    assert_eq!(vectors(t.path()), 1);
    assert_eq!(s.finish().unwrap(), 0, "nothing left the second time");
}

#[test]
fn without_a_model_the_save_says_search_is_by_text_only() {
    let t = tempfile::tempdir().unwrap();
    let mut s = Session::new(t.path().to_path_buf(), None, "mcp:prueba".into());
    let r = s
        .call("memory_save", &json!({"text": "uno", "kind": "semantic"}))
        .unwrap();
    assert!(r.contains("solo por texto"), "{r}");
    assert_eq!(s.finish().unwrap(), 0);
}
