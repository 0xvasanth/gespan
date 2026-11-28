//! The pull request as Gespan sees it, whatever host it lives on. Each host
//! adapter maps its own JSON into these types; nothing here knows GitHub,
//! GitLab or Azure DevOps.
use serde::{Deserialize, Serialize};

/// Which code host a pull request lives on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostKind {
    #[serde(rename = "github")]
    GitHub,
    #[serde(rename = "gitlab")]
    GitLab,
    #[serde(rename = "azuredevops")]
    AzureDevOps,
}

impl HostKind {
    /// The id the rest of Gespan stores: "github", "gitlab", "azuredevops".
    pub fn id(self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::GitLab => "gitlab",
            Self::AzureDevOps => "azuredevops",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "github" => Some(Self::GitHub),
            "gitlab" => Some(Self::GitLab),
            "azuredevops" => Some(Self::AzureDevOps),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::GitHub => "GitHub",
            Self::GitLab => "GitLab",
            Self::AzureDevOps => "Azure DevOps",
        }
    }
}

/// What a host can do beyond the basics every host has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// A review comment can span several lines.
    pub line_ranges: bool,
    /// A review can request changes, not only comment or approve.
    pub request_changes: bool,
    /// Changed files come with their patch text.
    pub file_patches: bool,
    /// Comments can be edited after they're posted.
    pub edit_comments: bool,
    /// Review threads can be resolved.
    pub resolve_threads: bool,
}

/// One pull request, by repository and number.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PrRef {
    /// "owner/name" on GitHub, the full project path on GitLab
    /// ("group/sub/project"), "project/repository" on Azure DevOps.
    pub repo: String,
    pub number: u64,
}

impl PrRef {
    pub fn new(repo: impl Into<String>, number: u64) -> Self {
        Self {
            repo: repo.into(),
            number,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repository {
    pub full_name: String,
    pub url: String,
    pub default_branch: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

/// Which pull requests a list asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateFilter {
    #[default]
    Open,
    Closed,
    Merged,
    All,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub state: PrState,
    pub draft: bool,
    pub author: String,
    pub source_branch: String,
    pub target_branch: String,
    pub url: String,
    pub created_at: String,
    pub updated_at: String,
    /// The commit the pull request's head points at, when the host says.
    pub head_sha: Option<String>,
    /// Who is asked to review.
    pub reviewers: Vec<String>,
}

impl PullRequest {
    pub fn pr_ref(&self) -> PrRef {
        PrRef::new(self.repo.clone(), self.number)
    }
}

/// Which side of the diff a line is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// The line as it was, a deleted line.
    Old,
    /// The line as it is, an added or unchanged line.
    #[default]
    New,
}

/// Where a review comment sits: one line, or `start_line` to `line`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineLocation {
    pub path: String,
    pub side: Side,
    pub line: u32,
    pub start_line: Option<u32>,
}

impl LineLocation {
    pub fn line(path: impl Into<String>, side: Side, line: u32) -> Self {
        Self {
            path: path.into(),
            side,
            line,
            start_line: None,
        }
    }
    pub fn range(path: impl Into<String>, side: Side, start: u32, end: u32) -> Self {
        Self {
            path: path.into(),
            side,
            line: end,
            start_line: Some(start),
        }
    }
    pub fn is_range(&self) -> bool {
        self.start_line.is_some_and(|start| start != self.line)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    /// The host's id for this comment, as the host adapter writes it; pass
    /// it back unchanged to edit the comment.
    pub id: String,
    pub author: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
    /// Written by the host itself ("added 2 commits"), not a person.
    pub system: bool,
}

/// A conversation on the pull request: general comments (no location) or a
/// review thread on lines, with its replies in order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    /// The host's id for this thread; pass it back unchanged to reply.
    pub id: String,
    pub location: Option<LineLocation>,
    /// None where the host has no resolving.
    pub resolved: Option<bool>,
    pub comments: Vec<Comment>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    pub sha: String,
    /// The message's first line.
    pub title: String,
    pub message: String,
    pub author: String,
    pub authored_at: String,
    pub url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    pub path: String,
    pub previous_path: Option<String>,
    pub status: FileStatus,
    /// Lines added and removed, when the host counts them.
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    /// The unified diff of this file, when the host gives it.
    pub patch: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    pub author: String,
    pub state: ReviewState,
    pub body: String,
    pub submitted_at: Option<String>,
}

/// The verdict of a review being sent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    #[default]
    Comment,
    Approve,
    RequestChanges,
}

/// A comment on lines, part of a review being sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftComment {
    pub location: LineLocation,
    pub body: String,
}

/// A review to send: a verdict, an optional summary and line comments.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewSubmission {
    pub verdict: Verdict,
    pub body: String,
    pub comments: Vec<DraftComment>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pending,
    Success,
    Failure,
    Skipped,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub status: CheckStatus,
    pub url: Option<String>,
}

/// How a pull request's checks stand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckSummary {
    pub passed: usize,
    pub failed: usize,
    pub pending: usize,
    pub skipped: usize,
}
