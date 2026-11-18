//! The sibling sessions one session should know about: who else works in the
//! same repository and its worktrees, what they're doing and which files
//! they're changing. Pure: the app supplies every [`Member`].
use serde::Serialize;
use std::path::PathBuf;

pub const MAX_ENTRIES: usize = 12;
pub const MAX_CHARS: usize = 2_000;
const MAX_TITLE: usize = 80;
const MAX_GOAL: usize = 160;
const MAX_FILES_SHOWN: usize = 5;
const HEADER: &str = "Other sessions in this repository (main checkout and its worktrees). Use sessions_list or sessions_read for more.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Working,
    WaitingOnUser,
    Held,
    Idle,
}

impl Status {
    fn words(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::WaitingOnUser => "waiting on the user",
            Self::Held => "on hold",
            Self::Idle => "idle",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Workspace {
    Main,
    Worktree { branch: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub id: i64,
    pub title: String,
    pub harness: String,
    pub status: Status,
    pub workspace: Workspace,
    pub goal: String,
    pub files: Vec<String>,
    /// From [`super::repo::repo_key`]; `None` is never anyone's sibling.
    pub repo: Option<PathBuf>,
    pub cwd: String,
    pub last_active_ms: u64,
    pub archived: bool,
    pub remote: bool,
    /// The title of the session that started this one, if one did.
    pub parent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Entry {
    #[serde(flatten)]
    pub member: Member,
    /// Files this sibling changes that the caller changes too.
    pub overlap: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Roster {
    pub entries: Vec<Entry>,
    /// Siblings beyond [`MAX_ENTRIES`].
    pub omitted: usize,
}

/// The caller's siblings: same repository, not archived, not remote, most
/// relevant first, capped at [`MAX_ENTRIES`].
pub fn build(caller: &Member, others: &[Member]) -> Roster {
    let Some(repo) = &caller.repo else {
        return Roster::default();
    };
    let mut entries: Vec<Entry> = others
        .iter()
        .filter(|m| m.id != caller.id && !m.archived && !m.remote)
        .filter(|m| m.repo.as_ref() == Some(repo))
        .map(|m| Entry {
            overlap: m
                .files
                .iter()
                .filter(|f| caller.files.contains(f))
                .cloned()
                .collect(),
            member: m.clone(),
        })
        .collect();
    entries.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then(b.member.last_active_ms.cmp(&a.member.last_active_ms))
    });
    let omitted = entries.len().saturating_sub(MAX_ENTRIES);
    entries.truncate(MAX_ENTRIES);
    Roster { entries, omitted }
}

fn rank(entry: &Entry) -> u8 {
    match entry.member.status {
        Status::Working if !entry.overlap.is_empty() => 0,
        Status::Working => 1,
        Status::WaitingOnUser => 2,
        Status::Held => 3,
        Status::Idle => 4,
    }
}

/// The roster as a block for an agent's prompt, within [`MAX_CHARS`].
/// `None` when there are no siblings.
pub fn render(roster: &Roster) -> Option<String> {
    if roster.entries.is_empty() {
        return None;
    }
    let mut lines = vec![HEADER.to_owned()];
    let mut used = HEADER.chars().count();
    // Room for the last line, which says how many were left out.
    let reserve = "\n…and 9999 more".chars().count();
    let mut shown = 0;
    for entry in &roster.entries {
        let block = entry_lines(entry);
        let size: usize = block.iter().map(|l| l.chars().count() + 1).sum();
        if used + size + reserve > MAX_CHARS {
            break;
        }
        used += size;
        lines.extend(block);
        shown += 1;
    }
    let left_out = roster.entries.len() - shown + roster.omitted;
    if left_out > 0 {
        lines.push(format!("…and {left_out} more"));
    }
    Some(lines.join("\n"))
}

fn entry_lines(entry: &Entry) -> Vec<String> {
    let m = &entry.member;
    let title = flatten(&m.title, MAX_TITLE);
    let place = match &m.workspace {
        Workspace::Main => "main checkout".to_owned(),
        Workspace::Worktree { branch } => format!("worktree {}", flatten(branch, MAX_TITLE)),
    };
    let mut line = format!(
        "• \"{title}\" ({}, {}) in {place}",
        flatten(&m.harness, 40),
        m.status.words()
    );
    if let Some(parent) = &m.parent {
        line.push_str(&format!(", started by \"{}\"", flatten(parent, MAX_TITLE)));
    }
    let goal = flatten(&m.goal, MAX_GOAL);
    if !goal.is_empty() {
        line.push_str(&format!(": {goal}."));
    }
    if !m.files.is_empty() {
        line.push_str(&format!(" Changing: {}", files(&m.files)));
    }
    let mut lines = vec![line];
    if !entry.overlap.is_empty() {
        lines.push(format!(
            "⚠ \"{title}\" also changes {}, which you changed.",
            files(&entry.overlap)
        ));
    }
    lines
}

fn files(files: &[String]) -> String {
    let shown: Vec<String> = files
        .iter()
        .take(MAX_FILES_SHOWN)
        .map(|f| flatten(f, 120))
        .collect();
    let mut text = shown.join(", ");
    if files.len() > MAX_FILES_SHOWN {
        text.push_str(&format!(" +{}", files.len() - MAX_FILES_SHOWN));
    }
    text
}

/// One line of untrusted text: whitespace collapsed, quotes softened, cut to
/// `max` characters, so no sibling can add lines to another's prompt.
fn flatten(text: &str, max: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let line = words.join(" ").replace('"', "'");
    if line.chars().count() <= max {
        return line;
    }
    let mut cut: String = line.chars().take(max - 1).collect();
    cut.push('…');
    cut
}

/// A hash that is the same across runs and platforms (64-bit FNV-1a), so a
/// roster sent once is recognised as unchanged.
pub fn fingerprint(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}
