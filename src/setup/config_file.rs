//! Editing another program's JSON configuration without touching what is not ours.
//!
//! The file is read whole, our key is changed in the parsed value, and the value is written back
//! through a temporary file in the same folder. With serde_json's `preserve_order` every other
//! key comes back in the place it had. A file that is not plain JSON (comments, trailing commas)
//! is refused before anything is written: rewriting it would lose what the person put there.

use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The JSON in `path`. A missing, empty or whitespace-only file is `{}`: some agents leave a
/// 0-byte config behind, and that is a config with nothing in it, not a broken one.
pub fn read_json(path: &Path) -> Result<Value, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Value::Object(Default::default()));
        }
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if text.trim().is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    serde_json::from_str(&text).map_err(|e| {
        format!(
            "{} tiene comentarios o no es JSON: edítalo a mano o quítale los comentarios ({e})",
            path.display()
        )
    })
}

/// Writes `value` to `path`, pretty-printed with a final newline, through a temporary file in the
/// same folder and a rename. An existing file keeps its permission bits (and a symlink is written
/// through, not replaced); a new one is 0600 and its missing folders are created.
pub fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    // A symlinked config (a dotfiles repository) is edited where it lives.
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = path.parent().ok_or("ruta sin carpeta")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mode = std::fs::metadata(&path)
        .map(|m| m.permissions().mode() & 0o7777)
        .unwrap_or(0o600);
    let mut text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    text.push('\n');

    let name = path.file_name().ok_or("ruta sin nombre")?.to_string_lossy();
    let tmp = dir.join(format!(".{name}.tmp-clio-memory"));
    let wrote = (|| {
        // Created private, so the content is never readable by others while it is half there;
        // the final bits are set explicitly (the umask would trim them otherwise).
        let _ = std::fs::remove_file(&tmp);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.set_permissions(std::fs::Permissions::from_mode(mode))?;
        f.sync_all()?;
        std::fs::rename(&tmp, &path)
    })();
    wrote.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// OpenCode's config: `~/.config/opencode/opencode.json`, or `opencode.jsonc` when that is the
/// only one there.
pub fn opencode_config(home: &Path) -> PathBuf {
    let dir = home.join(".config/opencode");
    let json = dir.join("opencode.json");
    let jsonc = dir.join("opencode.jsonc");
    if !json.exists() && jsonc.exists() {
        jsonc
    } else {
        json
    }
}
