//! Codex plan limits, from `codex app-server`'s `account/rateLimits/read`
//! and the `account/rateLimits/updated` notification each turn sends.
use super::{
    now_ms, number, timestamp, title_case, window, FetchError, ResetCredit, ResetCredits, Snapshot,
    Source, Window,
};
use crate::catalog;
use serde_json::Value;

pub(crate) struct Codex;
impl Source for Codex {
    fn banked_resets(&self) -> bool {
        true
    }
    fn consume_reset(
        &self,
        profile: &crate::accounts::Profile,
        credit: Option<&str>,
    ) -> Result<String, String> {
        consume_reset(profile, credit)
    }
    fn probe(
        &self,
        profile: &crate::accounts::Profile,
        _model: Option<&str>,
    ) -> Result<Snapshot, FetchError> {
        let value =
            catalog::codex_request(profile, "account/rateLimits/read", serde_json::json!({}))
                .map_err(|message| {
                    let lower = message.to_lowercase();
                    if [
                        "not installed",
                        "not signed in",
                        "not authenticated",
                        "authentication required",
                    ]
                    .iter()
                    .any(|s| lower.contains(s))
                    {
                        FetchError::unavailable("Sign in with codex login, then refresh usage.")
                    } else {
                        FetchError::failed(message)
                    }
                })?;
        let snapshot = parse(&value);
        if snapshot.windows.is_empty() && snapshot.reset_credits.is_none() {
            return Err(FetchError::unavailable(
                "No Codex subscription usage available for this account.",
            ));
        }
        Ok(snapshot)
    }
    fn parse(&self, raw: &str) -> Result<Snapshot, FetchError> {
        let value: Value = serde_json::from_str(raw.trim())
            .map_err(|_| FetchError::failed("Codex usage response was not JSON."))?;
        Ok(parse(value.get("result").unwrap_or(&value)))
    }
}

pub(super) fn parse(value: &Value) -> Snapshot {
    let wrapper = value.get("rateLimits").unwrap_or(value);
    let mut windows: Vec<Window> = Vec::new();
    // An explicitly identified window always wins over a positional fallback.
    for fallback_pass in [false, true] {
        for (key, fallback) in [("primary", 300), ("secondary", 10_080)] {
            let raw = &wrapper[key];
            let classified =
                number(raw, &["windowDurationMins", "window_duration_mins"]).and_then(|n| {
                    [300_u64, 10_080, 43_200]
                        .into_iter()
                        .find(|m| (n - *m as f64).abs() <= 1.)
                });
            if classified.is_none() != fallback_pass {
                continue;
            }
            let minutes = classified.unwrap_or(fallback);
            if !windows.iter().any(|w| w.minutes == minutes) {
                if let Some(w) = window(
                    raw,
                    minutes,
                    &["usedPercent", "used_percent", "used_percentage"],
                ) {
                    windows.push(w);
                }
            }
        }
    }
    windows.sort_by_key(|w| w.minutes);
    Snapshot {
        windows,
        updated_at: now_ms(),
        reset_credits: parse_reset_credits(value),
        plan: wrapper["planType"]
            .as_str()
            .filter(|plan| !plan.trim().is_empty())
            .map(title_case),
        ..Default::default()
    }
}
/// The windows in Codex's `account/rateLimits/updated` notification.
pub fn live_windows(params: &Value) -> Vec<Window> {
    parse(params).windows
}
pub(super) fn parse_reset_credits(value: &Value) -> Option<ResetCredits> {
    let raw = value
        .get("rateLimitResetCredits")
        .or_else(|| value.get("rate_limit_reset_credits"))?;
    let count = number(raw, &["availableCount", "available_count"])?;
    let credits = raw["credits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let id = row["id"].as_str().filter(|s| !s.trim().is_empty())?;
            Some(ResetCredit {
                id: id.into(),
                status: row["status"]
                    .as_str()
                    .filter(|s| matches!(*s, "available" | "redeeming" | "redeemed"))
                    .unwrap_or("unknown")
                    .into(),
                reset_type: if row["resetType"] == "codexRateLimits" {
                    "codexRateLimits"
                } else {
                    "unknown"
                }
                .into(),
                granted_at: timestamp(
                    row.get("grantedAt")
                        .or_else(|| row.get("granted_at"))
                        .unwrap_or(&Value::Null),
                ),
                expires_at: timestamp(
                    row.get("expiresAt")
                        .or_else(|| row.get("expires_at"))
                        .unwrap_or(&Value::Null),
                ),
                title: row["title"].as_str().map(str::to_owned),
                description: row["description"].as_str().map(str::to_owned),
            })
        })
        .collect();
    Some(ResetCredits {
        available_count: count.max(0.).floor() as u64,
        credits,
    })
}
pub fn consume_reset(
    profile: &crate::accounts::Profile,
    credit: Option<&str>,
) -> Result<String, String> {
    let mut params = serde_json::json!({"idempotencyKey": crate::accounts::request_id()});
    if let Some(credit) = credit {
        params["creditId"] = credit.into();
    }
    let result = catalog::codex_request(profile, "account/rateLimitResetCredit/consume", params)?;
    match result["outcome"].as_str() {
        Some(outcome @ ("reset" | "nothingToReset" | "noCredit" | "alreadyRedeemed")) => {
            Ok(outcome.into())
        }
        _ => Err("Codex returned an unknown reset result".into()),
    }
}
