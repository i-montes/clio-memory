//! Fake agent programs (`claude`, `codex`, `gemini`) for the setup tests. Each one appends its
//! arguments to `<dir>/<name>.log`, one per line and a `----` line after each call, so an argument
//! with a space in it is seen whole; and each answers `mcp get` the way the real one does.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// The `sh` that logs the call and defines `json_args` (its arguments as a JSON array of strings).
fn prelude(dir: &Path, name: &str) -> String {
    format!(
        r#"#!/bin/sh
for a in "$@"; do printf '%s\n' "$a"; done >> '{log}'
echo ---- >> '{log}'
json_args() {{
  sep=''; printf '['
  for a in "$@"; do printf '%s"%s"' "$sep" "$a"; sep=','; done
  printf ']'
}}
"#,
        log = dir.join(format!("{name}.log")).display()
    )
}

fn write_program(dir: &Path, name: &str, body: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let bin = dir.join(name);
    std::fs::write(&bin, format!("{}{body}", prelude(dir, name))).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

/// The calls a fake received, each one as its list of arguments.
pub fn calls(dir: &Path, name: &str) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(dir.join(format!("{name}.log"))).unwrap_or_default();
    text.split_terminator("----\n")
        .map(|call| call.lines().map(str::to_owned).collect())
        .collect()
}

/// Where the fake `claude` keeps what `mcp get` prints.
pub fn claude_registry(dir: &Path) -> PathBuf {
    dir.join("claude.registered")
}

/// The scope line `claude mcp get` prints for a server in the user's config.
pub const CLAUDE_USER_SCOPE: &str = "User config (available in all your projects)";
/// The same for one in the local config of the current directory.
pub const CLAUDE_LOCAL_SCOPE: &str = "Local config (private to you in this project)";

/// What Claude Code's `claude mcp get clio-memory` prints for a stdio server, as recorded (the
/// format in its bundle: `Command:` and `Args:` on lines of their own, the arguments joined by
/// spaces). `remove_scope` is the `-s` of the last line (`user`, `local`).
pub fn claude_get_output(scope: &str, remove_scope: &str, command: &str, args: &str) -> String {
    format!(
        "clio-memory:\n  Scope: {scope}\n  Status: ✔ Connected\n  Type: stdio\n  Command: {command}\n  Args: {args}\n  Environment:\n\nTo remove this server, run: claude mcp remove clio-memory -s {remove_scope}\n"
    )
}

/// A fake `claude`: `mcp add --scope user clio-memory -- <cmd> <args>` registers, `mcp get`
/// prints it (exit 1 when there is none), `mcp remove --scope user` forgets it (and fails, as the
/// real one does, when what is there is not in the user's scope).
pub fn fake_claude_mcp(dir: &Path) -> PathBuf {
    let reg = claude_registry(dir);
    write_program(
        dir,
        "claude",
        &format!(
            r#"reg='{reg}'
case "$1 $2" in
  "mcp get") [ -f "$reg" ] && cat "$reg" && exit 0
    echo "No MCP server found with name: $3" >&2; exit 1;;
  "mcp add") shift 6; cmd=$1; shift
    printf 'clio-memory:\n  Scope: User config (available in all your projects)\n  Status: ✔ Connected\n  Type: stdio\n  Command: %s\n  Args: %s\n  Environment:\n\nTo remove this server, run: claude mcp remove clio-memory -s user\n' "$cmd" "$*" > "$reg";;
  "mcp remove") grep -q 'Scope: User config' "$reg" 2>/dev/null \
      || {{ echo "No user-scoped MCP server found with name: $3" >&2; exit 1; }}
    rm -f "$reg";;
esac
"#,
            reg = reg.display()
        ),
    )
}

/// Where the fake `codex` keeps its server: in `CODEX_HOME`, as the real one keeps its config.
pub fn codex_registry(codex_home: &Path) -> PathBuf {
    codex_home.join("fake-mcp.json")
}

/// What `codex mcp get <name> --json` printed on codex-cli 0.159.2 for
/// `codex mcp add clio-memory-probe -- /bin/true mcp`.
pub fn codex_get_json(command: &str, args: &[&str]) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "name": "clio-memory",
        "enabled": true,
        "disabled_reason": null,
        "transport": {
            "type": "stdio",
            "command": command,
            "args": args,
            "env": null,
            "env_vars": [],
            "cwd": null
        },
        "enabled_tools": null,
        "disabled_tools": null,
        "startup_timeout_sec": null,
        "tool_timeout_sec": null
    }))
    .unwrap()
}

/// A fake `codex` keeping its one server under `$CODEX_HOME`. With `fail_add`, `mcp add` exits 1.
pub fn fake_codex(dir: &Path, fail_add: bool) -> PathBuf {
    let add = if fail_add {
        r#"echo "Error: could not write config.toml" >&2; exit 1"#
    } else {
        r#"shift 4; cmd=$1; shift
    printf '{"name":"clio-memory","enabled":true,"disabled_reason":null,"transport":{"type":"stdio","command":"%s","args":%s,"env":null,"env_vars":[],"cwd":null},"enabled_tools":null,"disabled_tools":null,"startup_timeout_sec":null,"tool_timeout_sec":null}\n' "$cmd" "$(json_args "$@")" > "$reg""#
    };
    write_program(
        dir,
        "codex",
        &format!(
            r#"[ -n "$CODEX_HOME" ] || {{ echo "CODEX_HOME not passed" >&2; exit 3; }}
reg="$CODEX_HOME/fake-mcp.json"
case "$1 $2" in
  "mcp get") [ -f "$reg" ] && cat "$reg" && exit 0
    echo "Error: No MCP server named '$3' found." >&2; exit 1;;
  "mcp add") {add};;
  "mcp remove") rm -f "$reg"; echo "Removed global MCP server '$3'.";;
esac
"#
        ),
    )
}

/// A fake `gemini`: `mcp add -s user clio-memory <cmd> <args>` writes `~/.gemini/settings.json`
/// (the documented `mcpServers.<name>.{command,args}`), `mcp remove -s user clio-memory` empties it.
pub fn fake_gemini(dir: &Path, home: &Path) -> PathBuf {
    let settings = home.join(".gemini/settings.json");
    write_program(
        dir,
        "gemini",
        &format!(
            r#"settings='{settings}'
mkdir -p "$(dirname "$settings")"
case "$1 $2" in
  "mcp add") shift 5; cmd=$1; shift
    printf '{{"mcpServers":{{"clio-memory":{{"command":"%s","args":%s}}}}}}\n' "$cmd" "$(json_args "$@")" > "$settings";;
  "mcp remove") printf '{{"mcpServers":{{}}}}\n' > "$settings";;
esac
"#,
            settings = settings.display()
        ),
    )
}
