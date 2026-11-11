#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! `setup model` also brings the runtime the model needs (ONNX Runtime for the CPU): nothing for
//! the person to install, and nothing heavy in the repository.

use std::path::Path;

use clio_memory::setup::{Archive, OBSOLETE, install_runtime, remove_obsolete, runtime_for};

fn names(a: &[&Archive]) -> Vec<&'static str> {
    a.iter().map(|a| a.into).collect()
}

#[test]
fn only_the_cpu_runtime_where_one_is_published() {
    assert_eq!(names(&runtime_for("linux", "x86_64")), ["ort"]);
    assert_eq!(names(&runtime_for("linux", "aarch64")), ["ort"]);
    assert_eq!(names(&runtime_for("macos", "aarch64")), ["ort"]);
    let cpu = runtime_for("linux", "x86_64")[0];
    assert!(
        cpu.url.ends_with("onnxruntime-linux-x64-1.31.1.tgz"),
        "{}",
        cpu.url
    );
    // A system with no runtime to download: no Granite there, the static model or text instead.
    assert!(runtime_for("windows", "x86_64").is_empty());
}

#[test]
fn every_archive_is_pinned_by_sha256_and_none_is_for_the_gpu() {
    for a in runtime_for("linux", "x86_64")
        .into_iter()
        .chain(runtime_for("linux", "aarch64"))
        .chain(runtime_for("macos", "aarch64"))
    {
        assert_eq!(a.sha256.len(), 64, "{}", a.url);
        assert!(a.url.starts_with("https://"), "{}", a.url);
        assert!(!a.url.contains("gpu"), "{}", a.url);
    }
}

#[test]
fn what_an_older_version_left_is_removed_and_nothing_else() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path();
    let granite = home.join(".cache/clio-models/granite-embedding-311m-multilingual-r2");
    let runtime = home.join(".cache/clio-models/onnxruntime");
    std::fs::create_dir_all(&granite).unwrap();
    std::fs::create_dir_all(runtime.join("ort/lib")).unwrap();
    std::fs::create_dir_all(runtime.join("cudnn/lib")).unwrap();
    std::fs::create_dir_all(runtime.join("cuda-cublas/lib")).unwrap();
    for f in ["model.onnx", "model_quint8_avx2.onnx", "tokenizer.json"] {
        std::fs::write(granite.join(f), "pesos").unwrap();
    }
    std::fs::write(runtime.join("cudnn/lib/libcudnn.so.9"), "lib").unwrap();
    std::fs::write(runtime.join("ort/lib/libonnxruntime.so"), "lib").unwrap();

    let said = remove_obsolete(home);
    assert_eq!(said.len(), 3, "{said:?}");
    assert!(said.iter().all(|s| s.starts_with("quitado")), "{said:?}");
    assert!(!granite.join("model.onnx").exists());
    assert!(!runtime.join("cudnn").exists());
    assert!(!runtime.join("cuda-cublas").exists());
    // What the quantized model runs with stays.
    assert!(granite.join("model_quint8_avx2.onnx").is_file());
    assert!(granite.join("tokenizer.json").is_file());
    assert!(runtime.join("ort/lib/libonnxruntime.so").is_file());
    // Once more: nothing left to remove, nothing said.
    assert!(remove_obsolete(home).is_empty());
    // Every path is under the models' own folder.
    assert!(
        OBSOLETE
            .iter()
            .all(|p| p.starts_with(".cache/clio-models/"))
    );
}

fn sha(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

#[test]
fn an_archive_is_unpacked_once_and_a_wrong_one_replaces_nothing() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("onnxruntime");
    let good = Archive {
        url: "https://example.invalid/ort.tgz",
        sha256: leak(sha(b"archivo")),
        into: "ort",
    };
    let fetched = std::cell::Cell::new(0);
    let fetch = |_: &str, to: &Path| {
        fetched.set(fetched.get() + 1);
        std::fs::write(to, b"archivo").map_err(|e| e.to_string())
    };
    let unpack = |_: &Path, into: &Path| {
        std::fs::create_dir_all(into.join("lib")).unwrap();
        std::fs::write(into.join("lib/libonnxruntime.so"), b"lib").map_err(|e| e.to_string())
    };
    install_runtime(&root, &[&good], &fetch, &unpack).unwrap();
    assert!(root.join("ort/lib/libonnxruntime.so").is_file());
    install_runtime(&root, &[&good], &fetch, &unpack).unwrap();
    assert_eq!(fetched.get(), 1, "a good runtime is not fetched again");

    let bad = Archive {
        sha256: leak(sha(b"otro")),
        ..good
    };
    let e = install_runtime(&root, &[&bad], &fetch, &unpack).unwrap_err();
    assert!(e.contains("sha256"), "{e}");
    assert!(
        root.join("ort/lib/libonnxruntime.so").is_file(),
        "the old one stays"
    );
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1, "no leftovers");
}
