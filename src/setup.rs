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
    /// Runs `systemctl` or `launchctl` to install the nightly sleep (see `install_sleep`).
    pub service: &'a dyn Fn(&[&str]) -> Result<(), String>,
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
        // The sleep is part of the memory: without a timer it never runs. A machine without
        // a user service manager still gets the rest.
        said.push(if sleep_installed(&self.home, &self.exe, &["sleep"]) {
            "el sueño nocturno ya está instalado".into()
        } else if sleep_present(&self.home) {
            // Clio's (`clio memory sleep`) also merges with the person's model: not replaced.
            "ya hay otro sueño nocturno instalado (de Clio u otro binario): no se cambió; `clio-memory setup sleep` lo reemplaza".into()
        } else {
            match install_sleep(&self.home, &self.exe, &["sleep"], self.service) {
                Ok(at) => format!("sueño nocturno instalado: {}", at.display()),
                Err(e) => format!("el sueño nocturno no se instaló ({e}); `clio-memory setup sleep` lo intenta de nuevo"),
            }
        });
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

/// The name of the sleep's unit (systemd) and the stem of its job (launchd), whoever installs it:
/// the last installation decides the binary.
pub const SLEEP_UNIT: &str = "clio-memory-sleep";
const SLEEP_LABEL: &str = "com.clio-memory.sleep";
/// The nightly slots: every half hour from 23:00 to 05:30, local time.
const SLEEP_HOURS: [u8; 7] = [23, 0, 1, 2, 3, 4, 5];

/// The systemd user service and timer that run `exe args` at night. No `Persistent`: a night the
/// machine was off is skipped, never made up by day.
pub fn sleep_units(exe: &Path, args: &[&str]) -> (String, String) {
    let quote = |s: &str| {
        format!(
            "\"{}\"",
            s.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('%', "%%")
        )
    };
    let mut line = vec![quote(&exe.to_string_lossy())];
    line.extend(args.iter().map(|a| quote(a)));
    let service = format!(
        "[Unit]\nDescription=clio-memory: la fase de sueño de la memoria\n\n[Service]\nType=oneshot\nExecStart={}\nNice=10\n",
        line.join(" ")
    );
    let timer = "[Unit]\nDescription=clio-memory: despierta el sueño cada media hora de noche\n\n[Timer]\nOnCalendar=*-*-* 23,00,01,02,03,04,05:00,30:00\nAccuracySec=1min\n\n[Install]\nWantedBy=timers.target\n".to_owned();
    (service, timer)
}

/// The launchd job (macOS) for the same slots.
pub fn launchd_plist(exe: &Path, args: &[&str]) -> String {
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let mut program = format!("<string>{}</string>", esc(&exe.to_string_lossy()));
    for a in args {
        program.push_str(&format!("<string>{}</string>", esc(a)));
    }
    let slots: String = SLEEP_HOURS
        .iter()
        .flat_map(|h| [0, 30].map(|m| (h, m)))
        .map(|(h, m)| {
            format!("<dict><key>Hour</key><integer>{h}</integer><key>Minute</key><integer>{m}</integer></dict>")
        })
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{SLEEP_LABEL}</string><key>ProgramArguments</key><array>{program}</array><key>StartCalendarInterval</key><array>{slots}</array><key>LowPriorityIO</key><true/><key>Nice</key><integer>10</integer></dict></plist>\n"
    )
}

/// Whether the sleep for `exe args` is already installed exactly so.
pub fn sleep_installed(home: &Path, exe: &Path, args: &[&str]) -> bool {
    let same = |p: PathBuf, text: &str| std::fs::read_to_string(p).is_ok_and(|t| t == text);
    if cfg!(target_os = "macos") {
        return same(
            home.join("Library/LaunchAgents")
                .join(format!("{SLEEP_LABEL}.plist")),
            &launchd_plist(exe, args),
        );
    }
    let dir = home.join(".config/systemd/user");
    let (service, timer) = sleep_units(exe, args);
    same(dir.join(format!("{SLEEP_UNIT}.service")), &service)
        && same(dir.join(format!("{SLEEP_UNIT}.timer")), &timer)
}

/// Whether some sleep is installed, whatever it runs.
fn sleep_present(home: &Path) -> bool {
    if cfg!(target_os = "macos") {
        return home
            .join("Library/LaunchAgents")
            .join(format!("{SLEEP_LABEL}.plist"))
            .is_file();
    }
    home.join(".config/systemd/user")
        .join(format!("{SLEEP_UNIT}.service"))
        .is_file()
}

/// Runs a service manager command (`systemctl --user …`, `launchctl …`).
pub fn service_command(args: &[&str]) -> Result<(), String> {
    let (program, rest) = args.split_first().ok_or("orden vacía")?;
    let out = Command::new(program)
        .args(rest)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "`{}` falló: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Installs the nightly sleep for `exe args` and turns it on; `run` executes the service
/// manager's commands. Returns the timer (or the launchd job) it wrote.
pub fn install_sleep(
    home: &Path,
    exe: &Path,
    args: &[&str],
    run: &dyn Fn(&[&str]) -> Result<(), String>,
) -> Result<PathBuf, String> {
    if cfg!(target_os = "macos") {
        let plist = home
            .join("Library/LaunchAgents")
            .join(format!("{SLEEP_LABEL}.plist"));
        write_private(&plist, &launchd_plist(exe, args))?;
        let path = plist.to_string_lossy();
        let _ = run(&["launchctl", "unload", &path]);
        run(&["launchctl", "load", "-w", &path])?;
        return Ok(plist);
    }
    if !cfg!(target_os = "linux") {
        return Err("este sistema no tiene un timer conocido (solo Linux y macOS)".into());
    }
    let dir = home.join(".config/systemd/user");
    let (service, timer) = sleep_units(exe, args);
    write_private(&dir.join(format!("{SLEEP_UNIT}.service")), &service)?;
    let at = dir.join(format!("{SLEEP_UNIT}.timer"));
    write_private(&at, &timer)?;
    run(&["systemctl", "--user", "daemon-reload"])?;
    run(&[
        "systemctl",
        "--user",
        "enable",
        "--now",
        &format!("{SLEEP_UNIT}.timer"),
    ])?;
    Ok(at)
}

/// Turns the nightly sleep off and deletes its files.
pub fn remove_sleep(
    home: &Path,
    run: &dyn Fn(&[&str]) -> Result<(), String>,
) -> Result<(), String> {
    let gone = |p: &Path| match std::fs::remove_file(p) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", p.display())),
        _ => Ok(()),
    };
    if cfg!(target_os = "macos") {
        let plist = home
            .join("Library/LaunchAgents")
            .join(format!("{SLEEP_LABEL}.plist"));
        let _ = run(&["launchctl", "unload", "-w", &plist.to_string_lossy()]);
        return gone(&plist);
    }
    let dir = home.join(".config/systemd/user");
    let _ = run(&[
        "systemctl",
        "--user",
        "disable",
        "--now",
        &format!("{SLEEP_UNIT}.timer"),
    ]);
    gone(&dir.join(format!("{SLEEP_UNIT}.timer")))?;
    gone(&dir.join(format!("{SLEEP_UNIT}.service")))?;
    run(&["systemctl", "--user", "daemon-reload"])
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

/// An archive of the runtime the model needs (ONNX Runtime, and CUDA's libraries for the GPU):
/// where from, its sha256, the folder under the runtime's it unpacks into, and the library that,
/// when the system already has it, makes it unnecessary.
#[derive(Clone, Copy, Debug)]
pub struct Archive {
    pub url: &'static str,
    pub sha256: &'static str,
    pub into: &'static str,
    pub system_lib: Option<&'static str>,
}

/// Where `setup model` leaves the runtime, relative to the home: beside the models, where Granite
/// looks for it.
pub const RUNTIME_DIR: &str = ".cache/clio-models/onnxruntime";

const fn ort(url: &'static str, sha256: &'static str) -> Archive {
    Archive {
        url,
        sha256,
        into: "ort",
        system_lib: None,
    }
}

const ORT_LINUX_X64: Archive = ort(
    concat!(
        "https://github.com/microsoft/onnxruntime/releases/download/v1.31.1/",
        "onnxruntime-linux-x64-1.31.1.tgz"
    ),
    "c849e36a1ef3e22d4ff217b9d1ce4a3cf5b9467b975610913ae959ad5b16bdc9",
);
const ORT_LINUX_AARCH64: Archive = ort(
    concat!(
        "https://github.com/microsoft/onnxruntime/releases/download/v1.31.1/",
        "onnxruntime-linux-aarch64-1.31.1.tgz"
    ),
    "5317505e2da99be65f8e2775ba3c2afd6d2961fd78aba1bd8f56159f48820c78",
);
const ORT_MACOS_ARM64: Archive = ort(
    concat!(
        "https://github.com/microsoft/onnxruntime/releases/download/v1.31.1/",
        "onnxruntime-osx-arm64-1.31.1.tgz"
    ),
    "2e1344a1926ee50eb25b38153cb6992c3567515aef0e677a1a520a1798896d6f",
);
/// With the CUDA execution provider; it also runs on the CPU when the GPU does not answer.
const ORT_LINUX_X64_GPU: Archive = ort(
    concat!(
        "https://github.com/microsoft/onnxruntime/releases/download/v1.31.1/",
        "onnxruntime-linux-x64-gpu_cuda13-1.31.1.tgz"
    ),
    "b2146862e691f7e338f5aff9c7a9a5567c2ca57cf01442a1adb3a13bf62d9095",
);
/// cuDNN 9 and the CUDA 13 libraries the CUDA provider loads, from NVIDIA's redistributables.
const CUDA: [Archive; 6] = [
    Archive {
        url: concat!(
            "https://developer.download.nvidia.com/compute/",
            "cudnn/redist/cudnn/linux-x86_64/cudnn-linux-x86_64-9.28.0.42_cuda13-archive.tar.xz"
        ),
        sha256: "06409e4ab0a78664dc313c5a71d4444e1f8a8cd3eceee74c11c6eef1c9178e1e",
        into: "cudnn",
        system_lib: Some("libcudnn.so.9"),
    },
    Archive {
        url: concat!(
            "https://developer.download.nvidia.com/compute/",
            "cuda/redist/cuda_cudart/linux-x86_64/cuda_cudart-linux-x86_64-13.5.37-archive.tar.xz"
        ),
        sha256: "26ca1b8963ce1b73e0e77113c711730545cacaaee7233c0526477d96d4c621fe",
        into: "cuda-cudart",
        system_lib: Some("libcudart.so.13"),
    },
    Archive {
        url: concat!(
            "https://developer.download.nvidia.com/compute/",
            "cuda/redist/libcublas/linux-x86_64/libcublas-linux-x86_64-13.10.0.20-archive.tar.xz"
        ),
        sha256: "6da2dd456733bc45235e344a6e6aab3e8f43fb775a835c5dc432f4a1fc9f93d2",
        into: "cuda-cublas",
        system_lib: Some("libcublas.so.13"),
    },
    Archive {
        url: concat!(
            "https://developer.download.nvidia.com/compute/",
            "cuda/redist/cuda_nvrtc/linux-x86_64/cuda_nvrtc-linux-x86_64-13.5.37-archive.tar.xz"
        ),
        sha256: "aa4df05e6d97304a1443af87690f20a7a38b0fef4169d34cf30c860ba5ee28fa",
        into: "cuda-nvrtc",
        system_lib: Some("libnvrtc.so.13"),
    },
    Archive {
        url: concat!(
            "https://developer.download.nvidia.com/compute/",
            "cuda/redist/libcurand/linux-x86_64/libcurand-linux-x86_64-10.4.5.25-archive.tar.xz"
        ),
        sha256: "b58397cff0012cf2d143fca148aa7a58831b758cfaf2d4433f35cf267ea9b19b",
        into: "cuda-curand",
        system_lib: Some("libcurand.so.10"),
    },
    Archive {
        url: concat!(
            "https://developer.download.nvidia.com/compute/",
            "cuda/redist/libcufft/linux-x86_64/libcufft-linux-x86_64-12.5.0.14-archive.tar.xz"
        ),
        sha256: "a1d046fc32fec7c2484baad25b22252b0fff770cd698968bd826da51c22a0a01",
        into: "cuda-cufft",
        system_lib: Some("libcufft.so.12"),
    },
];
/// CUDA 13 needs a driver of this branch or newer.
const CUDA13_DRIVER: u32 = 580;

/// The major version of the NVIDIA driver, from `/proc/driver/nvidia/version`.
pub fn driver_major(text: &str) -> Option<u32> {
    let line = text.lines().find(|l| l.contains("Kernel Module"))?;
    line.split_whitespace()
        .find_map(|w| w.split_once('.').and_then(|(m, _)| m.parse().ok()))
}

/// What this machine needs to run Granite: the CPU runtime; with an NVIDIA card whose driver runs
/// CUDA 13, the GPU one, cuDNN, and the CUDA libraries `system_libs` (`ldconfig -p`) lacks. A system
/// without a published runtime gets nothing (the static model, or text).
pub fn runtime_for(
    os: &str,
    arch: &str,
    nvidia_driver: Option<u32>,
    system_libs: &str,
) -> Vec<&'static Archive> {
    match (os, arch) {
        ("linux", "x86_64") if nvidia_driver.is_some_and(|d| d >= CUDA13_DRIVER) => {
            std::iter::once(&ORT_LINUX_X64_GPU)
                .chain(
                    CUDA.iter()
                        .filter(|a| !a.system_lib.is_some_and(|l| system_libs.contains(l))),
                )
                .collect()
        }
        ("linux", "x86_64") => vec![&ORT_LINUX_X64],
        ("linux", "aarch64") => vec![&ORT_LINUX_AARCH64],
        ("macos", "aarch64") => vec![&ORT_MACOS_ARM64],
        _ => Vec::new(),
    }
}

/// What this machine needs, asking the machine.
pub fn runtime_here() -> Vec<&'static Archive> {
    let driver = std::fs::read_to_string("/proc/driver/nvidia/version")
        .ok()
        .and_then(|t| driver_major(&t));
    let libs = Command::new("ldconfig")
        .arg("-p")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    runtime_for(std::env::consts::OS, std::env::consts::ARCH, driver, &libs)
}

/// Installs `archives` under `root`, each in its folder: one already there with the same sha256 is
/// kept; any other is fetched with `fetch(url, to)`, checked, unpacked with `unpack(file, into)`
/// beside it and only then put in place of the old one. A wrong archive replaces nothing.
pub fn install_runtime(
    root: &Path,
    archives: &[&Archive],
    fetch: &dyn Fn(&str, &Path) -> Result<(), String>,
    unpack: &dyn Fn(&Path, &Path) -> Result<(), String>,
) -> Result<Vec<String>, String> {
    let err = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
    std::fs::create_dir_all(root).map_err(|e| err(root, e))?;
    let mut said = Vec::new();
    for a in archives {
        let dir = root.join(a.into);
        let mark = dir.join(".sha256");
        if std::fs::read_to_string(&mark).ok().as_deref() == Some(a.sha256) {
            said.push(format!("ok        {}", a.into));
            continue;
        }
        let file = root.join(format!(".{}.download", a.into));
        let staging = root.join(format!(".{}.new", a.into));
        let _ = std::fs::remove_dir_all(&staging);
        let done = fetch(a.url, &file)
            .and_then(|()| match sha256_of(&file) {
                Some(s) if s == a.sha256 => Ok(()),
                _ => Err(format!(
                    "sha256 distinto para {}: no se reemplazó nada",
                    a.url
                )),
            })
            .and_then(|()| std::fs::create_dir_all(&staging).map_err(|e| err(&staging, e)))
            .and_then(|()| unpack(&file, &staging))
            .and_then(|()| {
                std::fs::write(staging.join(".sha256"), a.sha256).map_err(|e| err(&staging, e))
            });
        let _ = std::fs::remove_file(&file);
        if let Err(e) = done {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::rename(&staging, &dir).map_err(|e| err(&dir, e))?;
        said.push(format!("instalado {}", a.into));
    }
    Ok(said)
}

/// Downloads `url` to `to` with `curl`.
pub fn curl_get(curl: &Path, url: &str, to: &Path) -> Result<(), String> {
    let ok = Command::new(curl)
        .args(["-fL", "--retry", "3", "--progress-bar", "-o"])
        .arg(to)
        .arg(url)
        .status()
        .map_err(|e| format!("curl: {e}"))?
        .success();
    if ok {
        Ok(())
    } else {
        Err(format!("no se pudo descargar {url}"))
    }
}

/// Unpacks a `.tgz` or `.tar.xz` into `into`, without its top folder, with the system's `tar`.
pub fn tar_unpack(file: &Path, into: &Path) -> Result<(), String> {
    let ok = Command::new("tar")
        .arg("-xf")
        .arg(file)
        .args(["--strip-components=1", "-C"])
        .arg(into)
        .status()
        .map_err(|e| format!("tar: {e}"))?
        .success();
    if ok {
        Ok(())
    } else {
        Err(format!("no se pudo abrir {}", file.display()))
    }
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
        curl_get(curl, &url, to).map_err(|_| format!("no se pudo descargar {name}"))
    }
}
