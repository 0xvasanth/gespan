//! Context-window accounting, matching sessions/model/contextUsage.ts.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub used: u64,
    pub window: Option<u64>,
    /// The share in use, from providers such as Kiro that report no tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<f64>,
}
impl Usage {
    pub fn ratio(self) -> Option<f32> {
        self.window
            .filter(|w| *w > 0)
            .map(|w| (self.used as f64 / w as f64).min(1.) as f32)
            .or_else(|| self.percent.map(|p| (p / 100.).clamp(0., 1.) as f32))
    }
    /// Whether the reading counts tokens, rather than only a percent.
    pub fn has_tokens(self) -> bool {
        self.window.is_some_and(|w| w > 0)
    }
    /// A percent-only reading replaces any token level a previous provider
    /// left in this conversation.
    pub fn set_percent(&mut self, percent: f64) {
        *self = Self {
            used: 0,
            window: None,
            percent: Some(percent.clamp(0., 100.)),
        };
    }
    pub fn merge(&mut self, used: Option<u64>, window: Option<u64>) {
        if let Some(used) = used {
            self.used = used;
            self.percent = None;
        }
        if let Some(window) = window.filter(|w| *w > 0) {
            self.window = Some(window);
        }
    }
}

pub fn claude_used(usage: &Value) -> Option<u64> {
    let used = [
        "input_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
        "output_tokens",
    ]
    .iter()
    .map(|key| usage[key].as_u64().unwrap_or(0))
    .sum();
    (used > 0).then_some(used)
}

pub fn format_tokens(count: u64) -> String {
    let (value, suffix) = if count >= 1_000_000 {
        (count as f64 / 1_000_000., "M")
    } else if count >= 1000 {
        (count as f64 / 1000., "K")
    } else {
        return count.to_string();
    };
    if value < 10. {
        format!(
            "{}{suffix}",
            format!("{value:.1}")
                .trim_end_matches('0')
                .trim_end_matches('.')
        )
    } else {
        format!("{value:.0}{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_only_readings_fill_the_meter_until_tokens_return() {
        let mut usage = Usage {
            used: 150_000,
            window: Some(200_000),
            percent: None,
        };
        // Switching to Kiro: its percent wins over the old token level.
        usage.set_percent(1.7351999);
        assert!((usage.ratio().unwrap() - 0.017_352).abs() < 1e-6);
        assert!(!usage.has_tokens());
        // A provider counting tokens takes the meter back.
        usage.merge(Some(1000), Some(10_000));
        assert_eq!(usage.ratio(), Some(0.1));
        assert_eq!(usage.percent, None);
        usage.set_percent(250.);
        assert_eq!(usage.ratio(), Some(1.));
        // Saved readings from before the field existed still load.
        let saved: Usage = serde_json::from_str(r#"{"used":5,"window":10}"#).unwrap();
        assert_eq!(saved.ratio(), Some(0.5));
    }
}
