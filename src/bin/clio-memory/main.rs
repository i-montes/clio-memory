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
