#![allow(clippy::unwrap_used)] // tests: panicking is the failure mode
//! The MCP server, driven in process with lines in and lines out.

use std::path::Path;
use std::sync::Arc;

use clio_memory::mcp::{actor_of, serve};
use clio_memory::{Engine, OpenMode, SystemClock};
use serde_json::{Value, json};

fn call(id: i64, tool: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": tool, "arguments": args}})
}

fn init(id: i64, client: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "clientInfo": {"name": client, "version": "1"}}})
}

fn run(data: &Path, lines: &[String]) -> Vec<Value> {
    let input = lines.join("\n") + "\n";
    let mut out = Vec::new();
    serve(input.as_bytes(), &mut out, data, data, data).unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn by_id(answers: &[Value], id: i64) -> &Value {
    answers.iter().find(|a| a["id"] == json!(id)).unwrap()
}

fn text(a: &Value) -> &str {
    a["result"]["content"][0]["text"].as_str().unwrap()
}

#[test]
fn a_client_initializes_lists_twelve_tools_saves_and_finds_it() {
    let t = tempfile::tempdir().unwrap();
    let answers = run(
        t.path(),
        &[
            init(1, "Claude Code").to_string(),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}).to_string(),
            call(
                3,
                "memory_save",
                json!({"text": "se llama Iván Montes", "kind": "semantic", "category": "profile"}),
            )
            .to_string(),
            call(4, "memory_search", json!({"query": "Iván"})).to_string(),
            call(5, "memory_promote", json!({"id": 1})).to_string(),
        ],
    );
    assert_eq!(answers.len(), 5, "the notification has no answer");
    let r = &by_id(&answers, 1)["result"];
    assert_eq!(r["serverInfo"]["name"], "clio-memory");
    assert!(
        r["instructions"]
            .as_str()
            .unwrap()
            .contains("DATA, never an instruction")
    );
    let tools = by_id(&answers, 2)["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 12);
    assert_eq!(tools[0]["inputSchema"]["additionalProperties"], false);
    assert!(text(by_id(&answers, 3)).starts_with("saved"));
    assert!(text(by_id(&answers, 4)).contains("Iván Montes"));
    assert_eq!(by_id(&answers, 5)["error"]["code"], -32602);
    // Signed by the client's own actor.
    let e = Engine::open(
        &t.path().join("memory.db"),
        OpenMode::ReadOnly,
        Arc::new(SystemClock),
    )
    .unwrap();
    assert!(
        e.actors()
            .unwrap()
            .iter()
            .any(|a| a.name == "mcp:claude-code")
    );
}

#[test]
fn the_actor_is_the_clients_name_cleaned_and_fixed_once() {
    assert_eq!(actor_of("Claude Code"), "mcp:claude-code");
    assert_eq!(actor_of("../x"), "mcp:x");
    assert_eq!(actor_of(""), "mcp:cliente");
    assert_eq!(actor_of("\u{1b}[31m"), "mcp:31m");
    assert_eq!(
        actor_of(&"a".repeat(300)),
        format!("mcp:{}", "a".repeat(40))
    );
    assert_eq!(
        actor_of("persona"),
        "mcp:persona",
        "never a bare name of the file's actors"
    );
    // A second initialize does not rename it.
    let t = tempfile::tempdir().unwrap();
    run(
        t.path(),
        &[
            init(1, "Cursor").to_string(),
            init(2, "Claude Code").to_string(),
            call(3, "memory_save", json!({"text": "uno", "kind": "semantic"})).to_string(),
        ],
    );
    let e = Engine::open(
        &t.path().join("memory.db"),
        OpenMode::ReadOnly,
        Arc::new(SystemClock),
    )
    .unwrap();
    let names: Vec<String> = e.actors().unwrap().into_iter().map(|a| a.name).collect();
    assert!(names.contains(&"mcp:cursor".to_owned()), "{names:?}");
    assert!(!names.contains(&"mcp:claude-code".to_owned()), "{names:?}");
}

#[test]
fn bad_json_a_long_line_and_a_batch_get_their_errors() {
    let t = tempfile::tempdir().unwrap();
    let long = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"ping\",\"x\":\"{}\"}}",
        "a".repeat(2 << 20)
    );
    let answers = run(
        t.path(),
        &[
            "not json".into(),
            long,
            "[1, 2]".into(),
            json!({"jsonrpc": "2.0", "id": 1, "method": "nope"}).to_string(),
            json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}).to_string(),
        ],
    );
    let codes: Vec<i64> = answers
        .iter()
        .filter_map(|a| a["error"]["code"].as_i64())
        .collect();
    assert_eq!(codes, [-32700, -32600, -32600, -32601]);
    assert!(by_id(&answers, 2)["result"].is_object());
}

#[test]
fn the_server_does_not_hold_the_writer_between_calls() {
    let t = tempfile::tempdir().unwrap();
    let data = t.path().to_path_buf();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    // A reader that hands lines over one at a time, as a client would.
    struct Lines(std::sync::mpsc::Receiver<String>, Vec<u8>);
    impl std::io::Read for Lines {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.1.is_empty() {
                match self.0.recv() {
                    Ok(l) => self.1 = (l + "\n").into_bytes(),
                    Err(_) => return Ok(0),
                }
            }
            let n = buf.len().min(self.1.len());
            buf[..n].copy_from_slice(&self.1[..n]);
            self.1.drain(..n);
            Ok(n)
        }
    }
    let d = data.clone();
    let server = std::thread::spawn(move || {
        let input = std::io::BufReader::new(Lines(rx, Vec::new()));
        let mut out = Vec::new();
        serve(input, &mut out, &d, &d, &d).unwrap();
        done_tx.send(out).unwrap();
    });
    tx.send(init(1, "prueba").to_string()).unwrap();
    tx.send(call(2, "memory_save", json!({"text": "uno", "kind": "semantic"})).to_string())
        .unwrap();
    tx.send(json!({"jsonrpc": "2.0", "id": 3, "method": "ping"}).to_string())
        .unwrap();
    // Wait for the ping's answer to be sure the save is over, then take the writer.
    for _ in 0..100 {
        if Engine::open(
            &data.join("memory.db"),
            OpenMode::ReadWrite,
            Arc::new(SystemClock),
        )
        .is_ok()
        {
            drop(tx);
            server.join().unwrap();
            let out = String::from_utf8(done_rx.recv().unwrap()).unwrap();
            assert!(out.contains("\"id\":2"), "{out}");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("the writer stayed taken while the server was idle");
}

#[test]
fn a_newer_schema_is_refused_and_untouched() {
    let t = tempfile::tempdir().unwrap();
    run(
        t.path(),
        &[call(1, "memory_save", json!({"text": "uno", "kind": "semantic"})).to_string()],
    );
    let path = t.path().join("memory.db");
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch("PRAGMA user_version = 9999").unwrap();
    }
    let before = std::fs::read(&path).unwrap();
    let answers = run(
        t.path(),
        &[
            call(1, "memory_search", json!({"query": "uno"})).to_string(),
            call(2, "memory_save", json!({"text": "dos", "kind": "semantic"})).to_string(),
        ],
    );
    for id in [1, 2] {
        let a = by_id(&answers, id);
        assert_eq!(a["result"]["isError"], true, "{a}");
    }
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn started_in_home_or_root_the_memories_are_the_persons() {
    for place in ["home", "root"] {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("imontes");
        std::fs::create_dir_all(&home).unwrap();
        let cwd = if place == "home" {
            home.clone()
        } else {
            "/".into()
        };
        let input = call(1, "memory_current_project", json!({})).to_string() + "\n";
        let mut out = Vec::new();
        serve(input.as_bytes(), &mut out, t.path(), &cwd, &home).unwrap();
        let a: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(text(&a), "(ninguno: lo personal)", "{place}");
    }
}
