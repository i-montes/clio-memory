//! `clio-memory setup`: make clio-memory Claude Code's primary memory, and install the vector model.
//!
//! Claude Code is driven through its own command line (`claude mcp get/add/remove`): no file of its
//! configuration is edited by hand. What is written: the `clio-memory` MCP entry (user scope), the
//! skill `~/.claude/skills/clio-memory/SKILL.md`, and, only with the person's yes, one line in
//! `~/.claude/CLAUDE.md`. Something of the same name that is not this is never replaced without
//! the person's yes (or `--replace`). The model's files are pinned by sha256: a file that does not
//! match replaces nothing.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

/// The skill installed for Claude Code.
pub const SKILL: &str = include_str!("../skill/SKILL.md");
/// The line added to `~/.claude/CLAUDE.md` when the person says yes.
pub const CLAUDE_MD_LINE: &str =
    "- Mi memoria principal es clio-memory (MCP): úsala según la skill clio-memory.";
const NAME: &str = "clio-memory";

/// The default vector model: Hugging Face repository, pinned revision, and each file's sha256
/// (the same as Clio's `infra/models/install-potion.*.sh`).
pub const MODEL_REPO: &str = "minishlab/potion-multilingual-128M";
pub const MODEL_REV: &str = "73908c3438cf03b6a01bcb9611d62b23d0726f08";
pub const MODEL_FILES: [(&str, &str); 3] = [
    (
        "config.json",
        "595e4cab2093732efd5dbe084fd5c1826b5eea693b73b4c1fd971672867d2e54",
    ),
    (
        "tokenizer.json",
        "19f1909063da3cfe3bd83a782381f040dccea475f4816de11116444a73e1b6a1",
    ),
    (
        "model.safetensors",
        "14b5eb39cb4ce5666da8ad1f3dc6be4346e9b2d601c073302fa0a31bf7943397",
    ),
];

/// One run of `setup claude-code`.
pub struct ClaudeCode<'a> {
    pub home: PathBuf,
    /// The `claude` program.
    pub claude: PathBuf,
    /// This binary, by its absolute path: what the MCP entry runs.
    pub exe: PathBuf,
    /// Replace an MCP server or a skill of the same name that is not this one.
    pub replace: bool,
    /// The `CLAUDE.md` line: yes, no, or ask (`None`).
    pub claude_md: Option<bool>,
    /// Asks the person a yes/no question (false when there is nobody to ask).
    pub ask: &'a dyn Fn(&str) -> bool,
}

impl ClaudeCode<'_> {
    /// Does it, and says what it did (one line each), or why nothing was done.
    pub fn run(&self) -> Result<Vec<String>, String> {
        // `claude` first: without it nothing is written, not even the skill.
        self.claude(&["--version"]).map_err(|_| {
            format!(
                "no se encontró `claude` ({}): instala Claude Code o pon su ruta en CLIO_MEMORY_CLAUDE",
                self.claude.display()
            )
        })?;
        let ours = format!("{} mcp", self.exe.display());
        let registered = self.claude(&["mcp", "get", NAME]).ok();
        let ours_already = registered.as_deref().is_some_and(|r| r.contains(&ours));
        let skill = self.home.join(".claude/skills").join(NAME).join("SKILL.md");
        let skill_now = std::fs::read_to_string(&skill).ok();
        let skill_ok = skill_now.as_deref() == Some(SKILL);
        // Something of the same name that is not this one: only with a yes.
        let foreign = (
            registered.is_some() && !ours_already,
            skill_now.is_some() && !skill_ok,
        );
        if (foreign.0 || foreign.1) && !self.replace {
            let what = match foreign {
                (true, true) => "un servidor MCP y una skill llamados clio-memory",
                (true, false) => "un servidor MCP llamado clio-memory",
                _ => "una skill clio-memory distinta",
            };
            if !(self.ask)(&format!("Claude Code ya tiene {what}. ¿Reemplazarlo?")) {
                return Err(format!(
                    "Claude Code ya tiene {what}: no se cambió nada (usa --replace para reemplazarlo)"
                ));
            }
        }
        let mut said = Vec::new();
        if skill_ok {
            said.push(format!("la skill ya está en {}", skill.display()));
        } else {
            write_private(&skill, SKILL)?;
            said.push(format!("skill instalada en {}", skill.display()));
        }
        if ours_already {
            said.push("el servidor MCP clio-memory ya está registrado".into());
        } else {
            if registered.is_some() {
                self.claude(&["mcp", "remove", "--scope", "user", NAME])?;
            }
            let exe = self.exe.to_string_lossy();
            self.claude(&["mcp", "add", "--scope", "user", NAME, "--", &exe, "mcp"])?;
            said.push(format!("servidor MCP clio-memory registrado: {ours}"));
        }
        let md = self.home.join(".claude/CLAUDE.md");
        let text = std::fs::read_to_string(&md).unwrap_or_default();
        if text.lines().any(|l| l.trim() == CLAUDE_MD_LINE) {
            said.push("la línea de CLAUDE.md ya está".into());
        } else {
            let yes = self.claude_md.unwrap_or_else(|| {
                (self.ask)(&format!(
                    "¿Añado a {} la línea «{CLAUDE_MD_LINE}»?",
                    md.display()
                ))
            });
            if yes {
                let sep = if text.is_empty() || text.ends_with('\n') {
                    ""
                } else {
                    "\n"
                };
                write_private(&md, &format!("{text}{sep}{CLAUDE_MD_LINE}\n"))?;
                said.push(format!("línea añadida a {}", md.display()));
            }
        }
        Ok(said)
    }

    fn claude(&self, args: &[&str]) -> Result<String, String> {
        let out = Command::new(&self.claude)
            .args(args)
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(format!(
                "`claude {}` falló: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }
}

/// Writes `text` whole (a temporary file, then rename), making the folders it needs.
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    let dir = path.parent().ok_or("ruta sin carpeta")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = path.with_extension("tmp-clio-memory");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

fn sha256_of(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(
        Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    )
}

/// Installs `files` (name, sha256) in `dest`: a file already good is kept; any other is fetched
/// into a temporary file with `fetch(name, to)` and moved in only when its sha256 matches.
pub fn install_model(
    dest: &Path,
    files: &[(&str, &str)],
    fetch: &dyn Fn(&str, &Path) -> Result<(), String>,
) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let mut said = Vec::new();
    for (name, want) in files {
        let target = dest.join(name);
        if sha256_of(&target).as_deref() == Some(*want) {
            said.push(format!("ok        {name}"));
            continue;
        }
        let tmp = dest.join(format!(".{name}.download"));
        let fetched = fetch(name, &tmp).and_then(|()| {
            if sha256_of(&tmp).as_deref() == Some(*want) {
                Ok(())
            } else {
                Err(format!("sha256 distinto para {name}: no se reemplazó nada"))
            }
        });
        if let Err(e) = fetched {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        std::fs::rename(&tmp, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        said.push(format!("instalado {name}"));
    }
    Ok(said)
}

/// The real download, with `curl`, from the pinned revision.
pub fn curl_fetch(curl: &Path) -> impl Fn(&str, &Path) -> Result<(), String> + '_ {
    move |name, to| {
        let url = format!("https://huggingface.co/{MODEL_REPO}/resolve/{MODEL_REV}/{name}");
        let ok = Command::new(curl)
            .args(["-fL", "--retry", "3", "--progress-bar", "-o"])
            .arg(to)
            .arg(&url)
            .status()
            .map_err(|e| format!("curl: {e}"))?
            .success();
        if ok {
            Ok(())
        } else {
            Err(format!("no se pudo descargar {name}"))
        }
    }
}
