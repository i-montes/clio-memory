//! Who is calling `clio-memory`: which agent and which session.
//!
//! The agent comes from, in order, the `--agent` flag, `CLIO_MEMORY_AGENT`, the Claude Code
//! environment (`CLAUDECODE`), or the name of the nearest known ancestor process; with none of
//! them the caller is the command line itself (`cli`). Empty values count as absent.

/// The calling agent and the session it belongs to. Neither is ever empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub agent: String,
    pub session: String,
}

/// Program name of an ancestor process to agent name.
const KNOWN: &[(&str, &str)] = &[
    ("claude", "claude-code"),
    ("codex", "codex"),
    ("opencode", "opencode"),
    ("gemini", "gemini"),
    ("agy", "antigravity"),
    ("antigravity", "antigravity"),
    ("clio", "clio"),
];

/// How many levels above this process are looked at.
const LEVELS: usize = 8;

fn known(program: &str) -> Option<&'static str> {
    KNOWN.iter().find(|(p, _)| *p == program).map(|(_, a)| *a)
}

/// Works out the caller. `ancestors` gives, from the parent upward, (pid, program name); it is
/// only called when its answer can change the result.
///
/// The session is `CLAUDE_CODE_SESSION_ID` for Claude Code when it is set. Otherwise it is the
/// pid of the nearest known ancestor (the agent's own process, the same for every call it
/// makes), or `parent` when there is none (a shell, the nearest stable process).
pub fn caller_from(
    flag: Option<&str>,
    env: &dyn Fn(&str) -> Option<String>,
    ancestors: &dyn Fn() -> Vec<(u32, String)>,
    parent: u32,
) -> Caller {
    let set = |k: &str| env(k).filter(|v| !v.is_empty());
    let nearest = std::cell::OnceCell::new();
    let ancestor = || {
        *nearest.get_or_init(|| {
            ancestors()
                .into_iter()
                .find_map(|(pid, p)| known(&p).map(|a| (pid, a)))
        })
    };
    let agent = flag
        .filter(|f| !f.is_empty())
        .map(str::to_owned)
        .or_else(|| set("CLIO_MEMORY_AGENT"))
        .or_else(|| set("CLAUDECODE").map(|_| "claude-code".to_owned()))
        .or_else(|| ancestor().map(|(_, a)| a.to_owned()))
        .unwrap_or_else(|| "cli".to_owned());
    let session = Some(&agent)
        .filter(|a| *a == "claude-code")
        .and_then(|_| set("CLAUDE_CODE_SESSION_ID"))
        .unwrap_or_else(|| ancestor().map_or(parent, |(pid, _)| pid).to_string());
    Caller { agent, session }
}

/// Interpreters an agent may run under (Gemini CLI, npm shims): their process is named after
/// the interpreter, so the script they run names the agent.
const INTERPRETERS: &[&str] = &["node", "bun", "deno"];

/// The program name of the script an interpreter runs, from its arguments (the interpreter
/// first): the first one that is not an option (nor a `run` subcommand), by its file name,
/// without a `.js`/`.mjs`/`.cjs`/`.ts` extension.
fn script_name<'a>(args: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let arg = args
        .into_iter()
        .skip(1)
        .find(|a| !a.is_empty() && !a.starts_with('-') && *a != "run")?;
    let name = arg.rsplit('/').next()?;
    let bare = [".js", ".mjs", ".cjs", ".ts"]
        .iter()
        .find_map(|ext| name.strip_suffix(ext))
        .unwrap_or(name);
    Some(bare.to_owned())
}

/// (parent pid, program name) of a `/proc/<pid>/stat` line. The name sits between the first
/// `(` and the last `)` and may itself hold spaces and parentheses.
#[cfg(any(target_os = "linux", test))]
fn parse_stat(stat: &str) -> Option<(u32, String)> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let name = stat.get(open + 1..close)?;
    let ppid = stat
        .get(close + 1..)?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    Some((ppid, name.to_owned()))
}

/// (parent pid, program name) of a process, or None when it cannot be read.
#[cfg(target_os = "linux")]
fn inspect(pid: u32) -> Option<(u32, String)> {
    parse_stat(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// The script an interpreter process runs, from `/proc/<pid>/cmdline` (NUL-separated).
#[cfg(target_os = "linux")]
fn script(pid: u32) -> Option<String> {
    let line = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let line = String::from_utf8_lossy(&line);
    script_name(line.split('\0'))
}

/// `ps` with these arguments, by its absolute path (never one found on the `PATH`).
#[cfg(not(target_os = "linux"))]
fn ps(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("/bin/ps")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    String::from_utf8(out.stdout).ok()
}

/// Same through `ps`, for macOS and other unixes without /proc. `comm` may be a full path.
#[cfg(not(target_os = "linux"))]
fn inspect(pid: u32) -> Option<(u32, String)> {
    let text = ps(&["-o", "ppid=,comm=", "-p", &pid.to_string()])?;
    let (ppid, comm) = text.trim().split_once(char::is_whitespace)?;
    let name = comm.trim().rsplit('/').next()?.trim_start_matches('-');
    Some((ppid.parse().ok()?, name.to_owned()))
}

/// The script an interpreter process runs, from `ps -o args=`.
// shortcut: split on whitespace, so a script path with spaces is cut short; read the argv with
// sysctl(KERN_PROCARGS2) if that matters.
#[cfg(not(target_os = "linux"))]
fn script(pid: u32) -> Option<String> {
    script_name(ps(&["-o", "args=", "-p", &pid.to_string()])?.split_whitespace())
}

/// The processes above this one, from the parent upward, up to 8 levels: (pid, program name; for
/// an interpreter, its script's). Stops at pid 1 or at the first process that cannot be read.
pub fn ancestors() -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let mut pid = std::os::unix::process::parent_id();
    while pid > 1 && out.len() < LEVELS {
        let Some((ppid, name)) = inspect(pid) else {
            break;
        };
        let name = Some(&name)
            .filter(|n| INTERPRETERS.contains(&n.as_str()))
            .and_then(|_| script(pid))
            .unwrap_or(name);
        out.push((pid, name));
        pid = ppid;
    }
    out
}

/// The caller of this process, from the real environment and the real ancestors.
pub fn caller(flag: Option<&str>) -> Caller {
    caller_from(
        flag,
        &|k| std::env::var(k).ok(),
        &ancestors,
        std::os::unix::process::parent_id(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn the_flag_wins_then_the_variable_then_the_environment_then_an_ancestor() {
        let anc = [(700, "bash".to_owned()), (650, "codex".to_owned())];
        let c = caller_from(
            Some("gemini"),
            &env_of(&[("CLIO_MEMORY_AGENT", "opencode")]),
            &|| anc.to_vec(),
            700,
        );
        assert_eq!(c.agent, "gemini");
        let c = caller_from(
            None,
            &env_of(&[("CLIO_MEMORY_AGENT", "opencode"), ("CLAUDECODE", "1")]),
            &|| anc.to_vec(),
            700,
        );
        assert_eq!(c.agent, "opencode");
        let c = caller_from(
            None,
            &env_of(&[("CLAUDECODE", "1"), ("CLAUDE_CODE_SESSION_ID", "abc-123")]),
            &|| anc.to_vec(),
            700,
        );
        assert_eq!(
            (c.agent.as_str(), c.session.as_str()),
            ("claude-code", "abc-123")
        );
        let c = caller_from(None, &env_of(&[]), &|| anc.to_vec(), 700);
        assert_eq!((c.agent.as_str(), c.session.as_str()), ("codex", "650"));
    }

    #[test]
    fn nobody_known_is_the_command_line() {
        let c = caller_from(
            None,
            &env_of(&[]),
            &|| vec![(700, "bash".into()), (1, "systemd".into())],
            700,
        );
        assert_eq!((c.agent.as_str(), c.session.as_str()), ("cli", "700"));
    }

    #[test]
    fn empty_values_count_as_absent() {
        let anc = [(650, "codex".to_owned())];
        let c = caller_from(
            Some(""),
            &env_of(&[("CLIO_MEMORY_AGENT", ""), ("CLAUDECODE", "")]),
            &|| anc.to_vec(),
            700,
        );
        assert_eq!((c.agent.as_str(), c.session.as_str()), ("codex", "650"));
        let c = caller_from(
            None,
            &env_of(&[("CLAUDECODE", "1"), ("CLAUDE_CODE_SESSION_ID", "")]),
            &Vec::new,
            700,
        );
        assert_eq!(
            (c.agent.as_str(), c.session.as_str()),
            ("claude-code", "700")
        );
    }

    // The walk is slow (a `ps` per level on macOS): skipped when its answer cannot change the
    // result, and kept for the session when the agent came from the flag.
    #[test]
    fn the_ancestors_are_only_looked_at_when_they_matter() {
        let never = || -> Vec<(u32, String)> { panic!("the ancestors were walked") };
        let c = caller_from(
            None,
            &env_of(&[("CLAUDECODE", "1"), ("CLAUDE_CODE_SESSION_ID", "abc-123")]),
            &never,
            700,
        );
        assert_eq!(
            (c.agent.as_str(), c.session.as_str()),
            ("claude-code", "abc-123")
        );
        let walks = std::cell::Cell::new(0);
        let c = caller_from(
            Some("gemini"),
            &env_of(&[]),
            &|| {
                walks.set(walks.get() + 1);
                vec![(650, "codex".to_owned())]
            },
            700,
        );
        assert_eq!((c.agent.as_str(), c.session.as_str()), ("gemini", "650"));
        let c = caller_from(
            None,
            &env_of(&[]),
            &|| {
                walks.set(walks.get() + 1);
                vec![(650, "codex".to_owned())]
            },
            700,
        );
        assert_eq!((c.agent.as_str(), c.session.as_str()), ("codex", "650"));
        assert_eq!(walks.get(), 2, "once per caller");
    }

    #[test]
    fn an_interpreter_is_named_after_its_script() {
        let name = |line: &str| script_name(line.split('\0'));
        assert_eq!(
            name("node\0--no-warnings\0/usr/local/bin/gemini\0-p\0hola\0"),
            Some("gemini".into())
        );
        assert_eq!(
            name("/usr/bin/node\0/home/u/.npm/lib/node_modules/@openai/codex/bin/codex.js\0"),
            Some("codex".into())
        );
        assert_eq!(
            name("bun\0run\0/opt/opencode/index.mjs\0"),
            Some("index".into())
        );
        assert_eq!(name("deno\0run\0-A\0claude.ts\0"), Some("claude".into()));
        assert_eq!(name("node\0x.cjs"), Some("x".into()));
        assert_eq!(name("node\0--inspect\0"), None);
        assert_eq!(name("node"), None);
        // macOS: `ps -o args=` is split on whitespace.
        assert_eq!(
            script_name("node /usr/local/bin/gemini --yolo".split_whitespace()).as_deref(),
            Some("gemini")
        );
        assert_eq!(
            known(&name("node\0/usr/local/bin/gemini").unwrap()),
            Some("gemini")
        );
    }

    #[test]
    fn the_program_names_map_to_agents() {
        let table = [
            ("claude", "claude-code"),
            ("codex", "codex"),
            ("opencode", "opencode"),
            ("gemini", "gemini"),
            ("agy", "antigravity"),
            ("antigravity", "antigravity"),
            ("clio", "clio"),
        ];
        for (program, agent) in table {
            assert_eq!(known(program), Some(agent), "{program}");
        }
        assert_eq!(known("bash"), None);
        assert_eq!(known(""), None);
    }

    #[test]
    fn a_program_name_with_spaces_and_parentheses_is_read_whole() {
        assert_eq!(
            parse_stat("4242 (my (odd) prog) S 41 4242 4242 0 -1"),
            Some((41, "my (odd) prog".into()))
        );
        assert_eq!(parse_stat("basura"), None);
    }

    #[test]
    fn the_real_ancestors_are_bounded_and_start_at_the_parent() {
        let a = ancestors();
        assert!(a.len() <= LEVELS);
        if let Some((pid, _)) = a.first() {
            assert_eq!(*pid, std::os::unix::process::parent_id());
        }
    }
}
