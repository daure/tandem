use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::PathBuf,
};

use rusqlite::{TransactionBehavior, params};
use serde_json::{Value, json};

use super::Store;

impl Store {
    pub(crate) fn cancel_creation(&self, request: &str) -> Result<(), String> {
        self.connection()?.execute("DELETE FROM session_creation_requests WHERE namespace = ?1 AND id = ?2 AND session_id IS NULL", params![self.config.namespace, request]).map_err(|error| error.to_string())?;
        self.remove_identity_receipt(request);
        Ok(())
    }
    pub(crate) fn complete_creation(
        &self,
        request: &str,
        instance: &str,
        mut receipt: crate::store::opencode::PromptedSession,
    ) -> Result<crate::store::opencode::PromptedSession, String> {
        let completion = (|| -> Result<(), String> {
            self.recover_identities()?;
            if receipt.session_id.is_none() {
                use rusqlite::OptionalExtension;
                let identity: Option<(String, String)> = self.connection()?.query_row("SELECT session_id, server FROM session_creation_requests WHERE namespace = ?1 AND id = ?2 AND session_id IS NOT NULL", params![self.config.namespace, request], |row| Ok((row.get(0)?, row.get(1)?))).optional().map_err(|error| error.to_string())?;
                if let Some((id, server)) = identity {
                    if receipt
                        .server
                        .as_ref()
                        .is_some_and(|expected| expected != &server)
                    {
                        return Err("recovered session identity belongs to another server".into());
                    }
                    receipt.session_id = Some(id);
                    receipt.server = Some(server);
                }
            }
            self.finish_creation(request, instance, &receipt)
        })();
        if let Err(error) = completion {
            crate::diagnostics::record_error(
                "session correlation persistence failed",
                &std::io::Error::other(error.clone()),
            );
            let persistence_error = format!(
                "Session correlation persistence failed: {error}; inspect the conversation before retrying"
            );
            receipt.error = Some(match receipt.error.take() {
                Some(previous) => format!("{previous}; {persistence_error}"),
                None => persistence_error,
            });
        }
        Ok(receipt)
    }

    pub(crate) fn identity_receipt(&self, request: &str) -> PathBuf {
        self.config
            .home
            .join("runtime")
            .join(&self.config.namespace)
            .join("session-identities")
            .join(format!("{request}.json"))
    }

    pub(super) fn prepare_identity_receipt(
        &self,
        request: &str,
        directory: &str,
    ) -> Result<(), String> {
        let parent = super::super::runtime_db::directory(&self.config)?.join("session-identities");
        match fs::DirBuilder::new().mode(0o700).create(&parent) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
        let metadata = fs::symlink_metadata(&parent).map_err(|error| error.to_string())?;
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
            || fs::canonicalize(&parent).map_err(|error| error.to_string())? != parent
        {
            return Err(
                "session identity directory must be private and owned by the current user".into(),
            );
        }
        let mut file = super::super::config::private_file(&self.identity_receipt(request), true)
            .map_err(|error| error.to_string())?;
        file.write_all(
            json!({"request_id": request, "directory": directory})
                .to_string()
                .as_bytes(),
        )
        .map_err(|error| error.to_string())
    }

    pub(super) fn remove_identity_receipt(&self, request: &str) {
        if let Err(error) = fs::remove_file(self.identity_receipt(request))
            && error.kind() != std::io::ErrorKind::NotFound
        {
            crate::diagnostics::record_error("cannot remove session identity receipt", &error);
        }
    }

    pub(crate) fn recover_identities(&self) -> Result<(), String> {
        let connection = self.connection()?;
        let mut statement = connection.prepare_cached("SELECT id, directory, origin, instance FROM session_creation_requests WHERE namespace = ?1 AND session_id IS NULL").map_err(|error| error.to_string())?;
        let requests = statement
            .query_map([&self.config.namespace], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        drop(statement);
        drop(connection);
        for (request, directory, origin, instance) in requests {
            let path = self.identity_receipt(&request);
            let file = match fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&path)
            {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.to_string()),
            };
            let metadata = file.metadata().map_err(|error| error.to_string())?;
            if !metadata.is_file()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o077 != 0
            {
                return Err("session identity receipt must be a private regular file".into());
            }
            let mut text = String::new();
            file.take(16_385)
                .read_to_string(&mut text)
                .map_err(|error| error.to_string())?;
            if text.len() > 16_384 {
                return Err("session identity receipt exceeds 16 KiB".into());
            }
            // The companion can be midway through replacing the contents of the pinned file.
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            let (Some(id), Some(server)) = (value["session_id"].as_str(), value["server"].as_str())
            else {
                continue;
            };
            if value["request_id"] != request
                || value["directory"] != directory
                || !super::super::opencode::valid_id(id)
                || super::super::opencode::local_server(server).as_deref() != Some(server)
            {
                return Err("session identity receipt does not match its creation request".into());
            }
            let mut connection = self.connection()?;
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| error.to_string())?;
            let exists: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM session_creation_requests WHERE namespace = ?1 AND id = ?2 AND session_id IS NULL)", params![self.config.namespace, request], |row| row.get(0)).map_err(|error| error.to_string())?;
            if !exists {
                continue;
            }
            self.bind(&transaction, json!({"session_id": id, "server": server, "directory": directory, "instance": instance, "title": "", "activity": "unknown", "attached": false, "panes": [], "stale": true, "availability": "unverified", "origins": []}), origin.as_deref())?;
            transaction.execute("UPDATE session_creation_requests SET server = ?3, session_id = ?4 WHERE namespace = ?1 AND id = ?2", params![self.config.namespace, request, server, id]).map_err(|error| error.to_string())?;
            transaction.commit().map_err(|error| error.to_string())?;
            self.remove_identity_receipt(&request);
        }
        Ok(())
    }
}
