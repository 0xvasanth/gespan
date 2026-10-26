//! A card's agent fields (provider, model, cost, run time), computed from the
//! sessions linked to it each time the board loads. They are never stored,
//! and show only on the card face (per the display options) and as table
//! columns.
use gespan_core::agent::Settings;

/// What the app knows of one linked session.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AgentFacts {
    pub provider: String,
    pub model: Option<String>,
    pub run_time_ms: Option<u64>,
    /// No session records a cost yet; kept so a source can fill it later.
    pub cost_usd: Option<f64>,
}

/// The values a card shows; a field nothing can answer is `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AgentValues {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub cost: Option<String>,
    pub run_time: Option<String>,
}

impl AgentValues {
    /// The value of one field by its display-options key.
    pub(crate) fn get(&self, field: &str) -> Option<&str> {
        match field {
            "provider" => self.provider.as_deref(),
            "model" => self.model.as_deref(),
            "cost" => self.cost.as_deref(),
            "runTime" => self.run_time.as_deref(),
            _ => None,
        }
    }
}

pub(crate) fn label(field: &str) -> &'static str {
    match field {
        "provider" => "Provider",
        "model" => "Model",
        "cost" => "Cost",
        "runTime" => "Run time",
        _ => "",
    }
}

pub(crate) fn format_run_time(ms: u64) -> String {
    let minutes = (ms as f64 / 60_000.).round() as u64;
    if minutes < 1 {
        return "<1m".into();
    }
    let (hours, rest) = (minutes / 60, minutes % 60);
    match (hours, rest) {
        (0, rest) => format!("{rest}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, rest) => format!("{hours}h {rest}m"),
    }
}

fn format_cost(usd: f64) -> String {
    if usd < 0.01 {
        "<$0.01".into()
    } else {
        format!("${usd:.2}")
    }
}

fn provider_label(id: &str) -> String {
    gespan_core::provider::Kind::parse(id)
        .map(|kind| kind.label().to_string())
        .unwrap_or_else(|| id.to_string())
}

/// Providers and models joined without repeats; run time and cost summed.
pub(crate) fn agent_values(sessions: &[AgentFacts]) -> AgentValues {
    let mut providers: Vec<String> = Vec::new();
    let mut models: Vec<String> = Vec::new();
    for session in sessions {
        let provider = provider_label(&session.provider);
        if !providers.contains(&provider) {
            providers.push(provider);
        }
        if let Some(model) = session.model.as_ref().filter(|m| !m.trim().is_empty())
            && !models.contains(model)
        {
            models.push(model.clone());
        }
    }
    let times: Vec<u64> = sessions.iter().filter_map(|s| s.run_time_ms).collect();
    let costs: Vec<f64> = sessions.iter().filter_map(|s| s.cost_usd).collect();
    AgentValues {
        provider: (!providers.is_empty()).then(|| providers.join(", ")),
        model: (!models.is_empty()).then(|| models.join(", ")),
        cost: (!costs.is_empty()).then(|| format_cost(costs.iter().sum())),
        run_time: (!times.is_empty()).then(|| format_run_time(times.iter().sum())),
    }
}

/// One session's facts from its summary and saved settings. The model is the
/// settings' model, else the newest turn's. Run time sums the measured turns
/// (a steer joined to a turn is not counted twice); without any, it is the
/// span from the first turn's start to the session's last update.
pub(crate) fn facts(provider: &str, updated_at: u64, settings: Option<&Settings>) -> AgentFacts {
    let turns: Vec<_> = settings
        .map(|settings| {
            let mut turns: Vec<_> = settings.turns.iter().collect();
            turns.sort_by_key(|(index, _)| **index);
            turns.into_iter().map(|(_, turn)| turn).collect()
        })
        .unwrap_or_default();
    let model = settings
        .and_then(|settings| settings.model.clone())
        .filter(|model| !model.trim().is_empty())
        .or_else(|| {
            turns
                .iter()
                .rev()
                .map(|turn| turn.model.clone())
                .find(|model| !model.trim().is_empty())
        });
    let measured: Vec<u64> = turns
        .iter()
        .filter(|turn| turn.joined_turn.is_none() && turn.has_duration())
        .map(|turn| turn.duration_ms)
        .collect();
    let run_time_ms = if measured.is_empty() {
        turns
            .iter()
            .filter_map(|turn| turn.started_at)
            .min()
            .filter(|start| updated_at > *start)
            .map(|start| updated_at - start)
    } else {
        Some(measured.iter().sum())
    };
    AgentFacts {
        provider: provider.to_string(),
        model,
        run_time_ms,
        cost_usd: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::agent::TurnInfo;

    #[test]
    fn run_time_reads_minutes_and_hours() {
        assert_eq!(format_run_time(20_000), "<1m");
        assert_eq!(format_run_time(5 * 60_000), "5m");
        assert_eq!(format_run_time(120 * 60_000), "2h");
        assert_eq!(format_run_time(135 * 60_000), "2h 15m");
    }

    #[test]
    fn facts_sum_measured_turns_and_skip_joined_steers() {
        let mut settings = Settings::default();
        let turn = |ms: u64, joined: Option<usize>| TurnInfo {
            model: "gpt-5".into(),
            duration_ms: ms,
            duration_known: true,
            joined_turn: joined,
            ..TurnInfo::default()
        };
        settings.turns.insert(0, turn(60_000, None));
        settings.turns.insert(1, turn(120_000, None));
        settings.turns.insert(2, turn(999_000, Some(1)));
        let facts = facts("codex", 0, Some(&settings));
        assert_eq!(facts.run_time_ms, Some(180_000));
        assert_eq!(facts.model.as_deref(), Some("gpt-5"));
        assert_eq!(facts.cost_usd, None);
    }

    #[test]
    fn facts_without_timing_fall_back_to_first_start_until_last_update() {
        let mut settings = Settings::default();
        settings.turns.insert(
            0,
            TurnInfo {
                started_at: Some(1_000),
                ..TurnInfo::default()
            },
        );
        assert_eq!(
            facts("codex", 61_000, Some(&settings)).run_time_ms,
            Some(60_000)
        );
        assert_eq!(facts("codex", 61_000, None).run_time_ms, None);
    }

    #[test]
    fn values_join_distinct_providers_and_leave_cost_empty() {
        let values = agent_values(&[
            AgentFacts {
                provider: "codex".into(),
                model: Some("gpt-5".into()),
                run_time_ms: Some(60_000),
                cost_usd: None,
            },
            AgentFacts {
                provider: "codex".into(),
                model: Some("gpt-5".into()),
                run_time_ms: Some(120_000),
                cost_usd: None,
            },
        ]);
        assert_eq!(values.provider.as_deref(), Some("Codex"));
        assert_eq!(values.model.as_deref(), Some("gpt-5"));
        assert_eq!(values.run_time.as_deref(), Some("3m"));
        assert_eq!(values.cost, None);
        assert_eq!(agent_values(&[]), AgentValues::default());
    }
}
