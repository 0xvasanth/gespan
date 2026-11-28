//! What Gespan does with pull requests, the same on every host. The service
//! checks what it's asked before any request goes out (an empty comment, a
//! range the host can't take, a verdict it doesn't have) and puts what comes
//! back in one order, whichever host answered.
use super::{
    error::{HostError, Result},
    host::PullRequestHost,
    model::*,
};
use std::collections::{HashMap, HashSet};

pub struct PullRequestService {
    host: Box<dyn PullRequestHost>,
}

/// Oldest first; times a host wrote differently still compare.
fn by_time<T>(items: &mut [T], at: impl Fn(&T) -> &str) {
    items.sort_by_key(|item| crate::inbox::timestamp_ms(at(item)));
}

fn required(body: &str, what: &str) -> Result<String> {
    let body = body.trim();
    if body.is_empty() {
        return Err(HostError::Invalid(format!("{what} is empty")));
    }
    Ok(body.to_owned())
}

fn required_ref(pr: &PrRef) -> Result<()> {
    if pr.repo.trim().is_empty() || pr.number == 0 {
        return Err(HostError::Invalid("no pull request given".into()));
    }
    Ok(())
}

impl PullRequestService {
    pub fn new(host: Box<dyn PullRequestHost>) -> Self {
        Self { host }
    }
    pub fn kind(&self) -> HostKind {
        self.host.kind()
    }
    pub fn capabilities(&self) -> Capabilities {
        self.host.capabilities()
    }
    pub fn current_user(&self) -> Result<String> {
        self.host.current_user()
    }

    pub fn repositories(&self) -> Result<Vec<Repository>> {
        self.host.repositories()
    }

    /// A repository's pull requests, most recently updated first.
    pub fn pull_requests(&self, repo: &str, filter: StateFilter) -> Result<Vec<PullRequest>> {
        if repo.trim().is_empty() {
            return Err(HostError::Invalid("no repository given".into()));
        }
        let mut prs = self.host.pull_requests(repo, filter)?;
        by_time(&mut prs, |pr| &pr.updated_at);
        prs.reverse();
        Ok(prs)
    }

    pub fn pull_request(&self, pr: &PrRef) -> Result<PullRequest> {
        required_ref(pr)?;
        self.host.pull_request(pr)
    }

    /// Open pull requests waiting on your review, most recently updated
    /// first, each once.
    pub fn review_requests(&self) -> Result<Vec<PullRequest>> {
        let mut seen = HashSet::new();
        let mut prs: Vec<PullRequest> = self
            .host
            .review_requests()?
            .into_iter()
            .filter(|pr| pr.state == PrState::Open)
            .filter(|pr| seen.insert(pr.pr_ref()))
            .collect();
        by_time(&mut prs, |pr| &pr.updated_at);
        prs.reverse();
        Ok(prs)
    }

    /// The pull request's conversation between people: threads in the order
    /// they started, each one's comments oldest first. Notes the host wrote
    /// itself ("added 2 commits") are left out, and threads left empty.
    pub fn conversation(&self, pr: &PrRef) -> Result<Vec<Thread>> {
        required_ref(pr)?;
        let mut threads: Vec<Thread> = self
            .host
            .threads(pr)?
            .into_iter()
            .map(|mut thread| {
                thread.comments.retain(|c| !c.system);
                by_time(&mut thread.comments, |c| &c.created_at);
                thread
            })
            .filter(|thread| !thread.comments.is_empty())
            .collect();
        by_time(&mut threads, |t| &t.comments[0].created_at);
        Ok(threads)
    }

    /// Threads on lines of `path`.
    pub fn file_threads(&self, pr: &PrRef, path: &str) -> Result<Vec<Thread>> {
        Ok(self
            .conversation(pr)?
            .into_iter()
            .filter(|t| t.location.as_ref().is_some_and(|l| l.path == path))
            .collect())
    }

    pub fn comment(&self, pr: &PrRef, body: &str) -> Result<Comment> {
        required_ref(pr)?;
        let body = required(body, "The comment")?;
        self.host.add_comment(pr, &body)
    }

    pub fn reply(&self, pr: &PrRef, thread_id: &str, body: &str) -> Result<Comment> {
        required_ref(pr)?;
        if thread_id.trim().is_empty() {
            return Err(HostError::Invalid("no thread given".into()));
        }
        let body = required(body, "The reply")?;
        self.host.reply(pr, thread_id, &body)
    }

    pub fn edit_comment(&self, pr: &PrRef, comment_id: &str, body: &str) -> Result<Comment> {
        required_ref(pr)?;
        if !self.capabilities().edit_comments {
            return Err(HostError::Unsupported("editing comments"));
        }
        if comment_id.trim().is_empty() {
            return Err(HostError::Invalid("no comment given".into()));
        }
        let body = required(body, "The comment")?;
        self.host.edit_comment(pr, comment_id, &body)
    }

    pub fn resolve_thread(&self, pr: &PrRef, thread_id: &str, resolved: bool) -> Result<()> {
        required_ref(pr)?;
        if !self.capabilities().resolve_threads {
            return Err(HostError::Unsupported("resolving threads"));
        }
        self.host.resolve_thread(pr, thread_id, resolved)
    }

    /// The pull request's commits, oldest first, whichever order the host
    /// lists them in.
    pub fn commits(&self, pr: &PrRef) -> Result<Vec<Commit>> {
        required_ref(pr)?;
        let mut commits = self.host.commits(pr)?;
        by_time(&mut commits, |c| &c.authored_at);
        Ok(commits)
    }

    pub fn files(&self, pr: &PrRef) -> Result<Vec<ChangedFile>> {
        required_ref(pr)?;
        self.host.files(pr)
    }

    pub fn reviews(&self, pr: &PrRef) -> Result<Vec<Review>> {
        required_ref(pr)?;
        self.host.reviews(pr)
    }

    /// Where each reviewer stands: their latest approval or request for
    /// changes. A comment after it doesn't change it; a dismissal clears it.
    pub fn verdicts(&self, pr: &PrRef) -> Result<Vec<Review>> {
        let mut reviews = self.reviews(pr)?;
        reviews
            .sort_by_key(|r| crate::inbox::timestamp_ms(r.submitted_at.as_deref().unwrap_or("")));
        let mut latest: HashMap<String, Review> = HashMap::new();
        let mut order = Vec::new();
        for review in reviews {
            match review.state {
                ReviewState::Commented => continue,
                ReviewState::Dismissed => {
                    latest.remove(&review.author);
                }
                _ => {
                    if !latest.contains_key(&review.author) {
                        order.push(review.author.clone());
                    }
                    latest.insert(review.author.clone(), review);
                }
            }
        }
        Ok(order
            .into_iter()
            .filter_map(|a| latest.remove(&a))
            .collect())
    }

    /// Sends a review once it holds up: every line comment has words and a
    /// sensible place, the host can take its ranges and its verdict, and a
    /// comment-only review says something.
    pub fn submit_review(&self, pr: &PrRef, review: &ReviewSubmission) -> Result<()> {
        required_ref(pr)?;
        let capabilities = self.capabilities();
        if review.verdict == Verdict::RequestChanges && !capabilities.request_changes {
            return Err(HostError::Unsupported("requesting changes"));
        }
        if review.verdict != Verdict::Approve
            && review.body.trim().is_empty()
            && review.comments.is_empty()
        {
            return Err(HostError::Invalid(
                "A review needs a summary or a line comment".into(),
            ));
        }
        for draft in &review.comments {
            let location = &draft.location;
            if draft.body.trim().is_empty() {
                return Err(HostError::Invalid(format!(
                    "The comment on {} line {} is empty",
                    location.path, location.line
                )));
            }
            if location.path.trim().is_empty() || location.line == 0 {
                return Err(HostError::Invalid("A line comment has no line".into()));
            }
            if let Some(start) = location.start_line {
                if start == 0 || start > location.line {
                    return Err(HostError::Invalid(format!(
                        "Lines {start} to {} of {} are out of order",
                        location.line, location.path
                    )));
                }
            }
            if location.is_range() && !capabilities.line_ranges {
                return Err(HostError::Unsupported("comments on a range of lines"));
            }
        }
        self.host.submit_review(pr, review)
    }

    pub fn checks(&self, pr: &PrRef) -> Result<Vec<Check>> {
        required_ref(pr)?;
        self.host.checks(pr)
    }

    /// How the checks stand, counted.
    pub fn check_summary(&self, pr: &PrRef) -> Result<CheckSummary> {
        let mut summary = CheckSummary::default();
        for check in self.checks(pr)? {
            match check.status {
                CheckStatus::Success => summary.passed += 1,
                CheckStatus::Failure | CheckStatus::Cancelled => summary.failed += 1,
                CheckStatus::Pending => summary.pending += 1,
                CheckStatus::Skipped => summary.skipped += 1,
            }
        }
        Ok(summary)
    }
}
