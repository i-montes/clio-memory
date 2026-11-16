//! The client of Temis, the server that decides on pairs of memories with typed questions
//! (`POST <url>/v1/decide`). The token goes to `curl` in a config on its standard input, never as an
//! argument, and no error message carries it.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::{Value, json};

use crate::Relation;

pub const DEFAULT_MODEL: &str = "temis-latest";
/// Characters of each memory that leave the machine.
const MAX_CHARS: usize = 4000;
/// Consecutive failures that turn Temis off for this process.
const MAX_FAILURES: u32 = 3;

#[derive(Clone)]
pub struct Config {
    pub url: String,
    pub token: String,
    pub model: String,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("url", &self.url)
            .field("token", &"…")
            .field("model", &self.model)
            .finish()
    }
}

pub fn config_path(data: &Path) -> PathBuf {
    data.join("temis.json")
}

/// The configuration: both variables win over the file; with one of them only, or none, the file.
pub fn load(data: &Path, env: &dyn Fn(&str) -> Option<String>) -> Option<Config> {
    let var = |k: &str| {
        env(k)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    let file = std::fs::read_to_string(config_path(data))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| {
            let s = |k: &str| {
                v.get(k)?
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            };
            Some(Config {
                url: s("url")?,
                token: s("token")?,
                model: s("model").unwrap_or_else(|| DEFAULT_MODEL.into()),
            })
        });
    // The environment's URL is checked like `setup temis` checks one (the file's was, when it was
    // saved, and may carry `--allow-http`): a bad one is as if the variables were not set.
    match (var("CLIO_MEMORY_TEMIS_URL"), var("CLIO_MEMORY_TEMIS_TOKEN")) {
        (Some(url), Some(token)) => match check_url(&url, false) {
            Ok(url) => Some(Config {
                url,
                token,
                model: file.map_or_else(|| DEFAULT_MODEL.into(), |c| c.model),
            }),
            Err(_) => file,
        },
        _ => file,
    }
}

/// What the hosts use: the configuration as `load` reads it with the real environment, and the
/// real `curl`. Building it makes no call.
pub fn from_data(data: &Path) -> Option<Temis> {
    let cfg = load(data, &|k| std::env::var(k).ok())?;
    Some(Temis::new(cfg, curl_post(PathBuf::from("curl"))))
}

/// Written whole, 0600, or not at all.
pub fn save(data: &Path, cfg: &Config) -> Result<(), String> {
    let path = config_path(data);
    let fail = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    crate::agent::memory::private_dir(data)?;
    let mut text = serde_json::to_string_pretty(
        &json!({"url": cfg.url, "token": cfg.token, "model": cfg.model}),
    )
    .map_err(|e| fail(&e))?;
    text.push('\n');
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let written = crate::platform::create_private_file(&tmp)
        .map_err(|e| fail(&e))
        .and_then(|mut f| {
            f.write_all(text.as_bytes()).map_err(|e| fail(&e))?;
            f.sync_all().map_err(|e| fail(&e))?;
            std::fs::rename(&tmp, &path).map_err(|e| fail(&e))
        });
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Whether there was a file to remove.
pub fn remove(data: &Path) -> Result<bool, String> {
    let path = config_path(data);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// `https`, or `http` to this machine (or anywhere with `allow_http`); the URL without its trailing
/// slash. Nothing that could change what `curl` reads from its config (quotes, spaces, `@`, `?`).
pub fn check_url(url: &str, allow_http: bool) -> Result<String, String> {
    let bad = |why: &str| format!("la URL de Temis no sirve: {why}");
    let url = url.trim();
    if url
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || "\"\\@?#".contains(c))
    {
        return Err(bad("tiene espacios, comillas, @, ? o #"));
    }
    let (https, rest) = match (url.strip_prefix("https://"), url.strip_prefix("http://")) {
        (Some(r), _) => (true, r),
        (_, Some(r)) => (false, r),
        _ => return Err(bad("debe empezar por https://")),
    };
    let authority = rest.split('/').next().unwrap_or("");
    let (host, port) = match authority.strip_prefix('[') {
        Some(v6) => match v6.split_once(']') {
            Some((h, p)) => (format!("[{h}]"), p.strip_prefix(':').unwrap_or(p)),
            None => return Err(bad("el servidor no se entiende")),
        },
        None => match authority.split_once(':') {
            Some((h, p)) => (h.to_owned(), p),
            None => (authority.to_owned(), ""),
        },
    };
    if host.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad("el servidor o el puerto no se entienden"));
    }
    let here = ["localhost", "127.0.0.1", "[::1]"].contains(&host.as_str());
    if !https && !here && !allow_http {
        return Err(bad("solo se admite https (http solo para esta máquina)"));
    }
    Ok(url.trim_end_matches('/').to_owned())
}

/// (configuration, path such as `/v1/decide`, JSON body or `None` for a GET) -> (HTTP code, body).
pub type Post = dyn Fn(&Config, &str, Option<&str>) -> Result<(u16, String), String> + Send + Sync;

/// A `\` or `"` in a `curl` config value would end or bend it; so would a line break.
fn esc(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// The text `curl --config -` reads from its standard input.
pub fn curl_config(cfg: &Config, path: &str, body: Option<&str>) -> String {
    let mut lines = vec![format!(
        r#"url = "{}{}""#,
        esc(cfg.url.trim_end_matches('/')),
        esc(path)
    )];
    if body.is_some() {
        lines.push(r#"request = "POST""#.into());
    }
    lines.extend(["max-time = 10".into(), "silent".into(), "show-error".into()]);
    if body.is_some() {
        lines.push(r#"header = "Content-Type: application/json""#.into());
    }
    if path != "/health" {
        lines.push(format!(
            r#"header = "Authorization: Bearer {}""#,
            esc(&cfg.token)
        ));
    }
    if let Some(b) = body {
        lines.push(format!(r#"data-binary = "{}""#, esc(b)));
    }
    lines.push(r#"write-out = "\n%{http_code}""#.into());
    lines.join("\n") + "\n"
}

/// The real `Post`: `curl --config -` with the config on its standard input.
pub fn curl_post(curl: PathBuf) -> Box<Post> {
    Box::new(move |cfg, path, body| {
        // What curl says on stderr, one short line, without the token (raw or escaped).
        let scrub = |s: &str| {
            let line = s.lines().next().unwrap_or("").trim();
            let line = line.replace(&cfg.token, "…").replace(&esc(&cfg.token), "…");
            line.chars().take(120).collect::<String>()
        };
        let mut cmd = Command::new(&curl);
        cmd.args(["--config", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = crate::setup::retry_busy(|| cmd.spawn())
            .map_err(|e| format!("no se pudo ejecutar curl: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            // A curl that exits early closes the pipe: its status says why.
            let _ = stdin.write_all(curl_config(cfg, path, body).as_bytes());
        }
        let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
        if !out.status.success() {
            let said = scrub(&String::from_utf8_lossy(&out.stderr));
            return Err(format!("curl falló ({}): {said}", out.status));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let (body, code) = text.rsplit_once('\n').ok_or("curl no devolvió un código")?;
        let code = code
            .trim()
            .parse()
            .map_err(|_| "curl no devolvió un código")?;
        Ok((code, body.to_owned()))
    })
}

pub struct Temis {
    cfg: Config,
    post: Box<Post>,
    /// Failures in a row; a success sets it back to 0.
    failures: AtomicU32,
}

impl Temis {
    pub fn new(cfg: Config, post: Box<Post>) -> Self {
        Temis {
            cfg,
            post,
            failures: AtomicU32::new(0),
        }
    }

    fn off(&self) -> bool {
        self.failures.load(Ordering::Relaxed) >= MAX_FAILURES
    }

    fn record(&self, ok: bool) {
        if ok {
            self.failures.store(0, Ordering::Relaxed);
        } else {
            self.failures.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// One call to `/v1/decide`: the `answers` object. Does not touch the failure count.
    fn call(&self, state: &Value, questions: &Value) -> Result<Value, String> {
        let body = json!({"model": self.cfg.model, "state": state, "questions": questions});
        let (code, text) = (self.post)(&self.cfg, "/v1/decide", Some(&body.to_string()))?;
        let parsed: Option<Value> = serde_json::from_str(&text).ok();
        if code != 200 {
            return Err(refusal(code, parsed.as_ref()));
        }
        parsed
            .and_then(|mut v| v.get_mut("answers").map(Value::take))
            .filter(Value::is_object)
            .ok_or_else(|| "Temis respondió con algo que no se entiende".into())
    }

    /// The `answers` of Temis to `questions` about `state`. After three failures in a row it
    /// answers without calling.
    pub fn decide(&self, state: &Value, questions: &Value) -> Result<Value, String> {
        if self.off() {
            return Err("Temis no respondió tres veces seguidas: se deja para después".into());
        }
        let r = self.call(state, questions);
        self.record(r.is_ok());
        r
    }

    /// `decide`, and the answer to `id` read by `parse`; an odd answer is no answer, and counts as
    /// a failure like a refusal does.
    fn ask<T>(
        &self,
        state: Value,
        questions: Value,
        id: &str,
        parse: impl FnOnce(&Value) -> Option<T>,
    ) -> Option<T> {
        if self.off() {
            return None;
        }
        let got = self
            .call(&state, &questions)
            .ok()
            .and_then(|answers| parse(answers.get(id)?));
        self.record(got.is_some());
        got
    }

    /// How much of `original` is in `merged` (0 to 1), as Temis sees it.
    pub fn preserved(&self, original: &str, merged: &str) -> Option<f32> {
        self.ask(
            json!({"original": cut(original), "unido": cut(merged)}),
            json!({"se_conserva": {
                "type": "noul",
                "instructions": "Todo lo que dice `original` está también en `unido`, aunque sea con otras palabras."
            }}),
            "se_conserva",
            |a| {
                (a.get("type")? == "noul").then_some(())?;
                unit(a.get("noul")?)
            },
        )
    }

    /// How `proposal` stands to `other`, and how sure Temis is.
    pub fn relation(&self, proposal: &str, other: &str) -> Option<(Relation, f32)> {
        self.ask(
            json!({"propuesta": cut(proposal), "otra": cut(other)}),
            json!({"relacion": {
                "type": "choice",
                "instructions": "Decide cómo se relaciona `propuesta` con `otra`.",
                "criteria": {
                    "conflicts_with": "Se contradicen: no pueden ser ciertas a la vez.",
                    "scoped": "`propuesta` vale en un caso más concreto que `otra` (este proyecto, este caso).",
                    "compatible": "Hablan de lo mismo y pueden ser ciertas a la vez.",
                    "related": "Tratan un tema cercano, sin más.",
                    "ninguna": "No tienen que ver."
                }
            }}),
            "relacion",
            |a| {
                (a.get("type")? == "choice").then_some(())?;
                let relation = match a.get("choice")?.as_str()? {
                    "conflicts_with" => Relation::ConflictsWith,
                    "scoped" => Relation::Scoped,
                    "compatible" => Relation::Compatible,
                    "related" => Relation::Related,
                    "ninguna" => Relation::NotConflict,
                    _ => return None,
                };
                Some((relation, unit(a.get("confidence")?)?))
            },
        )
    }

    /// The model Temis names on `/health` (no token).
    pub fn health(&self) -> Result<String, String> {
        self.model_of("/health")
    }

    /// The model Temis names on `/v1/model` (with the token: it proves the token works).
    pub fn model(&self) -> Result<String, String> {
        self.model_of("/v1/model")
    }

    fn model_of(&self, path: &str) -> Result<String, String> {
        let (code, text) = (self.post)(&self.cfg, path, None)?;
        let v: Option<Value> = serde_json::from_str(&text).ok();
        if code != 200 {
            return Err(refusal(code, v.as_ref()));
        }
        ["model", "id"]
            .iter()
            .find_map(|k| v.as_ref()?.get(k)?.as_str().map(str::to_owned))
            .ok_or_else(|| "Temis respondió con algo que no se entiende".into())
    }
}

fn cut(s: &str) -> String {
    s.chars().take(MAX_CHARS).collect()
}

/// A number from 0 to 1.
fn unit(v: &Value) -> Option<f32> {
    v.as_f64()
        .filter(|n| (0.0..=1.0).contains(n))
        .map(|n| n as f32)
}

/// Only the kind of error, in a plain alphabet: never its message nor anything echoed.
fn refusal(code: u16, body: Option<&Value>) -> String {
    let kind = body
        .and_then(|v| v.pointer("/detail/error_type")?.as_str())
        .filter(|k| k.len() <= 40 && k.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
        .unwrap_or("desconocido");
    format!("Temis respondió {code} ({kind})")
}

impl crate::sleep::PairJudge for Temis {
    fn preserved(&self, original: &str, merged: &str) -> Option<f32> {
        Temis::preserved(self, original, merged)
    }
    fn relation(&self, proposal: &str, other: &str) -> Option<(Relation, f32)> {
        Temis::relation(self, proposal, other)
    }
}
