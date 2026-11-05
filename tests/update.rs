//! `update` runs the installer from its URL with `--yes`, and fails when it cannot get it or it fails.

use std::fs;

use clio_memory::setup::update;

#[test]
fn update_runs_the_installer_with_yes() {
    let t = tempfile::tempdir().unwrap();
    let out = t.path().join("args");
    let script = t.path().join("install.sh");
    fs::write(
        &script,
        format!("printf '%s' \"$*\" > '{}'\n", out.display()),
    )
    .unwrap();
    update(&format!("file://{}", script.display())).unwrap();
    assert_eq!(fs::read_to_string(&out).unwrap(), "--yes");
}

#[test]
fn update_fails_when_the_installer_is_missing_or_fails() {
    let t = tempfile::tempdir().unwrap();
    let missing = format!("file://{}", t.path().join("nope.sh").display());
    assert!(update(&missing).is_err());
    let failing = t.path().join("fail.sh");
    fs::write(&failing, "exit 3\n").unwrap();
    assert!(update(&format!("file://{}", failing.display())).is_err());
}
