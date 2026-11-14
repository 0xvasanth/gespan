//! The React Pilot habit schedule, claiming and run-history policy.
use super::files;
use chrono::{DateTime, Datelike, Duration, Local, Timelike};
use serde::{Deserialize, Serialize};
use std::path::Path;
pub const MAX: usize = 20;
pub const GRACE_MS: u64 = 2 * 60 * 60 * 1000;
pub const TIMEOUT_MS: u64 = 60 * 60 * 1000;
pub const APPROVAL_WAIT_MS: u64 = 12 * 60 * 60 * 1000;
pub const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    #[serde(alias = "kind")]
    pub schedule_kind: String,
    #[serde(default)]
    pub minute: u32,
    #[serde(default = "nine")]
    pub time: String,
    #[serde(default = "monday")]
    pub day_of_week: u32,
}
fn nine() -> String {
    "09:00".into()
}
fn monday() -> u32 {
    1
}
impl Default for Schedule {
    fn default() -> Self {
        Self {
            schedule_kind: "daily".into(),
            minute: 0,
            time: nine(),
            day_of_week: 1,
        }
    }
}
impl Schedule {
    pub fn validate(&self) -> Result<(), String> {
        let (h, m) = self.time.split_once(':').ok_or("Time must be HH:MM")?;
        if !["hourly", "daily", "weekdays", "weekly"].contains(&self.schedule_kind.as_str())
            || h.len() != 2
            || m.len() != 2
            || h.parse::<u32>().ok().is_none_or(|h| h > 23)
            || m.parse::<u32>().ok().is_none_or(|m| m > 59)
            || self.minute > 59
            || self.day_of_week > 6
        {
            return Err("Choose hourly, daily, weekdays or weekly, a valid 24-hour time, minute 0–59 and weekday 0–6".into());
        }
        Ok(())
    }
    pub fn label(&self) -> String {
        match self.schedule_kind.as_str() {
            "hourly" => format!("Hourly at :{:02}", self.minute),
            "weekdays" => format!("Weekdays at {}", self.time),
            "weekly" => format!(
                "{}s at {}",
                WEEKDAYS[self.day_of_week.min(6) as usize],
                self.time
            ),
            _ => format!("Daily at {}", self.time),
        }
    }
    /// How often it repeats, after "then": "every day", "every Monday".
    pub fn cadence(&self) -> String {
        match self.schedule_kind.as_str() {
            "hourly" => "every hour".into(),
            "weekdays" => "every weekday".into(),
            "weekly" => format!("every {}", WEEKDAYS[self.day_of_week.min(6) as usize]),
            _ => "every day".into(),
        }
    }
    /// When `at` falls, said from `now`: "tonight at 02:00", "today at
    /// 14:00", "tomorrow at 09:00", else "Mon, Nov 16 at 09:00". Late
    /// evening and the small hours of the next morning are both tonight.
    pub fn when(at: u64, now: u64) -> String {
        let local =
            |ms: u64| DateTime::from_timestamp_millis(ms as i64).map(|d| d.with_timezone(&Local));
        let (Some(at), Some(now)) = (local(at), local(now)) else {
            return String::new();
        };
        let time = at.format("%H:%M");
        let days = (at.date_naive() - now.date_naive()).num_days();
        match days {
            0 if at.hour() >= 18 || (at.hour() < 6 && now.hour() < 6) => {
                format!("tonight at {time}")
            }
            0 => format!("today at {time}"),
            1 if at.hour() < 6 && now.hour() >= 18 => format!("tonight at {time}"),
            1 => format!("tomorrow at {time}"),
            _ => format!("{} at {time}", at.format("%a, %b %-d")),
        }
    }
    pub fn next(&self, after: u64) -> Result<u64, String> {
        self.validate()?;
        let start = DateTime::from_timestamp_millis(after as i64)
            .ok_or("Invalid schedule date")?
            .with_timezone(&Local);
        let hour = self.time[..2].parse::<u32>().unwrap();
        let minute = self.time[3..].parse::<u32>().unwrap();
        // Iterate real minutes across DST transitions. Wall-clock predicates
        // avoid invalid or duplicated local-time arithmetic.
        let mut at =
            start.with_second(0).unwrap().with_nanosecond(0).unwrap() + Duration::minutes(1);
        for _ in 0..8 * 24 * 60 {
            let day = at.weekday().num_days_from_sunday();
            let matches = if self.schedule_kind == "hourly" {
                at.minute() == self.minute
            } else {
                at.hour() == hour
                    && at.minute() == minute
                    && match self.schedule_kind.as_str() {
                        "weekdays" => (1..=5).contains(&day),
                        "weekly" => day == self.day_of_week,
                        _ => true,
                    }
            };
            if matches {
                return Ok(at.timestamp_millis().max(0) as u64);
            }
            at += Duration::minutes(1);
        }
        Err("No next habit run found".into())
    }
}
/// What a scheduled run starts. Habits saved before this field read as an
/// instruction, which is what every habit used to be.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Target {
    /// The Pilot's Flow from its first step, as New run does.
    #[serde(rename_all = "camelCase")]
    Pipeline { flow_id: String },
    /// One step of the Pilot's Flow on its own.
    #[serde(rename_all = "camelCase")]
    Step { flow_id: String, step_id: String },
    /// A hidden run that follows `Habit::instructions`.
    #[default]
    Instruction,
}
impl Target {
    pub fn flow_id(&self) -> Option<&str> {
        match self {
            Self::Pipeline { flow_id } | Self::Step { flow_id, .. } => Some(flow_id),
            Self::Instruction => None,
        }
    }
    fn valid(&self) -> bool {
        match self {
            Self::Pipeline { flow_id } => !flow_id.trim().is_empty(),
            Self::Step { flow_id, step_id } => {
                !flow_id.trim().is_empty() && !step_id.trim().is_empty()
            }
            Self::Instruction => true,
        }
    }
}
/// When a scheduled run reports back in the chat. Habits saved before this
/// field read as `EveryRun`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tell {
    #[default]
    EveryRun,
    /// Only a run that needs the person, such as one that failed or found
    /// something to act on.
    OnlyIfNeeded,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Habit {
    pub id: String,
    pub name: String,
    /// The instruction a `Target::Instruction` run follows; empty for a
    /// Flow target.
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub target: Target,
    #[serde(default)]
    pub tell: Tell,
    pub schedule: Schedule,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub next_run_at: u64,
    pub last_run_at: Option<u64>,
    pub last_outcome: Option<String>,
    pub last_error: Option<String>,
    #[serde(default)]
    pub runs: Vec<Run>,
    #[serde(default)]
    pub run_requested: bool,
    #[serde(default)]
    pub running_since: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_card: Option<String>,
}
fn yes() -> bool {
    true
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub at: u64,
    #[serde(default)]
    pub duration_ms: u64,
    pub outcome: String,
    pub report: Option<String>,
    pub error: Option<String>,
}
impl Habit {
    /// An instruction habit that reports every run.
    pub fn new(
        name: String,
        instructions: String,
        schedule: Schedule,
        now: u64,
    ) -> Result<Self, String> {
        Self::create(
            name,
            Target::Instruction,
            instructions,
            Tell::EveryRun,
            schedule,
            now,
        )
    }
    pub fn create(
        name: String,
        target: Target,
        instructions: String,
        tell: Tell,
        schedule: Schedule,
        now: u64,
    ) -> Result<Self, String> {
        check(&name, &target, &instructions)?;
        let next_run_at = schedule.next(now)?;
        let instructions = match target {
            Target::Instruction => instructions.trim().into(),
            _ => String::new(),
        };
        Ok(Self {
            id: super::new_id(),
            name: name.trim().into(),
            instructions,
            target,
            tell,
            schedule,
            enabled: true,
            created_at: now,
            next_run_at,
            last_run_at: None,
            last_outcome: None,
            last_error: None,
            runs: Vec::new(),
            run_requested: false,
            running_since: None,
            from_card: None,
        })
    }
    /// Replaces what the person can edit. A changed schedule moves the next
    /// run; an unchanged one keeps it, so saving never skips a run.
    pub fn edit(
        &mut self,
        name: String,
        target: Target,
        instructions: String,
        tell: Tell,
        schedule: Schedule,
        now: u64,
    ) -> Result<(), String> {
        check(&name, &target, &instructions)?;
        if schedule != self.schedule || self.next_run_at == 0 {
            self.next_run_at = schedule.next(now)?;
        } else {
            schedule.validate()?;
        }
        self.name = name.trim().into();
        self.instructions = match target {
            Target::Instruction => instructions.trim().into(),
            _ => String::new(),
        };
        self.target = target;
        self.tell = tell;
        self.schedule = schedule;
        Ok(())
    }
    pub fn due(&self, now: u64) -> bool {
        self.running_since.is_none()
            && (self.run_requested
                || self.enabled && self.next_run_at <= now && now - self.next_run_at <= GRACE_MS)
    }
    pub fn finish(&mut self, run: Run) -> Result<(), String> {
        self.last_run_at = Some(run.at);
        self.last_outcome = Some(run.outcome.clone());
        self.last_error = run.error.clone();
        self.running_since = None;
        self.runs.insert(0, run);
        self.runs.truncate(20);
        Ok(())
    }
}
fn check(name: &str, target: &Target, instructions: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.chars().count() > 120 {
        return Err("A scheduled run needs a name".into());
    }
    if !target.valid() {
        return Err("Pick what this scheduled run starts".into());
    }
    if *target == Target::Instruction
        && (instructions.trim().is_empty() || instructions.len() > 32000)
    {
        return Err("A scheduled run needs an instruction".into());
    }
    Ok(())
}
pub fn load(root: &Path, id: &str) -> Result<Vec<Habit>, String> {
    let file = files::read(root, id, None, "habits.json")?;
    parse(file.text.as_deref())
}
fn parse(text: Option<&str>) -> Result<Vec<Habit>, String> {
    let Some(text) = text.filter(|t| !t.trim().is_empty()) else {
        return Ok(Vec::new());
    };
    let values: Vec<serde_json::Value> =
        serde_json::from_str(text).map_err(|e| format!("Cannot read habits: {e}"))?;
    Ok(values
        .into_iter()
        .filter_map(|mut value| {
            // The desktop parser accepts each recent run independently. A bad
            // history item must not make the habit (and its controls) disappear.
            if let Some(items) = value.get_mut("runs") {
                let runs: Vec<Run> = items
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|run| serde_json::from_value::<Run>(run.clone()).ok())
                    .filter(|run| {
                        ["posted", "quiet", "failed", "interrupted"].contains(&run.outcome.as_str())
                    })
                    .take(20)
                    .collect();
                *items = serde_json::to_value(runs).ok()?;
            }
            serde_json::from_value::<Habit>(value).ok()
        })
        .filter(|h| {
            !h.id.trim().is_empty()
                && !h.name.trim().is_empty()
                && h.target.valid()
                && (h.target != Target::Instruction || !h.instructions.trim().is_empty())
                && h.schedule.validate().is_ok()
        })
        .take(MAX)
        .collect())
}
/// Habits that came due while the app was on hold run once now, instead of
/// being skipped as too old or run once per missed slot.
pub fn catch_up(all: &mut [Habit], since: u64, now: u64) {
    for habit in all.iter_mut() {
        if habit.enabled && (since..=now).contains(&habit.next_run_at) {
            habit.next_run_at = now;
        }
    }
}
pub fn update<T>(
    root: &Path,
    id: &str,
    change: impl Fn(&mut Vec<Habit>) -> Result<T, String>,
) -> Result<T, String> {
    for _ in 0..5 {
        let file = files::read(root, id, None, "habits.json")?;
        let mut habits = parse(file.text.as_deref())?;
        let result = change(&mut habits)?;
        if habits.len() > MAX {
            return Err("At most 20 habits per Pilot".into());
        }
        let text = serde_json::to_string_pretty(&habits).map_err(|e| e.to_string())? + "\n";
        match files::save(root, id, None, "habits.json", &text, Some(&file.hash)) {
            Ok(_) => return Ok(result),
            Err(e) if e == files::CONFLICT => continue,
            Err(e) => return Err(e),
        }
    }
    Err("Habits kept changing; try again".into())
}
pub fn report(text: &str) -> Option<String> {
    let t = text.trim();
    let simple = t
        .chars()
        .filter(|c| !c.is_whitespace() && !"`*.".contains(*c))
        .collect::<String>();
    if simple == "NOTHING_TO_REPORT" {
        return None;
    }
    let text = t.trim_end_matches("NOTHING_TO_REPORT").trim();
    (!text.is_empty()).then(|| text.chars().take(4000).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hourly(next_run_at: u64, enabled: bool) -> Habit {
        let mut habit = Habit::new(
            "Heartbeat".into(),
            "Check the epic".into(),
            Schedule {
                schedule_kind: "hourly".into(),
                minute: 5,
                time: "09:00".into(),
                day_of_week: 1,
            },
            0,
        )
        .unwrap();
        habit.next_run_at = next_run_at;
        habit.enabled = enabled;
        habit
    }

    #[test]
    fn habits_due_during_a_hold_run_once_when_it_ends() {
        let hour = 3_600_000;
        let since = 10 * hour;
        let now = since + 3 * hour;
        let mut all = vec![
            hourly(since + hour, true),     // missed during the hold
            hourly(since - 5 * hour, true), // overdue before the hold: untouched
            hourly(now + hour, true),       // not due yet
            hourly(since + 2 * hour, false),
        ];
        catch_up(&mut all, since, now);
        assert_eq!(all[0].next_run_at, now);
        assert!(
            all[0].due(now),
            "a single catch-up run, not one per missed slot"
        );
        assert_eq!(all[1].next_run_at, since - 5 * hour);
        assert_eq!(all[2].next_run_at, now + hour);
        assert_eq!(all[3].next_run_at, since + 2 * hour);
    }

    #[test]
    fn desktop_habit_optional_timestamps_and_partial_history_remain_editable() {
        let text = r#"[{"id":"desktop","name":"Digest","instructions":"Check the news",
            "schedule":{"scheduleKind":"daily"},"fromCard":"suggestion-1",
            "runs":[{"at":100,"outcome":"posted","report":"**News**"},
                    {"at":"bad","outcome":"failed"},
                    {"at":80,"outcome":"quiet"}]}]"#;
        let habits = parse(Some(text)).unwrap();
        assert_eq!(habits.len(), 1);
        let h = &habits[0];
        assert_eq!(h.created_at, 0);
        assert_eq!(h.next_run_at, 0);
        assert_eq!(h.runs.len(), 2);
        assert_eq!(h.runs[0].duration_ms, 0);
        assert_eq!(h.runs[0].report.as_deref(), Some("**News**"));
        assert_eq!(h.from_card.as_deref(), Some("suggestion-1"));
        let serialized = serde_json::to_string(&habits).unwrap();
        assert_eq!(parse(Some(&serialized)).unwrap()[0].from_card, h.from_card);
    }

    #[test]
    fn malformed_history_does_not_hide_an_otherwise_valid_habit() {
        let text = r#"[{"id":"desktop","name":"Digest","instructions":"Check updates",
            "schedule":{"scheduleKind":"hourly","minute":15},"runs":null}]"#;
        let habits = parse(Some(text)).unwrap();
        assert_eq!(habits.len(), 1);
        assert!(habits[0].runs.is_empty());
    }

    #[test]
    fn habits_saved_before_targets_read_as_an_instruction_that_tells_every_run() {
        let text = r#"[{"id":"old","name":"Digest","instructions":"Check the news",
            "schedule":{"scheduleKind":"weekly","time":"09:00","dayOfWeek":1},
            "enabled":true,"createdAt":1,"nextRunAt":2,"lastRunAt":null,
            "lastOutcome":null,"lastError":null,"runs":[]}]"#;
        let habits = parse(Some(text)).unwrap();
        assert_eq!(habits.len(), 1);
        assert_eq!(habits[0].target, Target::Instruction);
        assert_eq!(habits[0].tell, Tell::EveryRun);
        assert_eq!(habits[0].instructions, "Check the news");
        assert_eq!(habits[0].schedule.label(), "Mondays at 09:00");
        // Saving writes both fields, and they read back the same.
        let saved = serde_json::to_value(&habits).unwrap();
        assert_eq!(
            saved[0]["target"],
            serde_json::json!({"kind": "instruction"})
        );
        assert_eq!(saved[0]["tell"], "everyRun");
        let again = parse(Some(&saved.to_string())).unwrap();
        assert_eq!(again[0].target, Target::Instruction);
    }

    #[test]
    fn flow_targets_need_no_instruction_but_do_need_their_ids() {
        let text = r#"[
            {"id":"a","name":"Release","schedule":{"scheduleKind":"daily"},
             "target":{"kind":"pipeline","flowId":"release"},"tell":"onlyIfNeeded",
             "lastRunAt":null,"lastOutcome":null,"lastError":null},
            {"id":"b","name":"Build","schedule":{"scheduleKind":"daily"},
             "target":{"kind":"step","flowId":"release","stepId":"build"},
             "lastRunAt":null,"lastOutcome":null,"lastError":null},
            {"id":"c","name":"No step","schedule":{"scheduleKind":"daily"},
             "target":{"kind":"step","flowId":"release","stepId":" "},
             "lastRunAt":null,"lastOutcome":null,"lastError":null},
            {"id":"d","name":"No words","schedule":{"scheduleKind":"daily"},
             "target":{"kind":"instruction"},"instructions":"  ",
             "lastRunAt":null,"lastOutcome":null,"lastError":null}]"#;
        let habits = parse(Some(text)).unwrap();
        let ids: Vec<_> = habits.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
        assert_eq!(habits[0].tell, Tell::OnlyIfNeeded);
        assert_eq!(habits[0].target.flow_id(), Some("release"));
        assert_eq!(
            habits[1].target,
            Target::Step {
                flow_id: "release".into(),
                step_id: "build".into()
            }
        );
    }

    #[test]
    fn creating_and_editing_check_what_the_target_needs() {
        let pipeline = Target::Pipeline {
            flow_id: "release".into(),
        };
        let h = Habit::create(
            "Nightly".into(),
            pipeline.clone(),
            "left over from an instruction".into(),
            Tell::OnlyIfNeeded,
            Schedule::default(),
            0,
        )
        .unwrap();
        assert_eq!(h.instructions, "", "a Flow target keeps no instruction");
        assert!(Habit::create(
            "Nightly".into(),
            Target::Instruction,
            " ".into(),
            Tell::EveryRun,
            Schedule::default(),
            0
        )
        .is_err());
        assert!(Habit::create(
            "Nightly".into(),
            Target::Step {
                flow_id: "release".into(),
                step_id: String::new()
            },
            String::new(),
            Tell::EveryRun,
            Schedule::default(),
            0
        )
        .is_err());

        let mut h = h;
        h.next_run_at = 42;
        // Same schedule: the next run stays, so a save never skips one.
        h.edit(
            " Nightly check ".into(),
            Target::Instruction,
            "Check main".into(),
            Tell::EveryRun,
            Schedule::default(),
            1_000,
        )
        .unwrap();
        assert_eq!(h.next_run_at, 42);
        assert_eq!(h.name, "Nightly check");
        assert_eq!(h.instructions, "Check main");
        assert_eq!(h.tell, Tell::EveryRun);
        // A new time moves it.
        let at_two = Schedule {
            time: "02:00".into(),
            ..Schedule::default()
        };
        h.edit(
            "Nightly check".into(),
            pipeline,
            "Check main".into(),
            Tell::OnlyIfNeeded,
            at_two.clone(),
            1_000,
        )
        .unwrap();
        assert_eq!(h.next_run_at, at_two.next(1_000).unwrap());
        assert_eq!(h.instructions, "");
        // A bad edit changes nothing.
        let before = h.name.clone();
        assert!(h
            .edit(
                "".into(),
                Target::Instruction,
                "x".into(),
                Tell::EveryRun,
                at_two,
                1_000
            )
            .is_err());
        assert_eq!(h.name, before);
    }

    #[test]
    fn the_next_run_reads_as_tonight_today_tomorrow_or_a_date() {
        use chrono::TimeZone;
        let at = |day, hour, minute| {
            Local
                .with_ymd_and_hms(2026, 11, day, hour, minute, 0)
                .single()
                .unwrap()
                .timestamp_millis() as u64
        };
        let evening = at(14, 21, 0);
        assert_eq!(Schedule::when(at(15, 2, 0), evening), "tonight at 02:00");
        assert_eq!(Schedule::when(at(14, 23, 0), evening), "tonight at 23:00");
        assert_eq!(Schedule::when(at(15, 9, 0), evening), "tomorrow at 09:00");
        let morning = at(14, 8, 0);
        assert_eq!(Schedule::when(at(14, 14, 30), morning), "today at 14:30");
        assert_eq!(Schedule::when(at(15, 2, 0), morning), "tomorrow at 02:00");
        assert_eq!(
            Schedule::when(at(14, 3, 0), at(14, 1, 0)),
            "tonight at 03:00"
        );
        assert_eq!(
            Schedule::when(at(16, 9, 0), morning),
            "Mon, Nov 16 at 09:00"
        );
        let weekly = Schedule {
            schedule_kind: "weekly".into(),
            day_of_week: 1,
            ..Schedule::default()
        };
        assert_eq!(weekly.cadence(), "every Monday");
        assert_eq!(Schedule::default().cadence(), "every day");
        assert_eq!(
            Schedule {
                schedule_kind: "hourly".into(),
                ..Schedule::default()
            }
            .cadence(),
            "every hour"
        );
    }
}
