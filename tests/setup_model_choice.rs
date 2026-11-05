#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! Choosing the vector model, what to delete to make the choice stick, and a download that reports
//! its bytes. Everything under a temporary home; the download runs against `file://` URLs.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use clio_memory::setup::model_choice::{
    ModelChoice, curl_get_watching, default_choice, granite_available, installed_best, to_remove,
};
use clio_memory::setup::{GRANITE, POTION, RUNTIME_DIR};

/// Fake files for `dir` under `home`: each one `bytes` long.
fn put(home: &Path, dir: &str, files: &[&str], bytes: usize) {
    let d = home.join(dir);
    std::fs::create_dir_all(&d).unwrap();
    for f in files {
        std::fs::write(d.join(f), vec![b'x'; bytes]).unwrap();
    }
}

fn put_granite(home: &Path) {
    put(home, GRANITE.dir, &["tokenizer.json", "model.onnx"], 10);
}

fn put_potion(home: &Path) {
    put(home, POTION.dir, &["config.json", "tokenizer.json"], 7);
}

#[test]
fn no_granite_on_intel_mac() {
    assert_eq!(default_choice("macos", "x86_64"), ModelChoice::Potion);
    assert!(!granite_available("macos", "x86_64"));
    assert!(granite_available("linux", "x86_64"));
    assert_eq!(default_choice("linux", "x86_64"), ModelChoice::Granite);
    assert_eq!(default_choice("macos", "aarch64"), ModelChoice::Granite);
}

#[test]
fn installed_best_prefers_granite() {
    let t = tempfile::tempdir().unwrap();
    assert_eq!(installed_best(t.path()), ModelChoice::None);
    put_potion(t.path());
    assert_eq!(installed_best(t.path()), ModelChoice::Potion);
    put_granite(t.path());
    assert_eq!(installed_best(t.path()), ModelChoice::Granite);
}

#[test]
fn installed_best_uses_the_criterion_of_the_loader() {
    // `EmbedModel::of_home` takes a folder that exists, whatever it holds: so does this.
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(t.path().join(POTION.dir)).unwrap();
    assert_eq!(installed_best(t.path()), ModelChoice::Potion);
    assert_eq!(
        clio_memory::agent::memory::EmbedModel::of_home(t.path()).dirs(),
        [t.path().join(POTION.dir)]
    );
}

#[test]
fn choosing_potion_over_granite_lists_granite_and_runtime() {
    let t = tempfile::tempdir().unwrap();
    put_granite(t.path());
    put(t.path(), RUNTIME_DIR, &["lib.so"], 100);
    put_potion(t.path());
    let got = to_remove(t.path(), ModelChoice::Potion);
    let paths: Vec<&PathBuf> = got.iter().map(|(p, _)| p).collect();
    assert_eq!(
        paths,
        [&t.path().join(GRANITE.dir), &t.path().join(RUNTIME_DIR)]
    );
    assert_eq!(got[0].1, 20);
    assert_eq!(got[1].1, 100);
}

#[test]
fn choosing_none_also_lists_potion() {
    let t = tempfile::tempdir().unwrap();
    put_granite(t.path());
    put_potion(t.path());
    let got = to_remove(t.path(), ModelChoice::None);
    assert!(got.contains(&(t.path().join(POTION.dir), 14)));
    assert!(got.contains(&(t.path().join(GRANITE.dir), 20)));
}

#[test]
fn choosing_granite_removes_nothing_and_missing_folders_are_skipped() {
    let t = tempfile::tempdir().unwrap();
    put_granite(t.path());
    put_potion(t.path());
    assert!(to_remove(t.path(), ModelChoice::Granite).is_empty());
    // Nothing installed: nothing to remove, whatever the choice.
    let empty = tempfile::tempdir().unwrap();
    assert!(to_remove(empty.path(), ModelChoice::None).is_empty());
}

#[test]
fn to_remove_stays_inside_the_models_folder() {
    let t = tempfile::tempdir().unwrap();
    put_granite(t.path());
    put_potion(t.path());
    put(t.path(), RUNTIME_DIR, &["lib.so"], 5);
    // Something of the person's beside the models is not ours to list.
    put(t.path(), ".cache/clio-models/other", &["keep"], 5);
    let root = t.path().join(".cache/clio-models");
    for choice in [ModelChoice::Granite, ModelChoice::Potion, ModelChoice::None] {
        for (p, _) in to_remove(t.path(), choice) {
            assert!(p.starts_with(&root), "{} is outside", p.display());
            assert_ne!(p, root);
            assert!(!p.ends_with("other"));
        }
    }
}

#[test]
fn granite_size_is_pinned() {
    let sum: u64 = GRANITE.files.iter().map(|f| f.size).sum();
    assert!((1_200_000_000..=1_300_000_000).contains(&sum), "{sum}");
    assert!(POTION.files.iter().all(|f| f.size > 0));
}

#[test]
fn watching_reports_the_final_size() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("src.bin");
    std::fs::write(&src, vec![b'y'; 4321]).unwrap();
    let to = t.path().join("out.bin");
    let seen = RefCell::new(Vec::new());
    curl_get_watching(
        Path::new("curl"),
        &format!("file://{}", src.display()),
        &to,
        &|n| seen.borrow_mut().push(n),
    )
    .unwrap();
    assert_eq!(std::fs::metadata(&to).unwrap().len(), 4321);
    assert_eq!(seen.borrow().last(), Some(&4321));
}

#[test]
fn watching_fails_when_curl_fails() {
    let t = tempfile::tempdir().unwrap();
    let to = t.path().join("out.bin");
    let got = curl_get_watching(
        Path::new("curl"),
        &format!("file://{}", t.path().join("nope").display()),
        &to,
        &|_| {},
    );
    assert!(got.unwrap_err().contains("nope"));
    // And with no curl at all.
    assert!(curl_get_watching(Path::new("/no/such/curl"), "file:///x", &to, &|_| {}).is_err());
}

#[cfg(unix)]
#[test]
fn watching_reports_bytes_while_curl_runs() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    // A curl that writes in two steps, a second apart: its `-o` file is the argument after `-o`.
    let fake = t.path().join("curl");
    std::fs::write(
        &fake,
        "#!/bin/sh\nwhile [ \"$1\" != -o ]; do shift; done\nprintf 12345 > \"$2\"\nsleep 1\nprintf 6789 >> \"$2\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let to = t.path().join("out.bin");
    let seen = RefCell::new(Vec::new());
    // Another test forking while the script was open for writing makes the first exec fail with
    // "Text file busy"; that goes away in a moment.
    for _ in 0..20 {
        match curl_get_watching(&fake, "file:///x", &to, &|n| seen.borrow_mut().push(n)) {
            Err(e) if e.contains("busy") => {
                std::thread::sleep(std::time::Duration::from_millis(50))
            }
            other => {
                other.unwrap();
                break;
            }
        }
    }
    let seen = seen.borrow();
    assert!(seen.contains(&5), "no progress seen while it ran: {seen:?}");
    assert_eq!(seen.last(), Some(&9));
    assert!(seen.windows(2).all(|w| w[0] <= w[1]));
}
