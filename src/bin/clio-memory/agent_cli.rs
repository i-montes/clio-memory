//! `clio-memory m <verb>`: the memory tools as commands an agent runs from its shell. Each one is
//! one tool call, to the daemon (or in this process when there is none); the tool's text goes to
//! stdout, and the exit code says whether it answered (0) or refused (1).

use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

use serde_json::{Map, Value};

use crate::args::{Rest, UsageError, parse_id};

fn usage<T>(msg: impl Into<String>) -> Result<T, UsageError> {
    Err(UsageError(msg.into()))
}

/// What follows `m`: `[--agent N] VERB ARGS…` as the agent's name, the tool and its arguments.
/// A TEXT of `-` is read from `stdin`, without its final newline.
pub fn parse(
    rest: Vec<String>,
    stdin: &mut dyn Read,
) -> Result<(Option<String>, &'static str, Value), UsageError> {
    let mut rest = rest.into_iter();
    let mut verb = rest.next();
    let mut agent = None;
    if verb.as_deref() == Some("--agent") {
        agent = Some(
            rest.next()
                .ok_or_else(|| UsageError("--agent needs a value".into()))?,
        );
        verb = rest.next();
    }
    let Some(verb) = verb else {
        return usage("expected a memory verb (see `clio-memory --help`)");
    };
    let mut rest = Rest(rest.collect());
    let mut args = Map::new();
    let tool = match verb.as_str() {
        "context" | "project" => {
            rest.none()?;
            if verb == "context" {
                "memory_context"
            } else {
                "memory_current_project"
            }
        }
        "search" => {
            if let Some(n) = rest.flag("--limit")? {
                args.insert("limit".into(), number("--limit", &n)?.into());
            }
            args.insert("query".into(), text(rest.positionals()?, stdin)?.into());
            "memory_search"
        }
        "get" | "forget" | "pin" => {
            args.insert("id".into(), id(&rest.one("ID")?)?);
            match verb.as_str() {
                "get" => "memory_get",
                "forget" => "memory_forget",
                _ => "memory_pin",
            }
        }
        "timeline" => {
            for k in ["before", "after"] {
                let flag = format!("--{k}");
                if let Some(n) = rest.flag(&flag)? {
                    args.insert(k.into(), number(&flag, &n)?.into());
                }
            }
            args.insert("id".into(), id(&rest.one("ID")?)?);
            "memory_timeline"
        }
        "save" => {
            let kind = rest.flag("--kind")?.unwrap_or_else(|| "semantic".into());
            if kind != "semantic" && kind != "procedural" {
                return usage(format!("unknown kind {kind:?} (semantic|procedural)"));
            }
            args.insert("kind".into(), kind.into());
            titled(&mut rest, &mut args)?;
            args.insert("text".into(), text(rest.positionals()?, stdin)?.into());
            "memory_save"
        }
        "update" | "consolidate" => {
            titled(&mut rest, &mut args)?;
            let mut words = rest.positionals()?;
            let first = if verb == "update" { "ID" } else { "IDS" };
            if words.len() < 2 {
                return usage(format!("expected {first} TEXT"));
            }
            let target = words.remove(0);
            if verb == "update" {
                args.insert("id".into(), id(&target)?);
            } else {
                // The tool reads `12,15` and `m12, m15` itself, and says what it wants.
                args.insert("ids".into(), target.into());
            }
            args.insert("text".into(), text(words, stdin)?.into());
            if verb == "update" {
                "memory_update"
            } else {
                "memory_consolidate"
            }
        }
        "summary" => {
            for (flag, key) in [
                ("--goal", "goal"),
                ("--accomplished", "accomplished"),
                ("--instructions", "instructions"),
                ("--discoveries", "discoveries"),
                ("--next-steps", "next_steps"),
                ("--files", "files"),
            ] {
                if let Some(v) = rest.flag(flag)? {
                    args.insert(key.into(), v.into());
                }
            }
            rest.none()?;
            if !(args.contains_key("goal") && args.contains_key("accomplished")) {
                return usage("summary needs --goal and --accomplished");
            }
            "memory_session_summary"
        }
        "revert" => {
            let change = rest.one("CHANGE")?;
            match change.parse::<i64>() {
                Ok(n) if n > 0 => args.insert("change".into(), n.into()),
                _ => return usage(format!("{change:?} is not a change number")),
            };
            "memory_revert"
        }
        other => return usage(format!("unknown memory verb {other:?}")),
    };
    Ok((agent, tool, Value::Object(args)))
}

/// `--title` and `--category`, when given.
fn titled(rest: &mut Rest, args: &mut Map<String, Value>) -> Result<(), UsageError> {
    for k in ["title", "category"] {
        if let Some(v) = rest.flag(&format!("--{k}"))? {
            args.insert(k.into(), v.into());
        }
    }
    Ok(())
}

fn id(s: &str) -> Result<Value, UsageError> {
    Ok(parse_id(s)?.0.into())
}

fn number(flag: &str, s: &str) -> Result<i64, UsageError> {
    s.parse()
        .or_else(|_| usage(format!("{flag} needs a number, not {s:?}")))
}

/// The words joined by one space, or with `-` alone what `stdin` holds without its final newline.
fn text(words: Vec<String>, stdin: &mut dyn Read) -> Result<String, UsageError> {
    match &words[..] {
        [] => usage("expected TEXT"),
        [dash] if dash == "-" => {
            let mut s = String::new();
            stdin
                .read_to_string(&mut s)
                .map_err(|e| UsageError(format!("the standard input: {e}")))?;
            let end = s
                .strip_suffix("\r\n")
                .or_else(|| s.strip_suffix('\n'))
                .map_or(s.len(), str::len);
            s.truncate(end);
            Ok(s)
        }
        _ => Ok(words.join(" ")),
    }
}

/// Runs one tool call on `<folder>/memory.db` (`db`) for the caller (`agent`, else as
/// `agent::caller` finds it), from the current folder. Prints the tool's text.
pub fn run(db: &Path, agent: Option<&str>, tool: &str, args: &Value) -> ExitCode {
    use clio_memory::agent::memory::DB_FILE;
    use clio_memory::daemon::Launch;
    if db.file_name().is_none_or(|n| n != DB_FILE) {
        crate::output::error(&format!(
            "error: m uses <folder>/{DB_FILE}: give --db a file called {DB_FILE}, or set CLIO_MEMORY_DATA"
        ));
        return ExitCode::from(2);
    }
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(e) => {
            crate::output::error(&format!("error: {e}"));
            return ExitCode::from(1);
        }
    };
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    // No binary path, no daemon: this process serves the call.
    let launch = (!crate::no_daemon())
        .then(std::env::current_exe)
        .and_then(Result::ok)
        .map(|exe| Launch { exe, env: vec![] });
    let caller = clio_memory::agent::caller::caller(agent);
    let answer = clio_memory::mcp::call_once(
        crate::commands::data_of(db),
        &cwd,
        &home,
        launch.as_ref(),
        &caller,
        tool,
        args,
    );
    let (text, code) = match &answer {
        Ok(text) => (text, 0),
        Err(text) => (text, 1),
    };
    crate::output::line(text);
    ExitCode::from(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parsed(args: &[&str]) -> Result<(Option<String>, &'static str, Value), UsageError> {
        parse(
            args.iter().map(|a| (*a).to_owned()).collect(),
            &mut &b""[..],
        )
    }

    fn call(args: &[&str]) -> (&'static str, Value) {
        let (_, tool, args) = parsed(args).unwrap();
        (tool, args)
    }

    fn refused(args: &[&str]) -> String {
        parsed(args).unwrap_err().0
    }

    #[test]
    fn context_takes_the_agent_before_the_verb() {
        let (agent, tool, args) = parsed(&["--agent", "codex", "context"]).unwrap();
        assert_eq!(
            (agent.as_deref(), tool, args),
            (Some("codex"), "memory_context", json!({}))
        );
        assert_eq!(parsed(&["context"]).unwrap().0, None);
    }

    #[test]
    fn search_joins_its_words_and_takes_a_limit() {
        assert_eq!(
            call(&["search", "cargo", "test", "--limit", "3"]),
            ("memory_search", json!({"query": "cargo test", "limit": 3}))
        );
        assert_eq!(
            call(&["search", "hola"]),
            ("memory_search", json!({"query": "hola"}))
        );
    }

    #[test]
    fn get_forget_and_pin_take_an_id_with_or_without_m() {
        assert_eq!(call(&["get", "m12"]), ("memory_get", json!({"id": 12})));
        assert_eq!(
            call(&["forget", "12"]),
            ("memory_forget", json!({"id": 12}))
        );
        assert_eq!(call(&["pin", "M7"]), ("memory_pin", json!({"id": 7})));
    }

    #[test]
    fn timeline_takes_before_and_after() {
        assert_eq!(
            call(&["timeline", "m3", "--before", "1", "--after", "4"]),
            ("memory_timeline", json!({"id": 3, "before": 1, "after": 4}))
        );
    }

    #[test]
    fn save_is_semantic_unless_said() {
        assert_eq!(
            call(&["save", "--title", "T", "--category", "C", "Qué:", "x"]),
            (
                "memory_save",
                json!({"text": "Qué: x", "kind": "semantic", "title": "T", "category": "C"})
            )
        );
        assert_eq!(
            call(&["save", "--kind", "procedural", "pasos"]),
            (
                "memory_save",
                json!({"text": "pasos", "kind": "procedural"})
            )
        );
        assert!(refused(&["save", "--kind", "episodic", "x"]).contains("unknown kind"));
    }

    #[test]
    fn update_takes_an_id_then_the_text() {
        assert_eq!(
            call(&["update", "m12", "--title", "T", "nuevo", "texto"]),
            (
                "memory_update",
                json!({"id": 12, "text": "nuevo texto", "title": "T"})
            )
        );
        assert_eq!(refused(&["update", "12"]), "expected ID TEXT");
    }

    #[test]
    fn consolidate_takes_the_ids_as_written() {
        assert_eq!(
            call(&["consolidate", "12,15", "--category", "C", "uno", "solo"]),
            (
                "memory_consolidate",
                json!({"ids": "12,15", "text": "uno solo", "category": "C"})
            )
        );
    }

    #[test]
    fn summary_needs_a_goal_and_what_was_done() {
        assert_eq!(
            call(&[
                "summary",
                "--goal",
                "G",
                "--accomplished",
                "A",
                "--instructions",
                "I",
                "--discoveries",
                "D",
                "--next-steps",
                "N",
                "--files",
                "F",
            ]),
            (
                "memory_session_summary",
                json!({"goal": "G", "accomplished": "A", "instructions": "I",
                    "discoveries": "D", "next_steps": "N", "files": "F"})
            )
        );
        assert_eq!(
            refused(&["summary", "--goal", "G"]),
            "summary needs --goal and --accomplished"
        );
    }

    #[test]
    fn revert_and_project() {
        assert_eq!(
            call(&["revert", "41"]),
            ("memory_revert", json!({"change": 41}))
        );
        assert_eq!(call(&["project"]), ("memory_current_project", json!({})));
    }

    #[test]
    fn a_dash_reads_the_text_without_its_last_newline() {
        for (input, text) in [
            ("dos\nlíneas\n", "dos\nlíneas"),
            ("con\r\n", "con"),
            ("uno\n\n", "uno\n"),
            ("sin", "sin"),
        ] {
            let (_, _, args) =
                parse(vec!["save".into(), "-".into()], &mut input.as_bytes()).unwrap();
            assert_eq!(args["text"], text, "{input:?}");
        }
    }

    #[test]
    fn mistakes_are_usage_errors() {
        assert_eq!(refused(&["save"]), "expected TEXT");
        assert_eq!(refused(&["baila"]), "unknown memory verb \"baila\"");
        assert!(refused(&["get", "doce"]).contains("is not an id"));
        assert!(refused(&["search", "x", "--limit", "tres"]).contains("needs a number"));
        assert!(refused(&[]).contains("expected a memory verb"));
    }
}
