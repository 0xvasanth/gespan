use url::Url;

use super::Tracker;

/// What the user pasted. `tracker` is `None` for a bare key, which could belong
/// to any tracker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedTicket {
    pub tracker: Option<Tracker>,
    pub key: String,
    /// Origin of a Jira or Plane URL, needed to find the instance again.
    pub site: Option<String>,
    /// Workspace slug of a Plane URL.
    pub workspace: Option<String>,
}

/// Accepts a bare key (`abc-12`) or a ticket URL from Jira, Linear or Plane.
pub fn parse_ticket(input: &str) -> Option<ParsedTicket> {
    let input = input.trim();
    if let Some(key) = normalize_key(input) {
        return Some(ParsedTicket {
            tracker: None,
            key,
            site: None,
            workspace: None,
        });
    }
    parse_url(&Url::parse(input).ok()?)
}

fn normalize_key(candidate: &str) -> Option<String> {
    let (project, number) = candidate.split_once('-')?;
    let mut chars = project.chars();
    let valid_project = chars.next()?.is_ascii_alphabetic()
        && project.len() <= 16
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    let valid_number =
        (1..=9).contains(&number.len()) && number.chars().all(|c| c.is_ascii_digit());
    (valid_project && valid_number).then(|| candidate.to_ascii_uppercase())
}

fn parse_url(url: &Url) -> Option<ParsedTicket> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?;
    let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let site = url.origin().ascii_serialization();
    let found = |tracker, key, site| {
        Some(ParsedTicket {
            tracker: Some(tracker),
            key,
            site,
            workspace: None,
        })
    };

    if host == "linear.app" {
        let at = segments.iter().position(|s| *s == "issue")?;
        return found(Tracker::Linear, normalize_key(segments.get(at + 1)?)?, None);
    }

    let selected_issue = url
        .query_pairs()
        .find(|(name, _)| name == "selectedIssue")
        .and_then(|(_, value)| normalize_key(&value));
    let jira = |key| found(Tracker::Jira, key, Some(site.clone()));

    if host.ends_with(".atlassian.net") {
        if let Some(key) = selected_issue {
            return jira(key);
        }
        let at = segments.iter().position(|s| *s == "browse")?;
        return jira(normalize_key(segments.get(at + 1)?)?);
    }

    // A browse path on any other host is told apart by depth: Jira Data Center
    // serves `/browse/KEY`, Plane nests it under a workspace slug.
    match segments.as_slice() {
        ["browse", key, ..] => jira(normalize_key(key)?),
        [workspace, "browse", key, ..] => Some(ParsedTicket {
            tracker: Some(Tracker::Plane),
            key: normalize_key(key)?,
            site: Some(site.clone()),
            workspace: Some(workspace.to_string()),
        }),
        _ => jira(selected_issue?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_keys() {
        assert_eq!(
            parse_ticket(" abc-12 ").unwrap(),
            ParsedTicket {
                tracker: None,
                key: "ABC-12".into(),
                site: None,
                workspace: None
            }
        );
        assert!(parse_ticket("12").is_none());
        assert!(parse_ticket("ABC-").is_none());
        assert!(parse_ticket("-12").is_none());
    }

    #[test]
    fn bare_key_grammar_boundaries() {
        assert!(parse_ticket("A_1-1").is_some());
        assert!(parse_ticket("1AB-1").is_none());
        assert!(parse_ticket("ABC-1234567890").is_none());
        assert!(parse_ticket("ABC-123456789").is_some());
        assert!(parse_ticket("ABCDEFGHIJKLMNOPQ-1").is_none());
        assert!(parse_ticket("ABCDEFGHIJKLMNOP-1").is_some());
        assert!(parse_ticket("ABC-1x").is_none());
        assert!(parse_ticket("").is_none());
    }

    #[test]
    fn jira_urls() {
        let p =
            parse_ticket("https://acme.atlassian.net/browse/ABC-12?focusedCommentId=9").unwrap();
        assert_eq!(
            (p.tracker, p.key.as_str(), p.site.as_deref()),
            (
                Some(Tracker::Jira),
                "ABC-12",
                Some("https://acme.atlassian.net")
            )
        );
        assert_eq!(
            parse_ticket(
                "https://acme.atlassian.net/jira/software/projects/ABC/boards/1?selectedIssue=ABC-7"
            )
            .unwrap()
            .key,
            "ABC-7"
        );
    }

    #[test]
    fn self_hosted_jira_browse_path() {
        let p = parse_ticket("https://jira.example.com/browse/ABC-9").unwrap();
        assert_eq!(
            (p.tracker, p.key.as_str(), p.site.as_deref()),
            (
                Some(Tracker::Jira),
                "ABC-9",
                Some("https://jira.example.com")
            )
        );
    }

    #[test]
    fn selected_issue_on_other_host_is_jira() {
        let p = parse_ticket("https://jira.example.com/secure/RapidBoard.jspa?selectedIssue=abc-4")
            .unwrap();
        assert_eq!((p.tracker, p.key.as_str()), (Some(Tracker::Jira), "ABC-4"));
    }

    #[test]
    fn linear_urls() {
        let p = parse_ticket("https://linear.app/acme/issue/ENG-42/fix-the-thing").unwrap();
        assert_eq!(
            (p.tracker, p.key.as_str(), p.site),
            (Some(Tracker::Linear), "ENG-42", None)
        );
        assert!(parse_ticket("https://linear.app/acme/project/foo").is_none());
    }

    #[test]
    fn plane_urls() {
        let p = parse_ticket("https://plane.example.com/acme/browse/PROJ-7/").unwrap();
        assert_eq!(
            (p.tracker, p.key.as_str(), p.site.as_deref()),
            (
                Some(Tracker::Plane),
                "PROJ-7",
                Some("https://plane.example.com")
            )
        );
        assert_eq!(p.workspace.as_deref(), Some("acme"));
        let cloud = parse_ticket("https://app.plane.so/acme/browse/WEB-3").unwrap();
        assert_eq!(cloud.tracker, Some(Tracker::Plane));
        assert_eq!(cloud.workspace.as_deref(), Some("acme"));
        let jira = parse_ticket("https://jira.example.com/browse/ABC-9").unwrap();
        assert_eq!(jira.workspace, None);
    }

    #[test]
    fn unknown_urls_are_rejected() {
        assert!(parse_ticket("https://example.com/ABC-12").is_none());
        assert!(parse_ticket("ftp://example.com/browse/ABC-12").is_none());
        assert!(parse_ticket("https://acme.atlassian.net/browse/nope").is_none());
        assert!(parse_ticket("not a url").is_none());
    }
}
