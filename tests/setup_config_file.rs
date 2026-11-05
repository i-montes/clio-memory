#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! Editing an agent's JSON config: only our key changes, everything else keeps its place, a file
//! with comments is refused untouched, and permission bits survive.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use clio_memory::setup::config_file::{opencode_config, read_json, write_json};
use serde_json::{Value, json};

fn keys(v: &Value) -> Vec<&str> {
    v.as_object().unwrap().keys().map(String::as_str).collect()
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
#[cfg_attr(
    not(feature = "cli"),
    ignore = "the order is kept with serde_json's preserve_order, which `cli` turns on"
)]
fn untouched_keys_keep_their_order() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("opencode.json");
    std::fs::write(
        &file,
        r#"{"$schema":"x","theme":"system","mcp":{"other":{"type":"local","command":["a"]}},"autoupdate":false}"#,
    )
    .unwrap();

    let mut v = read_json(&file).unwrap();
    v["mcp"]["clio-memory"] = json!({"type": "local", "command": ["/bin/clio-memory", "mcp"]});
    write_json(&file, &v).unwrap();

    let back = read_json(&file).unwrap();
    assert_eq!(keys(&back), ["$schema", "theme", "mcp", "autoupdate"]);
    assert_eq!(keys(&back["mcp"]), ["other", "clio-memory"]);
    assert_eq!(
        back["mcp"]["other"],
        json!({"type": "local", "command": ["a"]})
    );
    // Pretty-printed, with a final newline.
    assert!(std::fs::read_to_string(&file).unwrap().ends_with("}\n"));
}

#[test]
fn empty_file_is_an_empty_object() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("mcp_config.json");
    std::fs::write(&file, "").unwrap();
    assert_eq!(read_json(&file).unwrap(), json!({}));
    std::fs::write(&file, " \n\t\n").unwrap();
    assert_eq!(read_json(&file).unwrap(), json!({}));
    assert_eq!(
        read_json(&dir.path().join("missing.json")).unwrap(),
        json!({})
    );
}

#[test]
fn comments_are_refused_and_nothing_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("opencode.jsonc");
    let text = "{\n  // x\n  \"theme\": \"system\"\n}\n";
    std::fs::write(&file, text).unwrap();

    let err = read_json(&file).unwrap_err();
    assert!(err.contains("tiene comentarios o no es JSON"), "{err}");
    assert!(err.contains("quítale los comentarios"), "{err}");
    assert!(err.contains("opencode.jsonc"), "{err}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
}

#[test]
fn mode_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.json");
    std::fs::write(&file, "{}").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

    write_json(&file, &json!({"a": 1})).unwrap();
    assert_eq!(mode(&file), 0o644);
    assert_eq!(read_json(&file).unwrap(), json!({"a": 1}));
}

#[test]
fn a_new_file_is_private_and_its_folders_are_created() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a/b/mcp_config.json");
    write_json(&file, &json!({"mcpServers": {}})).unwrap();
    assert_eq!(mode(&file), 0o600);
    // No temporary left beside it.
    let left: Vec<_> = std::fs::read_dir(file.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(left, ["mcp_config.json"]);
}

#[test]
fn opencode_config_prefers_json_and_falls_back_to_jsonc() {
    let home = tempfile::tempdir().unwrap();
    let base = home.path().join(".config/opencode");
    assert_eq!(opencode_config(home.path()), base.join("opencode.json"));

    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(base.join("opencode.jsonc"), "{}").unwrap();
    assert_eq!(opencode_config(home.path()), base.join("opencode.jsonc"));

    std::fs::write(base.join("opencode.json"), "{}").unwrap();
    assert_eq!(opencode_config(home.path()), base.join("opencode.json"));
}

#[test]
fn a_symlinked_config_is_written_through() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("dotfiles/opencode.json");
    std::fs::create_dir_all(real.parent().unwrap()).unwrap();
    std::fs::write(&real, r#"{"a":1}"#).unwrap();
    let link = dir.path().join("opencode.json");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    write_json(&link, &json!({"a": 2})).unwrap();

    assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
    assert_eq!(read_json(&real).unwrap(), json!({"a": 2}));
}
