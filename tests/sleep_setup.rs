#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
#![cfg_attr(not(target_os = "linux"), allow(unused_imports, dead_code))] // some tests are Linux-only (systemd)
//! The sleep on a machine: the timer that wakes it at night, the command, the line the agent reads
//! the morning after and what `doctor` says. Nothing here touches the real system: the commands
//! that would talk to systemd are recorded instead.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use clio_memory::setup::{install_sleep, launchd_plist, remove_sleep, sleep_units};
use clio_memory::sleep::Report;
use clio_memory::tools::Session;
use serde_json::json;

const EXE: &str = "/opt/bin/clio-memory";

#[test]
fn the_timer_wakes_every_half_hour_from_23_to_0530() {
    let (service, timer) = sleep_units(Path::new(EXE), &["sleep"]);
    assert!(
        service.contains("ExecStart=\"/opt/bin/clio-memory\" \"sleep\""),
        "{service}"
    );
    assert!(service.contains("Type=oneshot"), "{service}");
    assert!(
        timer.contains("OnCalendar=*-*-* 23,00,01,02,03,04,05:00,30:00"),
        "{timer}"
    );
    assert!(!timer.contains("Persistent"), "{timer}");
    let plist = launchd_plist(Path::new(EXE), &["sleep"]);
    assert_eq!(plist.matches("<key>Hour</key>").count(), 14, "{plist}");
    assert!(
        plist.contains("<string>com.clio-memory.sleep</string>"),
        "{plist}"
    );
}

/// Records the service manager commands instead of running them.
#[derive(Default)]
struct Recorder(Mutex<Vec<String>>);

impl Recorder {
    fn run(&self, args: &[&str]) -> Result<(), String> {
        self.0.lock().unwrap().push(args.join(" "));
        Ok(())
    }
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

#[cfg(target_os = "linux")]
#[test]
fn install_writes_the_units_and_enables_the_timer() {
    let home = tempfile::tempdir().unwrap();
    let rec = Recorder::default();
    let run = |a: &[&str]| rec.run(a);
    let at = install_sleep(home.path(), Path::new(EXE), &["sleep"], &run).unwrap();
    let units = home.path().join(".config/systemd/user");
    assert_eq!(at, units.join("clio-memory-sleep.timer"));
    assert!(units.join("clio-memory-sleep.service").is_file());
    assert_eq!(
        rec.take(),
        vec![
            "systemctl --user daemon-reload",
            "systemctl --user enable --now clio-memory-sleep.timer"
        ]
    );
}

#[cfg(target_os = "linux")]
#[test]
fn remove_disables_and_deletes() {
    let home = tempfile::tempdir().unwrap();
    let rec = Recorder::default();
    let run = |a: &[&str]| rec.run(a);
    install_sleep(home.path(), Path::new(EXE), &["sleep"], &run).unwrap();
    rec.take();
    remove_sleep(home.path(), &run).unwrap();
    let units = home.path().join(".config/systemd/user");
    assert!(!units.join("clio-memory-sleep.timer").exists());
    assert!(!units.join("clio-memory-sleep.service").exists());
    assert_eq!(
        rec.take(),
        vec![
            "systemctl --user disable --now clio-memory-sleep.timer",
            "systemctl --user daemon-reload"
        ]
    );
}

/// A fake `systemctl` on the PATH: records what it was asked.
fn fake_systemctl(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("systemctl");
    std::fs::write(
        &bin,
        format!("#!/bin/sh\necho \"$*\" >> {}\n", dir.join("log").display()),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

#[cfg(target_os = "linux")]
#[test]
fn setup_claude_code_installs_the_sleep_too() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let fake = t.path().join("fake");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&fake).unwrap();
    fake_systemctl(&fake);
    let claude = fake.join("claude");
    std::fs::write(
        &claude,
        "#!/bin/sh\ncase \"$1 $2\" in \"mcp get\") exit 1;; esac\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", fake.display(), std::env::var("PATH").unwrap());
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_clio-memory"))
        .env("HOME", &home)
        .env("PATH", path)
        .env("CLIO_MEMORY_CLAUDE", &claude)
        .args(["setup", "claude-code", "--no-claude-md"])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("sueño"), "{out}");
    let timer = home.join(".config/systemd/user/clio-memory-sleep.timer");
    assert!(timer.is_file());
    let log = std::fs::read_to_string(fake.join("log")).unwrap();
    assert!(
        log.contains("--user enable --now clio-memory-sleep.timer"),
        "{log}"
    );
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn last_night(data: &Path) {
    let r = Report {
        started: now() - 3600,
        ended: now() - 3000,
        complete: true,
        quarantined: vec![(2, "sueño: secreto".into())],
        ..Report::default()
    };
    let dir = data.join("sleep");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{}.json", clio_memory::sleep::night_of(now()))),
        serde_json::to_string(&r).unwrap(),
    )
    .unwrap();
}

#[test]
fn memory_context_starts_with_last_nights_line() {
    let t = tempfile::tempdir().unwrap();
    let mut s = Session::new(t.path().to_path_buf(), None, "mcp:prueba".into());
    s.call(
        "memory_save",
        &json!({"text": "un dato", "kind": "semantic", "title": "dato", "category": "discovery"}),
    )
    .unwrap();
    last_night(t.path());
    let mut s = Session::new(t.path().to_path_buf(), None, "mcp:prueba".into());
    let out = s.call("memory_context", &json!({})).unwrap();
    assert!(
        out.starts_with("Anoche (sueño): 1 a cuarentena (m2 secreto)\n<<<UNTRUSTED-DATA"),
        "{out}"
    );
}

fn cli(data: &Path, args: &[&str]) -> std::process::Output {
    // No `PATH`: `doctor` finds none of this machine's agents.
    std::process::Command::new(env!("CARGO_BIN_EXE_clio-memory"))
        .env("HOME", data)
        .env("PATH", data.join("no-programs"))
        .env_remove("CLIO_MEMORY_CLAUDE")
        .env_remove("CODEX_HOME")
        .arg("--db")
        .arg(data.join("memory.db"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn cli_sleep_dry_run_prints_and_writes_nothing() {
    let t = tempfile::tempdir().unwrap();
    let mut s = Session::new(t.path().to_path_buf(), None, "mcp:prueba".into());
    s.call(
        "memory_save",
        &json!({"text": "un dato", "kind": "semantic", "title": "dato", "category": "discovery"}),
    )
    .unwrap();
    let db = std::fs::read(t.path().join("memory.db")).unwrap();
    let o = cli(t.path(), &["sleep", "--dry-run", "--now"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("solo mirar"), "{out}");
    assert_eq!(std::fs::read(t.path().join("memory.db")).unwrap(), db);
    assert!(!t.path().join("sleep").exists());
    // Without --now, a memory used a moment ago is left alone.
    let o = cli(t.path(), &["sleep"]);
    assert!(o.status.success());
    assert!(String::from_utf8_lossy(&o.stdout).contains("no durmió"));
}

#[test]
fn doctor_says_when_it_last_slept() {
    let t = tempfile::tempdir().unwrap();
    let mut s = Session::new(t.path().to_path_buf(), None, "mcp:prueba".into());
    s.call(
        "memory_save",
        &json!({"text": "un dato", "kind": "semantic", "title": "dato", "category": "discovery"}),
    )
    .unwrap();
    let o = cli(t.path(), &["doctor"]);
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("sueño: todavía no durmió"), "{out}");
    assert!(out.contains("setup sleep instala"), "{out}");
    // Some sleep is installed (Clio's, say): it only has not run yet, nothing to install.
    let unit = if cfg!(target_os = "macos") {
        t.path()
            .join("Library/LaunchAgents/com.clio-memory.sleep.plist")
    } else {
        t.path()
            .join(".config/systemd/user/clio-memory-sleep.service")
    };
    std::fs::create_dir_all(unit.parent().unwrap()).unwrap();
    std::fs::write(&unit, "ExecStart=\"/x/clio\" \"memory\" \"sleep\"\n").unwrap();
    let o = cli(t.path(), &["doctor"]);
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(
        out.contains("sueño: todavía no durmió (el timer ya está instalado)"),
        "{out}"
    );
    assert!(!out.contains("setup sleep"), "{out}");
    last_night(t.path());
    let o = cli(t.path(), &["doctor"]);
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("sueño: última noche"), "{out}");
    assert!(out.contains("completa"), "{out}");
    let o = cli(t.path(), &["sleep", "--report"]);
    assert!(String::from_utf8_lossy(&o.stdout).contains("sueño: secreto"));
}

#[test]
fn the_sleep_runs_at_the_same_priority_for_the_lock() {
    let (service, _) = sleep_units(Path::new(EXE), &["sleep"]);
    // An idle I/O class would hold the writer longer whenever anything else runs.
    assert!(!service.contains("IOSchedulingClass"), "{service}");
}

#[cfg(target_os = "linux")]
#[test]
fn setup_claude_code_leaves_another_sleep_in_place() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let fake = t.path().join("fake");
    std::fs::create_dir_all(&fake).unwrap();
    fake_systemctl(&fake);
    let claude = fake.join("claude");
    std::fs::write(
        &claude,
        "#!/bin/sh\ncase \"$1 $2\" in \"mcp get\") exit 1;; esac\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    let rec = Recorder::default();
    install_sleep(
        &home,
        Path::new("/opt/bin/clio"),
        &["memory", "sleep"],
        &|a| rec.run(a),
    )
    .unwrap();
    let timer = home.join(".config/systemd/user/clio-memory-sleep.service");
    let before = std::fs::read_to_string(&timer).unwrap();
    let said = clio_memory::setup::ClaudeCode {
        home: home.clone(),
        claude,
        exe: PathBuf::from(EXE),
        replace: false,
        claude_md: Some(false),
        ask: &|_| false,
        service: &|a| rec.run(a),
    }
    .run()
    .unwrap();
    assert_eq!(std::fs::read_to_string(&timer).unwrap(), before);
    assert!(said.iter().any(|l| l.contains("otro")), "{said:?}");
}
