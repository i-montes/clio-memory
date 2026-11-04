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

/// One file of a model: where it is in the repository, its name on disk, its sha256.
pub struct ModelFile {
    pub remote: &'static str,
    pub local: &'static str,
    pub sha256: &'static str,
}

/// A vector model as `setup model` installs it: Hugging Face repository, pinned revision, files,
/// and the folder under the home it goes to.
pub struct ModelSpec {
    pub repo: &'static str,
    pub rev: &'static str,
    pub dir: &'static str,
    pub files: &'static [ModelFile],
}

impl ModelSpec {
    /// `(local name, sha256)` of each file, as `install_model` takes them.
    pub fn pairs(&self) -> Vec<(&'static str, &'static str)> {
        self.files.iter().map(|f| (f.local, f.sha256)).collect()
    }

    /// The download address of the file called `local` on disk.
    pub fn url(&self, local: &str) -> Option<String> {
        let f = self.files.iter().find(|f| f.local == local)?;
        Some(format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            self.repo, self.rev, f.remote
        ))
    }
}

/// The default model: Granite Embedding 311M multilingual R2 (IBM, Apache-2.0), the full ONNX
/// model, on the GPU or the CPU (`bench/embeddings/results.md`).
pub const GRANITE: ModelSpec = ModelSpec {
    repo: "ibm-granite/granite-embedding-311m-multilingual-r2",
    rev: "44399559930365213510b1ee2eb15ded83374f0e",
    dir: ".cache/clio-models/granite-embedding-311m-multilingual-r2",
    files: &[
        ModelFile {
            remote: "tokenizer.json",
            local: "tokenizer.json",
            sha256: "0087c868b33bad550a78a08d19798cfd7f713cde4f020803b8f51f405503e15f",
        },
        ModelFile {
            remote: "onnx/model.onnx",
            local: "model.onnx",
            sha256: "75f9f258bf5013f5fe8a4dad61dd0fd16ac0cbaa7a106e3d3f41c2d04a42d541",
        },
    ],
};

/// The static model, the fallback (the same files as Clio's `infra/models/install-potion.*.sh`).
pub const POTION: ModelSpec = ModelSpec {
    repo: "minishlab/potion-multilingual-128M",
    rev: "73908c3438cf03b6a01bcb9611d62b23d0726f08",
    dir: ".cache/clio-models/potion-multilingual-128M",
    files: &[
        ModelFile {
            remote: "config.json",
            local: "config.json",
            sha256: "595e4cab2093732efd5dbe084fd5c1826b5eea693b73b4c1fd971672867d2e54",
        },
        ModelFile {
            remote: "tokenizer.json",
            local: "tokenizer.json",
            sha256: "19f1909063da3cfe3bd83a782381f040dccea475f4816de11116444a73e1b6a1",
        },
        ModelFile {
            remote: "model.safetensors",
            local: "model.safetensors",
            sha256: "14b5eb39cb4ce5666da8ad1f3dc6be4346e9b2d601c073302fa0a31bf7943397",
        },
    ],
};

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
        // «Text file busy» (ETXTBSY): the program was just written and another thread's child
        // still holds it for a moment; it is there, so try again.
        let mut tries = 0;
        let out = loop {
            match Command::new(&self.claude).args(args).output() {
                Err(e) if e.raw_os_error() == Some(26) && tries < 50 => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                other => break other.map_err(|e| e.to_string())?,
            }
        };
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

/// The real download, with `curl`, from the model's pinned revision.
pub fn curl_fetch<'a>(
    curl: &'a Path,
    spec: &'a ModelSpec,
) -> impl Fn(&str, &Path) -> Result<(), String> + 'a {
    move |name, to| {
        let url = spec
            .url(name)
            .ok_or_else(|| format!("{name} no es un archivo del modelo"))?;
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
