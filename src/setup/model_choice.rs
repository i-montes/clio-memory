//! Which vector model to install, what to delete so the choice is the one in use, and the size of
//! each folder the person is asked about.

use std::path::{Path, PathBuf};

use super::{GRANITE, POTION, RUNTIME_DIR, runtime_for};

/// The model the person picks. `None` is "no model": the memory searches by text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelChoice {
    Granite,
    Potion,
    None,
}

/// Whether a runtime for Granite is published for this system (not for macOS on Intel).
pub fn granite_available(os: &str, arch: &str) -> bool {
    !runtime_for(os, arch, None, "").is_empty()
}

/// Granite where it runs; the static model where it does not.
pub fn default_choice(os: &str, arch: &str) -> ModelChoice {
    if granite_available(os, arch) {
        ModelChoice::Granite
    } else {
        ModelChoice::Potion
    }
}

/// The best model installed in `home`, by the criterion `EmbedModel::of_home` loads with: its folder
/// exists, Granite's first.
pub fn installed_best(home: &Path) -> ModelChoice {
    if home.join(GRANITE.dir).is_dir() {
        ModelChoice::Granite
    } else if home.join(POTION.dir).is_dir() {
        ModelChoice::Potion
    } else {
        ModelChoice::None
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

pub use super::curl_get_watching;
