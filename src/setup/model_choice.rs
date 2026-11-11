//! Which vector model to install, what to delete so the choice is the one in use, and the size of
//! each folder the person is asked about.

use std::path::{Path, PathBuf};

use super::{GRANITE, ModelSpec, POTION, RUNTIME_DIR, runtime_for};

/// The model the person picks. `None` is "no model": the memory searches by text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelChoice {
    Granite,
    Potion,
    None,
}

/// Whether a runtime for Granite is published for this system (not for macOS on Intel).
pub fn granite_available(os: &str, arch: &str) -> bool {
    !runtime_for(os, arch).is_empty()
}

/// Granite where it runs; the static model where it does not.
pub fn default_choice(os: &str, arch: &str) -> ModelChoice {
    if granite_available(os, arch) {
        ModelChoice::Granite
    } else {
        ModelChoice::Potion
    }
}

/// Whether every file of `spec` is in its folder under `home`: `install_model` creates the folder
/// before it fetches anything, so a folder alone can be an install that failed or was cancelled.
fn is_installed(home: &Path, spec: &ModelSpec) -> bool {
    let dir = home.join(spec.dir);
    spec.files.iter().all(|f| dir.join(f.local).is_file())
}

/// Granite as versions before 0.9 installed it: the full model and its tokenizer. The person chose
/// Granite; it is not up to date (`up_to_date`), so `setup` brings the quantized model.
fn is_granite_of_before(home: &Path) -> bool {
    let dir = home.join(GRANITE.dir);
    ["tokenizer.json", "model.onnx"]
        .iter()
        .all(|f| dir.join(f).is_file())
}

/// The best model installed in `home`: the first of Granite and Potion with all its files, or
/// Granite as an older version installed it. (The loader, `EmbedModel::of_home`, takes any existing
/// folder and falls back at load time; this has no fallback, so it asks for the files.)
pub fn installed_best(home: &Path) -> ModelChoice {
    if is_installed(home, &GRANITE) || is_granite_of_before(home) {
        ModelChoice::Granite
    } else if is_installed(home, &POTION) {
        ModelChoice::Potion
    } else {
        ModelChoice::None
    }
}

/// Whether `choice` is installed with the files this version installs (an older Granite is not).
pub fn up_to_date(home: &Path, choice: ModelChoice) -> bool {
    match choice {
        ModelChoice::Granite => is_installed(home, &GRANITE),
        ModelChoice::Potion => is_installed(home, &POTION),
        ModelChoice::None => true,
    }
}

/// The folders to delete so `choice` is the model in use, with their size in bytes: only ones that
/// exist, and only the models' own under `~/.cache/clio-models`. Potion over Granite: Granite's
/// folder and the runtime only it needs; none: Potion's too.
pub fn to_remove(home: &Path, choice: ModelChoice) -> Vec<(PathBuf, u64)> {
    let dirs: &[&str] = match choice {
        ModelChoice::Granite => &[],
        ModelChoice::Potion => &[GRANITE.dir, RUNTIME_DIR],
        ModelChoice::None => &[GRANITE.dir, RUNTIME_DIR, POTION.dir],
    };
    dirs.iter()
        .map(|d| home.join(d))
        .filter(|p| p.is_dir())
        .map(|p| {
            let n = dir_size(&p);
            (p, n)
        })
        .collect()
}

/// The bytes of the files under `dir`, without following links.
fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}
