//! `clio-memory setup` with no subcommand: the whole flow, from the header to the closing line.
//!
//! The flow asks and tells through a `Screen`: `Plain` (here) prints one line per event, for no
//! terminal; the TUI draws the same events. Without a terminal, or with `--yes`, nothing is asked:
//! the flags and, where they are missing, the defaults of the spec decide (on a re-run, what is
//! there: the agents connected, the model installed, the lines as they are), and flags only add
//! (no agent is disconnected, no model deleted, no line taken out). A flag given in a terminal
//! answers its question the same way.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clio_memory::setup::agents::{AgentId, Env, Found, detect, shadowing, take_away_shadow};
use clio_memory::setup::instructions::instructions_file;
use clio_memory::setup::model_choice::{
    ModelChoice, default_choice, granite_available, installed_best, to_remove,
};
use clio_memory::setup::plan::{Selection, Step, apply, connected};
use clio_memory::setup::{
    GRANITE, ModelSpec, POTION, RUNTIME_DIR, curl_get_watching, driver_major, install_model,
    install_runtime, runtime_here, tar_unpack,
};

/// What `setup` was given on the command line (`args::Command::Setup`).
#[derive(Debug, Default)]
pub struct SetupArgs {
    pub yes: bool,
    pub from_installer: bool,
    pub agents: Option<Vec<AgentId>>,
    pub all: bool,
    pub model: Option<ModelChoice>,
    pub instructions: Option<bool>,
    pub replace: bool,
    /// Whether stderr is a terminal (with `yes`, it decides whether the flow asks: `interactive`).
    pub terminal: bool,
    /// The `curl` that downloads the model (`curl` on the `PATH`; a fake in tests).
    pub curl: PathBuf,
}

/// How the flow shows itself and asks. Every `choose_*` and `confirm` returns `None` when the
/// person cancelled.
pub trait Screen {
    fn header(&mut self, version: &str);
    fn machine(&mut self, os: &str, arch: &str, gpu: Option<&str>);
    /// clio-memory is installed at `at` (only when it comes from the installer).
    fn installed(&mut self, at: &Path);
    fn choose_agents(&mut self, found: &[Found], connected: &[AgentId]) -> Option<Vec<AgentId>>;
    fn choose_model(
        &mut self,
        options: &[ModelChoice],
        current: ModelChoice,
    ) -> Option<ModelChoice>;
    fn confirm(&mut self, question: &str, default: bool) -> Option<bool>;
    fn step(&mut self, step: &Step);
    /// A download of `label` starts (`total` bytes, when known): the returned function gets the
    /// bytes so far, many times a second.
    fn download(&mut self, label: &str, total: Option<u64>) -> Box<dyn Fn(u64) + '_>;
    fn note(&mut self, text: &str);
    /// The end: the agents where the memory is now, and whether something failed.
    fn done(&mut self, agents: &[AgentId], failed: bool);
}

/// One line per event on stdout, nothing drawn or redrawn: for no terminal (a pipe, a log). Asked
/// anything, it answers what was proposed.
pub struct Plain;

impl Plain {
    fn line(text: &str) {
        crate::output::line(text);
    }
}

impl Screen for Plain {
    fn header(&mut self, version: &str) {
        Self::line(&format!("clio-memory {version}"));
    }

    fn machine(&mut self, os: &str, arch: &str, gpu: Option<&str>) {
        let gpu = gpu.map(|g| format!(", {g}")).unwrap_or_default();
        Self::line(&format!("✓ Esta máquina · {os} {arch}{gpu}"));
    }

    fn installed(&mut self, at: &Path) {
        Self::line(&format!("✓ clio-memory instalado · {}", at.display()));
    }

    fn choose_agents(&mut self, found: &[Found], _connected: &[AgentId]) -> Option<Vec<AgentId>> {
        Some(found.iter().map(|f| f.agent).collect())
    }

    fn choose_model(
        &mut self,
        _options: &[ModelChoice],
        current: ModelChoice,
    ) -> Option<ModelChoice> {
        Some(current)
    }

    fn confirm(&mut self, _question: &str, default: bool) -> Option<bool> {
        Some(default)
    }

    fn step(&mut self, step: &Step) {
        let what = match step.agent {
            Some(agent) => agent.label(),
            None => step.what.as_str(),
        };
        // One line per event, even when a program's error took several.
        let one = |text: &str| text.lines().map(str::trim).collect::<Vec<_>>().join(" / ");
        Self::line(&match &step.outcome {
            Ok(detail) => format!("✓ {what} · {}", one(detail)),
            Err(why) => format!("✖ {what} · {}", one(why)),
        });
    }

    /// Nothing per tick: the model's step says how it ended.
    fn download(&mut self, _label: &str, _total: Option<u64>) -> Box<dyn Fn(u64) + '_> {
        Box::new(|_| {})
    }

    fn note(&mut self, text: &str) {
        Self::line(text);
    }

    fn done(&mut self, agents: &[AgentId], failed: bool) {
        if failed {
            Self::line("Algo no se pudo hacer (las líneas con ✖); lo demás quedó hecho.");
        }
        if agents.is_empty() {
            Self::line(
                "Listo. Ningún agente quedó conectado: cuando instales uno, corre `clio-memory setup`.",
            );
        } else {
            Self::line(&format!(
                "Listo. Abre {}: la memoria ya está ahí.",
                join_labels(agents)
            ));
        }
    }
}

/// Whether the flow asks: only with no `--yes` and a terminal on stderr. The one place this is
/// decided, so the screen and the flow agree.
pub fn interactive(yes: bool, terminal: bool) -> bool {
    !yes && terminal
}

/// `Claude Code`, `Claude Code y Codex`, `Claude Code, Codex y Gemini CLI`.
pub fn join_labels(agents: &[AgentId]) -> String {
    let labels: Vec<&str> = agents.iter().map(|a| a.label()).collect();
    match labels.split_last() {
        None => String::new(),
        Some((last, [])) => (*last).to_owned(),
        Some((last, rest)) => format!("{} y {last}", rest.join(", ")),
    }
}

/// The line to add to the shell's startup file so `bin_dir` is on the `PATH`, or `None` when it
/// already is. `home` only makes the line shorter (`$HOME/.local/bin`); `shell` is `$SHELL`.
pub fn path_hint(bin_dir: &Path, home: &Path, path_env: &str, shell: &str) -> Option<String> {
    if std::env::split_paths(path_env).any(|p| p == bin_dir) {
        return None;
    }
    // `~` only expands outside quotes, so fish gets `~/` and the rest quoted when it has to be.
    let quoted = |p: &Path| {
        let text = p.display().to_string();
        if text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c))
        {
            text
        } else {
            format!("\"{text}\"")
        }
    };
    let (sh, fish) = match bin_dir.strip_prefix(home) {
        Ok(rel) => (
            format!("$HOME/{}", rel.display()),
            format!("~/{}", quoted(rel)),
        ),
        Err(_) => (bin_dir.display().to_string(), quoted(bin_dir)),
    };
    let export = |rc: &str| format!(r#"echo 'export PATH="{sh}:$PATH"' >> {rc}"#);
    Some(match shell.rsplit('/').next().unwrap_or(shell) {
        "zsh" => export("~/.zshrc"),
        "bash" => export("~/.bashrc"),
        "fish" => format!("fish_add_path {fish}"),
        _ => export("~/.profile"),
    })
}

/// Better models rank higher: Granite over potion over none.
fn rank(m: ModelChoice) -> u8 {
    match m {
        ModelChoice::None => 0,
        ModelChoice::Potion => 1,
        ModelChoice::Granite => 2,
    }
}

fn model_label(m: ModelChoice) -> &'static str {
    match m {
        ModelChoice::Granite => "Granite 311M",
        ModelChoice::Potion => "potion",
        ModelChoice::None => "ninguno",
    }
}

/// `1,3 GB`, `512 MB`: the size shown before deleting.
fn human(bytes: u64) -> String {
    let (n, unit) = match bytes {
        b if b >= 1_000_000_000 => (b as f64 / 1e9, "GB"),
        b if b >= 1_000_000 => (b as f64 / 1e6, "MB"),
        b => (b as f64 / 1e3, "KB"),
    };
    format!("{n:.1} {unit}").replace('.', ",")
}

/// The flow's own step (not an agent's).
fn step(what: &str, outcome: Result<String, String>) -> Step {
    Step {
        agent: None,
        what: what.to_owned(),
        outcome,
    }
}

/// The person cancelled a question: nothing more is done.
fn cancelled(screen: &mut dyn Screen) -> ExitCode {
    screen.note("Cancelado: no se cambió nada más.");
    ExitCode::from(1)
}

/// The NVIDIA card, when there is one with its driver loaded.
fn gpu() -> Option<String> {
    let text = std::fs::read_to_string("/proc/driver/nvidia/version").ok()?;
    Some(match driver_major(&text) {
        Some(major) => format!("GPU NVIDIA (driver {major})"),
        None => "GPU NVIDIA".to_owned(),
    })
}

/// The whole of `setup`: what was chosen is done step by step, and one step that fails (an agent,
/// the model) leaves the others done and makes the exit code 1.
pub fn run(screen: &mut dyn Screen, env: &Env, args: SetupArgs) -> ExitCode {
    let asks = interactive(args.yes, args.terminal);
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    screen.header(env!("CARGO_PKG_VERSION"));
    let os_label = match os {
        "linux" => "Linux",
        "macos" => "macOS",
        other => other,
    };
    screen.machine(os_label, arch, gpu().as_deref());
    if args.from_installer {
        screen.installed(&env.exe);
    }

    // The agents: the flag's, else (asked, or) every one found; without asking, a re-run (some
    // agent connected already) keeps the ones connected. An agent whose config is left but that
    // is no longer installed is not kept: there is nothing to connect.
    let found = detect(env);
    let before: Vec<AgentId> = connected(env)
        .into_iter()
        .filter(|a| found.iter().any(|f| f.agent == *a))
        .collect();
    let rerun = !before.is_empty();
    let mut exclusive = false;
    let agents: Vec<AgentId> = match &args.agents {
        Some(list) => list.clone(),
        None if args.all || (!asks && !rerun) => found.iter().map(|f| f.agent).collect(),
        None if !asks => {
            let left: Vec<AgentId> = found
                .iter()
                .map(|f| f.agent)
                .filter(|a| !before.contains(a))
                .collect();
            if !left.is_empty() {
                let keys: Vec<&str> = left.iter().map(|a| a.key()).collect();
                screen.note(&format!(
                    "Se queda lo conectado. También está {}: `clio-memory setup --agents {}` lo conecta.",
                    join_labels(&left),
                    keys.join(",")
                ));
            }
            before.clone()
        }
        None => {
            let Some(chosen) = screen.choose_agents(&found, &before) else {
                return cancelled(screen);
            };
            let dropped: Vec<AgentId> = before
                .iter()
                .copied()
                .filter(|a| !chosen.contains(a))
                .collect();
            if !dropped.is_empty() {
                let question = format!("¿Desconecto {}?", join_labels(&dropped));
                match screen.confirm(&question, false) {
                    Some(yes) => exclusive = yes,
                    None => return cancelled(screen),
                }
            }
            chosen
        }
    };
    if found.is_empty() && args.agents.is_none() {
        screen.note(
            "No encontré ningún agente en esta máquina: el binario y el modelo sirven igual, y luego `clio-memory setup` los conecta.",
        );
    }

    // The model: the flag's, else (asked, or) the best one installed or the one recommended here.
    let best = installed_best(&env.home);
    let model = match args.model {
        Some(m) => m,
        // The one installed is kept; on a re-run with none, still none (nothing is downloaded
        // that was not asked for); on a first install, the one recommended here.
        None if !asks && best != ModelChoice::None => best,
        None if !asks && rerun => ModelChoice::None,
        None if !asks => default_choice(os, arch),
        None => {
            let mut options = Vec::new();
            if granite_available(os, arch) {
                options.push(ModelChoice::Granite);
            }
            options.extend([ModelChoice::Potion, ModelChoice::None]);
            let current = if best == ModelChoice::None {
                default_choice(os, arch)
            } else {
                best
            };
            match screen.choose_model(&options, current) {
                Some(m) => m,
                None => return cancelled(screen),
            }
        }
    };
    // A lesser model than the one installed deletes the better one, only when chosen on the
    // screen and confirmed: the loader always takes the best one there is.
    let mut removal = Vec::new();
    if asks && args.model.is_none() && rank(model) < rank(best) {
        let gone = to_remove(&env.home, model);
        if !gone.is_empty() {
            let size: u64 = gone.iter().map(|(_, n)| n).sum();
            let list: Vec<String> = gone.iter().map(|(p, _)| p.display().to_string()).collect();
            let question = format!(
                "¿Borro {} ({})? Los modelos de ~/.cache/clio-models también los usa Clio; si los necesita, se vuelven a bajar.",
                list.join(", "),
                human(size)
            );
            match screen.confirm(&question, false) {
                Some(true) => removal = gone,
                Some(false) => screen.note(&format!(
                    "Se queda {}: es el que se usa.",
                    model_label(best)
                )),
                None => return cancelled(screen),
            }
        }
    }

    // The instructions line: without asking, yes on a first install, and on a re-run as it is
    // (no line added to an agent that has none).
    let instructions = match args.instructions {
        Some(yes) => yes,
        None if !asks => !rerun,
        None if agents.is_empty() => true,
        None => {
            let files: Vec<String> = agents
                .iter()
                .map(|a| instructions_file(env, *a).display().to_string())
                .collect();
            screen.note(&format!("Archivos de instrucciones: {}", files.join(", ")));
            match screen.confirm(
                "¿Agrego una línea en el archivo de instrucciones de cada agente que apunte a la skill?",
                true,
            ) {
                Some(yes) => yes,
                None => return cancelled(screen),
            }
        }
    };

    // Everything is decided: now it is done.
    let mut failed = false;
    // With every agent unchecked and a yes, the selection is empty and still disconnects.
    if !agents.is_empty() || exclusive {
        let sel = Selection {
            agents,
            instructions,
            replace: args.replace,
            exclusive,
        };
        let steps = {
            let cell = RefCell::new(&mut *screen);
            let ask = |q: &str| asks && cell.borrow_mut().confirm(q, false).unwrap_or(false);
            apply(env, &sel, &ask)
        };
        for s in &steps {
            failed |= s.outcome.is_err();
            screen.step(s);
        }
    }
    // The chosen model first, even under a better one that is going: the better one is deleted
    // only once the chosen one is in place, so a failed download never leaves the memory without.
    let mut installed = true;
    for s in model_steps(screen, &env.home, model, &args.curl, !removal.is_empty()) {
        installed &= s.outcome.is_ok();
        screen.step(&s);
    }
    failed |= !installed;
    if !installed && !removal.is_empty() {
        screen.note(&format!(
            "No se borró {}: el modelo elegido no quedó instalado.",
            model_label(best)
        ));
        removal.clear();
    }
    for (dir, size) in &removal {
        let s = step(
            "modelo",
            std::fs::remove_dir_all(dir)
                .map(|()| format!("borrado {} ({})", dir.display(), human(*size)))
                .map_err(|e| format!("{}: {e}", dir.display())),
        );
        failed |= s.outcome.is_err();
        screen.step(&s);
    }

    if args.from_installer {
        let bin_dir = env
            .bin_dir
            .clone()
            .unwrap_or_else(|| env.home.join(".local/bin"));
        let path_env = std::env::join_paths(&env.path)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let shell = std::env::var("SHELL").unwrap_or_default();
        if let Some(line) = path_hint(&bin_dir, &env.home, &path_env, &shell) {
            screen.note(&format!(
                "{} no está en tu PATH. Para agregarla:\n  {line}\nMientras tanto: {} doctor",
                bin_dir.display(),
                env.exe.display()
            ));
        }
    }
    if let Some(other) = shadowing(env) {
        match take_away_shadow(env, &other) {
            Ok(said) => screen.note(&format!("{said}.")),
            Err(note) => screen.note(&format!("Ojo: {note}.")),
        }
    }
    // Only the agents that are here: a config left by an uninstalled one opens nothing.
    let here: Vec<AgentId> = connected(env)
        .into_iter()
        .filter(|a| found.iter().any(|f| f.agent == *a))
        .collect();
    screen.done(&here, failed);
    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// Installs `model` unless one as good is already there (the loader takes the best one, so
/// nothing lesser is downloaded) or, with `force`, because the better one is about to be deleted.
/// Nothing is deleted here.
fn model_steps(
    screen: &mut dyn Screen,
    home: &Path,
    model: ModelChoice,
    curl: &Path,
    force: bool,
) -> Vec<Step> {
    let best = installed_best(home);
    let spec: &ModelSpec = match model {
        ModelChoice::Granite => &GRANITE,
        ModelChoice::Potion => &POTION,
        ModelChoice::None if best == ModelChoice::None || force => {
            return vec![step(
                "modelo",
                Ok("ninguno: la búsqueda será por texto".to_owned()),
            )];
        }
        ModelChoice::None => {
            return vec![step(
                "modelo",
                Ok(format!(
                    "{} sigue instalado y es el que se usa",
                    model_label(best)
                )),
            )];
        }
    };
    if rank(best) >= rank(model) && !force {
        return vec![step(
            "modelo",
            Ok(format!("{} ya está instalado", model_label(best))),
        )];
    }
    let cell = RefCell::new(screen);
    // A download through `curl_get_watching`, its bytes on the screen. A failed one leaves its
    // partial file: `install_model` and `install_runtime` remove it.
    let watched = |label: &str, url: &str, total: Option<u64>, to: &Path| {
        let mut s = cell.borrow_mut();
        let tick = s.download(label, total);
        curl_get_watching(curl, url, to, &*tick)
    };
    let label = model_label(model);
    let dest = home.join(spec.dir);
    let fetch = |name: &str, to: &Path| {
        let url = spec
            .url(name)
            .ok_or_else(|| format!("{name} no es un archivo del modelo"))?;
        let size = spec.files.iter().find(|f| f.local == name).map(|f| f.size);
        watched(&format!("{label} · {name}"), &url, size, to)
    };
    let mut steps = vec![step(
        "modelo",
        install_model(&dest, &spec.pairs(), &fetch)
            .map(|_| format!("{label} listo en {}", dest.display())),
    )];
    if model != ModelChoice::Granite || steps[0].outcome.is_err() {
        return steps;
    }
    // What Granite runs on: the CPU runtime, or with an NVIDIA card the GPU one and the CUDA
    // libraries the system lacks.
    let runtime = runtime_here();
    if runtime.is_empty() {
        steps.push(step(
            "runtime",
            Err(
                "no hay ONNX Runtime publicado para este sistema: se usará potion o el texto"
                    .into(),
            ),
        ));
        return steps;
    }
    let gpu = runtime.len() > 1 || runtime[0].url.contains("gpu");
    let root = home.join(RUNTIME_DIR);
    let fetch = |url: &str, to: &Path| {
        let name = url.rsplit('/').next().unwrap_or(url);
        watched(name, url, None, to)
    };
    steps.push(step(
        "runtime",
        install_runtime(&root, &runtime, &fetch, &tar_unpack).map(|_| {
            format!(
                "listo en {} ({})",
                root.display(),
                if gpu {
                    "GPU, y el procesador si la GPU no responde"
                } else {
                    "procesador"
                }
            )
        }),
    ));
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_hint_cases() {
        let home = Path::new("/home/ana");
        let bin = Path::new("/home/ana/.local/bin");
        let path = "/usr/bin:/bin";
        let export = |rc: &str| format!(r#"echo 'export PATH="$HOME/.local/bin:$PATH"' >> {rc}"#);
        assert_eq!(
            path_hint(bin, home, path, "/bin/zsh"),
            Some(export("~/.zshrc"))
        );
        assert_eq!(
            path_hint(bin, home, path, "/usr/bin/bash"),
            Some(export("~/.bashrc"))
        );
        assert_eq!(
            path_hint(bin, home, path, "/usr/bin/fish"),
            Some("fish_add_path ~/.local/bin".to_owned())
        );
        assert_eq!(
            path_hint(bin, home, path, "/bin/dash"),
            Some(export("~/.profile"))
        );
        assert_eq!(path_hint(bin, home, path, ""), Some(export("~/.profile")));
        assert_eq!(
            path_hint(bin, home, "/usr/bin:/home/ana/.local/bin:/bin", "/bin/zsh"),
            None
        );
        assert_eq!(
            path_hint(bin, home, "/home/ana/.local/bin/", "/bin/zsh"),
            None
        );
        // `$CLIO_MEMORY_BIN_DIR` outside the home: the whole path, quoted when it has to be.
        assert_eq!(
            path_hint(Path::new("/opt/clio/bin"), home, path, "/usr/bin/fish"),
            Some("fish_add_path /opt/clio/bin".to_owned())
        );
        assert_eq!(
            path_hint(Path::new("/opt/my dir"), home, path, "/usr/bin/fish"),
            Some(r#"fish_add_path "/opt/my dir""#.to_owned())
        );
        assert_eq!(
            path_hint(Path::new("/home/ana/my bin"), home, path, "fish"),
            Some(r#"fish_add_path ~/"my bin""#.to_owned())
        );
        assert_eq!(
            path_hint(Path::new("/opt/my dir"), home, path, "/bin/zsh"),
            Some(r#"echo 'export PATH="/opt/my dir:$PATH"' >> ~/.zshrc"#.to_owned())
        );
    }

    #[test]
    fn labels_are_joined_with_commas_and_y() {
        use AgentId::{ClaudeCode, Codex, Gemini};
        assert_eq!(join_labels(&[Codex]), "Codex");
        assert_eq!(join_labels(&[Codex, Gemini]), "Codex y Gemini CLI");
        assert_eq!(
            join_labels(&[ClaudeCode, Codex, Gemini]),
            "Claude Code, Codex y Gemini CLI"
        );
    }

    #[test]
    fn interactive_needs_a_terminal_and_no_yes() {
        assert!(interactive(false, true));
        assert!(!interactive(true, true));
        assert!(!interactive(false, false));
        assert!(!interactive(true, false));
    }

    #[test]
    fn plain_never_asks_and_proposes_what_it_was_given() {
        let mut p = Plain;
        assert_eq!(p.confirm("¿?", false), Some(false));
        assert_eq!(p.confirm("¿?", true), Some(true));
        assert_eq!(
            p.choose_model(&[ModelChoice::Potion], ModelChoice::Potion),
            Some(ModelChoice::Potion)
        );
    }

    // ---- the interactive flow, driven by a scripted screen ----

    use clio_memory::setup::agents::Env;
    use clio_memory::setup::plan::connected;

    /// Answers from a script and keeps what it was shown.
    #[derive(Default)]
    struct Script {
        agents: Vec<AgentId>,
        model: Option<ModelChoice>,
        /// The answers to `confirm`, in order; a question past them fails the test.
        answers: Vec<bool>,
        asked: Vec<String>,
        steps: Vec<Step>,
        done: Option<Vec<AgentId>>,
    }

    impl Screen for Script {
        fn header(&mut self, _: &str) {}
        fn machine(&mut self, _: &str, _: &str, _: Option<&str>) {}
        fn installed(&mut self, _: &Path) {}
        fn choose_agents(&mut self, _: &[Found], _: &[AgentId]) -> Option<Vec<AgentId>> {
            Some(self.agents.clone())
        }
        fn choose_model(&mut self, _: &[ModelChoice], current: ModelChoice) -> Option<ModelChoice> {
            Some(self.model.unwrap_or(current))
        }
        fn confirm(&mut self, question: &str, _: bool) -> Option<bool> {
            assert!(
                self.asked.len() < self.answers.len(),
                "unexpected question: {question}"
            );
            self.asked.push(question.to_owned());
            Some(self.answers[self.asked.len() - 1])
        }
        fn step(&mut self, step: &Step) {
            self.steps.push(step.clone());
        }
        fn download(&mut self, _: &str, _: Option<u64>) -> Box<dyn Fn(u64) + '_> {
            Box::new(|_| {})
        }
        fn note(&mut self, _: &str) {}
        fn done(&mut self, agents: &[AgentId], _: bool) {
            self.done = Some(agents.to_vec());
        }
    }

    fn no_service(_: &[&str]) -> Result<(), String> {
        Ok(())
    }

    /// A home where OpenCode and Antigravity are found (by their folders), and nothing runs: no
    /// program on the `PATH`, the service manager a no-op.
    fn home() -> (tempfile::TempDir, Env<'static>) {
        let t = tempfile::tempdir().expect("temp dir");
        let home = t.path().join("home");
        for d in [".config/opencode", ".gemini/config"] {
            std::fs::create_dir_all(home.join(d)).expect("folder");
        }
        let env = Env {
            home,
            exe: t.path().join("bin/clio-memory"),
            path: Vec::new(),
            codex_home: None,
            claude: None,
            bin_dir: None,
            service: &no_service,
        };
        (t, env)
    }

    /// `run` in a terminal (no `--yes`), downloading with `curl`.
    fn ask(script: &mut Script, env: &Env, curl: &Path) {
        let args = SetupArgs {
            terminal: true,
            curl: curl.to_path_buf(),
            ..SetupArgs::default()
        };
        run(script, env, args);
    }

    /// OpenCode and Antigravity connected through the flow itself, with no model.
    fn both_connected(env: &Env) {
        let mut first = Script {
            agents: vec![AgentId::OpenCode, AgentId::Antigravity],
            model: Some(ModelChoice::None),
            answers: vec![true],
            ..Script::default()
        };
        ask(&mut first, env, Path::new("/nonexistent/curl"));
        assert_eq!(
            connected(env),
            [AgentId::OpenCode, AgentId::Antigravity],
            "{:?}",
            first.steps
        );
    }

    #[test]
    fn unchecking_every_agent_and_saying_yes_disconnects_them() {
        let (_t, env) = home();
        both_connected(&env);
        let mut s = Script {
            model: Some(ModelChoice::None),
            answers: vec![true],
            ..Script::default()
        };
        ask(&mut s, &env, Path::new("/nonexistent/curl"));
        assert_eq!(s.asked, ["¿Desconecto OpenCode y Antigravity?"]);
        assert_eq!(connected(&env), [], "{:?}", s.steps);
        assert_eq!(s.done, Some(vec![]));
        assert!(s.steps.iter().all(|st| st.outcome.is_ok()), "{:?}", s.steps);
    }

    #[test]
    fn unchecking_every_agent_and_saying_no_disconnects_nothing() {
        let (_t, env) = home();
        both_connected(&env);
        let mut s = Script {
            model: Some(ModelChoice::None),
            answers: vec![false],
            ..Script::default()
        };
        ask(&mut s, &env, Path::new("/nonexistent/curl"));
        assert_eq!(s.asked, ["¿Desconecto OpenCode y Antigravity?"]);
        let both = vec![AgentId::OpenCode, AgentId::Antigravity];
        assert_eq!(connected(&env), both);
        assert_eq!(s.done, Some(both));
    }

    /// Granite's files, as `installed_best` sees them.
    fn granite_installed(env: &Env) -> PathBuf {
        let dir = env.home.join(GRANITE.dir);
        std::fs::create_dir_all(&dir).expect("folder");
        for f in GRANITE.files {
            std::fs::write(dir.join(f.local), "pesos").expect("file");
        }
        dir
    }

    /// A `curl` that writes part of the file and fails, logging each call to `<dir>/curl.log`.
    fn failing_curl(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let curl = dir.join("curl");
        std::fs::write(
            &curl,
            format!(
                "#!/bin/sh\necho \"$@\" >> '{}'\nwhile [ $# -gt 0 ]; do [ \"$1\" = -o ] && printf partial > \"$2\"; shift; done\nexit 56\n",
                dir.join("curl.log").display()
            ),
        )
        .expect("curl");
        std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).expect("mode");
        curl
    }

    #[test]
    fn a_lesser_model_and_a_no_deletes_nothing_and_downloads_nothing() {
        let (t, env) = home();
        let granite = granite_installed(&env);
        let curl = failing_curl(t.path());
        let mut s = Script {
            model: Some(ModelChoice::Potion),
            answers: vec![false],
            ..Script::default()
        };
        ask(&mut s, &env, &curl);
        assert_eq!(s.asked.len(), 1);
        assert!(s.asked[0].starts_with("¿Borro "), "{:?}", s.asked);
        assert!(s.asked[0].contains("también los usa Clio"), "{:?}", s.asked);
        assert!(granite.join("model.onnx").is_file());
        assert!(!t.path().join("curl.log").exists(), "{:?}", s.steps);
        assert!(s.steps.iter().all(|st| st.outcome.is_ok()), "{:?}", s.steps);
    }

    // The chosen model is installed before the better one goes: a failed download keeps Granite.
    #[test]
    fn a_lesser_model_that_fails_to_download_keeps_the_better_one() {
        let (t, env) = home();
        let granite = granite_installed(&env);
        let curl = failing_curl(t.path());
        let mut s = Script {
            model: Some(ModelChoice::Potion),
            answers: vec![true],
            ..Script::default()
        };
        ask(&mut s, &env, &curl);
        assert!(t.path().join("curl.log").is_file(), "{:?}", s.steps);
        assert!(granite.join("model.onnx").is_file(), "{:?}", s.steps);
        assert!(
            s.steps
                .iter()
                .any(|st| st.what == "modelo" && st.outcome.is_err()),
            "{:?}",
            s.steps
        );
    }

    #[test]
    fn no_model_and_a_yes_deletes_the_installed_one() {
        let (t, env) = home();
        let granite = granite_installed(&env);
        let mut s = Script {
            model: Some(ModelChoice::None),
            answers: vec![true],
            ..Script::default()
        };
        ask(&mut s, &env, &failing_curl(t.path()));
        assert!(!granite.exists(), "{:?}", s.steps);
        assert!(!t.path().join("curl.log").exists());
        assert!(s.steps.iter().all(|st| st.outcome.is_ok()), "{:?}", s.steps);
    }
}
