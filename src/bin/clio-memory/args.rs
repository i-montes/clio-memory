//! Hand-written argument parsing: the grammar is small and fixed, and this crate takes no
//! dependency just for it.

use std::path::PathBuf;

use clio_memory::setup::agents::AgentId;
use clio_memory::setup::model_choice::ModelChoice;
use clio_memory::{JobState, MemoryId, MemoryType, OpId, Origin, Signal, State};

pub const USAGE: &str = "\
usage: clio-memory [--db PATH] [--actor NAME] [--model DIR] [--json] <command> [args]

agents:
  mcp                         serve the memory tools to an agent (MCP over stdin and stdout); the
                              file is <folder>/memory.db, the project is the current folder's
  setup [--yes] [--agents LIST|all] [--model granite|potion|none]
        [--instructions|--no-instructions] [--replace] [--from-installer]
                              connect clio-memory to the agents on this machine and install the
                              vector model and the nightly sleep; in a terminal it asks, and with
                              --yes or without a terminal it uses the flags and, where missing:
                              every agent found, granite where it runs (else potion; nothing is
                              downloaded when one as good is installed), the instructions line;
                              run again, it keeps what is there: the agents connected, the model
                              installed (none downloaded) and the lines as they are. Without
                              questions, flags only add: nothing is disconnected or deleted
                              (that is `uninstall`). LIST: claude-code,opencode,codex,gemini,
                              antigravity.
                              --from-installer: run by install.sh (says where the binary is and,
                              if its folder is not on the PATH, the line that adds it)
  setup claude-code [--replace] [--claude-md|--no-claude-md]
                              make clio-memory Claude Code's primary memory: register this MCP
                              server (user scope) and install the skill clio-memory; asks before
                              replacing anything of the same name and before adding a line to
                              ~/.claude/CLAUDE.md (the flags answer for it)
  setup sleep [--remove]      install (or remove) the nightly sleep: a user timer that runs
                              `clio-memory sleep` every half hour from 23:00 to 05:30
                              (setup claude-code installs it too)
  uninstall [--agents LIST] [--models] [--data] [--yes]
                              take clio-memory out of the agents: the MCP server, the skill (only
                              ours) and the instructions line; with no --agents, out of all of
                              them, with the nightly sleep and, last, this binary when it is in
                              ~/.local/bin (or $CLIO_MEMORY_BIN_DIR). LIST: claude-code,opencode,
                              codex,gemini,antigravity. --models deletes ~/.cache/clio-models;
                              --data deletes the memory: memory.db (with its -wal, -shm and lock
                              files) and the sleep's folder beside it, and nothing else of that
                              folder; it asks first in a terminal, and without one needs --yes
  sleep [--now] [--dry-run] [--report]
                              one night of sleep: check and copy the file, quarantine secrets
                              and likely attacks, fold repeats, purge what was discarded long
                              ago; only when nobody used the memory for 30 minutes and it did not
                              sleep in the last 20 hours (--now skips both waits, --dry-run looks
                              and changes nothing, --report prints the last night's report)
  setup model [potion]        download the vector model (pinned, sha256-checked): Granite
                              Embedding 311M (GPU or CPU, needs ONNX Runtime installed) to
                              ~/.cache/clio-models/granite-embedding-311m-multilingual-r2, or with
                              potion the static fallback, potion-multilingual-128M

inspect (open the file read-only; never write):
  scopes                      list scopes
  stats                       sizes, counts by state, queue
  doctor                      health checks (before the first memory, a line that there is
                              none yet), then one line per agent found (ok, FAIL, or -- for
                              not connected or unreadable without our skill); exit 1 if one
                              fails
  list SCOPE [--state S]      memories (S: active|quarantine|archived|replaced)
  get ID                      one memory (ID is 12 or m12)
  history ID                  every operation that touched a memory
  review SCOPE                quarantined memories waiting for a person
  jobs [--state S]            background jobs (S: pending|running|done|failed)
  recall SCOPE TEXT...        what the memory would bring up for TEXT
  context SCOPE TEXT...       the prompt context for TEXT (fixed blocks + recall)

change (open the file read-write; fail if another program holds its write lock):
  remember SCOPE TEXT... [--kind K] [--topic T] [--source S]
                              propose one memory through the normal rules (K: working|episodic|
                              semantic|procedural, default semantic); with --actor NAME it is
                              an agent's memory, without it a person's
  promote ID                  accept a quarantined memory
  archive ID                  take a memory out of circulation (nothing is deleted)
  revert OP                   undo one operation (see `history`)
  purge ID [--reason TEXT] [--yes]
                              erase a memory's content for good, from every operation too, and
                              leave a tombstone; asks first in a terminal (--yes skips it, for
                              tests); a person only, and nothing can undo it
  feedback ID useful|wrong    say whether a memory helped; three `wrong` send it to review
  retry-job ID                put a failed job back in the queue
  run-jobs [--heavy]          run queued jobs now (--heavy also runs backup and vacuum)
  backup PATH                 hot copy of the file to a new PATH (mode 0600)
  import PATH                 restore an export into a new, empty file (--db)
  import-memories SCOPE PATH [--origin user|agent|external]
                              load loose memories, one JSON object per line, through the
                              normal rules (default origin external: they wait in quarantine)

export (reads only):
  export PATH                 write the whole memory as JSON Lines to a new PATH (mode 0600)

options:
  --db PATH     the memory file (default: $CLIO_MEMORY_DATA/memory.db, else
                ~/.local/share/clio/memory.db, the one Clio shares)
  --actor NAME  act as this person instead of cli:$USER (write commands)
  --model DIR   local vector model for recall and context: Granite (ONNX) or model2vec
  --json        machine-readable output where supported
  -h, --help    this text
  -V, --version print the version and exit";

#[derive(Debug)]
pub struct UsageError(pub String);

#[derive(Debug)]
pub enum Command {
    /// The MCP server for an agent.
    Mcp,
    /// `setup` with no subcommand: the agents (`all`: every one found), the model and the
    /// instructions line, each `None` when not given (asked, or the default without a terminal).
    Setup {
        yes: bool,
        from_installer: bool,
        agents: Option<Vec<AgentId>>,
        all: bool,
        model: Option<ModelChoice>,
        instructions: Option<bool>,
        replace: bool,
    },
    /// `setup claude-code`: replace what has the same name; the CLAUDE.md line (None: ask).
    SetupClaudeCode {
        replace: bool,
        claude_md: Option<bool>,
    },
    /// `setup model` (Granite) or `setup model potion` (the static model).
    SetupModel {
        potion: bool,
    },
    SetupSleep {
        remove: bool,
    },
    /// `uninstall`: from `agents` (None: everything), and the models and the data on request.
    Uninstall {
        agents: Option<Vec<AgentId>>,
        models: bool,
        data: bool,
        yes: bool,
    },
    Sleep {
        now: bool,
        dry_run: bool,
        report: bool,
    },
    Scopes,
    Stats,
    Doctor,
    List {
        scope: String,
        state: Option<State>,
    },
    Get {
        id: MemoryId,
    },
    History {
        id: MemoryId,
    },
    Review {
        scope: String,
    },
    Jobs {
        state: Option<JobState>,
    },
    Recall {
        scope: String,
        text: String,
    },
    Context {
        scope: String,
        text: String,
    },
    Export {
        path: PathBuf,
    },
    Remember {
        scope: String,
        text: String,
        kind: MemoryType,
        topic: Option<String>,
        source: Option<String>,
    },
    Promote {
        id: MemoryId,
    },
    Archive {
        id: MemoryId,
    },
    Revert {
        op: OpId,
    },
    Purge {
        id: MemoryId,
        reason: String,
        yes: bool,
    },
    Feedback {
        id: MemoryId,
        signal: Signal,
    },
    RetryJob {
        id: i64,
    },
    RunJobs {
        heavy: bool,
    },
    Backup {
        path: PathBuf,
    },
    Import {
        path: PathBuf,
    },
    ImportMemories {
        scope: String,
        path: PathBuf,
        origin: Option<Origin>,
    },
}

impl Command {
    /// Whether the command needs the file open read-write (and so the writer lock).
    pub fn writes(&self) -> bool {
        !matches!(
            self,
            Self::Scopes
                | Self::Stats
                | Self::Doctor
                | Self::List { .. }
                | Self::Get { .. }
                | Self::History { .. }
                | Self::Review { .. }
                | Self::Jobs { .. }
                | Self::Recall { .. }
                | Self::Context { .. }
                | Self::Export { .. }
        )
    }
}

#[derive(Debug)]
pub struct Cli {
    pub db: PathBuf,
    /// Who the write commands act as; `cli:$USER` when absent.
    pub actor: Option<String>,
    pub model: Option<PathBuf>,
    pub json: bool,
    pub command: Command,
}

#[derive(Debug)]
pub enum Parsed {
    Help,
    Version,
    Run(Cli),
}

fn usage<T>(msg: impl Into<String>) -> Result<T, UsageError> {
    Err(UsageError(msg.into()))
}

/// The arguments after the command name: positionals and `--flags`.
struct Rest(Vec<String>);

impl Rest {
    /// Removes `--name VALUE` if present.
    fn flag(&mut self, name: &str) -> Result<Option<String>, UsageError> {
        let Some(i) = self.0.iter().position(|a| a == name) else {
            return Ok(None);
        };
        if i + 1 >= self.0.len() {
            return usage(format!("{name} needs a value"));
        }
        let value = self.0.remove(i + 1);
        self.0.remove(i);
        Ok(Some(value))
    }

    /// Removes every bare `--name` switch: one given twice (install.sh adds a `--yes` of its own)
    /// is the same as once.
    fn switch_all(&mut self, name: &str) -> bool {
        let before = self.0.len();
        self.0.retain(|a| a != name);
        self.0.len() != before
    }

    /// Removes a bare `--name` switch if present.
    fn switch(&mut self, name: &str) -> bool {
        match self.0.iter().position(|a| a == name) {
            Some(i) => {
                self.0.remove(i);
                true
            }
            None => false,
        }
    }

    /// Everything left must be positional.
    fn positionals(self) -> Result<Vec<String>, UsageError> {
        match self.0.iter().find(|a| a.starts_with("--")) {
            Some(bad) => usage(format!("unknown option {bad}")),
            None => Ok(self.0),
        }
    }

    /// No positionals at all.
    fn none(self) -> Result<(), UsageError> {
        match self.positionals()?.first() {
            Some(extra) => usage(format!("unexpected argument {extra:?}")),
            None => Ok(()),
        }
    }

    /// Exactly one positional, called `name` in the error message.
    fn one(self, name: &str) -> Result<String, UsageError> {
        let mut args = self.positionals()?;
        if args.len() != 1 {
            return usage(format!("expected {name}"));
        }
        Ok(args.remove(0))
    }

    /// Exactly two positionals.
    fn two(self, names: &str) -> Result<(String, String), UsageError> {
        let mut args = self.positionals()?;
        if args.len() != 2 {
            return usage(format!("expected {names}"));
        }
        let second = args.remove(1);
        Ok((args.remove(0), second))
    }

    /// One positional followed by free text (joined with spaces, so quoting is optional).
    fn with_text(self, scope: &str) -> Result<(String, String), UsageError> {
        let mut args = self.positionals()?;
        if args.len() < 2 {
            return usage(format!("expected {scope} TEXT"));
        }
        let first = args.remove(0);
        Ok((first, args.join(" ")))
    }
}

pub fn parse_id(s: &str) -> Result<MemoryId, UsageError> {
    let digits = s.strip_prefix(['m', 'M']).unwrap_or(s);
    match digits.parse::<i64>() {
        Ok(n) if n > 0 => Ok(MemoryId(n)),
        _ => usage(format!("{s:?} is not an id (use 12 or m12)")),
    }
}

fn parse_op(s: &str) -> Result<OpId, UsageError> {
    match s.parse::<i64>() {
        Ok(n) if n > 0 => Ok(OpId(n)),
        _ => usage(format!("{s:?} is not an operation id")),
    }
}

fn parse_origin(s: &str) -> Result<Origin, UsageError> {
    match s {
        "user" => Ok(Origin::User),
        "agent" => Ok(Origin::Agent),
        "external" => Ok(Origin::External),
        _ => usage(format!("unknown origin {s:?} (user|agent|external)")),
    }
}

fn parse_kind(s: &str) -> Result<MemoryType, UsageError> {
    match s {
        "working" => Ok(MemoryType::Working),
        "episodic" => Ok(MemoryType::Episodic),
        "semantic" => Ok(MemoryType::Semantic),
        "procedural" => Ok(MemoryType::Procedural),
        _ => usage(format!(
            "unknown kind {s:?} (working|episodic|semantic|procedural)"
        )),
    }
}

fn parse_signal(s: &str) -> Result<Signal, UsageError> {
    match s {
        "useful" => Ok(Signal::Useful),
        "wrong" => Ok(Signal::Wrong),
        _ => usage(format!("unknown signal {s:?} (useful|wrong)")),
    }
}

fn parse_state(s: &str) -> Result<State, UsageError> {
    match s {
        "active" => Ok(State::Active),
        "quarantine" => Ok(State::Quarantine),
        "archived" => Ok(State::Archived),
        "replaced" => Ok(State::Replaced),
        _ => usage(format!(
            "unknown state {s:?} (active|quarantine|archived|replaced)"
        )),
    }
}

fn parse_model_choice(s: &str) -> Result<ModelChoice, UsageError> {
    match s {
        "granite" => Ok(ModelChoice::Granite),
        "potion" => Ok(ModelChoice::Potion),
        "none" => Ok(ModelChoice::None),
        _ => usage(format!("unknown model {s:?} (granite|potion|none)")),
    }
}

fn parse_job_state(s: &str) -> Result<JobState, UsageError> {
    match s {
        "pending" => Ok(JobState::Pending),
        "running" => Ok(JobState::Running),
        "done" => Ok(JobState::Done),
        "failed" => Ok(JobState::Failed),
        _ => usage(format!(
            "unknown job state {s:?} (pending|running|done|failed)"
        )),
    }
}

/// `claude-code,codex`: agents by their keys.
fn parse_agents(list: &str) -> Result<Vec<AgentId>, UsageError> {
    list.split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(|k| {
            AgentId::from_key(k).ok_or_else(|| {
                let keys: Vec<&str> = AgentId::ALL.iter().map(|a| a.key()).collect();
                UsageError(format!("unknown agent {k:?} ({})", keys.join(", ")))
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .and_then(|agents| {
            if agents.is_empty() {
                usage("--agents needs at least one agent")
            } else {
                Ok(agents)
            }
        })
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Parsed, UsageError> {
    let mut args = args.into_iter();
    let (mut db, mut actor, mut model, mut json) = (None, None, None, false);
    let value = |args: &mut dyn Iterator<Item = String>, name: &str| {
        args.next()
            .ok_or_else(|| UsageError(format!("{name} needs a value")))
    };
    let name = loop {
        match args.next() {
            None => return usage("no command given"),
            Some(a) => match a.as_str() {
                "-h" | "--help" => return Ok(Parsed::Help),
                "-V" | "--version" => return Ok(Parsed::Version),
                "--db" => db = Some(PathBuf::from(value(&mut args, "--db")?)),
                "--actor" => actor = Some(value(&mut args, "--actor")?),
                "--model" => model = Some(PathBuf::from(value(&mut args, "--model")?)),
                "--json" => json = true,
                flag if flag.starts_with('-') => return usage(format!("unknown option {flag}")),
                _ => break a,
            },
        }
    };
    let mut rest = Rest(args.collect());
    let command = match name.as_str() {
        "scopes" => {
            rest.none()?;
            Command::Scopes
        }
        "mcp" => {
            rest.none()?;
            Command::Mcp
        }
        "setup" => {
            let replace = rest.switch("--replace");
            let remove = rest.switch("--remove");
            let claude_md = match (rest.switch("--claude-md"), rest.switch("--no-claude-md")) {
                (true, true) => return usage("--claude-md or --no-claude-md, not both"),
                (yes, no) => (yes || no).then_some(yes),
            };
            // `setup` alone: these are its flags, and no subcommand takes them.
            let yes = rest.switch_all("--yes");
            let from_installer = rest.switch_all("--from-installer");
            let agents = rest.flag("--agents")?;
            let model = rest.flag("--model")?;
            let instructions = match (
                rest.switch("--instructions"),
                rest.switch("--no-instructions"),
            ) {
                (true, true) => return usage("--instructions or --no-instructions, not both"),
                (yes, no) => (yes || no).then_some(yes),
            };
            let bare = yes
                || from_installer
                || agents.is_some()
                || model.is_some()
                || instructions.is_some();
            let what = rest.positionals()?;
            let what: Vec<&str> = what.iter().map(String::as_str).collect();
            let command = match what[..] {
                [] if claude_md.is_none() => {
                    let all = agents.as_deref() == Some("all");
                    Command::Setup {
                        yes,
                        from_installer,
                        agents: agents
                            .filter(|_| !all)
                            .map(|list| parse_agents(&list))
                            .transpose()?,
                        all,
                        model: model.map(|m| parse_model_choice(&m)).transpose()?,
                        instructions,
                        replace,
                    }
                }
                _ if bare => {
                    return usage(
                        "--yes, --from-installer, --agents, --model and --instructions go with \
                         setup alone",
                    );
                }
                ["claude-code"] => Command::SetupClaudeCode { replace, claude_md },
                ["model"] if !replace && claude_md.is_none() => {
                    Command::SetupModel { potion: false }
                }
                ["model", "potion"] if !replace && claude_md.is_none() => {
                    Command::SetupModel { potion: true }
                }
                ["sleep"] if !replace && claude_md.is_none() => Command::SetupSleep { remove },
                _ => {
                    return usage(
                        "setup, setup claude-code, setup model, setup model potion or setup sleep",
                    );
                }
            };
            if remove && !matches!(command, Command::SetupSleep { .. }) {
                return usage("--remove goes with setup sleep");
            }
            command
        }
        "uninstall" => {
            let agents = rest
                .flag("--agents")?
                .map(|list| parse_agents(&list))
                .transpose()?;
            let models = rest.switch("--models");
            let data = rest.switch("--data");
            let yes = rest.switch("--yes");
            rest.none()?;
            Command::Uninstall {
                agents,
                models,
                data,
                yes,
            }
        }
        "sleep" => {
            let now = rest.switch("--now");
            let dry_run = rest.switch("--dry-run");
            let report = rest.switch("--report");
            rest.none()?;
            Command::Sleep {
                now,
                dry_run,
                report,
            }
        }
        "stats" => {
            rest.none()?;
            Command::Stats
        }
        "doctor" => {
            rest.none()?;
            Command::Doctor
        }
        "list" => {
            let state = rest.flag("--state")?.map(|s| parse_state(&s)).transpose()?;
            let scope = rest.one("SCOPE")?;
            Command::List { scope, state }
        }
        "get" | "history" => {
            let id = parse_id(&rest.one("ID")?)?;
            if name == "get" {
                Command::Get { id }
            } else {
                Command::History { id }
            }
        }
        "review" => {
            let scope = rest.one("SCOPE")?;
            Command::Review { scope }
        }
        "jobs" => {
            let state = rest
                .flag("--state")?
                .map(|s| parse_job_state(&s))
                .transpose()?;
            rest.none()?;
            Command::Jobs { state }
        }
        "recall" | "context" => {
            let (scope, text) = rest.with_text("SCOPE")?;
            if name == "recall" {
                Command::Recall { scope, text }
            } else {
                Command::Context { scope, text }
            }
        }
        "export" => Command::Export {
            path: PathBuf::from(rest.one("PATH")?),
        },
        "remember" => {
            let kind = rest
                .flag("--kind")?
                .map_or(Ok(MemoryType::Semantic), |k| parse_kind(&k))?;
            let topic = rest.flag("--topic")?;
            let source = rest.flag("--source")?;
            let (scope, text) = rest.with_text("SCOPE")?;
            Command::Remember {
                scope,
                text,
                kind,
                topic,
                source,
            }
        }
        "promote" => Command::Promote {
            id: parse_id(&rest.one("ID")?)?,
        },
        "archive" => Command::Archive {
            id: parse_id(&rest.one("ID")?)?,
        },
        "revert" => Command::Revert {
            op: parse_op(&rest.one("OP")?)?,
        },
        "purge" => {
            let yes = rest.switch("--yes");
            let reason = rest
                .flag("--reason")?
                .unwrap_or_else(|| "purged by a person".into());
            Command::Purge {
                id: parse_id(&rest.one("ID")?)?,
                reason,
                yes,
            }
        }
        "feedback" => {
            let (id, signal) = rest.two("ID useful|wrong")?;
            Command::Feedback {
                id: parse_id(&id)?,
                signal: parse_signal(&signal)?,
            }
        }
        "retry-job" => {
            let id = rest.one("ID")?;
            match id.parse::<i64>() {
                Ok(id) if id > 0 => Command::RetryJob { id },
                _ => return usage(format!("{id:?} is not a job id")),
            }
        }
        "run-jobs" => {
            let heavy = rest.switch("--heavy");
            rest.none()?;
            Command::RunJobs { heavy }
        }
        "backup" => Command::Backup {
            path: PathBuf::from(rest.one("PATH")?),
        },
        "import" => Command::Import {
            path: PathBuf::from(rest.one("PATH")?),
        },
        "import-memories" => {
            let origin = rest
                .flag("--origin")?
                .map(|o| parse_origin(&o))
                .transpose()?;
            let (scope, path) = rest.two("SCOPE PATH")?;
            Command::ImportMemories {
                scope,
                path: PathBuf::from(path),
                origin,
            }
        }
        other => return usage(format!("unknown command {other:?}")),
    };
    // `setup` uses none of the options before the command: one given there is a mistake, not
    // something to ignore.
    if matches!(command, Command::Setup { .. }) {
        let given = [
            ("--db", db.is_some()),
            ("--actor", actor.is_some()),
            ("--model", model.is_some()),
            ("--json", json),
        ];
        if let Some((name, _)) = given.into_iter().find(|(_, given)| *given) {
            return usage(format!(
                "{name} does not go with setup (its own --model goes after it: setup --model none)"
            ));
        }
    }
    let db = match db {
        Some(db) => db,
        None => match clio_memory::agent::default_data() {
            Some(d) => d.join(clio_memory::agent::memory::DB_FILE),
            None => return usage("no --db, and neither CLIO_MEMORY_DATA nor HOME is set"),
        },
    };
    #[cfg(not(any(feature = "model2vec", feature = "granite")))]
    if model.is_some() {
        return usage("--model needs a build with the model2vec or granite feature");
    }
    Ok(Parsed::Run(Cli {
        db,
        actor,
        model,
        json,
        command,
    }))
}
