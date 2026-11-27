//! Read-only GitHub polling for linked session notifications. No GitHub calls
//! occur until the shell invokes poll; acknowledgment persistence stays in shell.
use gespan_core::{
    cli::Finished,
    session_groups::{GithubWorkItem, WorkItem},
    store::{Session, Store},
};
use gpui::{Context, EventEmitter};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::Path,
    time::{Duration, Instant},
};
const SEEN_KEY: &str = "linked-work-item-seen";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub id: String,
    pub kind: String,
    pub author: String,
    pub text: String,
    pub created_at: String,
    pub url: String,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Card {
    pub session: i64,
    pub item: GithubWorkItem,
    pub title: String,
    pub state: String,
    pub updated_at: i64,
    pub entries: Vec<Entry>,
    pub details_error: bool,
    pub truncated: bool,
}
impl Card {
    pub(crate) fn summary(&self) -> String {
        let count = |kind: &str| {
            self.entries
                .iter()
                .filter(|e| {
                    if kind == "comment" {
                        e.kind == "comment" || e.kind == "review_comment"
                    } else {
                        e.kind == kind
                    }
                })
                .count()
        };
        let parts = [
            (count("commit"), "commit"),
            (count("review"), "review"),
            (count("comment"), "comment"),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, k)| format!("{n} new {k}{}", if n == 1 { "" } else { "s" }))
        .collect::<Vec<_>>();
        if !parts.is_empty() {
            parts.join(" · ")
        } else if self.details_error {
            "Updated on GitHub · details unavailable".into()
        } else {
            "Metadata or status changed".into()
        }
    }
    pub(crate) fn terminal_label(&self) -> Option<&'static str> {
        match (
            self.item.kind.as_str(),
            self.state.to_ascii_lowercase().as_str(),
        ) {
            ("pr", "merged") => Some("Pull request merged"),
            ("pr", "closed") => Some("Pull request closed"),
            ("issue", "closed") => Some("Issue closed"),
            _ => None,
        }
    }
    pub(crate) fn prompt(&self) -> String {
        let first = self.entries.first();
        let instruction = match first.map(|e| e.kind.as_str()) {
            Some("commit") => "Review the new commit and continue the work where needed.",
            Some(_) => "Address the new feedback and continue the work where needed.",
            None => "Review the latest update and continue the work where needed.",
        };
        let details = if self.entries.is_empty() {
            vec![format!(
                "- {}; current state: {}",
                self.summary(),
                self.state
            )]
        } else {
            self.entries
                .iter()
                .take(12)
                .map(|e| {
                    format!(
                        "- {}{}: {}",
                        e.kind.replace('_', " "),
                        if e.author.is_empty() {
                            String::new()
                        } else {
                            format!(" by @{}", e.author)
                        },
                        if e.text.is_empty() {
                            "No message"
                        } else {
                            &e.text
                        }
                    )
                })
                .collect()
        };
        format!(
            "{instruction}\n\nThe linked GitHub {} has new activity:\n\n#{} {}\n{}\n\n{}",
            if self.item.kind == "pr" {
                "pull request"
            } else {
                "issue"
            },
            self.item.number,
            self.title,
            self.item.url,
            details.join("\n")
        )
    }
}
pub(crate) fn load_seen(store: &Store) -> Result<HashMap<i64, i64>, String> {
    Ok(store
        .setting(SEEN_KEY)?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default())
}
/// Merge the desktop's acknowledged UUID snapshots with the native cache.
/// Resolve source IDs through Store rather than treating UUIDs as native IDs.
pub(crate) fn load_seen_with_metadata(
    store: &Store,
    metadata: Option<&Value>,
) -> Result<HashMap<i64, i64>, String> {
    let mut seen = load_seen(store)?;
    let raw = metadata.and_then(|m| m.get("gespan.linkedSessionSeen"));
    let decoded = raw
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let values = decoded.as_ref().or(raw).and_then(Value::as_object);
    if let Some(values) = values {
        for (source_id, timestamp) in values {
            if let (Some(id), Some(at)) = (
                store.resolve_existing_session_id(source_id),
                timestamp.as_i64(),
            ) {
                let current = seen.entry(id).or_default();
                *current = (*current).max(at);
            }
        }
    }
    Ok(seen)
}
pub(crate) fn save_seen(store: &Store, seen: &HashMap<i64, i64>) -> Result<(), String> {
    store.save_setting(
        SEEN_KEY,
        &serde_json::to_string(seen).map_err(|e| e.to_string())?,
    )
}
pub(crate) enum GithubActivityEvent {
    Changed,
    Acknowledged(HashMap<i64, i64>),
}
pub(crate) struct GithubActivity {
    pub cards: HashMap<i64, Card>,
    seen: HashMap<i64, i64>,
    checked: Option<Instant>,
    checking: bool,
    /// Detail feeds by URL with the item `updated_at` they were fetched for:
    /// an unchanged item reuses them instead of calling gh every poll.
    details: Details,
    pace: Pace,
}
/// Detail feeds by URL. A feed that failed for good is kept as `Null`, so it
/// isn't asked for again until its item changes.
type Details = HashMap<String, (i64, Value)>;
/// How often to ask GitHub: every minute, and after a failed poll (GitHub
/// timing out, a rate limit, no network) wait 2, 4, 8 and then 15 minutes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Pace {
    failures: u32,
    retry_at: Option<Instant>,
}
const POLL_EVERY: Duration = Duration::from_secs(60);
const BACKOFF_MINUTES: [u64; 4] = [2, 4, 8, 15];
impl Pace {
    fn ready(&self, now: Instant) -> bool {
        self.retry_at.is_none_or(|at| now >= at)
    }
    fn record(&mut self, failed: bool, now: Instant) {
        if failed {
            self.failures += 1;
            let ix = (self.failures as usize - 1).min(BACKOFF_MINUTES.len() - 1);
            self.retry_at = Some(now + Duration::from_secs(BACKOFF_MINUTES[ix] * 60));
        } else {
            *self = Self::default();
        }
    }
}
impl EventEmitter<GithubActivityEvent> for GithubActivity {}
impl GithubActivity {
    pub(crate) fn new(seen: HashMap<i64, i64>) -> Self {
        Self {
            cards: HashMap::new(),
            seen,
            checked: None,
            checking: false,
            details: HashMap::new(),
            pace: Pace::default(),
        }
    }
    pub(crate) fn restore_seen(&mut self, seen: HashMap<i64, i64>) {
        for (id, at) in seen {
            let current = self.seen.entry(id).or_default();
            *current = (*current).max(at);
        }
        self.cards
            .retain(|id, card| card.updated_at > *self.seen.get(id).unwrap_or(&0));
    }
    pub(crate) fn acknowledge(&mut self, session: i64, cx: &mut Context<Self>) {
        if let Some(card) = self.cards.remove(&session) {
            let seen = self.seen.entry(session).or_default();
            *seen = (*seen).max(card.updated_at);
            if self.seen.len() > 500 {
                let mut entries: Vec<_> = self.seen.iter().map(|(id, at)| (*id, *at)).collect();
                entries.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
                self.seen = entries.into_iter().take(500).collect();
            }
            cx.emit(GithubActivityEvent::Acknowledged(self.seen.clone()));
            cx.emit(GithubActivityEvent::Changed);
            cx.notify();
        }
    }
    pub(crate) fn ready_to_poll(&self) -> bool {
        let now = Instant::now();
        !self.checking
            && self.pace.ready(now)
            && self.checked.is_none_or(|at| now - at >= POLL_EVERY)
    }
    pub(crate) fn poll(&mut self, sessions: Vec<Session>, cx: &mut Context<Self>) {
        if !self.ready_to_poll() {
            return;
        }
        self.checking = true;
        self.checked = Some(Instant::now());
        let seen = self.seen.clone();
        let details = std::mem::take(&mut self.details);
        let work = cx
            .background_executor()
            .spawn(async move { fetch(sessions, &seen, details, &gh) });
        cx.spawn(async move |this, cx| {
            let fetched = work.await;
            let _ = this.update(cx, |this, cx| {
                this.checking = false;
                this.details = fetched.details;
                this.pace.record(fetched.failed, Instant::now());
                // A failed poll keeps the cards from the last one.
                let Some(cards) = fetched.cards else {
                    return;
                };
                // An acknowledgment made while polling must not reappear.
                this.cards = cards
                    .into_iter()
                    .filter(|(id, c)| c.updated_at > *this.seen.get(id).unwrap_or(&0))
                    .collect();
                cx.emit(GithubActivityEvent::Changed);
                cx.notify();
            });
        })
        .detach();
    }
}
fn timestamp(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|d| d.timestamp_millis())
        .unwrap_or(0)
}
/// Runs gh with these arguments. Tests pass a fake.
type Gh<'a> = &'a dyn Fn(&[String]) -> Result<Finished, String>;
fn gh(args: &[String]) -> Result<Finished, String> {
    let binary = gespan_core::scm::resolve_gui_binary("gh").ok_or("GitHub CLI not found")?;
    gespan_core::cli::run(&binary, args, Path::new("/"), Duration::from_secs(20))
}
const DETAIL_JQ: &str = r#"if type == "array" then map({id,sha,user:{login:.user.login},author:{login:.author.login},body:((.body // "")[0:256]),state,submitted_at,created_at,html_url,commit:{message:((.commit.message // "")[0:256]),committer:{date:.commit.committer.date}}}) else {title,state,merged,updated_at} end"#;
fn api(gh: Gh, endpoint: &str) -> Result<Value, String> {
    let args = ["api", "--method", "GET", endpoint, "--jq", DETAIL_JQ].map(String::from);
    let finished = gh(&args)?;
    if !finished.success {
        return Err(format!("{} {}", finished.stderr, finished.stdout)
            .trim()
            .to_owned());
    }
    serde_json::from_str(&finished.stdout).map_err(|e| e.to_string())
}
/// Only canonical GitHub repository slugs enter a gh API endpoint or query.
fn canonical(item: &WorkItem) -> bool {
    let Some(item) = item.github() else {
        return false;
    };
    let parts = item.repo.split('/').collect::<Vec<_>>();
    item.number != 0
        && parts.len() == 2
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        })
}
fn summary_query(chunk: &[(String, u64)]) -> String {
    let mut query = String::from("query{");
    for (ix, (repo, number)) in chunk.iter().enumerate() {
        let Some((owner, name)) = repo.split_once('/') else {
            continue;
        };
        query.push_str(&format!(
            "i{ix}:repository(owner:\"{owner}\",name:\"{name}\"){{issueOrPullRequest(number:{number}){{...on PullRequest{{title state merged updatedAt}}...on Issue{{title state updatedAt}}}}}}"
        ));
    }
    query.push('}');
    query
}
/// Every linked item's summary, 50 to a GraphQL request, shaped like the REST
/// summary. An item GitHub can't find (deleted, moved, or private to you) is
/// left out and the rest are kept: gh exits with an error for such an answer
/// but still prints its `data`. Errors only when an answer has no `data` at
/// all, such as a rate limit or no network, and then nothing falls back to
/// asking item by item, which once spent the hour's REST budget in minutes.
fn summaries(gh: Gh, items: &[(String, u64)]) -> Result<HashMap<(String, u64), Value>, String> {
    let mut out = HashMap::new();
    for chunk in items.chunks(50) {
        let args = [
            "api".to_owned(),
            "graphql".to_owned(),
            "-f".to_owned(),
            format!("query={}", summary_query(chunk)),
        ];
        let finished = gh(&args)?;
        let data = serde_json::from_str::<Value>(&finished.stdout)
            .ok()
            .and_then(|body| body.get("data").filter(|d| d.is_object()).cloned());
        let Some(data) = data else {
            let error = if finished.stderr.is_empty() {
                finished.stdout
            } else {
                finished.stderr
            };
            return Err(if error.is_empty() {
                "GitHub gave no answer".to_owned()
            } else {
                error
            });
        };
        for (ix, item) in chunk.iter().enumerate() {
            let node = &data[format!("i{ix}")]["issueOrPullRequest"];
            if !node.is_object() {
                continue;
            }
            // REST reports a merged pull request as closed plus `merged`.
            let state = match node["state"].as_str().unwrap_or("") {
                "MERGED" => "closed".to_owned(),
                state => state.to_ascii_lowercase(),
            };
            out.insert(
                item.clone(),
                serde_json::json!({
                    "title": node["title"],
                    "state": state,
                    "merged": node["merged"].as_bool().unwrap_or(false),
                    "updated_at": node["updatedAt"],
                }),
            );
        }
    }
    Ok(out)
}
/// One poll's result. `cards` is None when the summaries couldn't be read,
/// so the last poll's cards stay; `failed` asks the next poll to wait.
struct Fetched {
    cards: Option<HashMap<i64, Card>>,
    details: Details,
    failed: bool,
}
fn fetch(sessions: Vec<Session>, seen: &HashMap<i64, i64>, previous: Details, gh: Gh) -> Fetched {
    let mut cards = HashMap::new();
    let mut details = Details::new();
    let sessions: Vec<_> = sessions
        .into_iter()
        .filter(|s| !s.archived && s.work_item.as_ref().is_some_and(canonical))
        .collect();
    let mut items: Vec<(String, u64)> = sessions
        .iter()
        .filter_map(|s| s.work_item.as_ref().and_then(|w| w.github()))
        .map(|item| (item.repo.clone(), item.number))
        .collect();
    items.sort();
    items.dedup();
    if items.is_empty() {
        return Fetched {
            cards: Some(cards),
            details,
            failed: false,
        };
    }
    let summaries = match summaries(gh, &items) {
        Ok(summaries) => summaries,
        Err(_) => {
            return Fetched {
                cards: None,
                details: previous,
                failed: true,
            };
        }
    };
    // Set once GitHub times out or rate-limits a detail feed: the rest of
    // this poll asks for nothing more.
    let mut throttled = false;
    for session in sessions {
        let Some(item) = session.work_item.as_ref().and_then(|w| w.github()).cloned() else {
            continue;
        };
        let base = format!(
            "repos/{}/{}s/{}",
            item.repo,
            if item.kind == "pr" { "pull" } else { "issue" },
            item.number
        );
        let Some(body) = summaries.get(&(item.repo.clone(), item.number)) else {
            continue;
        };
        let updated_at = timestamp(body["updated_at"].as_str().unwrap_or(""));
        let since = (session.updated_at.min(i64::MAX as u64) as i64)
            .max(*seen.get(&session.id).unwrap_or(&0));
        if updated_at <= since {
            continue;
        }
        let mut card = Card {
            session: session.id,
            title: body["title"].as_str().unwrap_or("").into(),
            state: if body["merged"].as_bool() == Some(true) {
                "merged".into()
            } else {
                body["state"].as_str().unwrap_or("").into()
            },
            updated_at,
            item,
            entries: Vec::new(),
            details_error: false,
            truncated: false,
        };
        let issue_base = format!("repos/{}/issues/{}", card.item.repo, card.item.number);
        let mut feeds = vec![(format!("{issue_base}/comments?per_page=100"), "comment")];
        if card.item.kind == "pr" {
            feeds.extend([
                (format!("{base}/reviews?per_page=100"), "review"),
                (format!("{base}/comments?per_page=100"), "review_comment"),
                (format!("{base}/commits?per_page=100"), "commit"),
            ]);
        }
        for (url, kind) in feeds {
            let body = match previous.get(&url).or_else(|| details.get(&url)) {
                Some((at, body)) if *at == updated_at => body.clone(),
                _ if throttled => {
                    card.details_error = true;
                    continue;
                }
                _ => match api(gh, &url) {
                    Ok(body) => body,
                    Err(error) if gespan_core::inbox::is_throttled(&error) => {
                        throttled = true;
                        card.details_error = true;
                        continue;
                    }
                    // Gone or not allowed: remembered, so it isn't asked
                    // for again until the item changes.
                    Err(_) => Value::Null,
                },
            };
            details.insert(url, (updated_at, body.clone()));
            let Some(entries) = body.as_array() else {
                card.details_error = true;
                continue;
            };
            card.truncated |= entries.len() == 100;
            for entry in entries {
                let commit = kind == "commit";
                let at = if commit {
                    entry["commit"]["committer"]["date"].as_str()
                } else if kind == "review" {
                    entry["submitted_at"].as_str()
                } else {
                    entry["created_at"].as_str()
                }
                .unwrap_or("");
                if timestamp(at) <= since {
                    continue;
                }
                let text = if commit {
                    entry["commit"]["message"]
                        .as_str()
                        .unwrap_or("")
                        .lines()
                        .next()
                        .unwrap_or("")
                        .into()
                } else {
                    let body = entry["body"].as_str().unwrap_or("");
                    if kind == "review" {
                        format!(
                            "{}: {body}",
                            entry["state"].as_str().unwrap_or("").replace('_', " ")
                        )
                    } else {
                        body.to_owned()
                    }
                };
                let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                let text = if text.chars().count() > 140 {
                    format!("{}…", text.chars().take(139).collect::<String>())
                } else {
                    text
                };
                card.entries.push(Entry {
                    id: if commit {
                        entry["sha"].as_str().unwrap_or("").into()
                    } else {
                        entry["id"].to_string()
                    },
                    kind: kind.into(),
                    author: entry["user"]["login"]
                        .as_str()
                        .or_else(|| entry["author"]["login"].as_str())
                        .unwrap_or("")
                        .into(),
                    text,
                    created_at: at.into(),
                    url: entry["html_url"].as_str().unwrap_or("").into(),
                });
            }
        }
        card.entries
            .sort_by_key(|e| std::cmp::Reverse(timestamp(&e.created_at)));
        cards.insert(card.session, card);
    }
    Fetched {
        cards: Some(cards),
        details,
        failed: throttled,
    }
}
#[cfg(test)]
#[path = "github_activity_tests.rs"]
mod tests;
