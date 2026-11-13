use super::entry::{Entry, Status};

/// The session-start block (and the project decision file) is capped here.
pub const START_BUDGET: usize = 8192;
/// The per-turn related block is capped here.
pub const TURN_BUDGET: usize = 2048;

pub const STANDING_INSTRUCTION: &str = "Gespan decision log. Follow every approved decision below. If one seems wrong for the case in front of you, still follow it, say so in your reply, and call decisions_flag with its id and your reason; never stop to ask about it. After the user answers a design question, call decisions_propose with the question, the user's exact answer, the decision and why. Entries marked not yet approved quote the user's own words; the rest of each is an agent's summary and has not been reviewed.";

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

const PENDING_HEADING: &str =
    "Not yet approved (each is an agent's summary of the user's quoted answer, not reviewed)";

/// The most characters of the user's answer a pending line quotes, so one
/// long answer cannot take the block's budget.
pub const ANSWER_QUOTE_CHARS: usize = 200;

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
        let text = assemble(STANDING_INSTRUCTION, &lines[..k], lines.len() - k);
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

fn assemble(opening: &str, kept: &[Line], omitted: usize) -> String {
    let mut out = String::from(opening);
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

/// Opens the per-turn related block. The standing instruction is left out:
/// the session-start block and the harness files already carry it, and
/// repeating it would take about a quarter of each turn's budget.
pub const RELATED_HEADING: &str =
    "Gespan decision log: entries related to this message, most relevant first.";

/// Render `entries` in the order given (a search ranking, best first)
/// within `budget_bytes`, for the per-turn related block.
///
/// Unlike [`render_block`] nothing is re-sorted: approved entries keep their
/// rank under their section (preferences, then decisions), and pending ones
/// keep theirs under the "Not yet approved" heading, last, each with its
/// marker before any field text. When over budget, entries are dropped from
/// the end of the ranking, so the best match is the last to go, and the
/// closing `N more decisions: use decisions_search` line counts toward the
/// budget. `included` lists the ids in the order shown.
pub fn render_ranked(entries: &[Entry], budget_bytes: usize) -> Block {
    use super::entry::Kind;
    let lines: Vec<Line> = entries
        .iter()
        .map(|e| Line {
            id: e.id,
            text: line(e),
            section: match (e.status, e.kind) {
                (Status::Pending, _) => Section::Pending,
                (_, Kind::Preference) => Section::Preferences,
                (_, Kind::Decision) => Section::Decisions,
            },
        })
        .collect();
    for k in (0..=lines.len()).rev() {
        let text = assemble(RELATED_HEADING, &lines[..k], lines.len() - k);
        if text.len() <= budget_bytes || k == 0 {
            let included = [Section::Preferences, Section::Decisions, Section::Pending]
                .into_iter()
                .flat_map(|want| lines[..k].iter().filter(move |l| l.section == want))
                .map(|l| l.id)
                .collect();
            return Block {
                text,
                included,
                omitted: lines.len() - k,
            };
        }
    }
    unreachable!("k == 0 always returns")
}

/// `- D-14 (title): text. Why: why.` on one line (no `Why:` when it is
/// empty). A pending entry carries its marker straight after the label,
/// before any field text.
fn line(e: &Entry) -> String {
    let marker = if e.status == Status::Pending {
        format!(" [{}]", pending_note(e))
    } else {
        String::new()
    };
    let why = flat(&e.why);
    let why = strip_one_dot(&why);
    let why = if why.is_empty() {
        String::new()
    } else {
        format!(" Why: {why}.")
    };
    format!(
        "- {}{} ({}): {}.{why}",
        e.label(),
        marker,
        flat(&e.title),
        strip_one_dot(&flat(&e.text)),
    )
}

/// What marks a pending entry: `not yet approved — the user said "<their
/// answer>" in session N`, quoting the user's own words (flattened, capped
/// at [`ANSWER_QUOTE_CHARS`]) so a reader can tell them from the agent's
/// summary that follows. An entry with no stored answer says "the user said
/// this" instead.
pub(crate) fn pending_note(e: &Entry) -> String {
    let said = match e.user_answer.as_deref().map(flat).filter(|a| !a.is_empty()) {
        Some(answer) => format!("\"{}\"", capped(&answer, ANSWER_QUOTE_CHARS)),
        None => "this".to_owned(),
    };
    match e.source.session {
        Some(s) => format!("not yet approved — the user said {said} in session {s}"),
        None => format!("not yet approved — the user said {said}"),
    }
}

/// `s` cut to `max` characters, with an ellipsis when it was longer.
fn capped(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", s[..cut].trim_end()),
        None => s.to_owned(),
    }
}

fn strip_one_dot(s: &str) -> &str {
    s.strip_suffix('.').unwrap_or(s)
}

/// Collapse every run of whitespace, newlines included, to one space, so an
/// entry can never span lines or forge another entry's line; and turn `<`
/// and `>` into `‹` and `›`, so no entry can open or close a tag (the
/// `<gespan_context>` block a turn wraps it in, or any other).
///
/// Every angle bracket goes, not only `gespan_` tags: harnesses and models
/// read other tags too (`</system>`, `<user>`), a partial escape is easy to
/// get around (`</ gespan_context>`, case), and entries are prose, where
/// `‹›` reads the same.
pub(crate) fn flat(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('<', "‹")
        .replace('>', "›")
}
