//! Kiro plan credits, read from the CLI's own `/usage` report.
use super::{now_ms, title_case, Amount, FetchError, Snapshot, Window};

pub(super) fn fetch_for(profile: &crate::accounts::Profile) -> Result<Snapshot, FetchError> {
    let output = crate::accounts::login::cli_output(
        crate::provider::Kind::Kiro,
        &["chat", "--no-interactive", "/usage"],
        profile,
    )
    .map_err(|message| {
        if message.contains("not found") {
            FetchError::unavailable(message)
        } else {
            FetchError::failed(message)
        }
    })?;
    parse(&output)
}

/// Reads the report `kiro-cli chat --no-interactive /usage` prints:
///
/// ```text
/// Estimated Usage | resets on 2026-12-01 | KIRO FREE
/// Credits (2.41 of 50 covered in plan), 4.8%
/// ```
pub(super) fn parse(output: &str) -> Result<Snapshot, FetchError> {
    static ANSI: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static CREDITS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let output = ANSI
        .get_or_init(|| regex::Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]").unwrap())
        .replace_all(output, "");
    let credits = CREDITS
        .get_or_init(|| {
            regex::Regex::new(
                r"(?i)credits\s*\(\s*([\d.,]+)\s+of\s+([\d.,]+)[^)]*\)(?:\s*,\s*([\d.]+)\s*%)?",
            )
            .unwrap()
        })
        .captures(&output);
    let Some(credits) = credits else {
        let lower = output.to_lowercase();
        return Err(
            if ["log in", "login", "not logged", "sign in"]
                .iter()
                .any(|s| lower.contains(s))
            {
                FetchError::unavailable("Sign in with kiro-cli login, then refresh usage.")
            } else {
                FetchError::failed("Kiro usage report was unexpected.")
            },
        );
    };
    let amount = |index: usize| {
        credits
            .get(index)?
            .as_str()
            .replace(',', "")
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite() && *n >= 0.)
    };
    let used = amount(1).ok_or_else(|| FetchError::failed("Kiro usage report was unexpected."))?;
    let limit = amount(2).filter(|n| *n > 0.);
    let percent = amount(3)
        .or_else(|| limit.map(|limit| used / limit * 100.))
        .unwrap_or(0.)
        .clamp(0., 100.);
    let header = output
        .lines()
        .find(|line| line.to_lowercase().contains("resets on"))
        .unwrap_or("");
    let resets_at = header
        .split('|')
        .find_map(|part| {
            part.trim()
                .to_lowercase()
                .strip_prefix("resets on")
                .map(|d| d.trim().to_owned())
        })
        .and_then(|day| chrono::NaiveDate::parse_from_str(&day, "%Y-%m-%d").ok())
        .and_then(|day| day.and_hms_opt(0, 0, 0))
        .map(|at| at.and_utc().timestamp_millis());
    let plan = header
        .split('|')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .nth(2)
        .map(title_case);
    let mut window = Window::new(percent, 43_200, resets_at);
    window.amount = Some(Amount {
        used,
        limit,
        unit: "credits".into(),
    });
    Ok(Snapshot {
        windows: vec![window],
        updated_at: now_ms(),
        plan,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_real_usage_report() {
        let snapshot = parse(include_str!("fixtures/kiro-usage.txt")).unwrap();
        assert_eq!(snapshot.plan.as_deref(), Some("Kiro Free"));
        let [window] = snapshot.windows.as_slice() else {
            panic!("one monthly window: {:?}", snapshot.windows)
        };
        assert_eq!(window.minutes, 43_200);
        assert_eq!(window.label(), "mo");
        assert_eq!(window.used_percent, 4.8);
        // 2026-12-01T00:00:00Z
        assert_eq!(window.resets_at, Some(1_796_083_200_000));
        assert_eq!(
            window.amount.as_ref().unwrap().label(),
            "2.41 of 50 credits"
        );
        assert!(window.scope.is_none());
    }

    #[test]
    fn colored_or_windows_output_still_parses_and_derives_a_missing_percent() {
        let report = "\x1b[1mEstimated Usage\x1b[0m | resets on 2026-12-01 | KIRO PRO\r\n\
                      Credits (1,250.5 of 2,000 covered in plan)\r\n";
        let snapshot = parse(report).unwrap();
        assert_eq!(snapshot.plan.as_deref(), Some("Kiro Pro"));
        let window = &snapshot.windows[0];
        assert!((window.used_percent - 62.525).abs() < 1e-9);
        assert_eq!(window.amount.as_ref().unwrap().limit, Some(2000.));
    }

    #[test]
    fn a_report_without_credits_is_an_error_not_zero_usage() {
        let signed_out =
            parse("You are not logged in, please log in with kiro-cli login").unwrap_err();
        assert!(signed_out.unavailable);
        let changed = parse("Estimated Usage | resets on 2026-12-01 | KIRO FREE\n").unwrap_err();
        assert!(!changed.unavailable);
        // Partial: no reset date or plan still shows the credits.
        let partial = parse("Credits (3 of 50 covered in plan), 6%").unwrap();
        assert_eq!(partial.windows[0].resets_at, None);
        assert_eq!(partial.plan, None);
        assert_eq!(partial.windows[0].used_percent, 6.);
    }
}
