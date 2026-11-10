//! Subscription usage.
//! Blocking probes belong on a worker. Credentials are read only, never rotated.
pub(crate) mod claude;
mod codex;
mod cursor;
mod devin;
mod kiro;
mod opencode;
mod pi;

/// Each provider's usage source, named by its row in the provider spec.
pub(crate) use {
    claude::Claude, codex::Codex, cursor::Cursor, devin::Devin, kiro::Kiro, opencode::OpenCode,
    pi::Pi,
};

pub use claude::live_windows as claude_live_windows;
pub use codex::{consume_reset, live_windows as codex_live_windows};

use crate::provider::Kind;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Window {
    pub used_percent: f64,
    pub minutes: u64,
    pub resets_at: Option<i64>,
    /// A limit that covers only part of the plan, such as one model's weekly
    /// cap. It never decides whether the account as a whole is usable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<Amount>,
}
/// What a window or balance counts, when the provider reports more than a percent.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Amount {
    pub used: f64,
    pub limit: Option<f64>,
    /// "credits", or an ISO currency code.
    pub unit: String,
}
impl Amount {
    pub fn label(&self) -> String {
        let value = |n: f64| {
            if self.unit == "credits" {
                format!("{}", (n * 100.).round() / 100.)
            } else if self.unit == "USD" {
                format!("${n:.2}")
            } else {
                format!("{n:.2} {}", self.unit)
            }
        };
        let used = value(self.used);
        let suffix = if self.unit == "credits" {
            " credits"
        } else {
            ""
        };
        match self.limit {
            Some(limit) => format!("{used} of {}{suffix}", value(limit)),
            None => format!("{used}{suffix}"),
        }
    }
}
impl Window {
    pub fn new(used_percent: f64, minutes: u64, resets_at: Option<i64>) -> Self {
        Self {
            used_percent,
            minutes,
            resets_at,
            scope: None,
            amount: None,
        }
    }
    pub fn label(&self) -> String {
        match self.minutes {
            300 => "5h".into(),
            1_440 => "1d".into(),
            10_080 => "wk".into(),
            43_200 => "mo".into(),
            n if n % 60 == 0 => format!("{}h", n / 60),
            n => format!("{n}m"),
        }
    }
    pub fn countdown(&self, now: i64) -> String {
        self.resets_at
            .map(|at| duration(at.saturating_sub(now)))
            .unwrap_or_else(|| self.label())
    }
    pub fn reset_label(&self, now: i64) -> String {
        match self.resets_at {
            Some(at) if at <= now => "Resets now".into(),
            Some(at) => format!("Resets in {}", duration(at - now)),
            None => format!("{} window", self.label()),
        }
    }
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub windows: Vec<Window>,
    pub updated_at: i64,
    #[serde(default)]
    pub reset_credits: Option<ResetCredits>,
    #[serde(default)]
    pub extra_usage_balance: Option<f64>,
    #[serde(default)]
    pub identity: Option<crate::accounts::Identity>,
    /// Pay-as-you-go spend beyond the plan, when the account has it turned on.
    #[serde(default)]
    pub extra_usage: Option<Amount>,
    #[serde(default)]
    pub plan: Option<String>,
}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ResetCredits {
    pub available_count: u64,
    pub credits: Vec<ResetCredit>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ResetCredit {
    pub id: String,
    pub status: String,
    pub reset_type: String,
    pub granted_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub title: Option<String>,
    pub description: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Limit {
    pub resets_at: Option<i64>,
    pub resume_at_reset: bool,
}
impl Limit {
    pub fn from_error(message: &str) -> Option<Self> {
        // Claude's synthetic assistant errors use this code before the result
        // frame arrives. Ordinary transient HTTP 429s must remain retryable.
        if message.trim().eq_ignore_ascii_case("rate_limit") {
            return Some(Self::default());
        }
        static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let pattern=PATTERN.get_or_init(||regex::Regex::new(r"(?i)(?:usage|spending|monthly|weekly|daily)[ _]limit[ _](?:reached|exceeded)|(?:quota|credits?) (?:exceeded|exhausted|depleted)|insufficient[_ ](?:quota|credits)|hit your (?:(?:usage|session) )?limit|credit balance is too low|reached (?:its|your) usage limit").unwrap());
        pattern.is_match(message).then(Self::default)
    }
    /// Extra-usage refusals fall back to the subscription and do not stop a turn.
    pub fn from_claude_info(info: &Value) -> Option<Self> {
        (info["status"] == "rejected"
            && info["isUsingOverage"] != true
            && info["rateLimitType"] != "overage")
            .then(|| Self {
                resets_at: timestamp(&info["resetsAt"]),
                resume_at_reset: false,
            })
    }
    pub fn resume_due(&self, now: i64, busy: bool) -> bool {
        !busy
            && self.resume_at_reset
            && self
                .resets_at
                .is_some_and(|at| now >= at.saturating_add(30_000))
    }
}
#[derive(Clone, Debug)]
pub struct FetchError {
    pub message: String,
    pub unavailable: bool,
}
impl FetchError {
    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            unavailable: true,
        }
    }
    fn failed(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            unavailable: false,
        }
    }
}

/// Where one provider's plan usage comes from. Each provider with usage has
/// its own module and its row in the provider spec names it.
pub trait Source: Sync {
    /// Reads the account's current usage. Blocking: run it on a worker.
    fn probe(
        &self,
        profile: &crate::accounts::Profile,
        model: Option<&str>,
    ) -> Result<Snapshot, FetchError>;
    /// Reads a report as the probe receives it, such as a saved capture.
    fn parse(&self, _raw: &str) -> Result<Snapshot, FetchError> {
        Err(FetchError::failed(
            "This provider's usage report cannot be read offline.",
        ))
    }
    /// The account can bank limit resets and spend them early.
    fn banked_resets(&self) -> bool {
        false
    }
    /// Spends one banked reset; `credit` picks which, else the provider's
    /// next. Blocking.
    fn consume_reset(
        &self,
        _profile: &crate::accounts::Profile,
        _credit: Option<&str>,
    ) -> Result<String, String> {
        Err("This provider has no banked resets.".into())
    }
}

/// Whether Gespan can read usage for `kind` at all.
pub fn has_usage(kind: Kind) -> bool {
    kind.spec().usage.is_some()
}
fn source(kind: Kind) -> Result<&'static dyn Source, FetchError> {
    kind.spec().usage.ok_or_else(|| {
        FetchError::unavailable(format!(
            "No subscription usage is available for {}.",
            kind.label()
        ))
    })
}
pub fn fetch(kind: Kind) -> Result<Snapshot, FetchError> {
    fetch_for(kind, &Default::default(), None)
}
pub fn fetch_for(
    kind: Kind,
    profile: &crate::accounts::Profile,
    model: Option<&str>,
) -> Result<Snapshot, FetchError> {
    source(kind)?.probe(profile, model)
}
/// Reads a provider's usage report as its probe receives it.
pub fn parse_report(kind: Kind, raw: &str) -> Result<Snapshot, FetchError> {
    source(kind)?.parse(raw)
}
impl Snapshot {
    /// The windows that limit the whole account, not one model or surface.
    pub fn plan_windows(&self) -> impl Iterator<Item = &Window> {
        self.windows.iter().filter(|w| w.scope.is_none())
    }
    /// Folds the windows a running turn reported into the last probe, so the
    /// footer moves with every turn instead of waiting for the next probe.
    /// A window is matched by its length and scope; the rest stay as probed.
    pub fn merge_live(&mut self, windows: &[Window], now: i64) {
        for live in windows {
            match self
                .windows
                .iter_mut()
                .find(|w| w.minutes == live.minutes && w.scope == live.scope)
            {
                Some(window) => {
                    window.used_percent = live.used_percent;
                    if live.resets_at.is_some() {
                        window.resets_at = live.resets_at;
                    }
                }
                None => self.windows.push(live.clone()),
            }
        }
        self.windows.sort_by_key(|w| (w.scope.is_some(), w.minutes));
        self.updated_at = now;
    }
    pub fn headroom(&self, now: i64) -> Option<f64> {
        self.plan_windows()
            .map(|w| {
                if w.resets_at.is_some_and(|at| at <= now) {
                    100.
                } else {
                    100. - w.used_percent.clamp(0., 100.)
                }
            })
            .reduce(f64::min)
    }
    pub fn exhausted_reset(&self) -> Option<i64> {
        let exhausted: Vec<_> = self
            .plan_windows()
            .filter(|w| w.used_percent >= 100.)
            .collect();
        if exhausted.is_empty() || exhausted.iter().any(|w| w.resets_at.is_none()) {
            None
        } else {
            exhausted.iter().filter_map(|w| w.resets_at).max()
        }
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}
pub fn duration(ms: i64) -> String {
    if ms <= 0 {
        return "now".into();
    }
    let minutes = ms / 60_000;
    let hours = minutes / 60;
    if hours >= 24 {
        let days = hours / 24;
        let rest = hours % 24;
        if rest > 0 {
            format!("{days}d {rest}h")
        } else {
            format!("{days}d")
        }
    } else if hours > 0 {
        let rest = minutes % 60;
        if rest > 0 {
            format!("{hours}h {rest}m")
        } else {
            format!("{hours}h")
        }
    } else {
        format!("{minutes}m")
    }
}
fn number(value: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| value[*key].as_f64().filter(|v| v.is_finite()))
}
fn timestamp(value: &Value) -> Option<i64> {
    let numeric = value
        .as_f64()
        .or_else(|| value.as_str()?.parse::<f64>().ok());
    if let Some(n) = numeric.filter(|n| n.is_finite() && *n > 0.) {
        return Some(if n > 10_000_000_000. { n } else { n * 1000. } as i64);
    }
    chrono::DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(|d| d.timestamp_millis())
}
fn window(value: &Value, minutes: u64, percent_keys: &[&str]) -> Option<Window> {
    Some(Window::new(
        number(value, percent_keys)?.clamp(0., 100.),
        minutes,
        timestamp(&value["resetsAt"]).or_else(|| timestamp(&value["resets_at"])),
    ))
}
fn title_case(text: &str) -> String {
    text.split(|c: char| c == '_' || c.is_whitespace())
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| {
                    first
                        .to_uppercase()
                        .chain(chars.flat_map(char::to_lowercase))
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect::<Vec<String>>()
        .join(" ")
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn explicit_codex_windows_win_before_positional_fallbacks() {
        let snapshot = codex::parse(
            &json!({"primary":{"usedPercent":99},"secondary":{"usedPercent":12,"windowDurationMins":300}}),
        );
        assert_eq!(snapshot.windows.len(), 1);
        assert_eq!(snapshot.windows[0].used_percent, 12.);
        let snapshot = codex::parse(
            &json!({"primary":{"usedPercent":44,"windowDurationMins":10080},"secondary":{"usedPercent":88}}),
        );
        assert_eq!(snapshot.windows.len(), 1);
        assert_eq!(snapshot.windows[0].used_percent, 44.);
    }
    #[test]
    fn reset_credit_details_keep_aliases_and_unknown_statuses() {
        let credits = codex::parse_reset_credits(&json!({"rate_limit_reset_credits":{"available_count":2.9,"credits":[
            {"id":"a","status":"available","resetType":"codexRateLimits","expires_at":1800000000},
            {"id":"b","status":"future-status","resetType":"future-type","grantedAt":"2026-10-18T12:30:00Z"},
            {"id":""}
        ]}})).unwrap();
        assert_eq!(credits.available_count, 2);
        assert_eq!(credits.credits.len(), 2);
        assert_eq!(credits.credits[0].expires_at, Some(1_800_000_000_000));
        assert_eq!(credits.credits[1].status, "unknown");
        assert_eq!(credits.credits[1].reset_type, "unknown");
        assert!(
            codex::parse_reset_credits(&json!({"rateLimitResetCredits":{"credits":[]}})).is_none()
        );
    }
    #[test]
    fn exhausted_accounts_wait_for_all_windows_and_resume_only_when_armed() {
        let mut snapshot = Snapshot {
            windows: vec![
                Window::new(100., 300, Some(100_000)),
                Window::new(100., 10080, Some(200_000)),
            ],
            ..Default::default()
        };
        assert_eq!(snapshot.headroom(99_000), Some(0.));
        assert_eq!(snapshot.headroom(150_000), Some(0.));
        assert_eq!(snapshot.headroom(200_000), Some(100.));
        assert_eq!(snapshot.exhausted_reset(), Some(200_000));
        snapshot.windows[1].resets_at = None;
        assert_eq!(snapshot.exhausted_reset(), None);
        let mut limit = Limit {
            resets_at: Some(200_000),
            resume_at_reset: false,
        };
        assert!(!limit.resume_due(230_000, false));
        limit.resume_at_reset = true;
        assert!(!limit.resume_due(229_999, false));
        assert!(!limit.resume_due(230_000, true));
        assert!(limit.resume_due(230_000, false));
        assert!(Limit::from_error("usage_limit_reached").is_some());
        assert!(Limit::from_error("insufficient_quota").is_some());
        assert!(Limit::from_error("rate_limit").is_some());
        assert!(Limit::from_error("You've hit your session limit · resets 5:30pm").is_some());
        assert!(Limit::from_error("429: rate limit exceeded, retry in 3 seconds").is_none());
        assert!(Limit::from_error("Network timeout").is_none());
    }
    #[test]
    fn opencode_go_requires_known_window_statuses() {
        let snapshot = opencode::parse(
            &json!({"usage":{"rolling":{"status":"ok","percent":15,"resetsAt":"1800000000"},"weekly":{"status":"rate-limited","percent":100},"monthly":{"status":"future","percent":20}}}),
        );
        assert_eq!(snapshot.windows.len(), 2);
        assert_eq!(snapshot.windows[0].resets_at, Some(1_800_000_000_000));
        assert_eq!(snapshot.windows[1].minutes, 10080);
    }

    #[test]
    fn codex_classifies_swapped_and_monthly_windows() {
        let snapshot = codex::parse(&json!({"rateLimits": {
            "primary": {"usedPercent": 35, "windowDurationMins": 10080, "resetsAt": 1800000000},
            "secondary": {"used_percent": 105, "window_duration_mins": 300, "resets_at": 1800000000000_i64}
        }}));
        assert_eq!(snapshot.windows.len(), 2);
        assert_eq!(snapshot.windows[0].minutes, 300);
        assert_eq!(snapshot.windows[0].used_percent, 100.);
        assert_eq!(snapshot.windows[1].minutes, 10_080);
        assert_eq!(snapshot.windows[0].resets_at, snapshot.windows[1].resets_at);
        let monthly =
            codex::parse(&json!({"primary": {"usedPercent": 12, "windowDurationMins": 43200}}));
        assert_eq!(monthly.windows[0].label(), "mo");
        assert!(codex::parse(&json!({"primary": {"usedPercent": "bad"}}))
            .windows
            .is_empty());
    }
    #[test]
    fn claude_parses_iso_resets_and_rejects_empty_responses() {
        let snapshot = claude::parse(&json!({
            "five_hour": {"utilization": 23.4, "resets_at": "2026-10-18T12:30:00Z"},
            "seven_day": {"utilization": -2, "resets_at": null}
        }))
        .unwrap();
        assert_eq!(snapshot.windows[0].used_percent, 23.4);
        assert_eq!(snapshot.windows[0].resets_at, Some(1792326600000));
        assert_eq!(snapshot.windows[1].used_percent, 0.);
        assert!(claude::parse(&json!({})).unwrap_err().unavailable);
        assert!(!claude::parse(&json!(null)).unwrap_err().unavailable);
    }
    fn fixture_lines(raw: &str) -> Vec<Value> {
        raw.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn claude_usage_api_keeps_plan_windows_and_adds_model_scoped_ones() {
        let value: Value =
            serde_json::from_str(include_str!("usage/fixtures/claude-oauth-usage.json")).unwrap();
        let snapshot = claude::parse(&value).unwrap();
        // `five_hour` is null for this account: only the weekly plan window.
        let plan: Vec<_> = snapshot.plan_windows().collect();
        assert_eq!(plan.len(), 1);
        assert_eq!((plan[0].minutes, plan[0].used_percent), (10_080, 2.));
        let scoped: Vec<_> = snapshot
            .windows
            .iter()
            .filter(|w| w.scope.is_some())
            .collect();
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].scope.as_deref(), Some("Fable"));
        assert_eq!((scoped[0].minutes, scoped[0].used_percent), (10_080, 0.));
        assert!(scoped[0].resets_at.is_some());
        // Extra usage is switched off on this account.
        assert_eq!(snapshot.extra_usage, None);
    }

    #[test]
    fn a_maxed_model_cap_does_not_mark_the_account_exhausted() {
        let mut fable = Window::new(100., 10_080, Some(500));
        fable.scope = Some("Fable".into());
        let snapshot = Snapshot {
            windows: vec![Window::new(40., 10_080, Some(500)), fable],
            ..Default::default()
        };
        assert_eq!(snapshot.headroom(0), Some(60.));
        assert_eq!(snapshot.exhausted_reset(), None);
    }

    #[test]
    fn claude_extra_usage_reads_minor_units_only_when_enabled() {
        let mut value: Value =
            serde_json::from_str(include_str!("usage/fixtures/claude-oauth-usage.json")).unwrap();
        value["spend"]["enabled"] = true.into();
        value["spend"]["used"]["amount_minor"] = 1234.into();
        value["spend"]["limit"] =
            serde_json::json!({"amount_minor":5000,"currency":"USD","exponent":2});
        let spend = claude::parse(&value).unwrap().extra_usage.unwrap();
        assert_eq!(spend.label(), "$12.34 of $50.00");
        value["spend"]["limit"] = Value::Null;
        assert_eq!(
            claude::parse(&value).unwrap().extra_usage.unwrap().label(),
            "$12.34"
        );
    }

    #[test]
    fn claude_stream_rate_limit_event_becomes_live_percent_windows() {
        let events = fixture_lines(include_str!("usage/fixtures/claude-stream.jsonl"));
        let event = events
            .iter()
            .find(|v| v["type"] == "rate_limit_event")
            .unwrap();
        let windows = claude_live_windows(&event["rate_limit_info"]);
        // 0.02 utilization is the 2% the usage API reports.
        assert_eq!(
            windows,
            vec![Window::new(2., 10_080, Some(1_794_272_400_000))]
        );
        assert!(claude_live_windows(&json!({"status":"allowed"})).is_empty());
    }

    #[test]
    fn codex_probe_and_live_notifications_agree() {
        let lines = fixture_lines(include_str!("usage/fixtures/codex-app-server.jsonl"));
        let probe = codex::parse(&lines.iter().find(|v| v["id"] == 1).unwrap()["result"]);
        assert_eq!(probe.plan.as_deref(), Some("Free"));
        assert_eq!(
            probe.windows,
            vec![Window::new(19., 43_200, Some(1_795_418_928_000))]
        );
        let live: Vec<_> = lines
            .iter()
            .filter(|v| v["method"] == "account/rateLimits/updated")
            .map(|v| codex_live_windows(&v["params"]))
            .collect();
        assert_eq!(live.len(), 2, "one per turn");
        assert!(live.iter().all(|w| *w == probe.windows));
    }

    #[test]
    fn live_windows_update_matching_windows_and_keep_the_rest() {
        let mut fable = Window::new(0., 10_080, None);
        fable.scope = Some("Fable".into());
        let mut snapshot = Snapshot {
            windows: vec![
                Window::new(10., 300, Some(1)),
                Window::new(2., 10_080, Some(2)),
                fable.clone(),
            ],
            updated_at: 5,
            ..Default::default()
        };
        snapshot.merge_live(&[Window::new(3., 10_080, None)], 9);
        assert_eq!(snapshot.windows[0], Window::new(10., 300, Some(1)));
        // A live reading without a reset keeps the probed one.
        assert_eq!(snapshot.windows[1], Window::new(3., 10_080, Some(2)));
        assert_eq!(snapshot.windows[2], fable);
        assert_eq!(snapshot.updated_at, 9);
        // A window the probe never saw appears; scoped ones stay last.
        snapshot.merge_live(&[Window::new(1., 43_200, Some(4))], 10);
        assert_eq!(
            snapshot
                .windows
                .iter()
                .map(|w| w.minutes)
                .collect::<Vec<_>>(),
            vec![300, 10_080, 43_200, 10_080]
        );
        assert!(snapshot.windows[3].scope.is_some());
    }

    #[test]
    fn reset_countdowns_match_the_footer() {
        for (ms, expected) in [
            (0, "now"),
            (-1, "now"),
            (47 * 60_000, "47m"),
            (234 * 60_000, "3h 54m"),
            (9060 * 60_000, "6d 7h"),
        ] {
            assert_eq!(duration(ms), expected);
        }
        let w = Window::new(4., 300, None);
        assert_eq!(w.countdown(0), "5h");
    }
}

#[cfg(test)]
mod has_usage_tests {
    use super::*;
    #[test]
    fn only_providers_with_a_usage_source_report_usage() {
        for kind in [
            Kind::Codex,
            Kind::Claude,
            Kind::Cursor,
            Kind::Devin,
            Kind::Kiro,
            Kind::OpenCode,
            Kind::Pi,
        ] {
            assert!(has_usage(kind), "{kind:?}");
        }
        for kind in [Kind::Antigravity, Kind::Hermes] {
            assert!(!has_usage(kind), "{kind:?}");
        }
    }
}
