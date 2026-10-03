use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use rusqlite::{Connection, OptionalExtension, params};
use tokio::sync::oneshot;

use super::AppService;
use crate::store::{
    completion::{DEFAULT_FADE_SECONDS, MAX_FADE_SECONDS},
    environments::StartupKind,
};

const BRANCH_INSTANCES_SETTING: &str = "instances.branch";
#[derive(Clone, Copy)]
enum OpencodeSetting {
    Integration,
    ClearHistory,
}

impl OpencodeSetting {
    fn key(self) -> &'static str {
        match self {
            Self::Integration => "integrations.opencode",
            Self::ClearHistory => "opencode.clear_history_on_creation",
        }
    }
}
type CommandReply = oneshot::Sender<Result<String, String>>;
type StartupReply = mpsc::Sender<Result<(), String>>;
type StartupHistory = BTreeMap<(String, StartupKind), Vec<u64>>;

#[derive(Clone, Copy)]
pub(super) enum FeedbackSetting {
    FadeSeconds,
    Sound,
    EventAcceptanceSound,
}

impl FeedbackSetting {
    fn key(self) -> &'static str {
        match self {
            Self::FadeSeconds => "completion.fade_seconds",
            Self::Sound => "completion.sound",
            Self::EventAcceptanceSound => "events.acceptance_sound",
        }
    }

    fn default_value(self) -> &'static str {
        match self {
            Self::FadeSeconds => "20",
            Self::Sound | Self::EventAcceptanceSound => "",
        }
    }
}

pub(super) struct Settings {
    branch_instances: AtomicBool,
    opencode: Arc<[AtomicBool; 2]>,
    feedback: Arc<[RwLock<String>; 3]>,
    startup_history: Arc<RwLock<StartupHistory>>,
    commands: mpsc::Sender<SettingsRequest>,
}

enum SettingsRequest {
    SetBranchInstances(bool),
    Opencode(OpencodeSetting, Option<bool>, CommandReply),
    Feedback(FeedbackSetting, Option<String>, CommandReply),
    RefreshStartupHistory(StartupReply),
    RecordStartup {
        template: String,
        kind: StartupKind,
        duration_milliseconds: u64,
        reply: StartupReply,
    },
    #[cfg(test)]
    Flush(mpsc::Sender<()>),
}

impl Settings {
    pub(super) fn open(path: PathBuf) -> Result<Self, rusqlite::Error> {
        let connection = Connection::open(path)?;
        connection.execute_batch(include_str!("../../migrations/0001_app_settings.sql"))?;
        connection.execute_batch(include_str!("../../migrations/0002_instance_startups.sql"))?;
        connection.execute_batch(include_str!(
            "../../migrations/0003_hot_instance_startups.sql"
        ))?;
        let branch_instances = connection
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                [BRANCH_INSTANCES_SETTING],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .is_none_or(|value| value == "true");
        connection.execute_batch(include_str!(
            "../../migrations/0004_completion_settings.sql"
        ))?;
        let feedback = Arc::new([
            RwLock::new(read_feedback(&connection, FeedbackSetting::FadeSeconds)?),
            RwLock::new(read_feedback(&connection, FeedbackSetting::Sound)?),
            RwLock::new(read_feedback(
                &connection,
                FeedbackSetting::EventAcceptanceSound,
            )?),
        ]);
        let opencode = Arc::new([
            AtomicBool::new(read_opencode(&connection, OpencodeSetting::Integration)?),
            AtomicBool::new(read_opencode(&connection, OpencodeSetting::ClearHistory)?),
        ]);
        let cached_opencode = Arc::clone(&opencode);
        let startup_history = Arc::new(RwLock::new(read_startup_history(&connection)?));
        let cached_feedback = Arc::clone(&feedback);
        let cached_startup_history = Arc::clone(&startup_history);
        let (commands, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("tandem-settings".into())
            .spawn(move || {
                persist_settings(
                    connection,
                    receiver,
                    cached_feedback,
                    cached_startup_history,
                    cached_opencode,
                )
            })
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        Ok(Self {
            branch_instances: AtomicBool::new(branch_instances),
            opencode,
            feedback,
            startup_history,
            commands,
        })
    }

    fn branch_instances(&self) -> bool {
        self.branch_instances.load(Ordering::Relaxed)
    }

    fn set_branch_instances(&self, enabled: bool) -> Result<(), String> {
        self.commands
            .send(SettingsRequest::SetBranchInstances(enabled))
            .map_err(|_| "settings worker stopped".to_owned())?;
        self.branch_instances.store(enabled, Ordering::Relaxed);
        Ok(())
    }

    fn feedback(&self, kind: FeedbackSetting) -> String {
        self.feedback[kind as usize]
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn feedback_request(
        &self,
        kind: FeedbackSetting,
        value: Option<String>,
    ) -> Result<oneshot::Receiver<Result<String, String>>, String> {
        let (sender, receiver) = oneshot::channel();
        self.commands
            .send(SettingsRequest::Feedback(kind, value, sender))
            .map_err(|_| "settings worker stopped".to_owned())?;
        Ok(receiver)
    }

    pub(super) fn refresh(&self) -> Result<(), String> {
        for kind in [
            FeedbackSetting::FadeSeconds,
            FeedbackSetting::Sound,
            FeedbackSetting::EventAcceptanceSound,
        ] {
            self.feedback_request(kind, None)?
                .blocking_recv()
                .map_err(|_| "settings worker stopped")??;
        }
        for kind in [OpencodeSetting::Integration, OpencodeSetting::ClearHistory] {
            self.opencode_request(kind, None)?
                .blocking_recv()
                .map_err(|_| "settings worker stopped")??;
        }
        let (sender, receiver) = mpsc::channel();
        self.commands
            .send(SettingsRequest::RefreshStartupHistory(sender))
            .map_err(|_| "settings worker stopped")?;
        receiver.recv().map_err(|_| "settings worker stopped")?
    }

    pub(super) fn opencode_enabled(&self) -> bool {
        self.opencode[OpencodeSetting::Integration as usize].load(Ordering::Acquire)
    }

    pub(super) fn clear_opencode_history(&self) -> bool {
        self.opencode[OpencodeSetting::ClearHistory as usize].load(Ordering::Acquire)
    }

    fn opencode_request(
        &self,
        kind: OpencodeSetting,
        enabled: Option<bool>,
    ) -> Result<oneshot::Receiver<Result<String, String>>, String> {
        let (sender, receiver) = oneshot::channel();
        self.commands
            .send(SettingsRequest::Opencode(kind, enabled, sender))
            .map_err(|_| "settings worker stopped")?;
        Ok(receiver)
    }

    pub(super) fn startup_averages(&self, kind: StartupKind) -> BTreeMap<String, u64> {
        let history = self
            .startup_history
            .read()
            .unwrap_or_else(|error| error.into_inner());
        history
            .iter()
            .filter(|((_, stored_kind), _)| *stored_kind == kind)
            .map(|((template, _), durations)| {
                (
                    template.clone(),
                    durations.iter().sum::<u64>() / durations.len() as u64,
                )
            })
            .collect()
    }

    pub(super) fn record_startup(
        &self,
        template: String,
        kind: StartupKind,
        duration_milliseconds: u64,
    ) -> Result<(), String> {
        let (sender, receiver) = mpsc::channel();
        self.commands
            .send(SettingsRequest::RecordStartup {
                template,
                kind,
                duration_milliseconds,
                reply: sender,
            })
            .map_err(|_| "settings worker stopped")?;
        receiver.recv().map_err(|_| "settings worker stopped")?
    }

    #[cfg(test)]
    fn flush(&self) {
        let (sender, receiver) = mpsc::channel();
        self.commands.send(SettingsRequest::Flush(sender)).unwrap();
        receiver.recv().unwrap();
    }
}

fn read_feedback(
    connection: &Connection,
    kind: FeedbackSetting,
) -> Result<String, rusqlite::Error> {
    connection
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [kind.key()],
            |row| row.get(0),
        )
        .optional()
        .map(|value| value.unwrap_or_else(|| kind.default_value().into()))
}

fn read_opencode(connection: &Connection, kind: OpencodeSetting) -> Result<bool, rusqlite::Error> {
    connection
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [kind.key()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map(|value| value.is_none_or(|value| value == "true"))
}

fn read_startup_history(connection: &Connection) -> Result<StartupHistory, rusqlite::Error> {
    let mut history = BTreeMap::new();
    read_startup_table(connection, StartupKind::Cold, &mut history)?;
    read_startup_table(connection, StartupKind::Hot, &mut history)?;
    Ok(history)
}

fn read_startup_table(
    connection: &Connection,
    kind: StartupKind,
    history: &mut StartupHistory,
) -> Result<(), rusqlite::Error> {
    let mut statement = connection.prepare(&format!(
        "SELECT template, duration_milliseconds FROM {} ORDER BY template, id DESC",
        startup_table(kind)
    ))?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
    })?;
    for row in rows {
        let (template, duration) = row?;
        let durations = history.entry((template, kind)).or_default();
        if durations.len() < 5 {
            durations.push(duration);
        }
    }
    Ok(())
}

fn persist_settings(
    connection: Connection,
    receiver: mpsc::Receiver<SettingsRequest>,
    feedback: Arc<[RwLock<String>; 3]>,
    startup_history: Arc<RwLock<StartupHistory>>,
    opencode: Arc<[AtomicBool; 2]>,
) {
    for command in receiver {
        match command {
            SettingsRequest::Opencode(kind, enabled, reply) => {
                let result = match enabled {
                    Some(enabled) => connection.execute(
                        "INSERT INTO app_settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                        params![kind.key(), enabled.to_string()],
                    ).map(|_| enabled),
                    None => read_opencode(&connection, kind),
                };
                if let Ok(enabled) = result { opencode[kind as usize].store(enabled, Ordering::Release); }
                let _ = reply.send(result.map(|value| value.to_string()).map_err(|error| error.to_string()));
            }
            SettingsRequest::SetBranchInstances(enabled) => {
                if let Err(error) = connection.execute(
                    "INSERT INTO app_settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![BRANCH_INSTANCES_SETTING, enabled.to_string()],
                ) {
                    crate::diagnostics::record_error("could not persist settings", &error);
                }
            }
            SettingsRequest::Feedback(kind, value, reply) => {
                let result = match value {
                    Some(value) => connection.execute(
                    "INSERT INTO app_settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![kind.key(), value],
                    ).map(|_| value),
                    None => read_feedback(&connection, kind),
                };
                finish_feedback(result, &feedback[kind as usize], reply);
            }
            SettingsRequest::RefreshStartupHistory(reply) => {
                let result = read_startup_history(&connection).map(|history| {
                    *startup_history.write().unwrap_or_else(|error| error.into_inner()) = history;
                });
                let _ = reply.send(result.map_err(|error| error.to_string()));
            }
            SettingsRequest::RecordStartup {
                template,
                kind,
                duration_milliseconds,
                reply,
            } => {
                let result = record_startup(
                    &connection,
                    &startup_history,
                    template,
                    kind,
                    duration_milliseconds,
                );
                let _ = reply.send(result.map_err(|error| error.to_string()));
            }
            #[cfg(test)]
            SettingsRequest::Flush(sender) => {
                let _ = sender.send(());
            }
        }
    }
}

fn record_startup(
    connection: &Connection,
    cache: &RwLock<StartupHistory>,
    template: String,
    kind: StartupKind,
    duration_milliseconds: u64,
) -> Result<(), rusqlite::Error> {
    let table = startup_table(kind);
    connection.execute(
        &format!("INSERT INTO {table} (template, duration_milliseconds) VALUES (?1, ?2)"),
        params![template, duration_milliseconds],
    )?;
    connection.execute(
        &format!(
            "DELETE FROM {table}
         WHERE template = ?1
           AND id NOT IN (
             SELECT id FROM {table} WHERE template = ?1 ORDER BY id DESC LIMIT 5
           )"
        ),
        [&template],
    )?;
    let mut cache = cache.write().unwrap_or_else(|error| error.into_inner());
    let durations = cache.entry((template, kind)).or_default();
    durations.insert(0, duration_milliseconds);
    durations.truncate(5);
    Ok(())
}

fn startup_table(kind: StartupKind) -> &'static str {
    match kind {
        StartupKind::Cold => "instance_startups",
        StartupKind::Hot => "hot_instance_startups",
    }
}

fn finish_feedback(
    result: Result<String, rusqlite::Error>,
    cache: &RwLock<String>,
    reply: CommandReply,
) {
    match &result {
        Ok(value) => *cache.write().unwrap_or_else(|error| error.into_inner()) = value.clone(),
        Err(error) => crate::diagnostics::record_error("feedback settings failed", error),
    }
    let _ = reply.send(result.map_err(|error| error.to_string()));
}

impl AppService {
    pub(crate) fn opencode_enabled(&self) -> bool {
        self.settings.opencode_enabled()
    }

    pub(crate) fn set_opencode_enabled(
        &self,
        enabled: bool,
    ) -> Result<oneshot::Receiver<Result<String, String>>, String> {
        self.set_opencode_setting(OpencodeSetting::Integration, enabled)
    }

    pub(crate) fn clear_opencode_history(&self) -> bool {
        self.settings.clear_opencode_history()
    }

    pub(crate) fn set_clear_opencode_history(
        &self,
        enabled: bool,
    ) -> Result<oneshot::Receiver<Result<String, String>>, String> {
        self.set_opencode_setting(OpencodeSetting::ClearHistory, enabled)
    }

    fn set_opencode_setting(
        &self,
        kind: OpencodeSetting,
        enabled: bool,
    ) -> Result<oneshot::Receiver<Result<String, String>>, String> {
        let saved = self.settings.opencode_request(kind, Some(enabled))?;
        let notifier = self.refresh.notifier.clone();
        let state = Arc::clone(&self.opencode);
        let (sender, receiver) = oneshot::channel();
        self.runtime.spawn(async move {
            let result = saved
                .await
                .unwrap_or_else(|_| Err("settings worker stopped".into()));
            if result.is_ok() {
                if matches!(kind, OpencodeSetting::Integration) {
                    state.reset();
                }
                let _ = tokio::task::spawn_blocking(move || {
                    notifier.publish(super::refresh::Refresh::Settings)
                })
                .await;
            }
            let _ = sender.send(result);
        });
        Ok(receiver)
    }

    pub(crate) fn completion_fade_seconds(&self) -> u64 {
        self.settings
            .feedback(FeedbackSetting::FadeSeconds)
            .parse::<u64>()
            .ok()
            .filter(|seconds| (1..=MAX_FADE_SECONDS).contains(seconds))
            .unwrap_or(DEFAULT_FADE_SECONDS)
    }

    pub(crate) fn completion_sound_choice(&self) -> String {
        self.settings.feedback(FeedbackSetting::Sound)
    }

    pub(crate) fn event_acceptance_sound_choice(&self) -> String {
        self.settings
            .feedback(FeedbackSetting::EventAcceptanceSound)
    }

    pub(crate) fn set_completion_fade_seconds(
        &self,
        value: String,
    ) -> Result<oneshot::Receiver<Result<String, String>>, String> {
        let seconds = value
            .parse::<u64>()
            .ok()
            .filter(|seconds| (1..=MAX_FADE_SECONDS).contains(seconds))
            .ok_or("Fade duration must be between 1 and 3600 seconds")?;
        self.set_feedback(FeedbackSetting::FadeSeconds, seconds.to_string())
    }

    pub(crate) fn set_completion_sound_choice(
        &self,
        value: String,
    ) -> Result<oneshot::Receiver<Result<String, String>>, String> {
        self.set_sound_choice(FeedbackSetting::Sound, value)
    }

    pub(crate) fn set_event_acceptance_sound_choice(
        &self,
        value: String,
    ) -> Result<oneshot::Receiver<Result<String, String>>, String> {
        self.set_sound_choice(FeedbackSetting::EventAcceptanceSound, value)
    }

    fn set_sound_choice(
        &self,
        kind: FeedbackSetting,
        value: String,
    ) -> Result<oneshot::Receiver<Result<String, String>>, String> {
        if !self.sound_choices.iter().any(|sound| sound.id == value) {
            return Err("The selected sound is unavailable".into());
        }
        let saved = self.set_feedback(kind, value.clone())?;
        self.play_sound(value);
        Ok(saved)
    }

    fn set_feedback(
        &self,
        kind: FeedbackSetting,
        value: String,
    ) -> Result<oneshot::Receiver<Result<String, String>>, String> {
        let saved = self.settings.feedback_request(kind, Some(value))?;
        let notifier = self.refresh.notifier.clone();
        let (sender, receiver) = oneshot::channel();
        self.runtime.spawn(async move {
            let result = saved
                .await
                .unwrap_or_else(|_| Err("settings worker stopped".into()));
            if result.is_ok() {
                let _ = tokio::task::spawn_blocking(move || {
                    notifier.publish(super::refresh::Refresh::Settings)
                })
                .await;
            }
            let _ = sender.send(result);
        });
        Ok(receiver)
    }

    pub(crate) fn branch_instances(&self) -> bool {
        self.settings.branch_instances()
    }

    pub(crate) fn set_branch_instances(&self, enabled: bool) -> Result<(), String> {
        self.settings.set_branch_instances(enabled)
    }

    #[cfg(test)]
    pub(crate) fn flush_settings(&self) {
        self.settings.flush();
    }
}

#[cfg(test)]
#[path = "tests/startup_settings.rs"]
mod startup_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_instances_defaults_to_on_and_is_written_to_sqlite() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.sqlite3");
        let settings = Settings::open(path.clone()).unwrap();
        assert!(settings.branch_instances());

        settings.set_branch_instances(false).unwrap();
        settings.flush();

        let value = Connection::open(path)
            .unwrap()
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                [BRANCH_INSTANCES_SETTING],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        assert_eq!(value, "false");
    }

    #[test]
    fn startup_history_keeps_five_latest_durations_per_template() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.sqlite3");
        let settings = Settings::open(path.clone()).unwrap();
        for duration in [1_000, 2_000, 3_000, 4_000, 5_000, 6_000] {
            settings
                .record_startup("website".into(), StartupKind::Cold, duration)
                .unwrap();
        }
        settings
            .record_startup("api".into(), StartupKind::Cold, 9_000)
            .unwrap();
        settings
            .record_startup("website".into(), StartupKind::Hot, 2_000)
            .unwrap();
        settings.flush();

        assert_eq!(
            settings.startup_averages(StartupKind::Cold),
            [("website".into(), 4_000), ("api".into(), 9_000)].into()
        );
        assert_eq!(
            settings.startup_averages(StartupKind::Hot),
            [("website".into(), 2_000)].into()
        );
        let count = Connection::open(path)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM instance_startups WHERE template = ?1",
                ["website"],
                |row| row.get::<_, u64>(0),
            )
            .unwrap();
        assert_eq!(count, 5);
    }

    #[test]
    fn service_snapshots_expose_startup_averages_without_running_instances() {
        let service = AppService::for_tests();
        assert!(
            service
                .environment_snapshot()
                .cold_startup_averages_milliseconds
                .is_empty()
        );
        for duration in [80_000, 88_000] {
            service
                .settings
                .record_startup("website".into(), StartupKind::Cold, duration)
                .unwrap();
        }
        service
            .settings
            .record_startup("website".into(), StartupKind::Hot, 12_000)
            .unwrap();
        service.flush_settings();
        let cold = service.queue_instance_for_tests("review", "website");
        let snapshot = service.environment_snapshot();
        assert_eq!(
            snapshot.cold_startup_averages_milliseconds["website"],
            84_000
        );
        assert_eq!(
            snapshot.hot_startup_averages_milliseconds["website"],
            12_000
        );
        assert_eq!(
            snapshot.startup["review"].estimate_milliseconds,
            Some(84_000)
        );

        service.complete_instance_for_tests(&cold.id, snapshot.instances[0].clone());
        service.queue_instance_for_tests("review", "website");
        assert_eq!(
            service.environment_snapshot().startup["review"].estimate_milliseconds,
            Some(12_000)
        );
    }
}
