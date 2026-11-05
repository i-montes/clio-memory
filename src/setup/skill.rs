//! Where the skill goes so every connected agent reads it exactly once if it can be helped.
//!
//! The agents share skill folders (OpenCode reads Claude's and the common `~/.agents/skills`, and
//! so on), so one copy per agent would show some of them the skill twice. `placement` picks the
//! fewest folders that cover everybody. A copy of ours, current or from a published version, is
//! ours to update or move; anything else at that path is never touched without `replace`.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::SKILL;
use super::agents::AgentId;

/// A folder agents read skills from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillDir {
    Claude,
    OpenCode,
    Agents,
    Antigravity,
}

impl SkillDir {
    /// In the order ties in `placement` are broken.
    pub const ALL: [SkillDir; 4] = [
        SkillDir::Claude,
        SkillDir::OpenCode,
        SkillDir::Agents,
        SkillDir::Antigravity,
    ];

    /// The skill's file under `home`.
    pub fn path(self, home: &Path) -> PathBuf {
        let root = match self {
            SkillDir::Claude => ".claude/skills",
            SkillDir::OpenCode => ".config/opencode/skills",
            SkillDir::Agents => ".agents/skills",
            SkillDir::Antigravity => ".gemini/config/skills",
        };
        home.join(root).join("clio-memory").join("SKILL.md")
    }

    /// The agents that read this folder.
    pub fn readers(self) -> &'static [AgentId] {
        match self {
            SkillDir::Claude => &[AgentId::ClaudeCode, AgentId::OpenCode],
            SkillDir::OpenCode => &[AgentId::OpenCode],
            SkillDir::Agents => &[AgentId::OpenCode, AgentId::Codex, AgentId::Gemini],
            SkillDir::Antigravity => &[AgentId::Antigravity],
        }
    }
}

/// The folders to put the skill in: of the sets that cover every agent in `agents`, the one that
/// shows the fewest agents two copies, then has the fewest folders, then is read by the fewest
/// agents in all (a lone OpenCode gets its own folder, not Claude's), then comes first in order.
pub fn placement(agents: &[AgentId]) -> Vec<SkillDir> {
    if agents.is_empty() {
        return Vec::new();
    }
    let mut best: Option<((usize, usize, usize), Vec<SkillDir>)> = None;
    // Subsets in the order of their folders (0b0001 is Claude alone, ...), so on equal cost the
    // first one found is the first in folder order: a later one must be strictly better.
    let mut subsets: Vec<Vec<SkillDir>> = (1u32..16)
        .map(|mask| {
            SkillDir::ALL
                .into_iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, d)| d)
                .collect()
        })
        .collect();
    subsets.sort_by_key(|s| {
        s.iter()
            .map(|d| SkillDir::ALL.iter().position(|x| x == d))
            .collect::<Vec<_>>()
    });
    for set in subsets {
        let copies = |a: &AgentId| set.iter().filter(|d| d.readers().contains(a)).count();
        if agents.iter().any(|a| copies(a) == 0) {
            continue;
        }
        let cost = (
            agents.iter().filter(|a| copies(a) >= 2).count(),
            set.len(),
            set.iter().map(|d| d.readers().len()).sum(),
        );
        if best.as_ref().is_none_or(|(c, _)| cost < *c) {
            best = Some((cost, set));
        }
    }
    best.map(|(_, set)| set).unwrap_or_default()
}

/// The sha256 of every skill clio-memory has published, besides the current one.
pub const KNOWN_SKILLS: &[&str] = &[
    // v0.2.0
    "e748bfac08f18e3288eaea2808a7609112775bd6586512732c931b97c135404a",
    // v0.4.0
    "c2482bda5826a43537eeb0773e88a7b96ba17dcafd062d47d63e2a8d01bce1c4",
    // v0.5.0
    "73eb801b86507af87bd643f0dd70478abddcb8418595b06f2ac70c1fb707524b",
    // v0.6.0
    "f82d55db3b943c3d0e1898431010cf27cb4fd8914b27f7c89226352c2e43f452",
];

/// What is at a skill's path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillFile {
    Absent,
    /// This version's skill.
    Current,
    /// A skill clio-memory published before: ours, to be updated.
    Old,
    /// Anything else (including what cannot be read): never ours to touch.
    Foreign,
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn skill_state(path: &Path) -> SkillFile {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return SkillFile::Absent,
        Err(_) => return SkillFile::Foreign,
    };
    let sha = sha256_hex(&bytes);
    if sha == sha256_hex(SKILL.as_bytes()) {
        SkillFile::Current
    } else if KNOWN_SKILLS.contains(&sha.as_str()) {
        SkillFile::Old
    } else {
        SkillFile::Foreign
    }
}

/// Writes the current skill in each folder of `placement(agents)` and removes ours (current or
/// old) from the other folders. A foreign skill in a folder to write is an error naming it, and
/// nothing is written, unless `replace`. A foreign one in a folder not used is left alone.
pub fn place_skill(home: &Path, agents: &[AgentId], replace: bool) -> Result<Vec<String>, String> {
    let wanted = placement(agents);
    if !replace {
        let blocked: Vec<String> = wanted
            .iter()
            .map(|d| d.path(home))
            .filter(|p| skill_state(p) == SkillFile::Foreign)
            .map(|p| p.display().to_string())
            .collect();
        if !blocked.is_empty() {
            return Err(format!(
                "ya hay otra skill en {} (no es la de clio-memory); usa --replace para reemplazarla",
                blocked.join(", ")
            ));
        }
    }
    let mut said = Vec::new();
    for dir in wanted.iter().copied() {
        let path = dir.path(home);
        let state = skill_state(&path);
        if state == SkillFile::Current {
            said.push(format!("skill al día en {}", path.display()));
            continue;
        }
        super::write_private(&path, SKILL)?;
        said.push(match state {
            SkillFile::Absent => format!("skill instalada en {}", path.display()),
            _ => format!("skill actualizada en {}", path.display()),
        });
    }
    for dir in SkillDir::ALL.into_iter().filter(|d| !wanted.contains(d)) {
        let path = dir.path(home);
        if matches!(skill_state(&path), SkillFile::Current | SkillFile::Old) {
            std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            // The folder goes too when nothing else of the person's is in it.
            if let Some(folder) = path.parent() {
                let _ = std::fs::remove_dir(folder);
            }
            said.push(format!("skill retirada de {}", path.display()));
        }
    }
    Ok(said)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sha256 of `skill/SKILL.md` as this version ships it.
    const CURRENT_SKILL_SHA: &str =
        "2e3637e12fd31d53632ed8574e94241bae03ca95d84ca6c016f836da0317df01";

    // An edited skill without its predecessor in `KNOWN_SKILLS` would make every installed copy
    // of the previous one «foreign»: never updated, and a ✖ on every agent.
    #[test]
    fn the_skill_is_the_pinned_one() {
        let sha = sha256_hex(SKILL.as_bytes());
        assert!(
            sha == CURRENT_SKILL_SHA,
            "skill/SKILL.md changed (sha256 {sha}). Add the previous hash, {CURRENT_SKILL_SHA}, \
             to KNOWN_SKILLS (with the version that published it) if a release shipped it, and \
             set CURRENT_SKILL_SHA to {sha}."
        );
        assert!(
            !KNOWN_SKILLS.contains(&CURRENT_SKILL_SHA),
            "the current skill is not an old one"
        );
    }
}
