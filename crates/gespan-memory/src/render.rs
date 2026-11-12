use crate::entry::{Entry, Status};

/// The session-start block (and the project decision file) is capped here.
pub const START_BUDGET: usize = 8192;
/// The per-turn related block is capped here.
pub const TURN_BUDGET: usize = 2048;

pub const STANDING_INSTRUCTION: &str = "Gespan decision log. Follow every approved decision below. If one seems wrong for the case in front of you, still follow it, say so in your reply, and call decisions_flag with its id and your reason; never stop to ask about it. After the user answers a design question, call decisions_propose with the question, the user's exact answer, the decision and why. Entries marked not yet approved come from the user's own words but have not been reviewed.";

/// A rendered block and what went into it.
#[derive(Debug, Clone)]
pub struct Block {
    pub text: String,
    /// Ids of the entries shown, in the order shown.
    pub included: Vec<i64>,
    /// Entries left out for lack of room (preferences included, when they
    /// alone overflowed the budget).
    pub omitted: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Preferences,
    Decisions,
    Pending,
}

const PENDING_HEADING: &str = "Not yet approved (from the user's own words, not reviewed)";

struct Line {
    id: i64,
    text: String,
    section: Section,
}

/// Render preferences then decisions as a Markdown list within `budget_bytes`.
///
/// Sections, in order: approved preferences (newest first), approved
/// decisions (pinned first, then newest), and, last, every pending entry of
/// either kind under its own heading (newest created first). Pending entries
/// never share a section with approved ones, and their marker precedes any
/// field text, so entry text cannot make a pending line look approved. The
/// budget counts the whole text: the instruction, headings, and the closing
/// `N more decisions: use decisions_search` line. Lines are dropped whole,
/// from the end, so pending entries go first, then decisions, and
/// preferences only when they alone overflow (oldest first). A heading with
/// nothing under it is not emitted. If the instruction alone exceeds the
/// budget the result is still the instruction with no entries.
pub fn render_block(prefs: &[Entry], decisions: &[Entry], budget_bytes: usize) -> Block {
    let recency = |e: &Entry| e.decided_ms.unwrap_or(e.created_ms);
    let pending = |e: &&Entry| e.status == Status::Pending;
    let approved = |e: &&Entry| e.status != Status::Pending;
    let mut p: Vec<&Entry> = prefs.iter().filter(approved).collect();
    p.sort_by_key(|e| std::cmp::Reverse((recency(e), e.id)));
    let mut d: Vec<&Entry> = decisions.iter().filter(approved).collect();
    d.sort_by_key(|e| std::cmp::Reverse((e.pinned, recency(e), e.id)));
    let mut n: Vec<&Entry> = prefs.iter().chain(decisions).filter(pending).collect();
    n.sort_by_key(|e| std::cmp::Reverse((e.created_ms, e.id)));

    let lines: Vec<Line> = [
        (Section::Preferences, p),
        (Section::Decisions, d),
        (Section::Pending, n),
    ]
    .into_iter()
    .flat_map(|(section, es)| {
        es.into_iter().map(move |e| Line {
            id: e.id,
            text: line(e),
            section,
        })
    })
    .collect();

    // Keep the longest prefix that fits. Fit is not monotonic in the prefix
    // length (the closing line disappears when nothing is omitted), so try
    // from the longest candidate down; a prefix whose lines alone overflow
    // cannot fit and is skipped without assembling it.
    let mut raw = 0usize;
    let mut max_k = 0usize;
    for l in &lines {
        raw += l.text.len() + 1;
        if raw > budget_bytes {
            break;
        }
        max_k += 1;
    }
    for k in (0..=max_k).rev() {
        let text = assemble(&lines[..k], lines.len() - k);
        if text.len() <= budget_bytes || k == 0 {
            return Block {
                included: lines[..k].iter().map(|l| l.id).collect(),
                omitted: lines.len() - k,
                text,
            };
        }
    }
    unreachable!("k == 0 always returns")
}

fn assemble(kept: &[Line], omitted: usize) -> String {
    let mut out = String::from(STANDING_INSTRUCTION);
    for (heading, want) in [
        ("Preferences", Section::Preferences),
        ("Decisions", Section::Decisions),
        (PENDING_HEADING, Section::Pending),
    ] {
        let mut section = kept.iter().filter(|l| l.section == want).peekable();
        if section.peek().is_none() {
            continue;
        }
        out.push_str("\n\n## ");
        out.push_str(heading);
        for l in section {
            out.push('\n');
            out.push_str(&l.text);
        }
    }
    if omitted > 0 {
        out.push_str(&format!(
            "\n\n{omitted} more decisions: use decisions_search"
        ));
    }
    out
}

/// `- D-14 (title): text. Why: why.` on one line. A pending entry carries its
/// marker straight after the label, before any field text.
fn line(e: &Entry) -> String {
    let marker = if e.status == Status::Pending {
        match e.source.session {
            Some(s) => format!(" [not yet approved — the user said this in session {s}]"),
            None => " [not yet approved — the user said this]".to_string(),
        }
    } else {
        String::new()
    };
    format!(
        "- {}{} ({}): {}. Why: {}.",
        e.label(),
        marker,
        flat(&e.title),
        strip_one_dot(&flat(&e.text)),
        strip_one_dot(&flat(&e.why)),
    )
}

fn strip_one_dot(s: &str) -> &str {
    s.strip_suffix('.').unwrap_or(s)
}

/// Collapse every run of whitespace, newlines included, to one space, so an
/// entry can never span lines or forge another entry's line.
fn flat(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
