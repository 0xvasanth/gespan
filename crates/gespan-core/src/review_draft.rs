//! Inline draft reviews: a reviewer's findings left on the pull or merge
//! request as a GitHub pending review or GitLab draft notes, each on its diff
//! line, for the user to submit or keep as a draft.
//!
//! The agent never writes to the forge. It ends its reply with a
//! `gespan-review` JSON block, and Gespan posts it here to the one pull or
//! merge request the run was started for, which comes from the run's event,
//! never from the agent. A finding whose line the diff does not show cannot
//! carry an inline comment, so it moves into the review's summary instead.
//! Nothing is submitted: GitHub keeps a pending review visible only to its
//! author, and GitLab draft notes stay private until published.

pub mod diff_map;

use crate::review_policy::{Forge, ReviewTarget};
use diff_map::{normalize_path, Anchor, DiffMap, Side};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The fence tag the agent's findings come in.
pub const BLOCK_TAG: &str = "gespan-review";
/// The most inline comments one review leaves; the rest join the summary.
pub const MAX_COMMENTS: usize = 50;
/// The longest comment or summary, in characters.
pub const MAX_BODY: usize = 6_000;

/// Appended to a pending-review run's prompt.
pub const INSTRUCTION: &str = "When your review is ready, end your final reply with one fenced block tagged gespan-review holding JSON in exactly this shape:\n\n```gespan-review\n{\"summary\": \"<verdict and overall review, in Markdown>\", \"comments\": [{\"path\": \"<file path as the diff names it>\", \"side\": \"new\", \"line\": 12, \"body\": \"<one finding>\"}]}\n```\n\nPut each finding on the line it is about. Use side \"new\" with the line number in the changed file for an added or unchanged line, and side \"old\" with the line number in the original file for a removed line. Only lines inside the diff's hunks can carry a comment; put anything else in the summary. Gespan leaves this as a draft review on the pull request, and the user decides whether to submit it. Do not post anything yourself, and do not change any files.";

/// One finding the agent made.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub path: String,
    pub side: Side,
    /// `None` when the agent gave no usable line.
    pub line: Option<u32>,
    pub body: String,
}

/// What the agent's reply asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Findings {
    pub summary: String,
    pub comments: Vec<Finding>,
}

#[derive(Deserialize)]
struct RawReview {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    comments: Vec<Value>,
}

/// Reads the agent's final reply. The last `gespan-review` block holds the
/// findings; a reply without a readable one becomes the summary, so the
/// review is never lost.
pub fn parse_reply(reply: &str) -> Findings {
    if let Some((block, start, end)) = last_block(reply) {
        if let Ok(raw) = serde_json::from_str::<RawReview>(block) {
            let comments = raw.comments.iter().filter_map(finding).collect();
            let mut summary = clip(raw.summary.trim());
            if summary.is_empty() {
                let mut rest = String::new();
                rest.push_str(reply[..start].trim());
                rest.push_str(reply[end..].trim());
                summary = clip(rest.trim());
            }
            return Findings { summary, comments };
        }
    }
    Findings {
        summary: clip(reply.trim()),
        comments: Vec::new(),
    }
}

fn finding(value: &Value) -> Option<Finding> {
    let body = clip(value.get("body")?.as_str()?.trim());
    if body.is_empty() {
        return None;
    }
    let path = normalize_path(value.get("path").and_then(Value::as_str).unwrap_or(""));
    let side = match value
        .get("side")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("old" | "left" | "base" | "deleted" | "removed") => Side::Old,
        _ => Side::New,
    };
    let line = value
        .get("line")
        .and_then(|l| l.as_u64().or_else(|| l.as_str()?.trim().parse().ok()))
        .filter(|l| *l >= 1)
        .and_then(|l| u32::try_from(l).ok());
    Some(Finding {
        path,
        side,
        line,
        body,
    })
}

/// The last fenced block tagged [`BLOCK_TAG`]: its text and where the whole
/// block starts and ends in `reply`.
fn last_block(reply: &str) -> Option<(&str, usize, usize)> {
    let mut found = None;
    let mut offset = 0;
    let mut open: Option<(usize, usize, usize)> = None; // (fence len, block start, body start)
    for line in reply.split_inclusive('\n') {
        let trimmed = line.trim();
        let ticks = trimmed.chars().take_while(|c| *c == '`').count();
        match open {
            None if ticks >= 3 && trimmed[ticks..].trim() == BLOCK_TAG => {
                open = Some((ticks, offset, offset + line.len()));
            }
            Some((fence, start, body)) if ticks >= fence && trimmed.len() == ticks => {
                found = Some((&reply[body..offset], start, offset + line.len()));
                open = None;
            }
            _ => {}
        }
        offset += line.len();
    }
    found
}

/// Clipped to [`MAX_BODY`], with slash commands made inert.
fn clip(text: &str) -> String {
    let text = inert(text);
    if text.chars().count() <= MAX_BODY {
        text
    } else {
        let mut out: String = text.chars().take(MAX_BODY).collect();
        out.push('…');
        out
    }
}

/// Escapes every line that starts with `/` outside a code fence, so posting
/// the text runs nothing: GitLab applies quick actions such as `/merge` or
/// `/approve` from a published note, bots act on `/commands`, and a
/// `/gespan review` line would start another review. `\/` renders as `/`.
pub fn inert(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut fence: Option<(char, usize)> = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let mark = trimmed.chars().next().filter(|c| *c == '`' || *c == '~');
        if let Some(c) = mark {
            let run = trimmed.chars().take_while(|x| *x == c).count();
            if run >= 3 {
                fence = match fence {
                    None => Some((c, run)),
                    Some((open, len)) if open == c && run >= len => None,
                    other => other,
                };
                out.push_str(line);
                continue;
            }
        }
        if fence.is_none() && trimmed.starts_with('/') {
            let indent = line.len() - trimmed.len();
            out.push_str(&line[..indent]);
            out.push('\\');
            out.push_str(trimmed);
        } else {
            out.push_str(line);
        }
    }
    out
}

/// A finding placed on its diff line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placed {
    pub anchor: Anchor,
    pub body: String,
}

/// Where each finding goes: inline on a diff line, or into the summary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub summary: String,
    pub inline: Vec<Placed>,
    pub general: Vec<Finding>,
}

impl Plan {
    /// Places every finding the diff shows, up to [`MAX_COMMENTS`].
    pub fn new(findings: Findings, map: &DiffMap) -> Self {
        let mut plan = Plan {
            summary: findings.summary,
            ..Default::default()
        };
        for finding in findings.comments {
            let anchor = finding
                .line
                .filter(|_| plan.inline.len() < MAX_COMMENTS)
                .and_then(|line| map.locate(&finding.path, finding.side, line));
            match anchor {
                Some(anchor) => plan.inline.push(Placed {
                    anchor,
                    body: finding.body,
                }),
                None => plan.general.push(finding),
            }
        }
        plan
    }

    /// Moves every inline comment into the summary, for a forge that refused
    /// them.
    fn all_general(&self) -> Plan {
        let mut general: Vec<Finding> = self
            .inline
            .iter()
            .map(|placed| Finding {
                path: placed.anchor.new_path.clone(),
                side: placed.anchor.side,
                line: Some(placed.anchor.line),
                body: placed.body.clone(),
            })
            .collect();
        general.extend(self.general.iter().cloned());
        Plan {
            summary: self.summary.clone(),
            inline: Vec::new(),
            general,
        }
    }

    /// The review's top-level text: the summary, then the findings that could
    /// not go inline, each with the place the agent meant.
    pub fn body(&self) -> String {
        let mut out = if self.summary.trim().is_empty() {
            "Review by Gespan.".to_string()
        } else {
            self.summary.trim().to_string()
        };
        if !self.general.is_empty() {
            out.push_str("\n\n**Notes on lines outside the diff**\n");
            for finding in &self.general {
                let place = match (finding.path.is_empty(), finding.line) {
                    (true, _) => String::new(),
                    (false, Some(line)) => {
                        let side = match finding.side {
                            Side::Old => " (old)",
                            Side::New => "",
                        };
                        format!("`{}` line {line}{side}: ", finding.path)
                    }
                    (false, None) => format!("`{}`: ", finding.path),
                };
                let body = finding.body.replace('\n', "\n  ");
                out.push_str(&format!("\n- {place}{body}"));
            }
        }
        out
    }
}

/// A comment as the user is shown it before deciding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShownComment {
    pub path: String,
    pub side: Side,
    pub line: u32,
    pub body: String,
}

/// How to find the draft again on its forge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Handle {
    Github { review_id: u64 },
    Gitlab { note_ids: Vec<u64> },
}

/// A draft review left on a pull or merge request, waiting on the user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftReview {
    pub forge: Forge,
    pub repo: String,
    pub number: u64,
    pub url: String,
    pub handle: Handle,
    /// The review's top-level text.
    pub body: String,
    pub inline: Vec<ShownComment>,
    /// Findings that went into the summary because the diff does not show
    /// their line.
    pub outside: usize,
}

impl DraftReview {
    pub fn target(&self) -> ReviewTarget {
        ReviewTarget {
            forge: self.forge,
            repo: self.repo.clone(),
            number: self.number,
        }
    }
}

fn shown(plan: &Plan) -> Vec<ShownComment> {
    plan.inline
        .iter()
        .map(|placed| ShownComment {
            path: placed.anchor.new_path.clone(),
            side: placed.anchor.side,
            line: placed.anchor.line,
            body: placed.body.clone(),
        })
        .collect()
}

// ---------------------------------------------------------------- GitHub

/// Runs `gh` with `args`. When `input` is given it is the JSON request body,
/// sent with `--input`.
pub type GhRunner<'a> = dyn FnMut(&[&str], Option<&str>) -> Result<String, String> + 'a;

/// The request body of a pending GitHub review: no `event`, so GitHub keeps
/// it as a draft only its author sees.
pub fn github_payload(plan: &Plan, head_sha: &str) -> Value {
    let comments: Vec<Value> = plan
        .inline
        .iter()
        .map(|placed| {
            json!({
                "path": placed.anchor.new_path,
                "line": placed.anchor.line,
                "side": match placed.anchor.side {
                    Side::Old => "LEFT",
                    Side::New => "RIGHT",
                },
                "body": placed.body,
            })
        })
        .collect();
    json!({
        "commit_id": head_sha,
        "body": plan.body(),
        "comments": comments,
    })
}

/// Leaves `findings` as a pending review on GitHub pull request `target`.
pub fn github_create(
    target: &ReviewTarget,
    findings: Findings,
    gh: &mut GhRunner<'_>,
) -> Result<DraftReview, String> {
    if target.forge != Forge::GitHub {
        return Err("Not a GitHub pull request".into());
    }
    let number = target.number.to_string();
    let head = gh(
        &[
            "pr",
            "view",
            &number,
            "--repo",
            &target.repo,
            "--json",
            "headRefOid",
            "--jq",
            ".headRefOid",
        ],
        None,
    )?;
    let head = head.trim().to_string();
    if head.len() < 7 || !head.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("GitHub did not return the pull request's head commit".into());
    }
    // A diff GitHub will not produce (too large) leaves every finding in the
    // summary rather than failing the review.
    let map = match gh(&["pr", "diff", &number, "--repo", &target.repo], None) {
        Ok(diff) => DiffMap::parse(&diff),
        Err(error) => {
            tracing::warn!(%error, "the pull request diff was not read; findings go in the summary");
            DiffMap::default()
        }
    };
    let mut plan = Plan::new(findings, &map);
    let endpoint = format!("repos/{}/pulls/{}/reviews", target.repo, target.number);
    let post = |gh: &mut GhRunner<'_>, plan: &Plan| {
        let payload = github_payload(plan, &head).to_string();
        gh(&["api", "--method", "POST", &endpoint], Some(&payload))
    };
    let response = match post(gh, &plan) {
        Ok(response) => response,
        // GitHub allows one pending review per author: say so plainly.
        Err(error) if error.to_ascii_lowercase().contains("pending review") => {
            return Err(
                "You already have a pending review on this pull request. Submit or discard it on GitHub, then ask for the review again."
                    .into(),
            );
        }
        // A line GitHub would not resolve fails the whole review; keep the
        // findings by moving them into the summary.
        Err(error) if !plan.inline.is_empty() => {
            tracing::warn!(%error, "GitHub refused the inline comments; posting them in the summary");
            plan = plan.all_general();
            post(gh, &plan)?
        }
        Err(error) => return Err(error),
    };
    let value: Value = serde_json::from_str(&response)
        .map_err(|_| "GitHub returned an unreadable review".to_string())?;
    let review_id = value
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "GitHub did not return the review".to_string())?;
    let url = value
        .get("html_url")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "https://github.com/{}/pull/{}#pullrequestreview-{review_id}",
                target.repo, target.number
            )
        });
    Ok(DraftReview {
        forge: Forge::GitHub,
        repo: target.repo.clone(),
        number: target.number,
        url,
        handle: Handle::Github { review_id },
        body: plan.body(),
        inline: shown(&plan),
        outside: plan.general.len(),
    })
}

fn github_submit(draft: &DraftReview, review_id: u64, gh: &mut GhRunner<'_>) -> Result<(), String> {
    let endpoint = format!(
        "repos/{}/pulls/{}/reviews/{review_id}/events",
        draft.repo, draft.number
    );
    let payload = json!({ "event": "COMMENT", "body": draft.body }).to_string();
    gh(&["api", "--method", "POST", &endpoint], Some(&payload)).map(|_| ())
}

// ---------------------------------------------------------------- GitLab

/// GitLab's REST API, as the connected account. Paths start at `/projects`.
pub trait GitlabHttp {
    fn get(&mut self, path: &str) -> Result<Value, String>;
    fn post_json(&mut self, path: &str, body: &Value) -> Result<Value, String>;
    fn put(&mut self, path: &str) -> Result<Value, String>;
    /// The merge request's changes as one unified patch.
    fn mr_patch(&mut self, repo: &str, number: u64) -> Result<String, String>;
}

/// `/projects/<encoded path>/merge_requests/<iid>`.
pub fn gitlab_mr_path(target: &ReviewTarget) -> String {
    let mut encoded = String::new();
    for byte in target.repo.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(*byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("/projects/{encoded}/merge_requests/{}", target.number)
}

/// The SHAs a GitLab diff position is pinned to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffRefs {
    pub base: String,
    pub start: String,
    pub head: String,
}

/// The request body of one positioned GitLab draft note. An added line has
/// only `new_line`, a removed one only `old_line`, and an unchanged one both.
pub fn gitlab_note_payload(placed: &Placed, refs: &DiffRefs) -> Value {
    let mut position = json!({
        "position_type": "text",
        "base_sha": refs.base,
        "start_sha": refs.start,
        "head_sha": refs.head,
        "old_path": placed.anchor.old_path,
        "new_path": placed.anchor.new_path,
    });
    if let Some(line) = placed.anchor.old_line {
        position["old_line"] = json!(line);
    }
    if let Some(line) = placed.anchor.new_line {
        position["new_line"] = json!(line);
    }
    json!({ "note": placed.body, "position": position })
}

/// Leaves `findings` as draft notes on GitLab merge request `target`: one for
/// the summary, then one per inline finding.
pub fn gitlab_create(
    target: &ReviewTarget,
    findings: Findings,
    http: &mut dyn GitlabHttp,
) -> Result<DraftReview, String> {
    if target.forge != Forge::GitLab {
        return Err("Not a GitLab merge request".into());
    }
    let mr_path = gitlab_mr_path(target);
    let mr = http.get(&mr_path)?;
    let sha = |key: &str| {
        mr.get("diff_refs")
            .and_then(|r| r.get(key))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let refs = match (sha("base_sha"), sha("start_sha"), sha("head_sha")) {
        (Some(base), Some(start), Some(head)) => Some(DiffRefs { base, start, head }),
        _ => None,
    };
    let map = match (&refs, http.mr_patch(&target.repo, target.number)) {
        (Some(_), Ok(patch)) => DiffMap::parse(&patch),
        (_, Err(error)) => {
            tracing::warn!(%error, "the merge request diff was not read; findings go in the summary");
            DiffMap::default()
        }
        (None, Ok(_)) => DiffMap::default(),
    };
    let mut plan = Plan::new(findings, &map);
    let notes = format!("{mr_path}/draft_notes");
    // Inline notes first: one GitLab refuses moves into the summary.
    let mut inline_ids = Vec::new();
    let mut placed_ok = Vec::new();
    if let Some(refs) = &refs {
        for placed in std::mem::take(&mut plan.inline) {
            match http.post_json(&notes, &gitlab_note_payload(&placed, refs)) {
                Ok(note) => match note.get("id").and_then(Value::as_u64) {
                    Some(id) => {
                        inline_ids.push(id);
                        placed_ok.push(placed);
                    }
                    None => return Err("GitLab did not return the draft note".into()),
                },
                Err(error) => {
                    tracing::warn!(%error, "GitLab refused a positioned draft note; it goes in the summary");
                    plan.general.push(Finding {
                        path: placed.anchor.new_path.clone(),
                        side: placed.anchor.side,
                        line: Some(placed.anchor.line),
                        body: placed.body,
                    });
                }
            }
        }
    }
    plan.inline = placed_ok;
    let summary = http.post_json(&notes, &json!({ "note": plan.body() }))?;
    let summary_id = summary
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "GitLab did not return the draft note".to_string())?;
    let mut note_ids = vec![summary_id];
    note_ids.extend(inline_ids);
    let url = mr
        .get("web_url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(DraftReview {
        forge: Forge::GitLab,
        repo: target.repo.clone(),
        number: target.number,
        url,
        handle: Handle::Gitlab { note_ids },
        body: plan.body(),
        inline: shown(&plan),
        outside: plan.general.len(),
    })
}

fn gitlab_submit(
    draft: &DraftReview,
    note_ids: &[u64],
    http: &mut dyn GitlabHttp,
) -> Result<(), String> {
    // Only Gespan's own notes: bulk publishing would also send any draft the
    // user started by hand.
    let notes = format!("{}/draft_notes", gitlab_mr_path(&draft.target()));
    for id in note_ids {
        http.put(&format!("{notes}/{id}/publish"))?;
    }
    Ok(())
}

// ---------------------------------------------------------------- Both

/// Opens the connected GitLab account's API.
pub type GitlabOpener<'a> = dyn FnMut() -> Result<Box<dyn GitlabHttp>, String> + 'a;

/// Leaves `findings` as a draft review on `target`.
pub fn create(
    target: &ReviewTarget,
    findings: Findings,
    gh: &mut GhRunner<'_>,
    gitlab: &mut GitlabOpener<'_>,
) -> Result<DraftReview, String> {
    match target.forge {
        Forge::GitHub => github_create(target, findings, gh),
        Forge::GitLab => gitlab_create(target, findings, &mut *gitlab()?),
    }
}

/// Submits a draft the user chose to send.
pub fn submit(
    draft: &DraftReview,
    gh: &mut GhRunner<'_>,
    gitlab: &mut GitlabOpener<'_>,
) -> Result<(), String> {
    match (&draft.handle, draft.forge) {
        (Handle::Github { review_id }, Forge::GitHub) => github_submit(draft, *review_id, gh),
        (Handle::Gitlab { note_ids }, Forge::GitLab) => {
            gitlab_submit(draft, note_ids, &mut *gitlab()?)
        }
        _ => Err("The draft review does not match its forge".into()),
    }
}

/// `gh` run in `root`, with a JSON body written to a temporary file.
pub fn run_gh_with_input(
    root: &std::path::Path,
    args: &[&str],
    input: Option<&str>,
) -> Result<String, String> {
    match input {
        None => crate::inbox::run_gh(root, args, true),
        Some(body) => {
            let file = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
            std::fs::write(file.path(), body).map_err(|e| e.to_string())?;
            let path = file.path().to_string_lossy().into_owned();
            let mut all: Vec<&str> = args.to_vec();
            all.extend(["--input", &path]);
            crate::inbox::run_gh(root, &all, true)
        }
    }
}

#[cfg(test)]
mod tests;
