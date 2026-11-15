//! What the sleep keeps between nights, in `<data>/sleep/` (0700, files 0600): its state, one
//! report a night (the last 60), a copy of the file a night (the last 3, so a purged secret does
//! not live on in a copy for long) and when the memory was last read (`last-use`). Nothing of it is
//! in the database.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::Report;

const COPIES_KEPT: usize = 3;
const REPORTS_KEPT: usize = 60;

/// Where the sleep is: the last operation it reviewed, the rules it reviewed with, when it last
/// finished a night and when the model last merged.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SleepState {
    pub cursor: i64,
    pub rules_version: u32,
    pub last_complete: i64,
    /// Memories a cap or a missing piece left for the next night.
    #[serde(default)]
    pub pending: Vec<i64>,
    /// When the model step last ran: at most once in `MERGE_EVERY`.
    #[serde(default)]
    pub last_merge: i64,
    /// Memories that form a near-duplicate group, waiting for the model's next turn.
    #[serde(default)]
    pub awaiting_merge: Vec<i64>,
}

/// The folder the sleep keeps all of this in, beside the memory file.
pub fn dir(data: &Path) -> PathBuf {
    data.join("sleep")
}

/// A missing or unreadable state is a first night.
pub(crate) fn load(data: &Path) -> SleepState {
    std::fs::read_to_string(dir(data).join("state.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn private_dir(data: &Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let d = dir(data);
    let fail = |e: std::io::Error| format!("{}: {e}", d.display());
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&d)
        .map_err(fail)?;
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)).map_err(fail)?;
    Ok(d)
}

/// Written whole or not at all: a temporary file (0600) renamed over the old one.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    let fail = |e: std::io::Error| format!("{}: {e}", path.display());
    let _ = std::fs::remove_file(&tmp);
    let mut f = crate::platform::create_private_file(&tmp).map_err(|e| e.to_string())?;
    f.write_all(bytes).map_err(fail)?;
    f.sync_all().map_err(fail)?;
    std::fs::rename(&tmp, path).map_err(fail)
}

pub(crate) fn save(data: &Path, state: &SleepState) -> Result<(), String> {
    let d = private_dir(data)?;
    let json = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
    write_private(&d.join("state.json"), &json)
}

/// `now` in `<data>/sleep/last-use`: whoever answers a call says the memory is in use. A failure
/// is silent (the call matters more than the mark), and a memory folder that is not there yet is
/// not made for it.
pub fn mark_used(data: &Path, now: i64) {
    if !data.is_dir() {
        return;
    }
    if let Ok(d) = private_dir(data) {
        let _ = write_private(&d.join("last-use"), now.to_string().as_bytes());
    }
}

/// When the memory was last read (`mark_used`); 0 when it never was or it cannot be read.
pub fn last_used(data: &Path) -> i64 {
    std::fs::read_to_string(dir(data).join("last-use"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

pub(crate) fn load_report(data: &Path, night: &str) -> Option<Report> {
    let s = std::fs::read_to_string(dir(data).join(format!("{night}.json"))).ok()?;
    serde_json::from_str(&s).ok()
}

pub(crate) fn save_report(data: &Path, night: &str, report: &Report) -> Result<(), String> {
    let d = private_dir(data)?;
    let json = serde_json::to_vec_pretty(report).map_err(|e| e.to_string())?;
    write_private(&d.join(format!("{night}.json")), &json)?;
    rotate(&d, ".json", REPORTS_KEPT);
    Ok(())
}

/// The newest night's report, if any.
pub fn last_report(data: &Path) -> Option<Report> {
    let night = names(&dir(data), ".json").pop()?;
    load_report(data, night.trim_end_matches(".json"))
}

/// Tonight's copy of the file, from a read-only connection: it never waits for or blocks a
/// writer. A copy already made tonight (by an earlier, interrupted try) is kept.
pub(crate) fn copy(db: &Path, data: &Path, night: &str) -> Result<(), String> {
    use rusqlite::{Connection, OpenFlags};
    let d = private_dir(data)?;
    let path = d.join(format!("memory-{night}.db"));
    if path.is_file() {
        return Ok(());
    }
    let tmp = path.with_extension("db.tmp");
    let _ = std::fs::remove_file(&tmp);
    let made = (|| -> Result<(), String> {
        let e = |e: rusqlite::Error| e.to_string();
        drop(crate::platform::create_private_file(&tmp).map_err(|e| e.to_string())?);
        let source =
            Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(e)?;
        let mut dest = Connection::open(&tmp).map_err(e)?;
        rusqlite::backup::Backup::new(&source, &mut dest)
            .map_err(e)?
            .run_to_completion(256, std::time::Duration::from_millis(10), None)
            .map_err(e)?;
        drop(dest);
        std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
    })();
    let _ = std::fs::remove_file(&tmp);
    made?;
    rotate(&d, ".db", COPIES_KEPT);
    Ok(())
}

/// The night's files (`YYYY-MM-DD…<suffix>`), oldest first: their names sort by date.
fn names(d: &Path, suffix: &str) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(d)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|f| f.file_type().is_ok_and(|t| t.is_file()))
        .map(|f| f.file_name().to_string_lossy().into_owned())
        .filter(|n| {
            n.ends_with(suffix)
                && n.trim_start_matches("memory-")
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit())
        })
        .collect();
    out.sort();
    out
}

fn rotate(d: &Path, suffix: &str, keep: usize) {
    let all = names(d, suffix);
    for old in &all[..all.len().saturating_sub(keep)] {
        let _ = std::fs::remove_file(d.join(old));
    }
}
