//! The five agents clio-memory connects to, and how each one is found on this machine.
//!
//! Detection is by program on the `PATH` (or a known folder for the agents that keep one), never
//! by running anything: a program that exists but fails is the connecting step's problem, which
//! can then say why.

use std::path::PathBuf;

/// An agent clio-memory can connect to. `ALL` is the order everything is shown and done in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AgentId {
    ClaudeCode,
    OpenCode,
    Codex,
    Gemini,
    Antigravity,
}

impl AgentId {
    pub const ALL: [AgentId; 5] = [
        AgentId::ClaudeCode,
        AgentId::OpenCode,
        AgentId::Codex,
        AgentId::Gemini,
        AgentId::Antigravity,
    ];

    /// The name used on the command line (`--agent codex`).
    pub fn key(self) -> &'static str {
        match self {
            AgentId::ClaudeCode => "claude-code",
            AgentId::OpenCode => "opencode",
            AgentId::Codex => "codex",
            AgentId::Gemini => "gemini",
            AgentId::Antigravity => "antigravity",
        }
    }

    /// The name shown to the person.
    pub fn label(self) -> &'static str {
        match self {
            AgentId::ClaudeCode => "Claude Code",
            AgentId::OpenCode => "OpenCode",
            AgentId::Codex => "Codex",
            AgentId::Gemini => "Gemini CLI",
            AgentId::Antigravity => "Antigravity",
        }
    }

    pub fn from_key(s: &str) -> Option<AgentId> {
        AgentId::ALL.into_iter().find(|a| a.key() == s)
    }
}

/// What detection and setup look at: the machine, passed in so tests can fake all of it.
pub struct Env<'a> {
    pub home: PathBuf,
    /// This binary (what the agents' MCP entries point at).
    pub exe: PathBuf,
    /// The directories of `PATH`.
    pub path: Vec<PathBuf>,
    /// `CODEX_HOME`.
    pub codex_home: Option<PathBuf>,
    /// `CLIO_MEMORY_CLAUDE`: the `claude` to drive instead of the one on the `PATH`.
    pub claude: Option<PathBuf>,
    /// `CLIO_MEMORY_BIN_DIR`: where `install.sh` was told to put the binary (else
    /// `~/.local/bin`).
    pub bin_dir: Option<PathBuf>,
    /// Runs the service manager (`systemctl`/`launchctl`) for the sleep timer.
    pub service: &'a dyn Fn(&[&str]) -> Result<(), String>,
}

impl Env<'_> {
    /// The first executable file called `name` in `path`.
    pub fn program(&self, name: &str) -> Option<PathBuf> {
        self.path
            .iter()
            .map(|dir| dir.join(name))
            .find(|p| is_executable(p))
    }
}

/// Another `clio-memory` that comes first on the `PATH` (an old `cargo install`): what runs when
/// the person types `clio-memory`, instead of this binary. `None` when the first one is this
/// binary (or a link to it) or there is none.
pub fn shadowing(env: &Env) -> Option<PathBuf> {
    let canon = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let first = env.program("clio-memory")?;
    (canon(&first) != canon(&env.exe)).then_some(first)
}

/// What to say of `shadowing`: the one that runs, and how to take it away.
pub fn shadowing_note(env: &Env, other: &std::path::Path) -> String {
    format!(
        "al escribir `clio-memory` corre {}, no este ({}): si es de `cargo install`, quítalo con `cargo uninstall clio-memory`",
        other.display(),
        env.exe.display()
    )
}

/// The `cargo install` root of `other` when it is a `cargo install` of this crate:
/// `<root>/bin/clio-memory`, with `<root>/.crates2.json` naming it. `None` for anything else, which
/// is only named, never taken away.
pub fn cargo_install_root(other: &std::path::Path) -> Option<PathBuf> {
    let bin = other.parent()?;
    if other.file_name()? != "clio-memory" || bin.file_name()? != "bin" {
        return None;
    }
    let root = bin.parent()?;
    let text = std::fs::read_to_string(root.join(".crates2.json")).ok()?;
    let records: serde_json::Value = serde_json::from_str(&text).ok()?;
    let ours = records.get("installs")?.as_object()?.iter().any(|(k, v)| {
        k.starts_with("clio-memory ")
            && v.get("bins")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|b| b.iter().any(|n| n == "clio-memory"))
    });
    ours.then(|| root.to_path_buf())
}

/// Takes away `other`, an old `cargo install` of this crate that runs first, with
/// `cargo uninstall` (so cargo's records stay right). `Ok` with what to say when it is gone; `Err`
/// with the note to give instead (it is not ours to take, there is no cargo, or cargo failed).
pub fn take_away_shadow(env: &Env, other: &std::path::Path) -> Result<String, String> {
    let note = || shadowing_note(env, other);
    let root = cargo_install_root(other).ok_or_else(note)?;
    let cargo = Some(root.join("bin/cargo"))
        .filter(|c| is_executable(c))
        .or_else(|| env.program("cargo"))
        .ok_or_else(note)?;
    let done = std::process::Command::new(cargo)
        .arg("uninstall")
        .arg("--root")
        .arg(&root)
        .arg("clio-memory")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !done || other.exists() {
        return Err(note());
    }
    Ok(format!(
        "Quité {}, un clio-memory viejo de `cargo install` que corría antes que este al escribir `clio-memory`",
        other.display()
    ))
}

fn is_executable(p: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// An agent that is installed, and the program or folder that says so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub agent: AgentId,
    pub at: PathBuf,
}

/// The agents present on this machine, in the order of `AgentId::ALL`.
pub fn detect(env: &Env) -> Vec<Found> {
    let folder = |rel: &str| Some(env.home.join(rel)).filter(|p| p.is_dir());
    AgentId::ALL
        .into_iter()
        .filter_map(|agent| {
            let at = match agent {
                // An override is taken as given: if it is wrong, connecting says so.
                AgentId::ClaudeCode => env.claude.clone().or_else(|| env.program("claude")),
                AgentId::OpenCode => env
                    .program("opencode")
                    .or_else(|| folder(".config/opencode")),
                AgentId::Codex => env.program("codex"),
                AgentId::Gemini => env.program("gemini"),
                AgentId::Antigravity => env
                    .program("agy")
                    .or_else(|| env.program("antigravity"))
                    .or_else(|| folder(".gemini/config")),
            };
            at.map(|at| Found { agent, at })
        })
        .collect()
}
