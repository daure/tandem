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
use crate::store::environments::StartupKind;

const BRANCH_INSTANCES_SETTING: &str = "instances.branch";
const OPEN_COMMAND_SETTING: &str = "instances.open_command";
const OPENCODE_SETTING: &str = "integrations.opencode";
type CommandReply = oneshot::Sender<Result<String, String>>;
type StartupHistory = BTreeMap<(String, StartupKind), Vec<u64>>;

#[derive(Clone, Copy)]
pub(super) enum WorkspaceCommand {
    Open,
    Close,
}

impl WorkspaceCommand {
    fn key(self) -> &'static str {
        match self {
            Self::Open => OPEN_COMMAND_SETTING,
            Self::Close => "instances.close_command",
        }
    }
}

pub(super) struct Settings {
    branch_instances: AtomicBool,
    opencode: Arc<AtomicBool>,
    workspace_commands: Arc<[RwLock<String>; 2]>,
    startup_history: Arc<RwLock<StartupHistory>>,
    commands: mpsc::Sender<SettingsRequest>,
}

enum SettingsRequest {
    SetBranchInstances(bool),
    Opencode(Option<bool>, CommandReply),
    SetCommand(WorkspaceCommand, String, CommandReply),
    ReadCommand(WorkspaceCommand, CommandReply),
    RecordStartup {
        template: String,
        kind: StartupKind,
        duration_milliseconds: u64,
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
        let workspace_commands = Arc::new([
            RwLock::new(read_command(&connection, WorkspaceCommand::Open)?),
            RwLock::new(read_command(&connection, WorkspaceCommand::Close)?),
        ]);
        let opencode = Arc::new(AtomicBool::new(read_opencode(&connection)?));
        let cached_opencode = Arc::clone(&opencode);
        let startup_history = Arc::new(RwLock::new(read_startup_history(&connection)?));
        let cached_commands = Arc::clone(&workspace_commands);
        let cached_startup_history = Arc::clone(&startup_history);
        let (commands, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("tandem-settings".into())
            .spawn(move || {
                persist_settings(
                    connection,
                    receiver,
                    cached_commands,
                    cached_startup_history,
                    cached_opencode,
                )
            })
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        Ok(Self {
            branch_instances: AtomicBool::new(branch_instances),
            opencode,
            workspace_commands,
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

    fn command(&self, kind: WorkspaceCommand) -> String {
        self.workspace_commands[kind as usize]
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn set_command(
        &self,
        kind: WorkspaceCommand,
        command: String,
    ) -> Result<oneshot::Receiver<Result<String, String>>, String> {
        if command.contains('\0') {
            return Err("workspace command must not contain NUL bytes".into());
        }
        let (sender, receiver) = oneshot::channel();
        self.commands
            .send(SettingsRequest::SetCommand(kind, command, sender))
            .map_err(|_| "settings worker stopped".to_owned())?;
        Ok(receiver)
    }

    pub(super) async fn read_open_command(&self) -> Result<String, String> {
        self.read_command(WorkspaceCommand::Open).await
    }

    async fn read_command(&self, kind: WorkspaceCommand) -> Result<String, String> {
        let (sender, receiver) = oneshot::channel();
        self.commands
            .send(SettingsRequest::ReadCommand(kind, sender))
            .map_err(|_| "settings worker stopped".to_owned())?;
        receiver
            .await
            .map_err(|_| "settings worker stopped".to_owned())?
    }

    pub(super) fn read_close_command(&self) -> Result<String, String> {
        self.read_command_blocking(WorkspaceCommand::Close)
    }

    pub(super) fn refresh_commands(&self) -> Result<(), String> {
        self.read_command_blocking(WorkspaceCommand::Open)?;
        self.read_close_command()?;
        self.opencode_request(None)?
            .blocking_recv()
            .map_err(|_| "settings worker stopped")??;
        Ok(())
    }

    pub(super) fn opencode_enabled(&self) -> bool {
        self.opencode.load(Ordering::Acquire)
    }

    fn opencode_request(
        &self,
        enabled: Option<bool>,
    ) -> Result<oneshot::Receiver<Result<String, String>>, String> {
        let (sender, receiver) = oneshot::channel();
        self.commands
            .send(SettingsRequest::Opencode(enabled, sender))
            .map_err(|_| "settings worker stopped")?;
        Ok(receiver)
    }

    fn read_command_blocking(&self, kind: WorkspaceCommand) -> Result<String, String> {
        let (sender, receiver) = oneshot::channel();
        self.commands
            .send(SettingsRequest::ReadCommand(kind, sender))
            .map_err(|_| "settings worker stopped".to_owned())?;
        receiver
            .blocking_recv()
            .map_err(|_| "settings worker stopped".to_owned())?
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
        self.commands
            .send(SettingsRequest::RecordStartup {
                template,
                kind,
                duration_milliseconds,
            })
            .map_err(|_| "settings worker stopped".to_owned())
    }

    #[cfg(test)]
    fn flush(&self) {
        let (sender, receiver) = mpsc::channel();
        self.commands.send(SettingsRequest::Flush(sender)).unwrap();
        receiver.recv().unwrap();
    }
}

fn read_command(
    connection: &Connection,
    kind: WorkspaceCommand,
) -> Result<String, rusqlite::Error> {
    connection
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [kind.key()],
            |row| row.get(0),
        )
        .optional()
        .map(Option::unwrap_or_default)
}

fn read_opencode(connection: &Connection) -> Result<bool, rusqlite::Error> {
    connection
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [OPENCODE_SETTING],
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
    workspace_commands: Arc<[RwLock<String>; 2]>,
    startup_history: Arc<RwLock<StartupHistory>>,
    opencode: Arc<AtomicBool>,
) {
    for command in receiver {
        match command {
            SettingsRequest::Opencode(enabled, reply) => {
                let result = match enabled {
                    Some(enabled) => connection.execute(
                        "INSERT INTO app_settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                        params![OPENCODE_SETTING, enabled.to_string()],
                    ).map(|_| enabled),
                    None => read_opencode(&connection),
                };
                if let Ok(enabled) = result { opencode.store(enabled, Ordering::Release); }
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
            SettingsRequest::SetCommand(kind, value, reply) => {
                let result = connection.execute(
                    "INSERT INTO app_settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![kind.key(), value],
                ).map(|_| value);
                finish_command(result, &workspace_commands[kind as usize], reply);
            }
            SettingsRequest::ReadCommand(kind, reply) => {
                finish_command(read_command(&connection, kind), &workspace_commands[kind as usize], reply);
            }
            SettingsRequest::RecordStartup {
                template,
                kind,
                duration_milliseconds,
            } => {
                if let Err(error) = record_startup(
                    &connection,
                    &startup_history,
                    template,
                    kind,
                    duration_milliseconds,
                ) {
                    crate::diagnostics::record_error("could not persist startup timing", &error);
                }
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

fn finish_command(
    result: Result<String, rusqlite::Error>,
    cache: &RwLock<String>,
    reply: CommandReply,
) {
    match &result {
        Ok(value) => *cache.write().unwrap_or_else(|error| error.into_inner()) = value.clone(),
        Err(error) => crate::diagnostics::record_error("workspace command settings failed", error),
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
        let saved = self.settings.opencode_request(Some(enabled))?;
        let notifier = self.refresh.notifier.clone();
        let state = Arc::clone(&self.opencode);
        let (sender, receiver) = oneshot::channel();
        self.runtime.spawn(async move {
            let result = saved
                .await
                .unwrap_or_else(|_| Err("settings worker stopped".into()));
            if result.is_ok() {
                state.reset();
                let _ = tokio::task::spawn_blocking(move || {
                    notifier.publish(super::refresh::Refresh::Settings)
                })
                .await;
            }
            let _ = sender.send(result);
        });
        Ok(receiver)
    }

    pub(crate) async fn configure_open_command(
        &self,
        command: String,
        confirmed: bool,
    ) -> Result<String, String> {
        if !confirmed {
            return Err(
                "confirmation_required: the open command executes with local host privileges"
                    .into(),
            );
        }
        self.set_open_command(command)?
            .await
            .map_err(|_| "settings worker stopped".to_owned())?
    }

    pub(crate) fn open_command(&self) -> String {
        self.settings.command(WorkspaceCommand::Open)
    }

    pub(crate) fn set_open_command(
        &self,
        command: String,
    ) -> Result<oneshot::Receiver<Result<String, String>>, String> {
        self.set_workspace_command(WorkspaceCommand::Open, command)
    }

    pub(crate) async fn configure_close_command(
        &self,
        command: String,
        confirmed: bool,
    ) -> Result<String, String> {
        if !confirmed {
            return Err(
                "confirmation_required: the close command executes with local host privileges"
                    .into(),
            );
        }
        self.set_close_command(command)?
            .await
            .map_err(|_| "settings worker stopped".to_owned())?
    }

    pub(crate) fn close_command(&self) -> String {
        self.settings.command(WorkspaceCommand::Close)
    }

    pub(crate) fn set_close_command(
        &self,
        command: String,
    ) -> Result<oneshot::Receiver<Result<String, String>>, String> {
        self.set_workspace_command(WorkspaceCommand::Close, command)
    }

    pub(crate) async fn get_close_command(&self) -> Result<String, String> {
        self.settings.read_command(WorkspaceCommand::Close).await
    }

    fn set_workspace_command(
        &self,
        kind: WorkspaceCommand,
        command: String,
    ) -> Result<oneshot::Receiver<Result<String, String>>, String> {
        let saved = self.settings.set_command(kind, command)?;
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

    pub(crate) async fn get_open_command(&self) -> Result<String, String> {
        self.settings.read_open_command().await
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
