//! Ask: a side conversation about one inbox item. The session is an ordinary
//! one that also carries this context; only the agent sees the wrapped prompt.
use super::Item;
use serde::{Deserialize, Serialize};

const INSTRUCTIONS: &str = include_str!("ask_instructions.md");

/// Saved with the session, and sent to the agent as reference data. The field
/// order is what `JSON.stringify` writes for the Tauri app's InboxAskContext.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Context {
    pub key: String,
    pub title: String,
    pub url: String,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Context {
    pub fn new(item: &Item, description: Option<String>) -> Self {
        Self {
            key: key(item),
            title: item.title.clone(),
            url: item.url.clone(),
            provider: item.provider.clone(),
            description: description.filter(|text| !text.is_empty()),
        }
    }
    pub fn session_title(&self) -> String {
        format!("Ask · {}", self.title)
    }
}

/// One conversation per item, however the provider spells its link.
pub fn key(item: &Item) -> String {
    if matches!(item.provider.as_str(), "linear" | "jira") {
        return format!("{}:{}", item.provider, item.id);
    }
    // An item without a usable link must still have a stable key.
    match url::Url::parse(item.url.trim()) {
        Ok(url) => {
            let host = url.host_str().unwrap_or_default();
            let port = url.port().map(|port| format!(":{port}"));
            let path = url.path();
            let path = path.strip_suffix('/').unwrap_or(path);
            format!(
                "{}:{}{}:{}",
                item.provider,
                host.to_lowercase(),
                port.unwrap_or_default(),
                path.to_lowercase()
            )
        }
        Err(_) => {
            let repo = item.repo.trim().to_lowercase();
            let repo = if repo.is_empty() { "unknown" } else { &repo };
            format!("{}:{repo}:{}:{}", item.provider, item.kind, item.number)
        }
    }
}

/// Providers the agent cannot read with a local CLI get the description with
/// the context. GitHub items are read through `gh`.
pub fn wants_description(item: &Item) -> bool {
    match item.provider.as_str() {
        "linear" | "jira" => true,
        "gitlab" | "azuredevops" => matches!(item.kind.as_str(), "issue" | "pr"),
        _ => false,
    }
}

/// The agent's copy of a message. The transcript keeps what the user typed.
pub fn prompt(context: &Context, text: &str) -> String {
    let context = serde_json::to_string(context).unwrap_or_else(|_| "{}".into());
    format!(
        "{}\n\nINBOX ITEM (reference data):\n{context}\n\nUSER MESSAGE:\n{text}",
        INSTRUCTIONS.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(provider: &str, id: &str, url: &str) -> Item {
        Item {
            provider: provider.into(),
            id: id.into(),
            url: url.into(),
            repo: " Acme/Widgets ".into(),
            kind: "pr".into(),
            number: 7,
            title: "Fix the flaky retry".into(),
            ..Default::default()
        }
    }

    #[test]
    fn trackers_key_by_their_issue_id_whatever_the_link() {
        let linear = item("linear", "lin-uuid", "https://linear.app/acme/issue/ENG-1");
        assert_eq!(key(&linear), "linear:lin-uuid");
        let jira = item("jira", "10042", "");
        assert_eq!(key(&jira), "jira:10042");
    }

    #[test]
    fn hosted_providers_key_by_their_link_ignoring_case_and_a_trailing_slash() {
        let github = item("github", "1", "https://GitHub.com/Acme/Widgets/pull/7/");
        assert_eq!(key(&github), "github:github.com:/acme/widgets/pull/7");
        assert_eq!(
            key(&github),
            key(&item(
                "github",
                "2",
                "https://github.com/acme/widgets/pull/7"
            )),
        );
        assert_eq!(
            key(&item(
                "gitlab",
                "1",
                "https://git.acme.dev:8443/acme/widgets/-/merge_requests/7"
            )),
            "gitlab:git.acme.dev:8443:/acme/widgets/-/merge_requests/7",
        );
        assert_eq!(
            key(&item(
                "azuredevops",
                "1",
                "https://dev.azure.com/acme/widgets/_workitems/edit/7"
            )),
            "azuredevops:dev.azure.com:/acme/widgets/_workitems/edit/7",
        );
    }

    #[test]
    fn an_item_without_a_usable_link_falls_back_to_its_repository_and_number() {
        for url in ["", "not a url"] {
            assert_eq!(
                key(&item("github", "1", url)),
                "github:acme/widgets:pr:7",
                "{url:?}"
            );
        }
        let mut orphan = item("gitlab", "1", "");
        orphan.repo = "  ".into();
        assert_eq!(key(&orphan), "gitlab:unknown:pr:7");
    }

    #[test]
    fn different_items_get_different_keys() {
        let first = item("github", "1", "https://github.com/acme/widgets/pull/7");
        let second = item("github", "1", "https://github.com/acme/widgets/pull/8");
        let other_provider = item("gitlab", "1", "https://github.com/acme/widgets/pull/7");
        assert_ne!(key(&first), key(&second));
        assert_ne!(key(&first), key(&other_provider));
    }

    #[test]
    fn the_prompt_wraps_the_message_in_the_instructions_and_the_item() {
        let context = Context::new(
            &item("linear", "lin-1", "https://linear.app/acme/issue/ENG-1"),
            Some("Retry \"forever\"".into()),
        );
        let wrapped = prompt(&context, "What should I check first?");
        let expected = format!(
            "{}\n\nINBOX ITEM (reference data):\n{}\n\nUSER MESSAGE:\nWhat should I check first?",
            include_str!("ask_instructions.md").trim(),
            r#"{"key":"linear:lin-1","title":"Fix the flaky retry","url":"https://linear.app/acme/issue/ENG-1","provider":"linear","description":"Retry \"forever\""}"#,
        );
        assert_eq!(wrapped, expected);
        assert!(wrapped.starts_with("# Inbox discussion instructions"));
    }

    #[test]
    fn a_missing_or_empty_description_is_left_out_of_the_context() {
        for description in [None, Some(String::new())] {
            let context = Context::new(&item("github", "1", ""), description);
            assert!(!prompt(&context, "hi").contains("description"));
        }
    }

    #[test]
    fn the_session_is_titled_after_the_item() {
        let context = Context::new(&item("github", "1", ""), None);
        assert_eq!(context.session_title(), "Ask · Fix the flaky retry");
    }

    #[test]
    fn only_providers_without_a_local_cli_carry_the_description() {
        assert!(wants_description(&item("linear", "1", "")));
        assert!(wants_description(&item("jira", "1", "")));
        assert!(wants_description(&item("gitlab", "1", "")));
        assert!(wants_description(&item("azuredevops", "1", "")));
        assert!(!wants_description(&item("github", "1", "")));
    }
}
