//! The diagnostics consent record from `docs/diagnostics/consent.md`: what
//! the user allowed, the first-run answers, Settings changes, and which send
//! queues and install ID go when consent is taken back.
use super::outbox::Kind;
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Deserializer, Serialize};

/// Raise this whenever Gespan starts sending something new, or somewhere
/// new; a lower stored version asks again.
pub const CONSENT_VERSION: u32 = 1;

/// The privacy notice the prompt and Settings link to.
pub const PRIVACY_NOTICE: &str = "https://github.com/0xvasanth/gespan/blob/main/docs/privacy.md";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CrashReports {
    Ask,
    Always,
    Never,
}

impl CrashReports {
    pub const ALL: [Self; 3] = [Self::Ask, Self::Always, Self::Never];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Always => "always",
            Self::Never => "never",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|choice| choice.as_str() == value)
    }
}

/// The stored record, field for field as the contract has it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Consent {
    pub consent_version: u32,
    pub crash_reports: CrashReports,
    pub metrics: bool,
    pub include_logs: bool,
    pub decided_at: Option<String>,
}

/// The contract's defaults: nothing is sent.
impl Default for Consent {
    fn default() -> Self {
        Self {
            consent_version: CONSENT_VERSION,
            crash_reports: CrashReports::Ask,
            metrics: false,
            include_logs: true,
            decided_at: None,
        }
    }
}

impl Consent {
    /// Whether sends of `kind` go out in the background, without a click.
    pub fn sends(&self, kind: Kind) -> bool {
        match kind {
            Kind::Crash => self.crash_reports == CrashReports::Always,
            Kind::Metrics => self.metrics,
        }
    }

    /// Opted in to something, so an install ID may exist.
    pub fn opted_in(&self) -> bool {
        Kind::ALL.into_iter().any(|kind| self.sends(kind))
    }
}

/// The first-run (or re-prompt) sheet's answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Always,
    Ask,
    Never,
    /// Closed without a choice: counts as "Ask me each time".
    Dismissed,
}

/// What Native keeps in its preferences.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConsentState {
    /// `None` until the user first answers or changes a choice.
    #[serde(deserialize_with = "readable_or_none")]
    pub record: Option<Consent>,
    /// The anonymous install ID, only while opted in to something.
    pub install_id: Option<String>,
}

/// An unreadable record is no record: defaults apply and the prompt shows
/// again, so a damaged file can never read as consent.
fn readable_or_none<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Consent>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).ok())
}

impl ConsentState {
    /// The record in force: the stored one, even from an older version
    /// (never more than it allowed), else the defaults.
    pub fn current(&self) -> Consent {
        self.record.clone().unwrap_or_default()
    }

    /// No answer yet, or an answer to an older `consent_version`.
    pub fn needs_prompt(&self) -> bool {
        self.record
            .as_ref()
            .is_none_or(|record| record.consent_version < CONSENT_VERSION)
    }

    /// Records a prompt answer. A re-prompt only asks about crash reports,
    /// so the stored `metrics` and `include_logs` stay. Returns the queues
    /// to empty.
    pub fn answer(&mut self, answer: Answer, now: &str) -> Vec<Kind> {
        let mut consent = self.record.clone().unwrap_or_default();
        consent.crash_reports = match answer {
            Answer::Always => CrashReports::Always,
            Answer::Ask | Answer::Dismissed => CrashReports::Ask,
            Answer::Never => CrashReports::Never,
        };
        consent.consent_version = CONSENT_VERSION;
        self.settle(consent, now)
    }

    /// A change made in Settings. It keeps the stored version, so a pending
    /// re-prompt still shows. Returns the queues to empty.
    pub fn change(&mut self, edit: impl FnOnce(&mut Consent), now: &str) -> Vec<Kind> {
        let mut consent = self.current();
        edit(&mut consent);
        self.settle(consent, now)
    }

    fn settle(&mut self, mut consent: Consent, now: &str) -> Vec<Kind> {
        consent.decided_at = Some(now.to_owned());
        if !consent.opted_in() {
            self.install_id = None;
        } else if self.install_id.is_none() {
            self.install_id = Some(uuid::Uuid::new_v4().to_string());
        }
        // Every kind that no longer sends on its own gets an empty queue,
        // whatever was queued before, so nothing stale is ever sent.
        let cleared = Kind::ALL
            .into_iter()
            .filter(|kind| !consent.sends(*kind))
            .collect();
        self.record = Some(consent);
        cleared
    }
}

/// `decided_at` for now: RFC 3339 UTC with milliseconds.
pub fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: &str = "2026-10-25T09:00:00.000Z";

    fn stored(value: serde_json::Value) -> ConsentState {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn defaults_send_nothing() {
        let state = ConsentState::default();
        assert!(state.needs_prompt());
        let consent = state.current();
        assert_eq!(
            serde_json::to_value(&consent).unwrap(),
            json!({
                "consent_version": 1,
                "crash_reports": "ask",
                "metrics": false,
                "include_logs": true,
                "decided_at": null
            })
        );
        for kind in Kind::ALL {
            assert!(!consent.sends(kind), "{kind:?}");
        }
        assert!(!consent.opted_in());
        assert_eq!(state.install_id, None);
    }

    #[test]
    fn first_run_answers_match_the_contract() {
        for (answer, crash) in [
            (Answer::Always, CrashReports::Always),
            (Answer::Ask, CrashReports::Ask),
            (Answer::Never, CrashReports::Never),
            (Answer::Dismissed, CrashReports::Ask),
        ] {
            let mut state = ConsentState::default();
            state.answer(answer, NOW);
            assert_eq!(
                state.record,
                Some(Consent {
                    consent_version: CONSENT_VERSION,
                    crash_reports: crash,
                    metrics: false,
                    include_logs: true,
                    decided_at: Some(NOW.into()),
                }),
                "{answer:?}"
            );
            assert!(!state.needs_prompt(), "{answer:?}");
            // No answer turns on error metrics.
            assert!(!state.current().sends(Kind::Metrics), "{answer:?}");
        }
    }

    #[test]
    fn reprompt_keeps_metrics_and_include_logs() {
        let mut state = stored(json!({
            "record": {
                "consent_version": 0,
                "crash_reports": "never",
                "metrics": true,
                "include_logs": false,
                "decided_at": "2026-01-01T00:00:00.000Z"
            },
            "install_id": "c2d8e4a1-7b3f-4e9c-a6d0-5f1b8e2c9a74"
        }));
        assert!(state.needs_prompt());
        // Until answered, the old record still decides what is sent.
        assert!(state.current().sends(Kind::Metrics));
        state.answer(Answer::Always, NOW);
        let consent = state.current();
        assert_eq!(consent.consent_version, CONSENT_VERSION);
        assert_eq!(consent.crash_reports, CrashReports::Always);
        assert!(consent.metrics);
        assert!(!consent.include_logs);
        assert_eq!(consent.decided_at.as_deref(), Some(NOW));
        assert!(!state.needs_prompt());
    }

    #[test]
    fn current_record_is_not_prompted() {
        let state = stored(json!({"record": {
            "consent_version": 1, "crash_reports": "ask", "metrics": false,
            "include_logs": true, "decided_at": NOW
        }}));
        assert!(!state.needs_prompt());
    }

    #[test]
    fn unreadable_record_is_no_record() {
        for record in [
            json!({"consent_version": 1, "crash_reports": "sometimes", "metrics": true,
                   "include_logs": true, "decided_at": null}),
            json!(3),
            json!({"metrics": true}),
        ] {
            let state = stored(json!({ "record": record }));
            assert_eq!(state.record, None, "{record}");
            assert!(state.needs_prompt());
            assert!(!state.current().sends(Kind::Metrics));
        }
    }

    #[test]
    fn install_id_created_on_first_opt_in() {
        let mut state = ConsentState::default();
        state.answer(Answer::Ask, NOW);
        assert_eq!(state.install_id, None);
        let mut state = ConsentState::default();
        state.answer(Answer::Always, NOW);
        let id = state.install_id.clone().expect("an install id");
        let parsed = uuid::Uuid::parse_str(&id).unwrap();
        assert_eq!(parsed.get_version_num(), 4);
    }

    #[test]
    fn install_id_survives_while_opted_in() {
        let mut state = ConsentState::default();
        state.change(|c| c.metrics = true, NOW);
        let id = state.install_id.clone().expect("metrics on opts in");
        state.change(|c| c.crash_reports = CrashReports::Always, NOW);
        state.change(|c| c.metrics = false, NOW);
        assert_eq!(state.install_id.as_deref(), Some(id.as_str()));
        state.answer(Answer::Always, NOW);
        assert_eq!(state.install_id.as_deref(), Some(id.as_str()));
    }

    #[test]
    fn opting_out_of_everything_deletes_the_install_id() {
        let mut state = ConsentState::default();
        state.change(|c| c.metrics = true, NOW);
        state.change(|c| c.crash_reports = CrashReports::Always, NOW);
        state.change(|c| c.crash_reports = CrashReports::Ask, NOW);
        assert!(state.install_id.is_some(), "metrics are still on");
        state.change(|c| c.metrics = false, NOW);
        assert_eq!(state.install_id, None);
    }

    #[test]
    fn revoking_returns_the_queues_to_empty() {
        let mut state = ConsentState::default();
        state.change(|c| c.metrics = true, NOW);
        let cleared = state.change(|c| c.crash_reports = CrashReports::Always, NOW);
        assert!(cleared.is_empty(), "both kinds now send: {cleared:?}");
        let cleared = state.change(|c| c.crash_reports = CrashReports::Never, NOW);
        assert_eq!(cleared, vec![Kind::Crash]);
        let cleared = state.change(|c| c.metrics = false, NOW);
        assert_eq!(cleared, vec![Kind::Crash, Kind::Metrics]);
        let cleared = state.answer(Answer::Never, NOW);
        assert_eq!(cleared, vec![Kind::Crash, Kind::Metrics]);
    }

    #[test]
    fn settings_changes_set_decided_at_but_keep_a_pending_reprompt() {
        let mut state = stored(json!({"record": {
            "consent_version": 0, "crash_reports": "ask", "metrics": false,
            "include_logs": true, "decided_at": null
        }}));
        state.change(|c| c.include_logs = false, NOW);
        let consent = state.current();
        assert_eq!(consent.decided_at.as_deref(), Some(NOW));
        assert!(!consent.include_logs);
        assert!(
            state.needs_prompt(),
            "only the prompt answers a new version"
        );
        // A first Settings change also stores a record, at the current version.
        let mut fresh = ConsentState::default();
        fresh.change(|c| c.include_logs = false, NOW);
        assert_eq!(fresh.current().consent_version, CONSENT_VERSION);
        assert!(!fresh.needs_prompt());
    }

    #[test]
    fn record_json_matches_the_contract() {
        let consent = Consent {
            consent_version: 1,
            crash_reports: CrashReports::Always,
            metrics: true,
            include_logs: true,
            decided_at: Some(NOW.into()),
        };
        assert_eq!(
            serde_json::to_value(&consent).unwrap(),
            json!({
                "consent_version": 1,
                "crash_reports": "always",
                "metrics": true,
                "include_logs": true,
                "decided_at": NOW
            })
        );
    }

    #[test]
    fn now_is_rfc3339_utc_with_millis() {
        let now = now();
        assert!(now.ends_with('Z'), "{now}");
        assert_eq!(now.len(), "2026-10-25T09:00:00.000Z".len(), "{now}");
        assert!(chrono::DateTime::parse_from_rfc3339(&now).is_ok());
    }
}
