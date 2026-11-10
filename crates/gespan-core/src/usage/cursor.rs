//! Cursor plan usage, read from the dashboard API the CLI's own `/usage`
//! command calls, with the token `agent login` keeps in the Keychain.
use super::{now_ms, Amount, FetchError, Snapshot, Source, Window};
use serde_json::{json, Value};
use std::time::Duration;

/// Only Cursor's own API ever sees the token.
const API: &str = "https://api2.cursor.sh/aiserver.v1.DashboardService";
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
/// What GetHardLimit reports for a spend limit that was never capped.
const NO_LIMIT: i64 = i32::MAX as i64;

pub(crate) struct Cursor;
impl Source for Cursor {
    fn probe(
        &self,
        _profile: &crate::accounts::Profile,
        _model: Option<&str>,
    ) -> Result<Snapshot, FetchError> {
        let token = token().ok_or_else(|| {
            FetchError::unavailable("Sign in with agent login, then refresh usage.")
        })?;
        let call = |method: &str| request(&token, method);
        // Usage decides; plan and spend limit only add to it.
        let usage = call("GetCurrentPeriodUsage")?;
        let report = json!({
            "usage": usage,
            "plan": call("GetPlanInfo").ok(),
            "hardLimit": call("GetHardLimit").ok(),
        });
        parse(&report)
    }
    fn parse(&self, raw: &str) -> Result<Snapshot, FetchError> {
        let report = serde_json::from_str(raw)
            .map_err(|_| FetchError::failed("Cursor usage report was unexpected."))?;
        parse(&report)
    }
}

#[cfg(target_os = "macos")]
fn token() -> Option<String> {
    super::claude::keychain_secret("cursor-access-token")
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
}
/// Elsewhere the CLI keeps its token where Gespan does not read it yet.
#[cfg(not(target_os = "macos"))]
fn token() -> Option<String> {
    None
}

fn request(token: &str, method: &str) -> Result<Value, FetchError> {
    // The token rides in a header: refuse any redirect that drops to plain HTTP.
    let agent = crate::net::agent()
        .timeout(HTTP_TIMEOUT)
        .https_only(true)
        .build();
    match agent
        .post(&format!("{API}/{method}"))
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .set("Connect-Protocol-Version", "1")
        .send_string("{}")
    {
        Ok(response) => response
            .into_string()
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .ok_or_else(|| FetchError::failed("Cursor usage report was unexpected.")),
        Err(ureq::Error::Status(401 | 403, _)) => Err(FetchError::unavailable(
            "Cursor sign-in expired. Run agent login, then refresh usage.",
        )),
        Err(ureq::Error::Status(status, _)) => Err(FetchError::failed(format!(
            "Cursor usage request failed ({status})"
        ))),
        Err(_) => Err(FetchError::failed("Cursor usage could not be reached.")),
    }
}

/// Connect's JSON writes 64-bit numbers as strings and leaves out zeros.
fn number(value: &Value, key: &str) -> Option<f64> {
    match value.get(key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|n| n.is_finite())
}

/// Reads `{"usage": GetCurrentPeriodUsage, "plan": GetPlanInfo, "hardLimit":
/// GetHardLimit}`: the included usage as one window over its period, split
/// into Auto and API like Cursor's own `/usage`, plus on-demand spend.
fn parse(report: &Value) -> Result<Snapshot, FetchError> {
    let unexpected = || FetchError::failed("Cursor usage report was unexpected.");
    let usage = report
        .get("usage")
        .filter(|v| v.is_object())
        .ok_or_else(unexpected)?;
    let plan_usage = usage.get("planUsage").ok_or_else(unexpected)?;
    let info = report.get("plan").and_then(|p| p.get("planInfo"));
    let minutes = match info
        .and_then(|i| i.get("includedUsagePeriod"))
        .and_then(Value::as_str)
    {
        Some("INCLUDED_USAGE_PERIOD_WEEKLY") => 10_080,
        Some("INCLUDED_USAGE_PERIOD_DAILY") => 1_440,
        _ => 43_200,
    };
    let resets_at = info
        .and_then(|i| number(i, "includedUsageResetsAt"))
        .or_else(|| number(usage, "billingCycleEnd"))
        .map(|at| at as i64)
        .filter(|at| *at > 0);
    let percent = |key: &str| number(plan_usage, key).map(|n| n.clamp(0., 100.));
    let auto = percent("autoPercentUsed");
    let api = percent("apiPercentUsed");
    let total = percent("totalPercentUsed")
        .or_else(|| {
            let spent = number(plan_usage, "includedSpend")?;
            let limit = number(plan_usage, "limit").filter(|l| *l > 0.)?;
            Some((spent / limit * 100.).clamp(0., 100.))
        })
        // An unused plan leaves every percent out.
        .unwrap_or(0.);
    let mut windows = vec![Window::new(total, minutes, resets_at)];
    if auto.is_some() || api.is_some() {
        for (scope, used) in [("Auto", auto), ("API", api)] {
            windows.push(Window {
                scope: Some(scope.into()),
                ..Window::new(used.unwrap_or(0.), minutes, resets_at)
            });
        }
    }
    Ok(Snapshot {
        windows,
        updated_at: now_ms(),
        plan: info
            .and_then(|i| i.get("planName"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(String::from),
        extra_usage: on_demand(usage, report.get("hardLimit")),
        ..Default::default()
    })
}

/// Spend beyond the plan, when on-demand usage is turned on. Cents in the
/// usage report, dollars in the hard limit.
fn on_demand(usage: &Value, hard_limit: Option<&Value>) -> Option<Amount> {
    let spend = usage.get("spendLimitUsage");
    let used = spend
        .and_then(|s| number(s, "individualUsed"))
        .unwrap_or(0.)
        / 100.;
    let cents = spend.and_then(|s| number(s, "individualLimit"));
    let limit = match (cents, hard_limit) {
        (Some(cents), _) if cents > 0. => Some(cents / 100.),
        (Some(_), _) => return None,
        (None, Some(hard)) => {
            if hard.get("noUsageBasedAllowed").and_then(Value::as_bool) == Some(true) {
                return None;
            }
            match number(hard, "hardLimit").map(|n| n as i64) {
                Some(n) if n >= NO_LIMIT => None,
                Some(n) if n > 0 => Some(n as f64),
                _ => return None,
            }
        }
        (None, None) => return None,
    };
    Some(Amount {
        used,
        limit,
        unit: "USD".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(usage: Value, plan: Value, hard_limit: Value) -> Value {
        json!({ "usage": usage, "plan": plan, "hardLimit": hard_limit })
    }
    fn weekly_premium() -> Value {
        json!({ "planInfo": {
            "planName": "Premium",
            "includedUsagePeriod": "INCLUDED_USAGE_PERIOD_WEEKLY",
            "includedUsageResetsAt": "1794907595093",
            "billingCycleEnd": "1795166687982"
        }})
    }

    #[test]
    fn reads_the_weekly_allowance_split_into_auto_and_api() {
        // Captured from a real Premium account; zeros are left out.
        let usage = json!({
            "billingCycleStart": "1794302795093",
            "billingCycleEnd": "1794907595093",
            "planUsage": {
                "remainingBonus": false,
                "autoPercentUsed": 0.13001083423618634,
                "totalPercentUsed": 0.13001083423618634
            },
            "spendLimitUsage": { "limitType": "user" },
            "enabled": true
        });
        let snapshot = parse(&report(
            usage,
            weekly_premium(),
            json!({ "hardLimit": 2147483647 }),
        ))
        .unwrap();
        assert_eq!(snapshot.plan.as_deref(), Some("Premium"));
        let [total, auto, api] = &snapshot.windows[..] else {
            panic!("{:?}", snapshot.windows);
        };
        assert_eq!((total.minutes, total.scope.as_deref()), (10_080, None));
        assert_eq!(total.resets_at, Some(1794907595093));
        assert!((total.used_percent - 0.13).abs() < 0.01);
        assert_eq!(auto.scope.as_deref(), Some("Auto"));
        assert!((auto.used_percent - 0.13).abs() < 0.01);
        assert_eq!((api.scope.as_deref(), api.used_percent), (Some("API"), 0.));
        // Only the plan-wide window decides the account's headroom.
        assert_eq!(snapshot.plan_windows().count(), 1);
        // On-demand is on with no cap: nothing spent, no limit.
        let spend = snapshot.extra_usage.unwrap();
        assert_eq!((spend.used, spend.limit), (0., None));
        assert_eq!(spend.label(), "$0.00");
    }

    #[test]
    fn falls_back_to_dollars_and_the_billing_cycle_without_plan_info() {
        let usage = json!({
            "billingCycleEnd": "1795166687982",
            "planUsage": { "includedSpend": 1500, "limit": 2000, "totalSpend": 1500 },
            "spendLimitUsage": { "individualUsed": 1234, "individualLimit": 5000 }
        });
        let snapshot = parse(&json!({ "usage": usage })).unwrap();
        assert_eq!(snapshot.plan, None);
        assert_eq!(snapshot.windows.len(), 1, "no Auto/API split reported");
        let window = &snapshot.windows[0];
        assert_eq!((window.minutes, window.used_percent), (43_200, 75.));
        assert_eq!(window.resets_at, Some(1795166687982));
        let spend = snapshot.extra_usage.unwrap();
        assert_eq!(spend.label(), "$12.34 of $50.00");
    }

    #[test]
    fn an_unused_plan_reads_as_empty_and_overflow_is_clamped() {
        let unused = parse(&report(
            json!({ "planUsage": {} }),
            weekly_premium(),
            json!({}),
        ))
        .unwrap();
        assert_eq!(unused.windows[0].used_percent, 0.);
        assert!(unused.extra_usage.is_none(), "no hard limit, no on-demand");
        let over =
            parse(&json!({ "usage": { "planUsage": { "totalPercentUsed": 140.5 } } })).unwrap();
        assert_eq!(over.windows[0].used_percent, 100.);
        assert_eq!(over.windows[0].resets_at, None);
    }

    #[test]
    fn on_demand_hides_when_it_is_off() {
        let usage = json!({ "planUsage": {}, "spendLimitUsage": { "limitType": "user" } });
        let off = |hard: Value| parse(&report(usage.clone(), json!({}), hard)).unwrap();
        assert!(off(json!({ "noUsageBasedAllowed": true, "hardLimit": 20 }))
            .extra_usage
            .is_none());
        assert!(off(json!({ "hardLimit": 0 })).extra_usage.is_none());
        assert_eq!(
            off(json!({ "hardLimit": 20 })).extra_usage.unwrap().label(),
            "$0.00 of $20.00"
        );
    }

    #[test]
    fn a_report_without_plan_usage_is_unexpected() {
        for raw in [
            json!({}),
            json!({ "usage": {} }),
            json!({ "usage": "oops" }),
        ] {
            let error = parse(&raw).unwrap_err();
            assert!(!error.unavailable, "{raw}");
        }
        assert!(Cursor.parse("not json").is_err());
    }
}
