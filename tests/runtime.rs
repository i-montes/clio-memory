#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! `setup model` also brings the runtime the model needs (ONNX Runtime, and CUDA's libraries when
//! there is an NVIDIA card): nothing for the person to install, and nothing heavy in the repository.

use std::path::Path;

use clio_memory::setup::{Archive, driver_major, install_runtime, runtime_for};

const LDCONFIG_CUDA: &str = "\tlibcudart.so.13 (libc6,x86-64) => /opt/cuda/lib64/libcudart.so.13
\tlibcublasLt.so.13 (libc6,x86-64) => /opt/cuda/lib64/libcublasLt.so.13
\tlibcublas.so.13 (libc6,x86-64) => /opt/cuda/lib64/libcublas.so.13
\tlibnvrtc.so.13 (libc6,x86-64) => /opt/cuda/lib64/libnvrtc.so.13
\tlibcurand.so.10 (libc6,x86-64) => /opt/cuda/lib64/libcurand.so.10
\tlibcufft.so.12 (libc6,x86-64) => /opt/cuda/lib64/libcufft.so.12
";

fn names(a: &[&Archive]) -> Vec<&'static str> {
    a.iter().map(|a| a.into).collect()
}

#[test]
fn the_driver_version_is_read_from_the_kernel_module() {
    let text = "NVRM version: NVIDIA UNIX Open Kernel Module for x86_64  615.78.08  Release Build  (dvs-builder@U22-I3-AE18-23-3)  Thu Oct 2 2026\nGCC version:  gcc version 15.2.1";
    assert_eq!(driver_major(text), Some(615));
    assert_eq!(driver_major("nada"), None);
}

#[test]
fn without_an_nvidia_card_only_the_cpu_runtime() {
    assert_eq!(names(&runtime_for("linux", "x86_64", None, "")), ["ort"]);
    assert_eq!(names(&runtime_for("linux", "aarch64", None, "")), ["ort"]);
    assert_eq!(names(&runtime_for("macos", "aarch64", None, "")), ["ort"]);
    let cpu = runtime_for("linux", "x86_64", None, "")[0];
    assert!(
        cpu.url.ends_with("onnxruntime-linux-x64-1.31.1.tgz"),
        "{}",
        cpu.url
    );
    // A system with no runtime to download: no Granite there, the static model or text instead.
    assert!(runtime_for("windows", "x86_64", None, "").is_empty());
}

#[test]
fn with_an_nvidia_card_the_gpu_runtime_and_only_the_cuda_libraries_the_system_lacks() {
    let all = runtime_for("linux", "x86_64", Some(615), "");
    assert_eq!(
        names(&all),
        [
            "ort",
            "cudnn",
            "cuda-cudart",
            "cuda-cublas",
            "cuda-nvrtc",
            "cuda-curand",
            "cuda-cufft"
        ]
    );
    assert!(all[0].url.contains("gpu_cuda13"), "{}", all[0].url);
    // This machine has CUDA 13 already: only the runtime and cuDNN come down.
    assert_eq!(
        names(&runtime_for("linux", "x86_64", Some(615), LDCONFIG_CUDA)),
        ["ort", "cudnn"]
    );
    // A driver too old for CUDA 13: the CPU runtime.
    assert_eq!(
        names(&runtime_for("linux", "x86_64", Some(550), "")),
        ["ort"]
    );
    assert!(
        !runtime_for("linux", "x86_64", Some(550), "")[0]
            .url
            .contains("gpu")
    );
}

#[test]
fn every_archive_is_pinned_by_sha256() {
    for a in runtime_for("linux", "x86_64", Some(615), "")
        .into_iter()
        .chain(runtime_for("linux", "x86_64", None, ""))
        .chain(runtime_for("linux", "aarch64", None, ""))
        .chain(runtime_for("macos", "aarch64", None, ""))
    {
        assert_eq!(a.sha256.len(), 64, "{}", a.url);
        assert!(a.url.starts_with("https://"), "{}", a.url);
    }
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
        system_lib: None,
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
