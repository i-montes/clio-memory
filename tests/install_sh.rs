#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! `install.sh`: the right binary or nothing. Each test runs the real script with a temporary
//! `HOME`, a fake `uname` (and `sysctl`, `gh`) first in `PATH`, and a release directory on disk
//! served through `file://`. The published "binary" is a script that logs its arguments.

use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use sha2::{Digest, Sha256};

const LINUX_ASSET: &str = "clio-memory-x86_64-unknown-linux-gnu";

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut hex, b| {
            write!(hex, "{b:02x}").unwrap();
            hex
        })
}

fn executable(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

struct Rig {
    _t: tempfile::TempDir,
    home: PathBuf,
    fake: PathBuf,
    releases: PathBuf,
    log: PathBuf,
    /// A readable regular file: stands in for a terminal where a test wants one.
    tty: PathBuf,
}

fn rig() -> Rig {
    let t = tempfile::tempdir().unwrap();
    let rig = Rig {
        home: t.path().join("home"),
        fake: t.path().join("fake"),
        releases: t.path().join("releases"),
        log: t.path().join("log"),
        tty: t.path().join("tty"),
        _t: t,
    };
    for dir in [&rig.home, &rig.fake, &rig.releases] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(&rig.tty, "").unwrap();
    executable(
        &rig.fake.join("uname"),
        "#!/bin/sh\ncase \"$1\" in -s) echo \"$FAKE_S\";; -m) echo \"$FAKE_M\";; esac\n",
    );
    rig
}

impl Rig {
    /// Publishes `asset` and `SHA256SUMS` under `releases/<sub>/` (the binary logs its arguments).
    fn publish_at(&self, sub: &str, asset: &str, sums: Option<&str>) {
        let dir = self.releases.join(sub);
        std::fs::create_dir_all(&dir).unwrap();
        let body = format!("#!/bin/sh\necho \"$*\" >> {}\n", self.log.display());
        std::fs::write(dir.join(asset), &body).unwrap();
        let line = format!("{}  {asset}\n", sha256(body.as_bytes()));
        std::fs::write(dir.join("SHA256SUMS"), sums.unwrap_or(&line)).unwrap();
    }

    fn publish(&self) {
        self.publish_at("latest/download", LINUX_ASSET, None);
    }

    fn bin(&self) -> PathBuf {
        self.home.join(".local/bin/clio-memory")
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    fn run(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let path = format!("{}:/usr/bin:/bin", self.fake.display());
        let url = format!("file://{}", self.releases.display());
        let mut cmd = Command::new("sh");
        cmd.arg(concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh"))
            .args(args)
            .env_clear()
            .env("HOME", &self.home)
            .env("PATH", path)
            .env("CLIO_MEMORY_DOWNLOAD_URL", url)
            .env("CLIO_MEMORY_TTY", "/nonexistent")
            .env("CLIO_MEMORY_SKIP_ATTESTATION", "1")
            .env("FAKE_S", "Linux")
            .env("FAKE_M", "x86_64")
            .stdin(Stdio::null());
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn unsupported_platform_installs_nothing() {
    let r = rig();
    r.publish();
    let o = r.run(&[], &[("FAKE_S", "FreeBSD"), ("FAKE_M", "amd64")]);
    assert_eq!(o.status.code(), Some(1));
    let e = err(&o);
    assert!(
        e.contains("No hay binario para esta máquina · FreeBSD amd64"),
        "{e}"
    );
    assert!(e.contains("Hay para: Linux x86_64, Linux aarch64, macOS arm64, macOS x86_64."));
    assert!(e.contains("No se instaló nada."), "{e}");
    assert!(!r.home.join(".local/bin").exists());
}

#[test]
fn wrong_sum_installs_nothing() {
    let r = rig();
    r.publish_at(
        "latest/download",
        LINUX_ASSET,
        Some(&format!("{}  {LINUX_ASSET}\n", "0".repeat(64))),
    );
    let o = r.run(&[], &[]);
    assert_eq!(o.status.code(), Some(1));
    let e = err(&o);
    assert!(
        e.contains("La suma sha256 no coincide: no se instaló nada."),
        "{e}"
    );
    assert!(!r.bin().exists());
}

#[test]
fn wrong_sum_keeps_the_old_binary() {
    let r = rig();
    std::fs::create_dir_all(r.bin().parent().unwrap()).unwrap();
    executable(&r.bin(), "#!/bin/sh\necho old\n");
    r.publish_at(
        "latest/download",
        LINUX_ASSET,
        Some(&format!("{}  {LINUX_ASSET}\n", "0".repeat(64))),
    );
    let o = r.run(&[], &[]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(
        std::fs::read_to_string(r.bin()).unwrap(),
        "#!/bin/sh\necho old\n"
    );
}

#[test]
fn asset_missing_from_sums_installs_nothing() {
    let r = rig();
    r.publish_at(
        "latest/download",
        LINUX_ASSET,
        Some(&format!("{}  some-other-asset\n", "1".repeat(64))),
    );
    let o = r.run(&[], &[]);
    assert_eq!(o.status.code(), Some(1));
    let e = err(&o);
    assert!(
        e.contains("no está en SHA256SUMS: no se instaló nada."),
        "{e}"
    );
    assert!(!r.bin().exists());
}

#[test]
fn failed_download_leaves_no_half_binary() {
    let r = rig();
    r.publish();
    std::fs::remove_file(r.releases.join("latest/download").join(LINUX_ASSET)).unwrap();
    let o = r.run(&[], &[]);
    assert_ne!(o.status.code(), Some(0));
    assert!(!r.bin().exists());
    let left = std::fs::read_dir(r.home.join(".local/bin"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(left, 0, "nothing may be left in the bin dir");
}

#[test]
fn installs_and_hands_over_without_tty() {
    let r = rig();
    r.publish();
    let o = r.run(&["--agents", "codex"], &[]);
    assert!(o.status.success(), "{}", err(&o));
    let mode = std::fs::metadata(r.bin()).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o755);
    assert_eq!(
        r.log().trim(),
        "setup --from-installer --yes --agents codex"
    );
    let s = out(&o);
    assert!(s.contains("┌  Clio memory · instalador"), "{s}");
    assert!(
        s.contains(&format!("◇  clio-memory {LINUX_ASSET} · sha256 ✓")),
        "{s}"
    );
}

#[test]
fn hands_over_with_a_terminal() {
    let r = rig();
    r.publish();
    let tty = r.tty.to_str().unwrap().to_owned();
    let o = r.run(&["--agents", "codex"], &[("CLIO_MEMORY_TTY", &tty)]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(r.log().trim(), "setup --from-installer --agents codex");
}

#[test]
fn same_binary_is_not_replaced() {
    let r = rig();
    r.publish();
    assert!(r.run(&[], &[]).status.success());
    // Age the installed file: a replacement would carry a fresh mtime.
    let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
    std::fs::File::options()
        .write(true)
        .open(r.bin())
        .unwrap()
        .set_modified(old)
        .unwrap();
    assert!(r.run(&[], &[]).status.success());
    let after = std::fs::metadata(r.bin()).unwrap().modified().unwrap();
    assert_eq!(after, old);
}

#[test]
fn different_binary_is_replaced() {
    let r = rig();
    r.publish();
    std::fs::create_dir_all(r.bin().parent().unwrap()).unwrap();
    executable(&r.bin(), "#!/bin/sh\necho old\n");
    assert!(r.run(&["--x"], &[]).status.success());
    assert!(r.log().contains("setup --from-installer --yes --x"));
}

#[test]
fn pinned_version_uses_that_tag() {
    let r = rig();
    r.publish_at("download/v9.9.9", LINUX_ASSET, None);
    let o = r.run(&[], &[("CLIO_MEMORY_VERSION", "v9.9.9")]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(r.bin().exists());
}

#[test]
fn failed_attestation_installs_nothing_and_shows_what_gh_said() {
    let r = rig();
    r.publish();
    executable(
        &r.fake.join("gh"),
        "#!/bin/sh\ncase \"$1 $2 $3\" in \"auth status \") exit 0;; \"attestation verify --help\") exit 0;; \"attestation verify \"*) echo 'X Verification failed' >&2; echo 'no attestations found' >&2; exit 1;; esac\n",
    );
    let o = r.run(&[], &[("CLIO_MEMORY_SKIP_ATTESTATION", "")]);
    assert_eq!(o.status.code(), Some(1));
    let e = err(&o);
    assert!(
        e.contains(
            "No se pudo verificar la atestación de GitHub de este binario: no se instaló nada."
        ),
        "{e}"
    );
    assert!(
        e.contains("\n   X Verification failed\n   no attestations found\n"),
        "{e}"
    );
    assert!(!r.bin().exists());
}

// A `gh` older than 2.49 has no `gh attestation`: that is no verification, not a forged binary.
#[test]
fn gh_without_attestation_installs_without_verification() {
    let r = rig();
    r.publish();
    let gh_log = r.fake.join("gh.log");
    executable(
        &r.fake.join("gh"),
        &format!(
            "#!/bin/sh\necho \"$*\" >> {}\ncase \"$1\" in auth) exit 0;; esac\necho 'unknown command \"attestation\" for \"gh\"' >&2\nexit 1\n",
            gh_log.display()
        ),
    );
    let o = r.run(&[], &[("CLIO_MEMORY_SKIP_ATTESTATION", "")]);
    assert!(o.status.success(), "{}", err(&o));
    let s = out(&o);
    assert!(s.contains("sin verificación de atestación"), "{s}");
    assert!(s.contains("2.49"), "{s}");
    assert!(r.bin().exists());
    let g = std::fs::read_to_string(gh_log).unwrap();
    assert!(
        !g.lines().any(|l| l.starts_with("attestation verify /")),
        "{g}"
    );
}

/// A `curl` first on the `PATH` that logs its arguments and then runs the real one (or, with
/// `fail`, fails).
fn logging_curl(r: &Rig, fail: bool) -> PathBuf {
    let log = r.fake.join("curl.log");
    let real = ["/usr/bin/curl", "/bin/curl"]
        .into_iter()
        .find(|p| Path::new(p).is_file())
        .unwrap();
    let then = if fail {
        "exit 22".to_owned()
    } else {
        format!("exec {real} \"$@\"")
    };
    executable(
        &r.fake.join("curl"),
        &format!("#!/bin/sh\necho \"$*\" >> {}\n{then}\n", log.display()),
    );
    log
}

// Over https a redirect may not drop to plain http; `file://` (the tests' releases) keeps working.
#[test]
fn https_downloads_refuse_redirects_off_https() {
    let r = rig();
    r.publish();
    let log = logging_curl(&r, true);
    let o = r.run(
        &[],
        &[(
            "CLIO_MEMORY_DOWNLOAD_URL",
            "https://example.invalid/releases",
        )],
    );
    assert_eq!(o.status.code(), Some(1));
    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(!calls.is_empty());
    assert!(
        calls.lines().all(|l| l.contains("--proto-redir =https")),
        "{calls}"
    );
    let r = rig();
    r.publish();
    let log = logging_curl(&r, false);
    let o = r.run(&[], &[]);
    assert!(o.status.success(), "{}", err(&o));
    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(!calls.is_empty());
    assert!(!calls.contains("--proto-redir"), "{calls}");
}

#[test]
fn passing_attestation_installs() {
    let r = rig();
    r.publish();
    let gh_log = r.fake.join("gh.log");
    executable(
        &r.fake.join("gh"),
        &format!("#!/bin/sh\necho \"$*\" >> {}\nexit 0\n", gh_log.display()),
    );
    let o = r.run(&[], &[("CLIO_MEMORY_SKIP_ATTESTATION", "")]);
    assert!(o.status.success(), "{}", err(&o));
    let g = std::fs::read_to_string(gh_log).unwrap();
    assert!(g.contains("attestation verify"), "{g}");
    assert!(g.contains("--repo i-montes/clio-memory"), "{g}");
    assert!(r.bin().exists());
}

#[test]
fn gh_without_session_skips_attestation() {
    let r = rig();
    r.publish();
    executable(
        &r.fake.join("gh"),
        "#!/bin/sh\ncase \"$1 $2\" in \"auth status\") exit 1;; esac\nexit 0\n",
    );
    let o = r.run(&[], &[("CLIO_MEMORY_SKIP_ATTESTATION", "")]);
    assert!(o.status.success(), "{}", err(&o));
    let s = out(&o);
    assert!(
        s.contains("sin verificación de atestación (gh no está o no tiene sesión)"),
        "{s}"
    );
    assert!(r.bin().exists());
}

#[test]
fn hand_over_can_be_skipped() {
    let r = rig();
    r.publish();
    let o = r.run(&[], &[("CLIO_MEMORY_HAND_OVER", "0")]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(r.bin().exists());
    assert_eq!(r.log(), "");
}

#[test]
fn custom_bin_dir_is_used() {
    let r = rig();
    r.publish();
    let dir = r.home.join("elsewhere");
    let o = r.run(&[], &[("CLIO_MEMORY_BIN_DIR", dir.to_str().unwrap())]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(dir.join("clio-memory").exists());
    assert!(!r.bin().exists());
}

#[test]
fn rosetta_uses_arm64() {
    let r = rig();
    r.publish_at("latest/download", "clio-memory-aarch64-apple-darwin", None);
    executable(&r.fake.join("sysctl"), "#!/bin/sh\necho 1\n");
    let o = r.run(&[], &[("FAKE_S", "Darwin"), ("FAKE_M", "x86_64")]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("clio-memory-aarch64-apple-darwin"));
}

#[test]
fn intel_mac_without_rosetta_uses_x86_64() {
    let r = rig();
    r.publish_at("latest/download", "clio-memory-x86_64-apple-darwin", None);
    executable(&r.fake.join("sysctl"), "#!/bin/sh\necho 0\n");
    let o = r.run(&[], &[("FAKE_S", "Darwin"), ("FAKE_M", "x86_64")]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("clio-memory-x86_64-apple-darwin"));
}
