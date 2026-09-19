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

const BRANCH_INSTANCES_SETTING: &str = "instances.branch";
const OPEN_COMMAND_SETTING: &str = "instances.open_command";
type CommandReply = oneshot::Sender<Result<String, String>>;

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
    workspace_commands: Arc<[RwLock<String>; 2]>,
    startup_history: Arc<RwLock<BTreeMap<String, Vec<u64>>>>,
    commands: mpsc::Sender<SettingsRequest>,
}

enum SettingsRequest {
    SetBranchInstances(bool),
    SetCommand(WorkspaceCommand, String, CommandReply),
    ReadCommand(WorkspaceCommand, CommandReply),
    RecordStartup {
        template: String,
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
                )
            })
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        Ok(Self {
            branch_instances: AtomicBool::new(branch_instances),
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
        Ok(())
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

    pub(super) fn startup_averages(&self) -> BTreeMap<String, u64> {
        let history = self
            .startup_history
            .read()
            .unwrap_or_else(|error| error.into_inner());
        history
            .iter()
            .map(|(template, durations)| {
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
        duration_milliseconds: u64,
    ) -> Result<(), String> {
        self.commands
            .send(SettingsRequest::RecordStartup {
                template,
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

fn read_startup_history(
    connection: &Connection,
) -> Result<BTreeMap<String, Vec<u64>>, rusqlite::Error> {
    let mut statement = connection.prepare(
        "SELECT template, duration_milliseconds FROM instance_startups ORDER BY template, id DESC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
    })?;
    let mut history = BTreeMap::<String, Vec<u64>>::new();
    for row in rows {
        let (template, duration) = row?;
        let durations = history.entry(template).or_default();
        if durations.len() < 5 {
            durations.push(duration);
        }
    }
    Ok(history)
}

fn persist_settings(
    connection: Connection,
    receiver: mpsc::Receiver<SettingsRequest>,
    workspace_commands: Arc<[RwLock<String>; 2]>,
    startup_history: Arc<RwLock<BTreeMap<String, Vec<u64>>>>,
) {
    for command in receiver {
        match command {
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
                duration_milliseconds,
            } => {
                if let Err(error) = record_startup(
                    &connection,
                    &startup_history,
                    template,
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
    cache: &RwLock<BTreeMap<String, Vec<u64>>>,
    template: String,
    duration_milliseconds: u64,
) -> Result<(), rusqlite::Error> {
    connection.execute(
        "INSERT INTO instance_startups (template, duration_milliseconds) VALUES (?1, ?2)",
        params![template, duration_milliseconds],
    )?;
    connection.execute(
        "DELETE FROM instance_startups
         WHERE template = ?1
           AND id NOT IN (
             SELECT id FROM instance_startups WHERE template = ?1 ORDER BY id DESC LIMIT 5
           )",
        [&template],
    )?;
    let mut cache = cache.write().unwrap_or_else(|error| error.into_inner());
    let durations = cache.entry(template).or_default();
    durations.insert(0, duration_milliseconds);
    durations.truncate(5);
    Ok(())
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
            settings.record_startup("website".into(), duration).unwrap();
        }
        settings.record_startup("api".into(), 9_000).unwrap();
        settings.flush();

        assert_eq!(
            settings.startup_averages(),
            [("website".into(), 4_000), ("api".into(), 9_000)].into()
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
                .startup_averages_milliseconds
                .is_empty()
        );
        for duration in [80_000, 88_000] {
            service
                .settings
                .record_startup("website".into(), duration)
                .unwrap();
        }
        service.flush_settings();
        assert_eq!(
            service.environment_snapshot().startup_averages_milliseconds["website"],
            84_000
        );
    }
}
