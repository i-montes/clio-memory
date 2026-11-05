//! The sleep's rules that need no model: what looks like an attack on the memory (MINJA,
//! AgentPoison, OWASP ASI06), and whether a merged text says only what its originals said.
//! Memory text is data: these rules only read it.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::model::{MemoryId, MemoryRow};

/// Bumped whenever a rule here changes: that night the sleep reviews every memory once again.
pub const RULES_VERSION: u32 = 1;

/// Cosine similarity a merged text keeps with each of its originals, at least (Granite).
pub const MERGE_SIMILARITY: f32 = 0.80;

/// Words of four letters or more a merged text may bring that its originals did not have.
const NEW_WORDS: usize = 2;

/// The folded words of a text (lowercase, no accents), split at anything not a letter or digit.
fn words(text: &str) -> BTreeSet<String> {
    fold(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A kind of signal. Hidden characters are enough on their own; any other needs company.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Family {
    Hidden,
    Injection,
    Exfil,
    External,
}

impl Family {
    pub fn label(self) -> &'static str {
        match self {
            Family::Hidden => "caracteres ocultos",
            Family::Injection => "inyección",
            Family::Exfil => "exfiltración",
            Family::External => "externo",
        }
    }
}

/// Phrases that try to steer the agent, after `fold` (lowercase, no accents). In Spanish and
/// English, plus the tags and fake endings of chat formats.
const INJECTION: &[&str] = &[
    "ignora las instrucciones",
    "ignora todas las instrucciones",
    "ignora tus instrucciones",
    "olvida las instrucciones",
    "olvida tus instrucciones",
    "a partir de ahora eres",
    "ahora eres",
    "no le digas al usuario",
    "no se lo digas al usuario",
    "sin decirle al usuario",
    "prompt del sistema",
    "nuevas instrucciones:",
    "ignore previous instructions",
    "ignore all previous",
    "ignore the above",
    "disregard previous",
    "disregard prior",
    "disregard all prior",
    "forget your instructions",
    "you are now",
    "system prompt",
    "do not tell the user",
    "don't tell the user",
    "new instructions:",
    "<system>",
    "</system>",
    "<|im_start|>",
    "<|im_end|>",
    "<|system|>",
    "</conversation>",
    "[inst]",
    "[/inst]",
    "<datos>",
    "</datos>",
];

static EXFIL: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"\b(curl|wget)\b[^|\n]*\|\s*(sudo\s+)?(ba|z)?sh\b",
        r"base64\s+(-d|--decode)\b[^|\n]*\|",
        r"[?&](token|key|secret|api_key|apikey|password|passwd)=",
        r"/dev/tcp/",
        r"\bnc\s+(-\w+\s+)*-e\b",
    ]
    .into_iter()
    .map(|p| Regex::new(p).expect("static exfil patterns compile"))
    .collect()
});

static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b(?:https?://|www\.)[^\s<>"'`)\]]+"#).expect("static url pattern compiles")
});

static PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:~|\.{1,2})?/[\w.-]+(?:/[\w.-]+)+|\b[A-Za-z]:\\[^\s]+")
        .expect("static path pattern compiles")
});

fn hidden(c: char) -> bool {
    matches!(c,
        '\u{200B}'..='\u{200F}'
        | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{2069}'
        | '\u{FEFF}'
        | '\u{E0000}'..='\u{E007F}')
        || (c.is_control() && !matches!(c, '\n' | '\t' | '\r'))
}

/// Lowercase, accents removed: what the phrase lists are compared against.
fn fold(text: &str) -> String {
    text.nfd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
}

fn injections(folded: &str) -> BTreeSet<&'static str> {
    INJECTION
        .iter()
        .copied()
        .filter(|p| folded.contains(p))
        .collect()
}

/// The families of signals in `text`; `external` says it came from outside content.
pub fn attack_families(text: &str, external: bool) -> Vec<Family> {
    let mut out = Vec::new();
    if text.chars().any(hidden) {
        out.push(Family::Hidden);
    }
    if !injections(&fold(text)).is_empty() {
        out.push(Family::Injection);
    }
    if EXFIL.iter().any(|r| r.is_match(text)) {
        out.push(Family::Exfil);
    }
    if external {
        out.push(Family::External);
    }
    out
}

/// Hidden characters alone, or two families: one phrase alone is often a memory about security.
pub fn looks_like_attack(text: &str, external: bool) -> bool {
    let families = attack_families(text, external);
    families.contains(&Family::Hidden) || families.len() >= 2
}

pub fn has_secret(text: &str) -> bool {
    crate::secrets::find_secret(text)
}

/// Why a merged text was refused. The group is left as it was and reported for review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Rejected {
    Json,
    Sources,
    Longer,
    NewUrl,
    NewPath,
    NewCommand,
    NewInjection,
    /// Words or numbers the originals did not have.
    NewWords,
    /// A title with words the originals did not have.
    Title,
    Secret,
    Drift(MemoryId),
}

/// A merged text that passed every check.
#[derive(Debug, Clone, PartialEq)]
pub struct Merged {
    pub text: String,
    pub title: Option<String>,
    pub sources: Vec<MemoryId>,
}

#[derive(Deserialize)]
struct Answer {
    texto: String,
    #[serde(default)]
    titulo: Option<String>,
    fuentes: Vec<serde_json::Value>,
}

fn source_id(v: &serde_json::Value) -> Option<i64> {
    match v {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(s) => s.trim().trim_start_matches('m').parse().ok(),
        _ => None,
    }
}

/// Checks the model's raw answer for a group: strict JSON `{"texto","titulo","fuentes"}` whose
/// sources are exactly the group, no longer than the originals together, with no URL, path,
/// command, injection phrase or hidden character they lacked, no secret, and similar enough to
/// each of them (`similarity` gives `None` without vectors, which skips that check).
pub fn validate_merge(
    originals: &[MemoryRow],
    answer: &str,
    similarity: &dyn Fn(&str, &str) -> Option<f32>,
) -> Result<Merged, Rejected> {
    // A reply wrapped in a code fence is still the same object.
    let json = match (answer.find('{'), answer.rfind('}')) {
        (Some(a), Some(b)) if a < b => &answer[a..=b],
        _ => return Err(Rejected::Json),
    };
    let a: Answer = serde_json::from_str(json).map_err(|_| Rejected::Json)?;
    let text = a.texto.trim().to_owned();
    let title = a
        .titulo
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty());
    if text.is_empty() {
        return Err(Rejected::Json);
    }
    let want: BTreeSet<i64> = originals.iter().map(|r| r.id.0).collect();
    let got: Option<Vec<i64>> = a.fuentes.iter().map(source_id).collect();
    let got = got.ok_or(Rejected::Sources)?;
    if got.len() != want.len() || got.iter().copied().collect::<BTreeSet<_>>() != want {
        return Err(Rejected::Sources);
    }
    let before: usize = originals.iter().map(|r| r.content.chars().count()).sum();
    if text.chars().count() > before {
        return Err(Rejected::Longer);
    }
    let said = format!("{text}\n{}", title.as_deref().unwrap_or(""));
    if has_secret(&said) {
        return Err(Rejected::Secret);
    }
    let source = originals
        .iter()
        .map(|r| format!("{}\n{}", r.content, r.title.as_deref().unwrap_or("")))
        .collect::<Vec<_>>()
        .join("\n");
    if URL.find_iter(&said).any(|u| !source.contains(u.as_str())) {
        return Err(Rejected::NewUrl);
    }
    let without_urls = URL.replace_all(&said, " ");
    let source_without_urls = URL.replace_all(&source, " ");
    if PATH
        .find_iter(&without_urls)
        .any(|p| !source_without_urls.contains(p.as_str()))
    {
        return Err(Rejected::NewPath);
    }
    // By what it says, not by its kind: one command in an original does not allow another.
    if EXFIL
        .iter()
        .flat_map(|r| r.find_iter(&said))
        .any(|m| !source.contains(m.as_str()))
    {
        return Err(Rejected::NewCommand);
    }
    let had: BTreeSet<&str> = injections(&fold(&source));
    if !injections(&fold(&said)).is_subset(&had)
        || said.chars().any(|c| hidden(c) && !source.contains(c))
    {
        return Err(Rejected::NewInjection);
    }
    // What it adds: an instruction in plain words passes every check above. Every number and
    // every word of four letters or more must come from the originals, but for a word or two of
    // paraphrase; the title, wholly.
    let known = words(&source_without_urls);
    if let Some(t) = &title
        && !words(t).is_subset(&known)
    {
        return Err(Rejected::Title);
    }
    let new: Vec<String> = words(&URL.replace_all(&text, " "))
        .into_iter()
        .filter(|w| {
            !known.contains(w) && (w.chars().count() >= 4 || w.chars().any(|c| c.is_ascii_digit()))
        })
        .collect();
    if new.iter().any(|w| w.chars().any(|c| c.is_ascii_digit())) || new.len() > NEW_WORDS {
        return Err(Rejected::NewWords);
    }
    for r in originals {
        if similarity(&text, &r.content).is_some_and(|s| s < MERGE_SIMILARITY) {
            return Err(Rejected::Drift(r.id));
        }
    }
    Ok(Merged {
        text,
        title,
        sources: originals.iter().map(|r| r.id).collect(),
    })
}
