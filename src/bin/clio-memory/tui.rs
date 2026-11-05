//! The setup screen in a terminal: the clack-style wizard of the spec («Lo que decidió Iván»),
//! drawn with `cliclack` under the Clio theme. Each step goes under the last one, joined by the
//! guide line, and what is done stays above as the record:
//!
//! ```text
//! ┌   Clio  memory · la memoria de tu agente · v0.6.0
//! │
//! ◇  Esta máquina · Linux x86_64 · GPU NVIDIA (driver 550)
//! │
//! ◆  ¿A qué agentes le conecto la memoria?
//! │  ◼ Claude Code   encontrado en ~/.claude
//! │  ◻ Todos los que encuentre
//! └  espacio marca · enter sigue
//! ```
//!
//! Everything goes to stderr, like the prompts. Text that comes from outside (paths, an agent's
//! error) is cleaned before it is drawn.

use std::cell::Cell;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use cliclack::{Theme, ThemeState};
use clio_memory::setup::agents::{AgentId, Found};
use clio_memory::setup::model_choice::ModelChoice;
use clio_memory::setup::plan::Step;
use clio_memory::setup::{GRANITE, ModelSpec, POTION};
use console::{Style, measure_text_width};

use crate::output::clean;
use crate::palette::{ColorMode, HIELO, Palette, palette, template_color};
use crate::setup_flow::{Screen, join_labels};

const BAR: &str = "│";
const BAR_END: &str = "└";

/// What the footer of the prompt on screen says (`└  espacio marca · enter sigue`): the theme
/// draws every prompt alike, so the screen tells it which one is up.
#[derive(Clone, Copy)]
#[repr(u8)]
enum Hint {
    None,
    Multiselect,
    Select,
    Confirm,
}

impl Hint {
    fn text(code: u8) -> &'static str {
        match code {
            c if c == Hint::Multiselect as u8 => "espacio marca · enter sigue",
            c if c == Hint::Select as u8 => "↑↓ elige · enter sigue",
            c if c == Hint::Confirm as u8 => "←→ elige · enter sigue",
            _ => "",
        }
    }
}

/// The Clio look for cliclack's prompts: the guide line in ceniza, ◆ and the selection in ámbar,
/// ◇ in verdeSuave, ✖ in rojoApagado, spinners and bars in hielo over borde.
struct ClioTheme {
    pal: Palette,
    hint: Arc<AtomicU8>,
}

impl ClioTheme {
    fn bar(&self) -> String {
        self.pal.ceniza.apply_to(BAR).to_string()
    }
}

impl Theme for ClioTheme {
    fn bar_color(&self, _state: &ThemeState) -> Style {
        self.pal.ceniza.clone()
    }

    fn state_symbol_color(&self, state: &ThemeState) -> Style {
        match state {
            ThemeState::Active => self.pal.ambar.clone(),
            ThemeState::Submit => self.pal.verde_suave.clone(),
            ThemeState::Cancel | ThemeState::Error(_) => self.pal.rojo_apagado.clone(),
        }
    }

    fn state_symbol(&self, state: &ThemeState) -> String {
        let symbol = match state {
            ThemeState::Active => "◆",
            ThemeState::Submit => "◇",
            ThemeState::Cancel => "■",
            ThemeState::Error(_) => "✖",
        };
        self.state_symbol_color(state).apply_to(symbol).to_string()
    }

    fn radio_symbol(&self, state: &ThemeState, selected: bool) -> String {
        match state {
            ThemeState::Active | ThemeState::Error(_) if selected => {
                self.pal.ambar.apply_to("●").to_string()
            }
            ThemeState::Active | ThemeState::Error(_) => self.pal.ceniza.apply_to("○").to_string(),
            _ => String::new(),
        }
    }

    fn checkbox_symbol(&self, state: &ThemeState, selected: bool, _active: bool) -> String {
        match state {
            ThemeState::Active | ThemeState::Error(_) if selected => {
                self.pal.ambar.apply_to("◼").to_string()
            }
            ThemeState::Active | ThemeState::Error(_) => self.pal.ceniza.apply_to("◻").to_string(),
            _ => String::new(),
        }
    }

    /// `◆  question`, the question in hueso.
    fn format_header(&self, state: &ThemeState, prompt: &str) -> String {
        prompt
            .lines()
            .enumerate()
            .map(|(i, line)| {
                let lead = if i == 0 {
                    self.state_symbol(state)
                } else {
                    self.bar()
                };
                format!("{lead}  {}\n", self.pal.hueso.apply_to(line))
            })
            .collect()
    }

    /// While it is asked, `└  <the keys>`; once answered (or cancelled), the guide line goes on.
    fn format_footer_with_message(&self, state: &ThemeState, message: &str) -> String {
        match state {
            ThemeState::Active => {
                let hint = Hint::text(self.hint.load(Ordering::Relaxed));
                let text = [message, hint]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("  ");
                format!(
                    "{}\n",
                    self.pal.ceniza.apply_to(format!("{BAR_END}  {text}"))
                )
            }
            ThemeState::Submit | ThemeState::Cancel => format!("{}\n", self.bar()),
            ThemeState::Error(err) => format!(
                "{}  {}\n",
                self.pal.ceniza.apply_to(BAR_END),
                self.pal.rojo_apagado.apply_to(err)
            ),
        }
    }

    /// A select item: `● label · hint`, every hint shown (in ceniza) while it is asked.
    fn radio_item(&self, state: &ThemeState, selected: bool, label: &str, hint: &str) -> String {
        match state {
            ThemeState::Cancel | ThemeState::Submit if !selected => String::new(),
            ThemeState::Cancel => self
                .pal
                .ceniza
                .clone()
                .strikethrough()
                .apply_to(label)
                .to_string(),
            ThemeState::Submit => self.pal.ceniza_clara.apply_to(label).to_string(),
            ThemeState::Active | ThemeState::Error(_) => {
                let style = if selected {
                    &self.pal.hueso
                } else {
                    &self.pal.ceniza_clara
                };
                let hint = if hint.is_empty() {
                    String::new()
                } else {
                    self.pal.ceniza.apply_to(format!(" · {hint}")).to_string()
                };
                format!(
                    "{} {}{hint}",
                    self.radio_symbol(state, selected),
                    style.apply_to(label)
                )
            }
        }
    }

    /// A multiselect item: `◼ label   hint`, the hint (where it was found) always shown. The one
    /// under the cursor is in hueso, the others in cenizaClara; with no colors, a `›` marks it.
    fn format_multiselect_item(
        &self,
        state: &ThemeState,
        selected: bool,
        active: bool,
        label: &str,
        hint: &str,
    ) -> String {
        let item = match state {
            ThemeState::Cancel | ThemeState::Submit if !selected => return String::new(),
            ThemeState::Cancel => self
                .pal
                .ceniza
                .clone()
                .strikethrough()
                .apply_to(label.trim_end())
                .to_string(),
            ThemeState::Submit => self.pal.ceniza_clara.apply_to(label.trim_end()).to_string(),
            ThemeState::Active | ThemeState::Error(_) => {
                let style = if active {
                    &self.pal.hueso
                } else {
                    &self.pal.ceniza_clara
                };
                let hint = if hint.is_empty() {
                    String::new()
                } else {
                    format!(" {}", self.pal.ceniza.apply_to(hint))
                };
                format!(
                    "{} {}{hint}",
                    self.checkbox_symbol(state, selected, active),
                    style.apply_to(label)
                )
            }
        };
        let gap =
            if active && self.pal.mode == ColorMode::None && matches!(state, ThemeState::Active) {
                " ›"
            } else {
                "  "
            };
        format!("{}{gap}{item}\n", self.bar())
    }

    /// `● Sí / ○ No`.
    fn format_confirm(&self, state: &ThemeState, confirm: bool) -> String {
        let yes = self.radio_item(state, confirm, "Sí", "");
        let no = self.radio_item(state, !confirm, "No", "");
        let divider = match state {
            ThemeState::Active => self.pal.ceniza.apply_to(" / ").to_string(),
            _ => String::new(),
        };
        format!("{}  {yes}{divider}{no}\n", self.bar())
    }

    // cliclack's own frame pieces fall back to ASCII (`T`, `|`, `—`) when the locale does not say
    // UTF-8; the rest of the screen is UTF-8 anyway, so these are drawn here, always the same.

    fn format_intro(&self, title: &str) -> String {
        format!(
            "{}  {title}\n{}\n",
            self.pal.ceniza.apply_to("┌"),
            self.bar()
        )
    }

    fn format_outro(&self, message: &str) -> String {
        format!("{}  {message}\n", self.pal.ceniza.apply_to(BAR_END))
    }

    fn format_select_item(
        &self,
        state: &ThemeState,
        selected: bool,
        label: &str,
        hint: &str,
    ) -> String {
        match state {
            ThemeState::Cancel | ThemeState::Submit if !selected => String::new(),
            _ => format!(
                "{}  {}\n",
                self.bar(),
                self.radio_item(state, selected, label, hint)
            ),
        }
    }

    fn spinner_chars(&self) -> String {
        "◒◐◓◑".to_owned()
    }

    /// A single progress line: `◒  msg` while it runs, `◇  msg` and the guide line once stopped.
    /// (The screen never groups bars.)
    fn format_progress_with_state(
        &self,
        msg: &str,
        _grouped: bool,
        _last: bool,
        state: &ThemeState,
    ) -> String {
        match state {
            ThemeState::Active => msg.to_owned(),
            _ if msg.is_empty() => format!("\n{}", self.bar()),
            _ => format!("{}  {msg}\n{}", self.state_symbol(state), self.bar()),
        }
    }

    /// `◒  …`, the spinner in hielo.
    fn format_progress_start(&self, template: &str, grouped: bool, last: bool) -> String {
        let spinner = match template_color(self.pal.mode, HIELO) {
            Some(color) => format!("{{spinner:.{color}}}"),
            None => "{spinner}".to_owned(),
        };
        self.format_progress_with_state(
            &format!("{spinner}  {template}"),
            grouped,
            last,
            &ThemeState::Active,
        )
    }
}

/// The screen in a terminal.
pub struct Tui {
    pal: Palette,
    hint: Arc<AtomicU8>,
    home: PathBuf,
    /// Whether this machine has an NVIDIA card (`machine`): Granite's hint says where it runs.
    gpu: bool,
    /// The last question was cancelled: the note that follows closes the screen.
    cancelled: bool,
}

impl Tui {
    /// Sets the Clio theme, and console's own switch to agree with `mode` (so `NO_COLOR` leaves no
    /// escape at all, not even cliclack's own styles). `home` shortens paths to `~/…`.
    pub fn new(mode: ColorMode, home: PathBuf) -> Self {
        let colors = mode != ColorMode::None;
        console::set_colors_enabled(colors);
        console::set_colors_enabled_stderr(colors);
        let pal = palette(mode);
        let hint = Arc::new(AtomicU8::new(Hint::None as u8));
        cliclack::set_theme(ClioTheme {
            pal: pal.clone(),
            hint: Arc::clone(&hint),
        });
        Tui {
            pal,
            hint,
            home,
            gpu: false,
            cancelled: false,
        }
    }

    fn out(text: &str) {
        let _ = std::io::stderr().lock().write_all(text.as_bytes());
    }

    fn bar(&self) -> String {
        self.pal.ceniza.apply_to(BAR).to_string()
    }

    /// `symbol  first line`, the next lines under the guide line, then the guide line alone.
    fn entry(&self, symbol: &str, first: &str, rest: &[String]) {
        let mut text = format!("{symbol}  {first}\n");
        for line in rest {
            text.push_str(&format!("{}  {line}\n", self.bar()));
        }
        text.push_str(&format!("{}\n", self.bar()));
        Self::out(&text);
    }

    /// `~/.claude` for a path under the home.
    fn tilde(&self, path: &Path) -> String {
        clean(&match path.strip_prefix(&self.home) {
            Ok(rel) if rel.as_os_str().is_empty() => "~".to_owned(),
            Ok(rel) => format!("~/{}", rel.display()),
            Err(_) => path.display().to_string(),
        })
    }

    /// Text from outside (a step's detail, a note) as lines to draw: cleaned, the home written
    /// `~`, and wrapped to the terminal so a long path stays to the right of the guide line.
    fn lines(&self, text: &str) -> Vec<String> {
        let home = format!("{}/", self.home.display());
        let short = |line: &str| {
            let line = clean(line.trim_end());
            if self.home.parent().is_some() {
                line.replace(&home, "~/")
            } else {
                line
            }
        };
        let text: Vec<String> = text.lines().map(short).collect();
        cliclack::termwrap(&text.join("\n"), 3)
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// Asks with the footer of `hint`; an answer, or `None` when cancelled (Esc, Ctrl-C) or the
    /// terminal failed.
    fn ask<T>(&mut self, hint: Hint, prompt: impl FnOnce() -> std::io::Result<T>) -> Option<T> {
        self.hint.store(hint as u8, Ordering::Relaxed);
        let answer = prompt().ok();
        self.hint.store(Hint::None as u8, Ordering::Relaxed);
        self.cancelled = answer.is_none();
        answer
    }
}

/// `1,28 GB`, `531 MB`: a model's size, all its files.
fn size(spec: &ModelSpec) -> String {
    let bytes: u64 = spec.files.iter().map(|f| f.size).sum();
    if bytes >= 1_000_000_000 {
        format!("{:.2} GB", bytes as f64 / 1e9).replace('.', ",")
    } else {
        format!("{:.0} MB", bytes as f64 / 1e6)
    }
}

/// `12,3 MB`; under a megabyte, `271 B` or `18,6 KB`.
fn mb(bytes: u64) -> String {
    match bytes {
        b if b < 1_000 => format!("{b} B"),
        b if b < 1_000_000 => format!("{:.1} KB", b as f64 / 1e3).replace('.', ","),
        b => format!("{:.1} MB", b as f64 / 1e6).replace('.', ","),
    }
}

/// A literal for an indicatif template: its braces doubled.
fn literal(text: &str) -> String {
    text.replace('{', "{{").replace('}', "}}")
}

/// One download on screen. Dropped when the download ends: a complete one stays as a line, any
/// other is cleared (the model's step says how it ended).
struct Download {
    bar: cliclack::ProgressBar,
    label: String,
    total: Option<u64>,
    bytes: Cell<u64>,
    pal: Palette,
}

/// The bar's width, in cells.
const BAR_WIDTH: u64 = 30;

impl Download {
    /// The bar is drawn here, not by indicatif: the filled part in hielo, the empty one in borde
    /// (with no colors, `■` and `□`), then the MB.
    fn tick(&self, bytes: u64) {
        self.bytes.set(bytes);
        let message = match self.total {
            Some(total) => {
                let filled = (bytes.min(total) * BAR_WIDTH)
                    .checked_div(total)
                    .unwrap_or(0);
                let (full, empty) = match self.pal.mode {
                    ColorMode::None => ("■", "□"),
                    _ => ("━", "━"),
                };
                format!(
                    "{}{}  {}",
                    self.pal.hielo.apply_to(full.repeat(filled as usize)),
                    self.pal
                        .borde
                        .apply_to(empty.repeat((BAR_WIDTH - filled) as usize)),
                    self.pal
                        .ceniza_clara
                        .apply_to(format!("{} / {}", mb(bytes), mb(total)))
                )
            }
            None => self.pal.ceniza_clara.apply_to(mb(bytes)).to_string(),
        };
        self.bar.set_message(message);
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        match self.total {
            Some(total) if self.bytes.get() >= total => self.bar.stop(format!(
                "{} {}",
                self.label,
                self.pal.ceniza_clara.apply_to(format!("· {}", mb(total)))
            )),
            _ => self.bar.clear(),
        }
    }
}

impl Screen for Tui {
    /// `┌   Clio  memory · la memoria de tu agente · v0.6.0`: « Clio » in grafito on ámbar.
    fn header(&mut self, version: &str) {
        let stamp = match self.pal.mode {
            ColorMode::None => "Clio".to_owned(),
            _ => self.pal.sello.apply_to(" Clio ").to_string(),
        };
        let title = format!(
            "{stamp} {}{}",
            self.pal.hueso.clone().bold().apply_to("memory"),
            self.pal
                .ceniza
                .apply_to(format!(" · la memoria de tu agente · v{version}"))
        );
        let _ = cliclack::intro(title);
    }

    fn machine(&mut self, os: &str, arch: &str, gpu: Option<&str>) {
        self.gpu = gpu.is_some();
        let mut detail = format!(" · {os} {arch}");
        if let Some(gpu) = gpu {
            detail.push_str(&format!(" · {}", clean(gpu)));
        }
        self.entry(
            &self.pal.verde_suave.apply_to("◇").to_string(),
            &format!(
                "{}{}",
                self.pal.hueso.apply_to("Esta máquina"),
                self.pal.ceniza_clara.apply_to(detail)
            ),
            &[],
        );
    }

    /// `◇  clio-memory instalado · ~/.local/bin · sha256 ✓`: only after the installer, which
    /// checked the sum before it ran this.
    fn installed(&mut self, at: &Path) {
        let dir = at.parent().unwrap_or(at);
        self.entry(
            &self.pal.verde_suave.apply_to("◇").to_string(),
            &format!(
                "{}{}{}",
                self.pal.hueso.apply_to("clio-memory instalado"),
                self.pal
                    .ceniza_clara
                    .apply_to(format!(" · {} · sha256 ", self.tilde(dir))),
                self.pal.verde_suave.apply_to("✓")
            ),
            &[],
        );
    }

    /// The agents found, each with where; then «Todos los que encuentre», which counts as all.
    /// Checked at first: the ones connected, or on a first run every one found.
    fn choose_agents(&mut self, found: &[Found], connected: &[AgentId]) -> Option<Vec<AgentId>> {
        let mut items: Vec<(AgentId, String)> = found
            .iter()
            .map(|f| (f.agent, format!("encontrado en {}", self.tilde(&f.at))))
            .collect();
        // One connected and no longer found is still shown, so leaving it checked keeps it.
        for a in connected {
            if !items.iter().any(|(i, _)| i == a) {
                items.push((*a, "conectado".to_owned()));
            }
        }
        if items.is_empty() {
            return Some(Vec::new());
        }
        let width = items
            .iter()
            .map(|(a, _)| measure_text_width(a.label()))
            .max()
            .unwrap_or(0);
        let checked: Vec<Option<AgentId>> = if connected.is_empty() {
            found.iter().map(|f| Some(f.agent)).collect()
        } else {
            connected.iter().copied().map(Some).collect()
        };
        let mut prompt = cliclack::multiselect("¿A qué agentes le conecto la memoria?")
            .required(false)
            .initial_values(checked);
        for (agent, hint) in &items {
            let pad = " ".repeat(width + 2 - measure_text_width(agent.label()));
            prompt = prompt.item(Some(*agent), format!("{}{pad}", agent.label()), hint);
        }
        prompt = prompt.item(None, "Todos los que encuentre", "");
        let chosen = self.ask(Hint::Multiselect, || prompt.interact())?;
        let mut agents: Vec<AgentId> = chosen.iter().flatten().copied().collect();
        if chosen.contains(&None) {
            agents.extend(found.iter().map(|f| f.agent));
        }
        agents.sort();
        agents.dedup();
        Some(agents)
    }

    fn choose_model(
        &mut self,
        options: &[ModelChoice],
        current: ModelChoice,
    ) -> Option<ModelChoice> {
        if !options.contains(&ModelChoice::Granite) {
            self.note("Granite no tiene runtime para Mac Intel: se recomienda potion");
        }
        let mut prompt =
            cliclack::select("¿Qué modelo instalo para la búsqueda?").initial_value(current);
        for option in options {
            let (label, hint) = match option {
                ModelChoice::Granite => (
                    "Granite 311M",
                    format!(
                        "{} · recomendado ({})",
                        size(&GRANITE),
                        if self.gpu { "GPU" } else { "procesador" }
                    ),
                ),
                ModelChoice::Potion => ("potion", format!("{} · liviano", size(&POTION))),
                ModelChoice::None => ("ninguno", "búsqueda por texto".to_owned()),
            };
            prompt = prompt.item(*option, label, hint);
        }
        self.ask(Hint::Select, || prompt.interact())
    }

    fn confirm(&mut self, question: &str, default: bool) -> Option<bool> {
        let mut prompt = cliclack::confirm(self.lines(question).join("\n")).initial_value(default);
        self.ask(Hint::Confirm, || prompt.interact())
    }

    /// `◇  Claude Code · conectado`, or `✖  Codex · <why>` with the why's lines under it.
    fn step(&mut self, step: &Step) {
        self.cancelled = false;
        let what = match step.agent {
            Some(agent) => agent.label(),
            None => step.what.as_str(),
        };
        let (symbol, text) = match &step.outcome {
            Ok(detail) => (self.pal.verde_suave.apply_to("◇"), detail),
            Err(why) => (self.pal.rojo_apagado.apply_to("✖"), why),
        };
        // `what` in hueso, the rest in cenizaClara: wrapped as one text, then split again.
        let what = clean(what);
        let lines = self.lines(&format!("{what} · {text}"));
        let Some((first, rest)) = lines.split_first() else {
            return;
        };
        let first = match first.strip_prefix(&what) {
            Some(after) => format!(
                "{}{}",
                self.pal.hueso.apply_to(&what),
                self.pal.ceniza_clara.apply_to(after)
            ),
            None => self.pal.ceniza_clara.apply_to(first).to_string(),
        };
        let rest: Vec<String> = rest
            .iter()
            .map(|l| self.pal.ceniza_clara.apply_to(l).to_string())
            .collect();
        self.entry(&symbol.to_string(), &first, &rest);
    }

    /// A bar in hielo over borde when the size is known, else a spinner with the MB so far.
    fn download(&mut self, label: &str, total: Option<u64>) -> Box<dyn Fn(u64) + '_> {
        self.cancelled = false;
        let label = self.pal.hueso.apply_to(clean(label)).to_string();
        let separator = if total.is_some() { "  " } else { " · " };
        let bar =
            cliclack::spinner().with_template(&format!("{}{separator}{{msg}}", literal(&label)));
        bar.start("");
        let d = Download {
            bar,
            label,
            total,
            bytes: Cell::new(0),
            pal: self.pal.clone(),
        };
        d.tick(0);
        Box::new(move |bytes| d.tick(bytes))
    }

    /// `│  text`, in cenizaClara; right after a cancelled question, `└  text` closes the screen.
    fn note(&mut self, text: &str) {
        let lines: Vec<String> = self
            .lines(text)
            .iter()
            .map(|l| self.pal.ceniza_clara.apply_to(l).to_string())
            .collect();
        if std::mem::take(&mut self.cancelled) {
            let mut out = String::new();
            for (i, line) in lines.iter().enumerate() {
                let lead = if i == 0 {
                    self.pal.ceniza.apply_to(BAR_END).to_string()
                } else {
                    " ".to_owned()
                };
                out.push_str(&format!("{lead}  {line}\n"));
            }
            Self::out(&out);
            return;
        }
        let Some((first, rest)) = lines.split_first() else {
            return;
        };
        self.entry(&self.bar(), first, rest);
    }

    /// `└  Listo. Abre …: la memoria ya está ahí.` and, under it in ceniza, where to go next.
    fn done(&mut self, agents: &[AgentId], failed: bool) {
        self.cancelled = false;
        if failed {
            self.entry(
                &self.bar(),
                &self
                    .pal
                    .rojo_apagado
                    .apply_to("Algo no se pudo hacer (las líneas con ✖); lo demás quedó hecho.")
                    .to_string(),
                &[],
            );
        }
        let main = if agents.is_empty() {
            "Listo. Ningún agente quedó conectado: cuando instales uno, corre `clio-memory setup`."
                .to_owned()
        } else {
            format!(
                "Listo. Abre {}: la memoria ya está ahí.",
                join_labels(agents)
            )
        };
        let _ = cliclack::outro(format!(
            "{}\n   {}",
            self.pal.hueso.apply_to(main),
            self.pal.ceniza.apply_to(
                "clio-memory doctor revisa todo · clio-memory setup cambia lo que elegiste"
            )
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // What the model question shows: every file of the model, as it is downloaded.
    #[test]
    fn sizes_on_screen() {
        // 33 384 821 + 1 247 170 481 bytes.
        assert_eq!(size(&GRANITE), "1,28 GB");
        // 271 + 18 616 131 + 512 361 560 bytes.
        assert_eq!(size(&POTION), "531 MB");
        assert_eq!(mb(271), "271 B");
        assert_eq!(mb(18_616), "18,6 KB");
        assert_eq!(mb(512_361_560), "512,4 MB");
    }
}
