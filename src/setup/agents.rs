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
