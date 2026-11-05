#![allow(clippy::unwrap_used)] // test helpers: panicking is the failure mode

mod common;

use std::path::Path;
use std::process::{Command, Output};
use std::sync::Arc;

use clio_memory::*;
use common::*;

/// The binary, kept away from this machine's agents: `doctor` looks for them in the home and
/// the `PATH`, and here there are none.
fn isolated(cmd: &mut Command) -> &mut Command {
    cmd.env("HOME", "/nonexistent/clio-memory-test-home")
        .env("PATH", "/nonexistent/clio-memory-test-path")
        .env_remove("CLIO_MEMORY_CLAUDE")
        .env_remove("CODEX_HOME")
}

fn run(db: Option<&Path>, args: &[&str]) -> Output {
    // Tests run in parallel threads. While one thread holds an engine open, another can fork a
    // child that inherits a copy of the lock file until its `exec`; the lock then outlives the
    // first thread's `drop(engine)` for a moment. That is the harness, not the product: wait it
    // out. The only tests that want the lock error use the library, not this helper.
    for _ in 0..60 {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_clio-memory"));
        if let Some(db) = db {
            cmd.arg("--db").arg(db);
        }
        let o = isolated(&mut cmd)
            .env("USER", "tester")
            .env("CLIO_MEMORY_DATA", "/nonexistent/clio-memory-test")
            .args(args)
            .output()
            .unwrap();
        let locked = o.status.code() == Some(1)
            && String::from_utf8_lossy(&o.stderr).contains("another process owns the write lock");
        if !locked {
            return o;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_clio-memory"));
    if let Some(db) = db {
        cmd.arg("--db").arg(db);
    }
    isolated(&mut cmd)
        .env("USER", "tester")
        .env("CLIO_MEMORY_DATA", "/nonexistent/clio-memory-test")
        .args(args)
        .output()
        .unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}

fn err(o: &Output) -> String {
    String::from_utf8(o.stderr.clone()).unwrap()
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}

/// A scope "main" with an active memory, a quarantined one and a pending Embed job.
fn populated() -> Fixture {
    let f = fixture_with(Arc::new(HashEmbedder::new("m", 256)));
    f.engine
        .propose(prop(f.scope, f.person, "vive en Medellín"))
        .unwrap();
    quarantined(&f, "dato de fuera");
    f
}

#[test]
fn no_command_or_unknown_command_exits_2_with_usage() {
    let f = populated();
    let o = run(None, &[]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).to_lowercase().contains("usage"), "{}", err(&o));
    let o = run(Some(&f.path), &["frobnicate"]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("frobnicate"));
    // Without --db the shared file is used: here one that does not exist.
    let o = run(None, &["scopes"]);
    assert_eq!(code(&o), 1);
    assert!(
        err(&o).contains("/nonexistent/clio-memory-test/memory.db"),
        "{}",
        err(&o)
    );
    let o = run(Some(&f.path), &["get", "not-an-id"]);
    assert_eq!(code(&o), 2);
    let o = run(Some(&f.path), &["list"]);
    assert_eq!(code(&o), 2, "list needs a scope");
    let o = run(None, &["--help"]);
    assert_eq!(code(&o), 0);
    assert!(out(&o).contains("clio-memory [--db PATH]") && out(&o).contains("mcp"));
}

#[test]
fn version_prints_the_crate_version() {
    // Scripts (install.sh, the release workflow) compare this line; it must not open the file.
    let dir = tempfile::tempdir().unwrap();
    for flag in ["--version", "-V"] {
        let o = Command::new(env!("CARGO_BIN_EXE_clio-memory"))
            .env("CLIO_MEMORY_DATA", dir.path())
            .arg(flag)
            .output()
            .unwrap();
        assert_eq!(code(&o), 0, "{flag}: {}", err(&o));
        assert_eq!(
            out(&o),
            format!("clio-memory {}\n", env!("CARGO_PKG_VERSION")),
            "{flag}"
        );
    }
    assert!(listing(dir.path()).is_empty());
}

#[test]
fn scopes_stats_list_get_history_review_jobs_print_what_the_library_holds() {
    let f = populated();
    let o = run(Some(&f.path), &["scopes"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(out(&o).contains("main") && out(&o).contains("es"));

    let o = run(Some(&f.path), &["stats"]);
    assert_eq!(code(&o), 0);
    assert!(out(&o).contains("main") && out(&o).contains("operations"));

    let o = run(Some(&f.path), &["list", "main"]);
    assert!(out(&o).contains("vive en Medellín") && out(&o).contains("dato de fuera"));
    let o = run(Some(&f.path), &["list", "main", "--state", "quarantine"]);
    assert!(!out(&o).contains("Medellín") && out(&o).contains("dato de fuera"));
    let o = run(Some(&f.path), &["list", "main", "--state", "bogus"]);
    assert_eq!(code(&o), 2);

    for id in ["1", "m1"] {
        let o = run(Some(&f.path), &["get", id]);
        assert_eq!(code(&o), 0);
        assert!(out(&o).contains("vive en Medellín") && out(&o).contains("Active"));
    }
    let o = run(Some(&f.path), &["get", "99"]);
    assert_eq!(code(&o), 1);
    assert!(err(&o).to_lowercase().contains("not found"));

    let o = run(Some(&f.path), &["history", "1"]);
    assert!(out(&o).contains("Add"));

    let o = run(Some(&f.path), &["review", "main"]);
    assert!(out(&o).contains("dato de fuera") && !out(&o).contains("Medellín"));

    let o = run(Some(&f.path), &["jobs"]);
    assert!(out(&o).contains("Embed") && out(&o).contains("Pending"));
    let o = run(Some(&f.path), &["jobs", "--state", "failed"]);
    assert!(!out(&o).contains("Embed"));

    let o = run(Some(&f.path), &["list", "nope"]);
    assert_eq!(code(&o), 1);
    assert!(err(&o).contains("unknown scope") && err(&o).contains("main"));
}

#[test]
fn json_flag_prints_valid_json() {
    let f = populated();
    let json = |args: &[&str]| -> serde_json::Value {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        let o = run(Some(&f.path), &all);
        assert_eq!(code(&o), 0, "{args:?}: {}", err(&o));
        serde_json::from_str(&out(&o)).unwrap_or_else(|e| panic!("{args:?}: {e}: {}", out(&o)))
    };
    assert_eq!(json(&["scopes"])[0]["name"], "main");
    assert!(json(&["stats"])["file_bytes"].as_u64().unwrap() > 0);
    let doctor = json(&["doctor"]);
    assert_eq!(doctor["ok"], true);
    assert!(doctor["checks"].as_array().unwrap().len() >= 8);
    assert_eq!(json(&["list", "main"]).as_array().unwrap().len(), 2);
    assert_eq!(json(&["get", "1"])["content"], "vive en Medellín");
    assert_eq!(json(&["history", "1"]).as_array().unwrap().len(), 1);
    assert_eq!(json(&["jobs"])[0]["kind"], "Embed");
}

#[test]
fn doctor_exits_0_when_healthy_and_1_when_a_check_fails() {
    let f = populated();
    let o = run(Some(&f.path), &["doctor"]);
    assert_eq!(code(&o), 0, "{}", out(&o));
    rusqlite::Connection::open(&f.path)
        .unwrap()
        .execute("UPDATE memories SET content = 'x' WHERE id = 1", [])
        .unwrap();
    let o = run(Some(&f.path), &["doctor"]);
    assert_eq!(code(&o), 1);
    assert!(out(&o).contains("projection_matches_log") && out(&o).contains("FAIL"));
}

#[test]
fn recall_and_context_work_without_a_model() {
    let f = populated();
    let o = run(Some(&f.path), &["recall", "main", "Medellín"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(out(&o).contains("vive en Medellín"));
    let o = run(Some(&f.path), &["recall", "main", "astronomía"]);
    assert_eq!(code(&o), 0);
    assert!(out(&o).contains("nothing relevant"));
    let o = run(Some(&f.path), &["context", "main", "Medellín"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(out(&o).contains("vive en Medellín"));
}

#[test]
fn read_commands_work_while_another_process_holds_the_writer_lock() {
    let f = populated(); // `f.engine` keeps the writer lock for the whole test
    for args in [
        vec!["scopes"],
        vec!["list", "main"],
        vec!["recall", "main", "Medellín"],
    ] {
        let o = run(Some(&f.path), &args);
        assert_eq!(code(&o), 0, "{args:?}: {}", err(&o));
    }
}

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| !n.ends_with("-wal") && !n.ends_with("-shm"))
        .collect();
    names.sort();
    names
}

#[test]
fn read_commands_never_write() {
    let f = populated();
    let (path, clock) = (f.path.clone(), f.clock.clone());
    let jobs_before = job_rows(&path);
    let dir = path.parent().unwrap().to_owned();
    drop(f.engine);
    let bytes = std::fs::read(&path).unwrap();
    let names = listing(&dir);
    for args in [
        vec!["recall", "main", "Medellín"],
        vec!["context", "main", "Medellín"],
        vec!["list", "main"],
        vec!["stats"],
        vec!["doctor"],
    ] {
        let o = run(Some(&path), &args);
        assert_eq!(code(&o), 0, "{args:?}: {}", err(&o));
    }
    // SQLite itself keeps `-wal`/`-shm` next to a WAL database while someone reads it; `listing`
    // leaves those out, and the main file and every other name must be untouched
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "the file changed");
    assert_eq!(listing(&dir), names, "files appeared or vanished");
    assert_eq!(job_rows(&path), jobs_before, "a job was queued");
    let _ = clock;
}

#[test]
fn missing_database_is_a_clear_error_and_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("none.db");
    let o = run(Some(&db), &["scopes"]);
    assert_eq!(code(&o), 1);
    assert!(err(&o).to_lowercase().contains("not found"), "{}", err(&o));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

// S-25: whoever can register a person can promote; the command line has no way to do it
#[test]
fn the_cli_has_no_way_to_register_actors_create_scopes_or_set_retention() {
    let f = populated();
    for word in [
        "register-actor",
        "register_actor",
        "create-scope",
        "create_scope",
        "set-retention",
        "set_retention",
        "retention",
    ] {
        let o = run(Some(&f.path), &[word, "x"]);
        assert_eq!(code(&o), 2, "{word} should be an unknown command");
    }
}

#[cfg(not(any(feature = "model2vec", feature = "granite")))]
#[test]
fn the_model_flag_needs_a_model_feature() {
    let f = populated();
    let o = run(
        Some(&f.path),
        &["--model", "/nowhere", "recall", "main", "x"],
    );
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("model2vec"));
}

#[cfg(feature = "granite")]
#[test]
fn the_model_flag_takes_a_granite_folder() {
    let Some(dir) = std::env::var_os("CLIO_MEMORY_GRANITE_DIR") else {
        eprintln!("skipped: CLIO_MEMORY_GRANITE_DIR is not set");
        return;
    };
    let f = populated();
    let o = run(
        Some(&f.path),
        &["--model", dir.to_str().unwrap(), "recall", "main", "x"],
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
}

#[test]
fn terminal_escapes_in_memories_are_not_printed_raw() {
    let f = populated();
    f.engine
        .propose(prop(
            f.scope,
            f.person,
            "hola \u{1b}[31mrojo\u{1b}[0m amigo\r",
        ))
        .unwrap();
    for args in [
        vec!["list", "main"],
        vec!["get", "3"],
        vec!["recall", "main", "rojo"],
        vec!["context", "main", "rojo"],
        vec!["history", "3"],
    ] {
        let o = run(Some(&f.path), &args);
        assert_eq!(code(&o), 0, "{args:?}: {}", err(&o));
        assert!(!o.stdout.contains(&0x1b), "{args:?} printed a raw ESC");
        assert!(!o.stdout.contains(&b'\r'), "{args:?} printed a raw CR");
    }
    let o = run(Some(&f.path), &["list", "main"]);
    assert!(out(&o).contains("rojo"), "the text itself still shows");
    // JSON escapes control characters on its own
    let o = run(Some(&f.path), &["--json", "get", "3"]);
    assert!(!o.stdout.contains(&0x1b));
}

// ---- write commands ----

/// `populated()` with the engine closed, so the CLI can take the writer lock.
/// Memory 1 is active, memory 2 is quarantined; operations 1 and 2 added them, 3 forgot memory 2.
fn closed() -> (tempfile::TempDir, std::path::PathBuf) {
    let Fixture {
        engine, path, _dir, ..
    } = populated();
    drop(engine);
    (_dir, path)
}

fn state_of(db: &Path, id: &str) -> String {
    let o = run(Some(db), &["--json", "get", id]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    serde_json::from_str::<serde_json::Value>(&out(&o)).unwrap()["state"]
        .as_str()
        .unwrap()
        .to_string()
}

// S-25: promote, archive and revert are done by a person actor, and are in the history under that name
#[test]
fn promote_archive_revert_work_as_a_person() {
    let (_dir, db) = closed();
    let o = run(Some(&db), &["promote", "2"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert_eq!(state_of(&db, "2"), "Active");
    let o = run(Some(&db), &["history", "m2"]);
    assert!(
        out(&o).contains("Promote") && out(&o).contains("cli:tester"),
        "{}",
        out(&o)
    );

    let o = run(Some(&db), &["archive", "1"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert_eq!(state_of(&db, "1"), "Archived");
    // operations 1 and 2 were the adds, 3 the forget, 4 the promote, 5 the archive
    let o = run(Some(&db), &["revert", "5"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert_eq!(state_of(&db, "1"), "Active");

    assert_eq!(
        code(&run(Some(&db), &["promote", "1"])),
        1,
        "already active"
    );
    assert_eq!(code(&run(Some(&db), &["revert", "999"])), 1);
    assert_eq!(code(&run(Some(&db), &["revert", "x"])), 2);
}

#[test]
fn feedback_through_the_cli_can_flag_a_memory() {
    let (_dir, db) = closed();
    let o = run(Some(&db), &["feedback", "1", "useful"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(out(&o).contains("0.60"), "{}", out(&o));
    for _ in 0..3 {
        assert_eq!(code(&run(Some(&db), &["feedback", "1", "wrong"])), 0);
    }
    assert_eq!(state_of(&db, "1"), "Quarantine");
    assert!(out(&run(Some(&db), &["history", "1"])).contains("Flag"));
    assert_eq!(
        code(&run(Some(&db), &["feedback", "1", "wrong"])),
        1,
        "not active any more"
    );
    assert_eq!(code(&run(Some(&db), &["feedback", "1", "meh"])), 2);
}

fn job_states(db: &Path) -> Vec<(String, String)> {
    job_rows(db)
}

#[test]
fn run_jobs_applies_queued_work_and_leaves_heavy_jobs_unless_asked() {
    let f = fixture(); // no embedder, so no Embed jobs
    f.engine
        .propose(prop(f.scope, f.person, "vive en Medellín"))
        .unwrap();
    f.engine
        .recall(f.scope, "Medellín", &RecallOpts::default())
        .unwrap(); // queues a Touch
    f.engine.enqueue_vacuum().unwrap();
    let (path, _dir) = (f.path.clone(), f._dir);
    drop(f.engine);

    let o = run(Some(&path), &["run-jobs"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(
        out(&o).contains("done 1") && out(&o).contains("skipped heavy 1"),
        "{}",
        out(&o)
    );
    assert!(job_states(&path).contains(&("Touch".into(), "Done".into())));
    assert!(job_states(&path).contains(&("Vacuum".into(), "Pending".into())));

    let o = run(Some(&path), &["run-jobs", "--heavy"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(job_states(&path).contains(&("Vacuum".into(), "Done".into())));
}

#[test]
fn run_jobs_will_not_fail_embed_jobs_it_cannot_run() {
    let (_dir, db) = closed(); // an embedder queued an Embed job; the CLI has no model
    let o = run(Some(&db), &["run-jobs"]);
    assert_eq!(code(&o), 1);
    assert!(err(&o).contains("--model"), "{}", err(&o));
    assert_eq!(
        job_states(&db),
        [("Embed".to_string(), "Pending".to_string())]
    );
}

// S-23: a backup made from the command line is 0600 and never overwrites
#[test]
fn backup_creates_a_private_copy_that_opens() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, db) = closed(); // has a pending Embed job the backup must not run
    let copy = dir.path().join("copy.db");
    let o = run(Some(&db), &["backup", copy.to_str().unwrap()]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert_eq!(
        std::fs::metadata(&copy).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(code(&run(Some(&copy), &["doctor"])), 0);
    assert!(out(&run(Some(&copy), &["list", "main"])).contains("vive en Medellín"));
    assert_eq!(
        job_states(&db),
        [
            ("Embed".to_string(), "Pending".to_string()),
            ("Backup".to_string(), "Done".to_string()),
        ]
    );

    // never over an existing file, and without leaving a failed job behind
    let o = run(Some(&db), &["backup", copy.to_str().unwrap()]);
    assert_eq!(code(&o), 1);
    assert!(err(&o).contains("overwrite"), "{}", err(&o));
    assert!(out(&run(Some(&db), &["jobs", "--state", "failed"])).is_empty());
}

// S-23: an export made from the command line is 0600
#[test]
fn export_then_import_through_the_cli_round_trips() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, db) = closed();
    let dump = dir.path().join("dump.jsonl");
    let o = run(Some(&db), &["export", dump.to_str().unwrap()]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert_eq!(
        std::fs::metadata(&dump).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let restored = dir.path().join("restored.db");
    let o = run(Some(&restored), &["import", dump.to_str().unwrap()]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert_eq!(code(&run(Some(&restored), &["doctor"])), 0);
    assert_eq!(
        out(&run(Some(&restored), &["list", "main"])),
        out(&run(Some(&db), &["list", "main"]))
    );
    // exporting the restored file gives the same lines (only the header carries a time)
    let again = dir.path().join("again.jsonl");
    assert_eq!(
        code(&run(Some(&restored), &["export", again.to_str().unwrap()])),
        0
    );
    let body = |p: &Path| -> Vec<String> {
        std::fs::read_to_string(p)
            .unwrap()
            .lines()
            .skip(1)
            .map(String::from)
            .collect()
    };
    assert_eq!(body(&dump), body(&again));
    // importing again is refused: the file has data now
    let o = run(Some(&restored), &["import", dump.to_str().unwrap()]);
    assert_eq!(code(&o), 1);
}

#[test]
fn export_refuses_to_overwrite() {
    let (dir, db) = closed();
    let dump = dir.path().join("dump.jsonl");
    std::fs::write(&dump, "precious").unwrap();
    let o = run(Some(&db), &["export", dump.to_str().unwrap()]);
    assert_eq!(code(&o), 1);
    assert!(err(&o).contains("overwrite"), "{}", err(&o));
    assert_eq!(std::fs::read_to_string(&dump).unwrap(), "precious");
}

#[test]
fn import_memories_reports_rejected_lines_and_exits_1() {
    let (dir, db) = closed();
    let file = dir.path().join("loose.jsonl");
    std::fs::write(
        &file,
        "{\"content\":\"usa Rust\"}\nthis is not json\n{\"content\":\"tiene un perro\"}\n",
    )
    .unwrap();
    let o = run(
        Some(&db),
        &["import-memories", "main", file.to_str().unwrap()],
    );
    assert_eq!(code(&o), 1, "{}", err(&o));
    assert!(out(&o).contains("line 2"), "{}", out(&o));
    assert!(
        out(&o).contains("read 3") && out(&o).contains("added 2"),
        "{}",
        out(&o)
    );
    let list = out(&run(Some(&db), &["list", "main", "--state", "active"]));
    assert!(list.contains("usa Rust") && list.contains("tiene un perro"));
    let actors = out(&run(Some(&db), &["--json", "stats"]));
    assert!(!actors.is_empty());

    // a clean file exits 0; --origin user vouches for it
    std::fs::write(&file, "{\"content\":\"es zurdo\"}\n").unwrap();
    let o = run(
        Some(&db),
        &[
            "import-memories",
            "main",
            file.to_str().unwrap(),
            "--origin",
            "user",
        ],
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(out(&o).contains("added 1"), "{}", out(&o));
    assert_eq!(
        code(&run(
            Some(&db),
            &[
                "import-memories",
                "main",
                file.to_str().unwrap(),
                "--origin",
                "bogus"
            ]
        )),
        2
    );
    assert_eq!(
        code(&run(
            Some(&db),
            &["import-memories", "nope", file.to_str().unwrap()]
        )),
        1
    );
}

#[test]
fn write_commands_fail_clearly_while_another_process_holds_the_writer_lock() {
    let f = populated(); // keeps the writer lock
    for args in [
        vec!["promote", "2"],
        vec!["run-jobs"],
        vec!["feedback", "1", "useful"],
    ] {
        let o = run(Some(&f.path), &args);
        assert_eq!(code(&o), 1, "{args:?}");
        assert!(err(&o).contains("lock"), "{args:?}: {}", err(&o));
    }
    // reading is still fine, and so is exporting (it only reads)
    let dump = f.path.with_file_name("dump.jsonl");
    assert_eq!(
        code(&run(Some(&f.path), &["export", dump.to_str().unwrap()])),
        0
    );
}

#[test]
fn write_commands_do_not_create_a_missing_database() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("none.db");
    let o = run(Some(&db), &["promote", "1"]);
    assert_eq!(code(&o), 1);
    assert!(err(&o).to_lowercase().contains("not found"), "{}", err(&o));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn actor_flag_must_name_a_person() {
    let (_dir, db) = closed();
    let o = run(Some(&db), &["--actor", "bot", "promote", "2"]); // "bot" is an agent
    assert_eq!(code(&o), 1);
    assert!(err(&o).contains("person"), "{}", err(&o));
    assert_eq!(state_of(&db, "2"), "Quarantine");
    // an unknown name becomes a person
    let o = run(Some(&db), &["--actor", "marta", "promote", "2"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(out(&run(Some(&db), &["history", "2"])).contains("marta"));
}

#[test]
fn the_default_actor_is_created_once_and_reused() {
    let (_dir, db) = closed();
    for _ in 0..2 {
        assert_eq!(code(&run(Some(&db), &["feedback", "1", "useful"])), 0);
    }
    let named = |like: &str| -> i64 {
        rusqlite::Connection::open(&db)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM actors WHERE name = ?1", [like], |r| {
                r.get(0)
            })
            .unwrap()
    };
    assert_eq!(named("cli:tester"), 1);
    // without $USER the name falls back to "unknown"
    let o = Command::new(env!("CARGO_BIN_EXE_clio-memory"))
        .env_remove("USER")
        .arg("--db")
        .arg(&db)
        .args(["feedback", "1", "useful"])
        .output()
        .unwrap();
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert_eq!(named("cli:unknown"), 1);
}

#[test]
fn a_failed_import_into_a_new_file_leaves_nothing_behind() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.jsonl");
    std::fs::write(&bad, "this is not an export\n").unwrap();
    let db = dir.path().join("new.db");
    let o = run(Some(&db), &["import", bad.to_str().unwrap()]);
    assert_eq!(code(&o), 1);
    assert!(err(&o).contains("import rejected"), "{}", err(&o));
    let left: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(left, ["bad.jsonl"]);
    // a missing input file is the same: no database appears
    let o = run(
        Some(&db),
        &["import", dir.path().join("nope").to_str().unwrap()],
    );
    assert_eq!(code(&o), 1);
    assert!(!db.exists());
}

// ---- fixes from the final review ----

#[test]
fn backup_that_could_not_happen_is_not_reported_as_written() {
    let (dir, db) = closed();
    let copy = dir.path().join("copy.db");
    // a directory where the temporary file should go: an error no retry would fix quickly
    std::fs::create_dir(dir.path().join("copy.db.tmp")).unwrap();
    let o = run(Some(&db), &["backup", copy.to_str().unwrap()]);
    assert_eq!(code(&o), 1, "{}", out(&o));
    assert!(!out(&o).contains("backup written"), "{}", out(&o));
    assert!(!copy.exists());
    assert!(err(&o).to_lowercase().contains("backup"), "{}", err(&o));
}

#[test]
fn run_jobs_sees_a_waiting_embed_job_under_a_thousand_newer_ones() {
    let (_dir, db) = closed(); // one Embed job, queued first
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("BEGIN").unwrap();
    for _ in 0..1001 {
        conn.execute(
            "INSERT INTO jobs(kind, payload, priority, state, run_after, created_at)
             VALUES ('Touch', '{\"ids\":[1],\"at\":1}', 5, 'Pending', 0, 1)",
            [],
        )
        .unwrap();
    }
    conn.execute_batch("COMMIT").unwrap();
    drop(conn);
    let o = run(Some(&db), &["run-jobs"]);
    assert_eq!(code(&o), 1, "{}", out(&o));
    assert!(err(&o).contains("--model"), "{}", err(&o));
    assert!(job_states(&db).contains(&("Embed".to_string(), "Pending".to_string())));
}

#[test]
fn run_jobs_exits_1_when_a_job_failed() {
    let f = fixture(); // no embedder: no Embed jobs
    let taken = f.path.with_file_name("taken.db");
    std::fs::write(&taken, "x").unwrap();
    f.engine.enqueue_backup(&taken).unwrap(); // the destination exists: fails for good
    let (path, _dir) = (f.path.clone(), f._dir);
    drop(f.engine);
    let o = run(Some(&path), &["run-jobs", "--heavy"]);
    assert_eq!(code(&o), 1, "{}", out(&o));
    assert!(out(&o).contains("failed 1"), "{}", out(&o));
}

/// A new file with one scope, made through the library: the CLI cannot do it (S-25).
fn new_db_with_scope(dir: &Path) -> std::path::PathBuf {
    let db = dir.join("new.db");
    let engine = Engine::open(&db, OpenMode::ReadWrite, Arc::new(SystemClock)).unwrap();
    engine.create_scope("work", "es").unwrap();
    db
}

#[test]
fn remember_writes_through_the_normal_rules() {
    let dir = tempfile::tempdir().unwrap();
    let db = new_db_with_scope(dir.path());
    let o = run(
        Some(&db),
        &[
            "remember",
            "work",
            "vive",
            "en",
            "Medellín",
            "--topic",
            "ciudad",
        ],
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(out(&o).contains("Add"), "{}", out(&o));
    let o = run(Some(&db), &["remember", "work", "vive en Medellín"]);
    assert!(out(&o).contains("Noop"), "same content twice: {}", out(&o));
    let listed = out(&run(Some(&db), &["list", "work", "--state", "active"]));
    assert!(listed.contains("Medellín"), "{listed}");
    assert!(out(&run(Some(&db), &["recall", "work", "Medellín"])).contains("Medellín"));
}

#[test]
fn remember_as_an_agent_goes_through_the_agent_rules() {
    let dir = tempfile::tempdir().unwrap();
    let db = new_db_with_scope(dir.path());
    let o = run(
        Some(&db),
        &[
            "--actor",
            "claude",
            "remember",
            "work",
            "el CLI no crea ámbitos",
            "--kind",
            "episodic",
        ],
    );
    assert_eq!(code(&o), 0, "{}", err(&o));
    let got = out(&run(Some(&db), &["get", "1"]));
    assert!(got.contains("Agent") && got.contains("Episodic"), "{got}");
}

#[test]
fn remember_needs_an_existing_scope_and_file() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("x.db");
    let o = run(Some(&db), &["remember", "work", "algo"]);
    assert_eq!(code(&o), 1);
    assert!(err(&o).contains("database not found"), "{}", err(&o));
    assert!(!db.exists(), "remember must not create the file");
    let db = new_db_with_scope(dir.path());
    let o = run(Some(&db), &["remember", "otro", "algo"]);
    assert_eq!(code(&o), 1);
    assert!(err(&o).contains("unknown scope"), "{}", err(&o));
    let o = run(Some(&db), &["remember", "work", "algo", "--kind", "raro"]);
    assert_eq!(code(&o), 2);
}

// S-25: purge is for a person, asks first, and leaves a tombstone that `get` and `history` show
#[test]
fn purge_needs_confirmation_a_person_and_leaves_a_tombstone() {
    let (_dir, db) = closed();
    // no terminal and no --yes: refused, nothing erased
    let o = run(Some(&db), &["purge", "1"]);
    assert_eq!(code(&o), 1);
    assert!(err(&o).contains("--yes"), "{}", err(&o));
    assert_eq!(state_of(&db, "1"), "Active");
    // an agent cannot (the actor "bot" is an agent)
    let o = run(Some(&db), &["--actor", "bot", "purge", "1", "--yes"]);
    assert_eq!(code(&o), 1);
    assert_eq!(state_of(&db, "1"), "Active");

    let o = run(Some(&db), &["purge", "m1", "--yes", "--reason", "pedido"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert_eq!(out(&o).lines().count(), 1, "{}", out(&o));
    assert!(out(&o).contains("purged m1 (op "), "{}", out(&o));

    let got = out(&run(Some(&db), &["get", "1"]));
    assert!(got.contains("Purged") && got.contains("pedido"), "{got}");
    assert!(!got.contains("Medell"), "{got}");
    let json = out(&run(Some(&db), &["--json", "get", "1"]));
    assert!(json.contains("\"reason\":\"pedido\"") && !json.contains("Medell"));
    let hist = out(&run(Some(&db), &["history", "1"]));
    assert!(hist.contains("Purge") && !hist.contains("Medell"), "{hist}");
    let hist = out(&run(Some(&db), &["--json", "history", "1"]));
    assert!(!hist.contains("Medell"), "{hist}");
    assert!(!dump_all(&db).contains("Medell"));

    let o = run(Some(&db), &["purge", "1", "--yes"]);
    assert_eq!(code(&o), 1);
    assert!(err(&o).contains("already purged"), "{}", err(&o));
    let o = run(Some(&db), &["purge", "99", "--yes"]);
    assert!(err(&o).contains("does not exist"), "{}", err(&o));
    assert_eq!(code(&run(Some(&db), &["get", "99"])), 1);
    assert_eq!(code(&run(Some(&db), &["doctor"])), 0);
    assert_eq!(code(&run(Some(&db), &["purge"])), 2);
}

#[test]
fn without_db_the_default_file_is_used() {
    let dir = tempfile::tempdir().unwrap();
    {
        let e = Engine::open(
            &dir.path().join("memory.db"),
            OpenMode::ReadWrite,
            Arc::new(SystemClock),
        )
        .unwrap();
        e.create_scope("main", "es").unwrap();
    }
    let o = Command::new(env!("CARGO_BIN_EXE_clio-memory"))
        .env("CLIO_MEMORY_DATA", dir.path())
        .arg("scopes")
        .output()
        .unwrap();
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(out(&o).contains("main"), "{}", out(&o));
}

#[test]
fn without_home_or_clio_memory_data_nothing_is_created_in_the_current_folder() {
    let dir = tempfile::tempdir().unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_clio-memory"))
        .env_remove("HOME")
        .env_remove("CLIO_MEMORY_DATA")
        .current_dir(dir.path())
        .arg("mcp")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert_eq!(code(&o), 2, "{}", err(&o));
    assert!(err(&o).contains("CLIO_MEMORY_DATA"), "{}", err(&o));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

// ---- the agents: `uninstall` and the agent lines of `doctor` ----

/// A machine for the agents' commands: a temporary home, a `PATH` with only `bin` (a fake
/// `systemctl` there, so the real one is never touched), and the memory in `data`.
struct Machine {
    _t: tempfile::TempDir,
    home: std::path::PathBuf,
    bin: std::path::PathBuf,
    data: std::path::PathBuf,
    /// This binary as it sees itself: what its MCP entries point at.
    exe: std::path::PathBuf,
}

fn machine() -> Machine {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let bin = t.path().join("bin");
    let data = t.path().join("data");
    for d in [&home, &bin, &data] {
        std::fs::create_dir_all(d).unwrap();
    }
    let systemctl = bin.join("systemctl");
    std::fs::write(&systemctl, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&systemctl, std::fs::Permissions::from_mode(0o755)).unwrap();
    {
        let e = Engine::open(
            &data.join("memory.db"),
            OpenMode::ReadWrite,
            Arc::new(SystemClock),
        )
        .unwrap();
        e.create_scope("main", "es").unwrap();
    }
    let exe = std::fs::canonicalize(env!("CARGO_BIN_EXE_clio-memory")).unwrap();
    Machine {
        _t: t,
        home,
        bin,
        data,
        exe,
    }
}

impl Machine {
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_clio-memory"))
            .env("HOME", &self.home)
            .env("PATH", &self.bin)
            .env("CLIO_MEMORY_DATA", &self.data)
            .env_remove("CLIO_MEMORY_CLAUDE")
            .env_remove("CLIO_MEMORY_BIN_DIR")
            .env_remove("CODEX_HOME")
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap()
    }

    /// OpenCode connected to this binary, with its skill.
    fn opencode_connected(&self) -> std::path::PathBuf {
        let config = self.home.join(".config/opencode/opencode.json");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(
            &config,
            serde_json::json!({"mcp": {"clio-memory": {"type": "local", "command": [self.exe, "mcp"], "enabled": true}}})
                .to_string(),
        )
        .unwrap();
        let skill = self
            .home
            .join(".config/opencode/skills/clio-memory/SKILL.md");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        std::fs::write(&skill, clio_memory::setup::SKILL).unwrap();
        config
    }
}

#[test]
fn uninstall_data_needs_yes_without_tty() {
    let m = machine();
    let config = m.opencode_connected();
    let before = std::fs::read(&config).unwrap();
    let o = m.run(&["uninstall", "--data"]);
    assert!(matches!(code(&o), 1 | 2), "{}", err(&o));
    assert!(err(&o).contains("--yes"), "{}", err(&o));
    assert!(
        m.data.join("memory.db").is_file(),
        "the memory is still there"
    );
    assert_eq!(std::fs::read(&config).unwrap(), before, "nothing was done");
}

#[test]
fn uninstall_with_data_and_yes_takes_everything_out() {
    let m = machine();
    let config = m.opencode_connected();
    let models = m.home.join(".cache/clio-models/potion-multilingual-128M");
    std::fs::create_dir_all(&models).unwrap();
    std::fs::write(models.join("model.safetensors"), "pesos").unwrap();
    let o = m.run(&["uninstall", "--data", "--models", "--yes"]);
    assert_eq!(code(&o), 0, "{}{}", out(&o), err(&o));
    assert!(!m.data.join("memory.db").exists(), "{}", out(&o));
    assert!(m.data.is_dir(), "the folder itself stays");
    assert!(!m.home.join(".cache/clio-models").exists());
    assert!(m.home.is_dir());
    let left: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert!(left.get("mcp").is_none(), "{left}");
    assert!(!m.home.join(".config/opencode/skills/clio-memory").exists());
    assert!(out(&o).contains("✔ OpenCode"), "{}", out(&o));
}

#[test]
fn uninstall_without_data_keeps_the_memory_and_takes_the_agents_listed() {
    let m = machine();
    let config = m.opencode_connected();
    let o = m.run(&["uninstall", "--agents", "antigravity"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(
        std::fs::read_to_string(&config)
            .unwrap()
            .contains("clio-memory")
    );
    let o = m.run(&["uninstall", "--agents", "opencode,antigravity"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(
        !std::fs::read_to_string(&config)
            .unwrap()
            .contains("clio-memory")
    );
    assert!(m.data.join("memory.db").is_file());
    let o = m.run(&["uninstall", "--agents", "vscode"]);
    assert_eq!(code(&o), 2);
    assert!(err(&o).contains("vscode"), "{}", err(&o));
}

#[test]
fn doctor_lists_agents() {
    use std::os::unix::fs::PermissionsExt;
    let m = machine();
    m.opencode_connected();
    // Gemini CLI is there, not connected; Antigravity points at an older binary.
    let gemini = m.bin.join("gemini");
    std::fs::write(&gemini, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&gemini, std::fs::Permissions::from_mode(0o755)).unwrap();
    let old = m.home.join(".cargo/bin/clio-memory");
    let antigravity = m.home.join(".gemini/config/mcp_config.json");
    std::fs::create_dir_all(antigravity.parent().unwrap()).unwrap();
    std::fs::write(
        &antigravity,
        serde_json::json!({"mcpServers": {"clio-memory": {"command": old, "args": ["mcp"]}}})
            .to_string(),
    )
    .unwrap();
    let o = m.run(&["doctor"]);
    let text = out(&o);
    assert_eq!(code(&o), 1, "{text}");
    assert!(text.contains("ok   agente opencode: conectado\n"), "{text}");
    assert!(
        text.contains("--   agente gemini: no conectado\n"),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "FAIL agente antigravity: el MCP apunta a {}\n",
            old.display()
        )),
        "{text}"
    );
    assert!(!text.contains("agente claude-code"), "{text}");
    // The agent lines come after the checks.
    assert!(
        text.find("ok   agente").unwrap() > text.find("info sueño").unwrap(),
        "{text}"
    );
    // Only FAIL changes the exit code.
    std::fs::remove_file(&antigravity).unwrap();
    let o = m.run(&["doctor"]);
    assert_eq!(code(&o), 0, "{}", out(&o));
    let o = m.run(&["--json", "doctor"]);
    let v: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["agents"][0]["agent"], "opencode");
    assert_eq!(v["agents"][0]["ok"], true);
}

#[test]
fn uninstall_data_deletes_only_the_memorys_files() {
    let m = machine();
    // The memory's own files, and its sleep's folder...
    let ours = [
        "memory.db-wal",
        "memory.db-shm",
        "memory.db.lock",
        "memory.db.sleep.lock",
    ];
    for name in ours {
        std::fs::write(m.data.join(name), "de la memoria").unwrap();
    }
    std::fs::create_dir_all(m.data.join("sleep")).unwrap();
    std::fs::write(m.data.join("sleep/state.json"), "{}").unwrap();
    // ...beside what Clio keeps in the same folder.
    let foreign = [
        ("vault.bin", &b"\x00secreto\xff"[..]),
        ("runs/x", b"una corrida"),
        ("projects.json", b"{\"p\": 1}"),
    ];
    for (name, bytes) in foreign {
        let p = m.data.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
    let o = m.run(&["uninstall", "--data", "--yes"]);
    assert_eq!(code(&o), 0, "{}{}", out(&o), err(&o));
    assert!(!m.data.join("memory.db").exists(), "{}", out(&o));
    for name in ours {
        assert!(!m.data.join(name).exists(), "{name}");
    }
    assert!(!m.data.join("sleep").exists());
    for (name, bytes) in foreign {
        assert_eq!(std::fs::read(m.data.join(name)).unwrap(), bytes, "{name}");
    }
}
