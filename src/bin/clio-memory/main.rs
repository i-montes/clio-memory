//! `clio-memory`: inspect and maintain a memory file without writing code.
//! The commands that only read open the file read-only, so they work while another program
//! has it open and never change it; the ones that change something open it read-write and
//! fail with a clear message if another program holds its write lock.

mod args;
mod commands;
mod output;
mod write;

use std::process::ExitCode;

fn main() -> ExitCode {
    match args::parse(std::env::args().skip(1)) {
        Ok(args::Parsed::Help) => {
            output::line(args::USAGE);
            ExitCode::SUCCESS
        }
        Ok(args::Parsed::Run(args::Cli {
            command: args::Command::SetupClaudeCode { replace, claude_md },
            ..
        })) => done(setup_claude_code(replace, claude_md)),
        Ok(args::Parsed::Run(args::Cli {
            command: args::Command::SetupModel,
            ..
        })) => done(setup_model()),
        Ok(args::Parsed::Run(cli)) if matches!(cli.command, args::Command::Mcp) => {
            match mcp(&cli.db) {
                Ok(()) => ExitCode::SUCCESS,
                Err(message) => {
                    output::error(&format!("error: {message}"));
                    ExitCode::from(1)
                }
            }
        }
        Ok(args::Parsed::Run(cli)) => match commands::run(&cli) {
            Ok(code) => code,
            Err(message) => {
                output::error(&format!("error: {message}"));
                ExitCode::from(1)
            }
        },
        Err(args::UsageError(message)) => {
            // The full text is ~14 KB; a caller (often an agent) pays for it on every typo.
            output::error(&format!(
                "error: {message}\nhelp: `clio-memory --help` prints the usage"
            ));
            ExitCode::from(2)
        }
    }
}

/// `clio-memory mcp`: the file must be called `memory.db` (the agent layer's name for it).
fn mcp(db: &std::path::Path) -> Result<(), String> {
    use clio_memory::agent::memory::DB_FILE;
    if db.file_name().is_none_or(|n| n != DB_FILE) {
        return Err(format!(
            "mcp uses <folder>/{DB_FILE}: give --db a file called {DB_FILE}, or set CLIO_MEMORY_DATA"
        ));
    }
    let data = db.parent().unwrap_or(std::path::Path::new("."));
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    clio_memory::mcp::serve(
        std::io::stdin().lock(),
        std::io::stdout().lock(),
        data,
        &cwd,
        &home,
    )
}

fn done(r: Result<Vec<String>, String>) -> ExitCode {
    match r {
        Ok(lines) => {
            for l in lines {
                output::line(&l);
            }
            ExitCode::SUCCESS
        }
        Err(message) => {
            output::error(&format!("error: {message}"));
            ExitCode::from(1)
        }
    }
}

fn home() -> Result<std::path::PathBuf, String> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(std::path::PathBuf::from)
        .filter(|h| h.is_absolute())
        .ok_or_else(|| "HOME is not set".to_owned())
}

/// A yes/no question on the terminal; no terminal is a no.
fn ask(question: &str) -> bool {
    use std::io::{BufRead, IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        return false;
    }
    eprint!("{question} [s/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    let _ = std::io::stdin().lock().read_line(&mut answer);
    matches!(
        answer.trim().to_lowercase().as_str(),
        "s" | "si" | "sí" | "y" | "yes"
    )
}

fn setup_claude_code(replace: bool, claude_md: Option<bool>) -> Result<Vec<String>, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let claude = std::env::var_os("CLIO_MEMORY_CLAUDE")
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "claude".into());
    clio_memory::setup::ClaudeCode {
        home: home()?,
        claude: claude.into(),
        exe,
        replace,
        claude_md,
        ask: &ask,
    }
    .run()
}

fn setup_model() -> Result<Vec<String>, String> {
    use clio_memory::setup::{MODEL_FILES, curl_fetch, install_model};
    let dest = home()?.join(clio_memory::agent::memory::MODEL_REL);
    let curl = std::path::PathBuf::from("curl");
    let mut said = install_model(&dest, &MODEL_FILES, &curl_fetch(&curl))?;
    said.push(format!("modelo listo en {}", dest.display()));
    Ok(said)
}
