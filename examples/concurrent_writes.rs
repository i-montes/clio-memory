//! Concurrent writers on one memory file (D11, task 342 of Clio's list). K processes save at the
//! same time, each one the way an MCP call does it: open for writing (waiting up to 2 s for the
//! writer lock), save, let go. Every tenth save of every process is the same text as the other
//! processes' (a fact two agents learn at once). Afterwards the file is read back: no save that
//! was answered "saved" may be missing, a shared fact must be there once, and the report gives
//! the latency of a save and how many gave up waiting for the lock.
//!
//! `cargo run --release --example concurrent_writes -- [processes] [saves each]` (default 8 50)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use clio_memory::agent::{Draft, EmbedModel, Memory, Place};
use clio_memory::{MemoryType, OpKind, State};

const PROJECT: &str = "concurrencia";

fn shared(j: usize) -> String {
    format!(
        "Hecho compartido número {}: dos agentes lo aprendieron a la vez",
        j / 10
    )
}

fn own(i: usize, j: usize) -> String {
    format!("El proceso {i} guardó su recuerdo propio número {j}")
}

/// One writer process: prints one line per save, `ok <kind> <memory> <micros>` or `err <micros> <why>`.
fn child(data: &Path, i: usize, saves: usize) {
    let model = EmbedModel::none();
    let actor = format!("mcp:w{i}");
    for j in 0..saves {
        let text = if j % 10 == 0 { shared(j) } else { own(i, j) };
        let t = Instant::now();
        let r = Memory::open_write(data, &model, Some(PROJECT), &actor).and_then(|m| {
            m.save(
                &Draft::plain(&text, MemoryType::Semantic),
                &actor,
                Place::Project,
                false,
            )
        });
        let us = t.elapsed().as_micros();
        match r {
            Ok(o) => println!("ok {:?} {} {us}", o.kind, o.memory.0),
            Err(e) => println!("err {us} {}", e.replace('\n', " ")),
        }
    }
}

fn pct(sorted: &[u128], p: usize) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    sorted[(sorted.len() - 1) * p / 100] as f64 / 1000.0
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("child") {
        child(
            Path::new(&args[2]),
            args[3].parse().expect("the measurement cannot go on"),
            args[4].parse().expect("the measurement cannot go on"),
        );
        return;
    }
    let k: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(8);
    let saves: usize = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(50);
    let dir = tempfile::tempdir().expect("the measurement cannot go on");
    let data: PathBuf = dir.path().join("data");
    // The file exists before the race, as it does on a machine in use.
    drop(
        Memory::open_write(&data, &EmbedModel::none(), Some(PROJECT), "mcp:setup")
            .expect("the measurement cannot go on"),
    );

    let exe = std::env::current_exe().expect("the measurement cannot go on");
    let start = Instant::now();
    let children: Vec<_> = (0..k)
        .map(|i| {
            Command::new(&exe)
                .args([
                    "child",
                    &data.display().to_string(),
                    &i.to_string(),
                    &saves.to_string(),
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("the measurement cannot go on")
        })
        .collect::<Vec<_>>();
    // All of them started before any is waited for: they race.
    let children: Vec<_> = children.into_iter().map(|c| c.wait_with_output()).collect();
    let wall = start.elapsed();

    let mut latencies = Vec::new();
    let mut failed_latencies = Vec::new();
    let mut errors: HashMap<String, usize> = HashMap::new();
    let mut saved_ids = Vec::new();
    let (mut adds, mut noops) = (0usize, 0usize);
    for c in children {
        let out = c.expect("the measurement cannot go on");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let parts: Vec<&str> = line.splitn(4, ' ').collect();
            match parts.as_slice() {
                ["ok", kind, id, us] => {
                    latencies.push(us.parse::<u128>().expect("the measurement cannot go on"));
                    if *kind == format!("{:?}", OpKind::Noop) {
                        noops += 1;
                    } else {
                        adds += 1;
                    }
                    saved_ids.push(id.parse::<i64>().expect("the measurement cannot go on"));
                }
                ["err", us, why @ ..] => {
                    failed_latencies
                        .push(us.parse::<u128>().expect("the measurement cannot go on"));
                    *errors.entry(why.join(" ")).or_default() += 1;
                }
                _ => panic!("unexpected line: {line}"),
            }
        }
    }
    latencies.sort_unstable();

    // Read it back.
    let m = Memory::open_read(&data, &EmbedModel::none(), Some(PROJECT), "mcp:setup")
        .expect("the measurement cannot go on")
        .expect("the measurement cannot go on");
    let scope = m
        .scope_of(Place::Project)
        .expect("the measurement cannot go on");
    let rows = m
        .engine()
        .list(scope, Some(State::Active), None)
        .expect("the measurement cannot go on");
    let missing = saved_ids
        .iter()
        .filter(|id| !rows.iter().any(|r| r.id.0 == **id))
        .count();
    let mut by_text: HashMap<&str, usize> = HashMap::new();
    for r in &rows {
        *by_text.entry(r.content.as_str()).or_default() += 1;
    }
    let shared_texts = (0..saves).step_by(10).count();
    let shared_copies: usize = (0..saves)
        .step_by(10)
        .map(|j| by_text.get(shared(j).as_str()).copied().unwrap_or(0))
        .sum();

    println!(
        "| processes × saves | wall (s) | saved | of them NOOP | gave up | missing after | shared facts (copies / distinct) | save p50 / p95 / p99 (ms) |"
    );
    println!("|---|---|---|---|---|---|---|---|");
    println!(
        "| {k} × {saves} | {:.1} | {} | {noops} | {} | {missing} | {shared_copies} / {shared_texts} | {:.1} / {:.1} / {:.1} |",
        wall.as_secs_f64(),
        adds + noops,
        failed_latencies.len(),
        pct(&latencies, 50),
        pct(&latencies, 95),
        pct(&latencies, 99),
    );
    for (why, n) in &errors {
        println!("- {n} × {why}");
    }
    if !failed_latencies.is_empty() {
        failed_latencies.sort_unstable();
        println!(
            "- a save that gave up waited {:.0} ms (median)",
            pct(&failed_latencies, 50)
        );
    }
}
