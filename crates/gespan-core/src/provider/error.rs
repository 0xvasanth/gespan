//! Why a provider turn ended without finishing. Turns still report errors as
//! `String`, so each variant prints the exact words callers compared against
//! before, and `of` reads them back. New code names the variant instead of
//! writing or matching "Stopped" and "Held" by hand.
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderError {
    /// The user stopped the turn.
    Stopped,
    /// The user held the session; the turn ended like a stop.
    Held,
    /// Anything else, with the message the user sees.
    Failed(String),
}

impl ProviderError {
    const STOPPED: &'static str = "Stopped";
    const HELD: &'static str = "Held";

    /// Reads a turn's error message back. Only the exact words count, as
    /// the `==` checks this replaces did: "Stopped by the CLI" is a failure.
    pub fn of(message: &str) -> Self {
        match message {
            Self::STOPPED => Self::Stopped,
            Self::HELD => Self::Held,
            other => Self::Failed(other.to_owned()),
        }
    }
    pub fn is_stopped(message: &str) -> bool {
        message == Self::STOPPED
    }
    /// Stop and Hold both end a turn on purpose: no error row.
    pub fn ended_by_user(message: &str) -> bool {
        message == Self::STOPPED || message == Self::HELD
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stopped => Self::STOPPED,
            Self::Held => Self::HELD,
            Self::Failed(message) => message,
        })
    }
}
impl std::error::Error for ProviderError {}

impl From<ProviderError> for String {
    fn from(error: ProviderError) -> Self {
        match error {
            ProviderError::Failed(message) => message,
            other => other.to_string(),
        }
    }
}
impl From<String> for ProviderError {
    fn from(message: String) -> Self {
        Self::of(&message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_variant_prints_the_words_callers_already_match() {
        assert_eq!(String::from(ProviderError::Stopped), "Stopped");
        assert_eq!(String::from(ProviderError::Held), "Held");
        assert_eq!(
            String::from(ProviderError::Failed("Provider exited".into())),
            "Provider exited"
        );
        assert_eq!(ProviderError::Held.to_string(), "Held");
    }

    #[test]
    fn messages_read_back_as_the_variant_that_wrote_them() {
        for error in [
            ProviderError::Stopped,
            ProviderError::Held,
            ProviderError::Failed("Codex CLI not found.".into()),
            ProviderError::Failed(String::new()),
        ] {
            assert_eq!(ProviderError::from(String::from(error.clone())), error);
        }
    }

    #[test]
    fn only_the_exact_words_mean_stopped_or_held() {
        for near in [
            "stopped",
            " Stopped",
            "Stopped.",
            "Stopped by the CLI",
            "held",
            "Held ",
        ] {
            assert_eq!(
                ProviderError::of(near),
                ProviderError::Failed(near.into()),
                "{near:?}"
            );
            assert!(!ProviderError::ended_by_user(near), "{near:?}");
            assert!(!ProviderError::is_stopped(near), "{near:?}");
        }
    }

    #[test]
    fn stop_and_hold_end_a_turn_on_purpose_and_nothing_else_does() {
        assert!(ProviderError::ended_by_user("Stopped"));
        assert!(ProviderError::ended_by_user("Held"));
        assert!(ProviderError::is_stopped("Stopped"));
        assert!(!ProviderError::is_stopped("Held"));
        assert!(!ProviderError::ended_by_user(""));
        assert!(!ProviderError::ended_by_user(
            "Provider exited before completing the turn"
        ));
    }
}
