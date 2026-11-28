//! Polish: a review comment rewritten in a chosen tone by a one-off agent turn
//! that runs in the background, with no tools and no session of its own. The
//! prompt names the pull request, and the agent answers inside `<polished>`
//! tags so the rewrite can be lifted out of whatever else it says.
use serde::{Deserialize, Serialize};

/// How a polished comment should read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    #[default]
    Direct,
    Friendly,
    Suggestion,
}
impl Tone {
    pub const ALL: [Tone; 3] = [Tone::Direct, Tone::Friendly, Tone::Suggestion];
    pub fn label(self) -> &'static str {
        match self {
            Tone::Direct => "Direct",
            Tone::Friendly => "Friendly",
            Tone::Suggestion => "Suggestion",
        }
    }
    fn guidance(self) -> &'static str {
        match self {
            Tone::Direct => {
                "Direct: say what needs to change and why, plainly and briefly. No filler, no hedging, still respectful."
            }
            Tone::Friendly => {
                "Friendly: warm and collegial. Thank the author where it fits and keep the request clear."
            }
            Tone::Suggestion => {
                "Suggestion: phrase it as a proposal or a question the author can take or leave, such as \"What do you think about…\" or \"Could we…\"."
            }
        }
    }
}

const OPEN: &str = "<polished>";
const CLOSE: &str = "</polished>";

/// What the agent is sent to polish `draft`, a comment on the pull request
/// named by `pull_request` (its repository, number, title and link).
pub fn prompt(pull_request: &str, draft: &str, tone: Tone) -> String {
    format!(
        "Rewrite my review comment on the pull request {} in this tone. {}\n\n\
         Keep every technical point, file name, line and code reference exactly as written. \
         Do not add new requests or claims. Write it as me, in the first person, in GitHub \
         Markdown. Do not run tools or change any files.\n\n\
         Answer with only the rewritten comment between {OPEN} and {CLOSE}.\n\n\
         My comment:\n{}",
        pull_request.trim(),
        tone.guidance(),
        draft.trim()
    )
}

/// The rewrite in the agent's reply: the last `<polished>` block, trimmed.
/// `None` when the reply has no complete, non-empty block.
pub fn extract(reply: &str) -> Option<String> {
    let end = reply.rfind(CLOSE)?;
    let start = reply[..end].rfind(OPEN)? + OPEN.len();
    let text = reply[start..end].trim();
    (!text.is_empty()).then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_carries_the_tone_and_the_draft_and_asks_for_tags() {
        let text = prompt(
            "acme/web#562 \"Toasty\" (https://github.com/acme/web/pull/562)",
            "  this is wrong, add the index \n",
            Tone::Friendly,
        );
        assert!(text.starts_with(
            "Rewrite my review comment on the pull request acme/web#562 \"Toasty\" \
             (https://github.com/acme/web/pull/562) in this tone."
        ));
        assert!(text.contains("Friendly: warm"));
        assert!(text.ends_with("My comment:\nthis is wrong, add the index"));
        assert!(text.contains("<polished> and </polished>"));
        assert_ne!(
            prompt("a/b#1", "x", Tone::Direct),
            prompt("a/b#1", "x", Tone::Suggestion)
        );
    }

    #[test]
    fn the_rewrite_is_the_last_complete_block() {
        assert_eq!(
            extract(
                "Sure.\n<polished>\nGood catch, thanks. Could you add an index?\n</polished>\n"
            ),
            Some("Good catch, thanks. Could you add an index?".into())
        );
        assert_eq!(
            extract("<polished>first</polished> then <polished> second </polished>"),
            Some("second".into())
        );
        assert_eq!(extract("no tags at all"), None);
        assert_eq!(extract("<polished>   </polished>"), None);
        assert_eq!(extract("<polished>cut off"), None);
        assert_eq!(extract("</polished> before <polished>"), None);
    }
}
