use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub lyrics: crate::lyrics_view::LyricsPreferences,
    pub subtitles: crate::subtitles::SubtitlePreferences,
    pub language: String,
    pub monitor: usize,
    pub notch_column: u8,
    pub notch_bottom: bool,
    pub scale: f32,
    pub open_click: bool,
    pub minimal: bool,
    pub hide_fullscreen: bool,
    pub reduced_motion: bool,
    pub sounds: bool,
    pub startup: bool,
    pub focus_minutes: u32,
    pub short_break_minutes: u32,
    pub long_break_minutes: u32,
    pub water_goal_ml: u32,
    pub water_bottle_ml: u32,
    pub water_start_hour: u32,
    pub water_end_hour: u32,
    pub week_monday: bool,
    pub calendar_week: bool,
    pub shelf_clear: bool,
    pub hud_volume: bool,
    pub hud_brightness: bool,
    pub hud_caps: bool,
    pub hud_language: bool,
    #[serde(default = "default_show_battery")]
    pub show_battery: bool,
    pub popup_battery: bool,
    pub popup_wifi: bool,
    pub popup_bluetooth: bool,
}

fn default_show_battery() -> bool {
    crate::services::is_laptop()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lyrics: Default::default(),
            subtitles: Default::default(),
            language: "pt-BR".into(),
            monitor: 0,
            notch_column: 1,
            notch_bottom: false,
            scale: 1.0,
            open_click: false,
            minimal: false,
            hide_fullscreen: true,
            reduced_motion: false,
            sounds: true,
            startup: false,
            focus_minutes: 25,
            short_break_minutes: 5,
            long_break_minutes: 15,
            water_goal_ml: 1500,
            water_bottle_ml: 250,
            water_start_hour: 9,
            water_end_hour: 21,
            week_monday: true,
            calendar_week: false,
            shelf_clear: false,
            hud_volume: false,
            hud_brightness: false,
            hud_caps: false,
            hud_language: false,
            show_battery: default_show_battery(),
            popup_battery: true,
            popup_wifi: true,
            popup_bluetooth: true,
        }
    }
}

impl Settings {
    pub fn normalize(&mut self) {
        self.notch_column = self.notch_column.min(2);
        self.notch_bottom = false;
        self.lyrics.normalize();
        self.subtitles.normalize();
        if self.language != "en" {
            self.language = "pt-BR".into();
        }
        self.scale = if self.scale.is_finite() {
            self.scale.clamp(0.75, 2.0)
        } else {
            1.0
        };
        self.focus_minutes = self.focus_minutes.clamp(1, 240);
        self.short_break_minutes = self.short_break_minutes.clamp(1, 120);
        self.long_break_minutes = self.long_break_minutes.clamp(1, 240);
        self.water_goal_ml = self.water_goal_ml.clamp(250, 10000);
        self.water_bottle_ml = self.water_bottle_ml.clamp(50, 3000);
        self.water_start_hour = self.water_start_hour.min(23);
        self.water_end_hour = self.water_end_hour.clamp(self.water_start_hour + 1, 24);
    }
    pub fn en(&self) -> bool {
        self.language == "en"
    }
    pub fn text<'a>(&self, pt: &'a str, en: &'a str) -> &'a str {
        if self.en() {
            en
        } else {
            pt
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub due_at: Option<i64>,
    pub done: bool,
    pub color: u32,
    #[serde(default)]
    pub notified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShelfKind {
    File,
    Folder,
    Text,
    Link,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShelfItem {
    pub id: u64,
    pub kind: ShelfKind,
    pub value: String,
}
impl ShelfItem {
    pub fn label(&self) -> String {
        match self.kind {
            ShelfKind::File | ShelfKind::Folder => std::path::Path::new(&self.value)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.value.clone()),
            _ => self
                .value
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(100)
                .collect(),
        }
    }
    pub fn available(&self) -> bool {
        !matches!(self.kind, ShelfKind::File | ShelfKind::Folder)
            || std::path::Path::new(&self.value).exists()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WaterState {
    pub day: String,
    pub drinks_ml: Vec<u32>,
    /// Parallel to drinks_ml. Legacy amounts have no known timestamp.
    pub drink_times: Vec<Option<i64>>,
    pub history: BTreeMap<String, u32>,
    pub last_reminder_slot: u32,
    pub snooze_until: Option<i64>,
}
impl WaterState {
    pub fn total_ml(&self) -> u32 {
        self.drinks_ml
            .iter()
            .fold(0u32, |sum, value| sum.saturating_add(*value))
    }
    pub fn hourly_ml(&self, day: NaiveDate) -> [u32; 24] {
        let mut hours = [0u32; 24];
        for (amount, timestamp) in self.drinks_ml.iter().zip(&self.drink_times) {
            if let Some(time) = timestamp.and_then(|t| Local.timestamp_opt(t, 0).single()) {
                if time.date_naive() == day {
                    let total = &mut hours[time.hour() as usize];
                    *total = total.saturating_add(*amount);
                }
            }
        }
        hours
    }
    fn align_times(&mut self) {
        self.drink_times.resize(self.drinks_ml.len(), None);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PomodoroPhase {
    Focus,
    ShortBreak,
    LongBreak,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Pomodoro {
    pub phase: PomodoroPhase,
    pub remaining_seconds: u64,
    pub ends_at: Option<i64>,
    pub completed_focus: u32,
}
impl Default for Pomodoro {
    fn default() -> Self {
        Self {
            phase: PomodoroPhase::Focus,
            remaining_seconds: 1500,
            ends_at: None,
            completed_focus: 0,
        }
    }
}
impl Pomodoro {
    pub fn remaining(&self, now: i64) -> u64 {
        self.ends_at
            .map(|t| t.saturating_sub(now).max(0) as u64)
            .unwrap_or(self.remaining_seconds)
    }
    pub fn running(&self) -> bool {
        self.ends_at.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppState {
    pub schema_version: u32,
    pub settings: Settings,
    pub tasks: Vec<Task>,
    pub shelf: Vec<ShelfItem>,
    pub water: WaterState,
    pub pomodoro: Pomodoro,
    pub next_id: u64,
}
impl Default for AppState {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            settings: Settings::default(),
            tasks: vec![],
            shelf: vec![],
            water: WaterState::default(),
            pomodoro: Pomodoro::default(),
            next_id: 1,
        }
    }
}

#[derive(Debug, Clone)]
pub enum AppCommand {
    Settings(Box<Settings>),
    AddTask {
        title: String,
        due_at: Option<i64>,
    },
    EditTask {
        id: u64,
        title: String,
        due_at: Option<i64>,
        color: u32,
    },
    ToggleTask(u64),
    DeleteTask(u64),
    SnoozeTask {
        id: u64,
        minutes: u32,
    },
    AddShelf {
        kind: ShelfKind,
        value: String,
    },
    RemoveShelf(u64),
    ClearShelf,
    RecordDrink,
    UndoDrink,
    SnoozeWater(u32),
    ResumeWater,
    StartPomodoro,
    PausePomodoro,
    ResetPomodoro,
    SetPomodoroPhase(PomodoroPhase),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    Changed,
    Reminder { title: String, body: String },
}

impl AppState {
    fn id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }
    pub fn normalize(&mut self) {
        self.settings.normalize();
        self.water.align_times();
        self.pomodoro.remaining_seconds = self.pomodoro.remaining_seconds.min(86400);
        self.next_id = self.next_id.max(
            self.tasks
                .iter()
                .map(|t| t.id)
                .chain(self.shelf.iter().map(|s| s.id))
                .max()
                .unwrap_or(0)
                .saturating_add(1),
        );
    }
    pub fn phase_seconds(&self, phase: PomodoroPhase) -> u64 {
        60 * match phase {
            PomodoroPhase::Focus => self.settings.focus_minutes,
            PomodoroPhase::ShortBreak => self.settings.short_break_minutes,
            PomodoroPhase::LongBreak => self.settings.long_break_minutes,
        } as u64
    }
    pub fn apply(&mut self, command: AppCommand, now: DateTime<Local>) -> Vec<AppEvent> {
        let stamp = now.timestamp();
        self.roll_day(&now);
        match command {
            AppCommand::Settings(mut settings) => {
                settings.normalize();
                self.settings = *settings;
            }
            AppCommand::AddTask { title, due_at } => {
                let title = title.trim().chars().take(500).collect::<String>();
                if title.is_empty() {
                    return vec![];
                }
                let id = self.id();
                self.tasks.push(Task {
                    id,
                    title,
                    due_at,
                    done: false,
                    color: 0x8B8CF8,
                    notified: false,
                });
            }
            AppCommand::EditTask {
                id,
                title,
                due_at,
                color,
            } => {
                if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
                    let title = title.trim().chars().take(500).collect::<String>();
                    if title.is_empty() {
                        return vec![];
                    }
                    t.title = title;
                    t.due_at = due_at;
                    t.color = color;
                    t.notified = false;
                }
            }
            AppCommand::ToggleTask(id) => {
                if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
                    t.done = !t.done;
                }
            }
            AppCommand::DeleteTask(id) => self.tasks.retain(|t| t.id != id),
            AppCommand::SnoozeTask { id, minutes } => {
                if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
                    t.due_at = Some(stamp + i64::from(minutes.clamp(1, 1440)) * 60);
                    t.notified = false;
                }
            }
            AppCommand::AddShelf { kind, value } => {
                if value.trim().is_empty()
                    || self
                        .shelf
                        .iter()
                        .any(|s| s.kind == kind && s.value == value)
                {
                    return vec![];
                }
                let id = self.id();
                self.shelf.push(ShelfItem { id, kind, value });
            }
            AppCommand::RemoveShelf(id) => self.shelf.retain(|s| s.id != id),
            AppCommand::ClearShelf => self.shelf.clear(),
            AppCommand::RecordDrink => {
                self.water.align_times();
                self.water.drinks_ml.push(self.settings.water_bottle_ml);
                self.water.drink_times.push(Some(stamp));
            }
            AppCommand::UndoDrink => {
                self.water.align_times();
                self.water.drinks_ml.pop();
                self.water.drink_times.pop();
            }
            AppCommand::SnoozeWater(minutes) => {
                self.water.snooze_until = Some(stamp + i64::from(minutes.clamp(1, 1440)) * 60)
            }
            AppCommand::ResumeWater => self.water.snooze_until = None,
            AppCommand::StartPomodoro => {
                if self.pomodoro.ends_at.is_none() {
                    if self.pomodoro.remaining_seconds == 0 {
                        self.pomodoro.remaining_seconds = self.phase_seconds(self.pomodoro.phase);
                    }
                    self.pomodoro.ends_at = Some(stamp + self.pomodoro.remaining_seconds as i64);
                }
            }
            AppCommand::PausePomodoro => {
                self.pomodoro.remaining_seconds = self.pomodoro.remaining(stamp);
                self.pomodoro.ends_at = None;
            }
            AppCommand::ResetPomodoro => {
                self.pomodoro.ends_at = None;
                self.pomodoro.remaining_seconds = self.phase_seconds(self.pomodoro.phase);
            }
            AppCommand::SetPomodoroPhase(phase) => {
                self.pomodoro.phase = phase;
                self.pomodoro.ends_at = None;
                self.pomodoro.remaining_seconds = self.phase_seconds(phase);
            }
        }
        vec![AppEvent::Changed]
    }
    fn roll_day(&mut self, now: &DateTime<Local>) -> bool {
        let day = format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day());
        if self.water.day == day {
            return false;
        }
        if !self.water.day.is_empty() {
            self.water
                .history
                .insert(self.water.day.clone(), self.water.total_ml());
        }
        self.water.day = day;
        self.water.drinks_ml.clear();
        self.water.drink_times.clear();
        self.water.last_reminder_slot = 0;
        self.water.snooze_until = None;
        true
    }
    pub fn tick(&mut self, now: DateTime<Local>) -> Vec<AppEvent> {
        let mut events = vec![];
        if self.roll_day(&now) {
            events.push(AppEvent::Changed);
        }
        let stamp = now.timestamp();
        if self.pomodoro.ends_at.is_some_and(|t| t <= stamp) {
            if self.pomodoro.phase == PomodoroPhase::Focus {
                self.pomodoro.completed_focus += 1;
            }
            self.pomodoro.ends_at = None;
            self.pomodoro.remaining_seconds = 0;
            events.push(AppEvent::Reminder {
                title: "Pomodoro".into(),
                body: self.settings.text("Tempo concluído!", "Time is up!").into(),
            });
            events.push(AppEvent::Changed);
        }
        let due: Vec<_> = self
            .tasks
            .iter_mut()
            .filter(|t| !t.done && !t.notified && t.due_at.is_some_and(|d| d <= stamp))
            .map(|t| {
                t.notified = true;
                t.title.clone()
            })
            .collect();
        if !due.is_empty() {
            events.push(AppEvent::Reminder {
                title: self.settings.text("Tarefas", "Tasks").into(),
                body: due.join(" · "),
            });
            events.push(AppEvent::Changed);
        }
        let hour = now.hour();
        if hour >= self.settings.water_start_hour
            && hour < self.settings.water_end_hour
            && self.water.total_ml() < self.settings.water_goal_ml
        {
            let active_minutes =
                (self.settings.water_end_hour - self.settings.water_start_hour) * 60;
            let portions = self
                .settings
                .water_goal_ml
                .div_ceil(self.settings.water_bottle_ml)
                .max(1);
            let interval = (active_minutes / portions).max(1);
            let elapsed = (hour - self.settings.water_start_hour) * 60 + now.minute();
            let slot = elapsed / interval;
            let snooze_due = self.water.snooze_until.is_some_and(|t| t <= stamp);
            if (slot > self.water.last_reminder_slot || snooze_due)
                && self.water.snooze_until.is_none_or(|t| t <= stamp)
            {
                self.water.last_reminder_slot = slot;
                self.water.snooze_until = None;
                events.push(AppEvent::Reminder {
                    title: self
                        .settings
                        .text("Hora de beber água", "Time to drink")
                        .into(),
                    body: format!(
                        "{} / {} ml",
                        self.water.total_ml(),
                        self.settings.water_goal_ml
                    ),
                });
                events.push(AppEvent::Changed);
            }
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    fn time(hour: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 29, hour, 0, 0).unwrap()
    }
    #[test]
    fn timer_completes_once_after_suspend() {
        let mut app = AppState::default();
        let now = time(8);
        app.apply(AppCommand::StartPomodoro, now);
        assert_eq!(
            app.tick(now + Duration::hours(2))
                .iter()
                .filter(|e| matches!(e, AppEvent::Reminder { .. }))
                .count(),
            1
        );
        assert!(!app
            .tick(now + Duration::hours(3))
            .iter()
            .any(|e| matches!(e, AppEvent::Reminder { title, .. } if title == "Pomodoro")));
    }
    #[test]
    fn pause_resume_preserves_remaining() {
        let mut app = AppState::default();
        let now = time(8);
        app.apply(AppCommand::StartPomodoro, now);
        app.apply(AppCommand::PausePomodoro, now + Duration::minutes(5));
        assert_eq!(app.pomodoro.remaining_seconds, 1200);
        app.apply(AppCommand::StartPomodoro, now + Duration::hours(1));
        assert_eq!(
            app.pomodoro
                .remaining((now + Duration::hours(1)).timestamp()),
            1200
        );
    }
    #[test]
    fn water_rollover_archives_and_resets() {
        let mut app = AppState::default();
        let now = time(10);
        app.apply(AppCommand::RecordDrink, now);
        app.tick(now + Duration::days(1));
        assert_eq!(app.water.total_ml(), 0);
        assert_eq!(app.water.history["2026-10-29"], 250);
    }
    #[test]
    fn water_does_not_replay_missed_slots() {
        let mut app = AppState::default();
        let now = time(18);
        let first = app.tick(now);
        assert_eq!(
            first
                .iter()
                .filter(|e| matches!(e, AppEvent::Reminder { .. }))
                .count(),
            1
        );
        assert!(app.tick(now).is_empty());
    }
    #[test]
    fn due_tasks_aggregate_and_snooze() {
        let mut app = AppState::default();
        let now = time(8);
        for title in ["One", "Two"] {
            app.apply(
                AppCommand::AddTask {
                    title: title.into(),
                    due_at: Some(now.timestamp()),
                },
                now,
            );
        }
        assert_eq!(
            app.tick(now)
                .iter()
                .filter(|e| matches!(e, AppEvent::Reminder { .. }))
                .count(),
            1
        );
        assert!(app.tick(now).is_empty());
        app.apply(AppCommand::SnoozeTask { id: 1, minutes: 5 }, now);
        assert!(app.tick(now + Duration::minutes(4)).is_empty());
        assert_eq!(app.tick(now + Duration::minutes(5)).len(), 2);
    }
    #[test]
    fn water_snooze_can_fire_inside_same_slot() {
        let mut app = AppState::default();
        let now = time(10);
        app.tick(now);
        app.apply(AppCommand::SnoozeWater(5), now);
        assert!(app.tick(now + Duration::minutes(4)).is_empty());
        assert_eq!(
            app.tick(now + Duration::minutes(5))
                .iter()
                .filter(|e| matches!(e, AppEvent::Reminder { .. }))
                .count(),
            1
        );
        assert!(app.tick(now + Duration::minutes(6)).is_empty());
    }
    #[test]
    fn resuming_water_clears_pause_without_changing_consumption() {
        let mut app = AppState::default();
        let now = time(10);
        app.apply(AppCommand::RecordDrink, now);
        let total = app.water.total_ml();
        app.apply(AppCommand::SnoozeWater(15), now);
        assert_eq!(app.water.snooze_until, Some(now.timestamp() + 15 * 60));
        app.apply(AppCommand::ResumeWater, now);
        assert_eq!(app.water.snooze_until, None);
        assert_eq!(app.water.total_ml(), total);
    }
    #[test]
    fn water_timestamps_keep_legacy_amounts_unknown() {
        let mut app = AppState::default();
        let now = time(10);
        app.water = serde_json::from_str(r#"{"day":"2026-10-29","drinks_ml":[200,300]}"#).unwrap();
        assert!(app.water.drink_times.is_empty());
        app.apply(AppCommand::RecordDrink, now);
        assert_eq!(app.water.drink_times, [None, None, Some(now.timestamp())]);
        assert_eq!(app.water.total_ml(), 750);
        assert_eq!(app.water.hourly_ml(now.date_naive())[10], 250);
        assert_eq!(
            app.water.hourly_ml(now.date_naive()).iter().sum::<u32>(),
            250
        );
        app.apply(AppCommand::UndoDrink, now);
        assert_eq!(app.water.drinks_ml, [200, 300]);
        assert_eq!(app.water.drink_times, [None, None]);
        app.tick(now + Duration::days(1));
        assert!(app.water.drink_times.is_empty());
        assert!(app.water.drinks_ml.is_empty());
    }
    #[test]
    fn hourly_water_uses_only_the_selected_local_day() {
        let mut app = AppState::default();
        let now = time(10);
        app.apply(AppCommand::RecordDrink, now);
        app.apply(AppCommand::RecordDrink, now + Duration::hours(2));
        let hours = app.water.hourly_ml(now.date_naive());
        assert_eq!((hours[10], hours[12]), (250, 250));
        assert_eq!(
            app.water.hourly_ml((now + Duration::days(1)).date_naive()),
            [0; 24]
        );
        let encoded = serde_json::to_string(&app).unwrap();
        let restored: AppState = serde_json::from_str(&encoded).unwrap();
        assert_eq!(restored.water.hourly_ml(now.date_naive()), hours);
    }

    #[test]
    fn settings_show_battery_serialization_and_default() {
        let expected_default = crate::services::is_laptop();
        let default_settings = Settings::default();
        assert_eq!(default_settings.show_battery, expected_default);

        // Deserializing without show_battery uses default
        let empty_json = "{}";
        let parsed: Settings = serde_json::from_str(empty_json).unwrap();
        assert_eq!(parsed.show_battery, expected_default);

        // Explicit true
        let parsed_true: Settings = serde_json::from_str(r#"{"show_battery": true}"#).unwrap();
        assert!(parsed_true.show_battery);

        // Explicit false
        let parsed_false: Settings = serde_json::from_str(r#"{"show_battery": false}"#).unwrap();
        assert!(!parsed_false.show_battery);
    }

    #[test]
    fn settings_notch_bottom_normalized_to_false() {
        let json = r#"{"notch_bottom": true, "notch_column": 2}"#;
        let mut parsed: Settings = serde_json::from_str(json).unwrap();
        assert!(parsed.notch_bottom);
        parsed.normalize();
        assert!(!parsed.notch_bottom);
        assert_eq!(parsed.notch_column, 2);
    }

    #[test]
    fn settings_legacy_hud_compatibility() {
        let json = r#"{"hud_volume": true, "hud_brightness": false}"#;
        let parsed: Settings = serde_json::from_str(json).unwrap();
        assert!(parsed.hud_volume);
        assert!(!parsed.hud_brightness);
    }
}

#[cfg(test)]
mod legacy_brand_tests {
    use super::*;
    #[test]
    fn legacy_mascot_is_ignored_without_resetting_settings() {
        let settings: Settings = serde_json::from_str(
            r#"{"mascot":true,"startup":true,"scale":1.5,"water_goal_ml":2400}"#,
        )
        .unwrap();
        assert!(settings.startup);
        assert_eq!(settings.scale, 1.5);
        assert_eq!(settings.water_goal_ml, 2400);
        assert!(serde_json::to_value(settings)
            .unwrap()
            .get("mascot")
            .is_none());
    }
}
