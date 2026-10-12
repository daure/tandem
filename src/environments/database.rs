use std::{
    fs,
    ops::{Deref, DerefMut},
    os::unix::fs::MetadataExt,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use rusqlite::Connection;

const IDLE_LIMIT: usize = 4;
const STATEMENT_LIMIT: usize = 64;

#[derive(Clone, Copy, PartialEq)]
struct Identity {
    device: u64,
    inode: u64,
    mode: u32,
    uid: u32,
    gid: u32,
}

impl From<&fs::Metadata> for Identity {
    fn from(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            uid: metadata.uid(),
            gid: metadata.gid(),
        }
    }
}

struct Idle {
    connection: Connection,
    identity: Identity,
}

pub(super) struct Pool {
    path: PathBuf,
    idle: Mutex<Vec<Idle>>,
}

impl Pool {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            idle: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn get(
        self: &Arc<Self>,
        initialize: impl FnOnce(&Connection) -> rusqlite::Result<()>,
    ) -> Result<Lease, String> {
        let identity = match fs::metadata(&self.path) {
            Ok(metadata) => Some(Identity::from(&metadata)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.to_string()),
        };
        let cached = {
            let mut idle = self.idle.lock().unwrap_or_else(|error| error.into_inner());
            idle.retain(|entry| Some(entry.identity) == identity);
            idle.pop()
        };
        let cached = match cached {
            Some(cached) => cached,
            None => {
                let connection = Connection::open(&self.path).map_err(|error| error.to_string())?;
                connection
                    .busy_timeout(Duration::from_secs(5))
                    .map_err(|error| error.to_string())?;
                connection.set_prepared_statement_cache_capacity(STATEMENT_LIMIT);
                initialize(&connection).map_err(|error| error.to_string())?;
                let metadata = fs::metadata(&self.path).map_err(|error| error.to_string())?;
                Idle {
                    connection,
                    identity: Identity::from(&metadata),
                }
            }
        };
        Ok(Lease {
            cached: Some(cached),
            pool: Arc::clone(self),
        })
    }
}

pub(super) struct Lease {
    cached: Option<Idle>,
    pool: Arc<Pool>,
}

impl Deref for Lease {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        &self
            .cached
            .as_ref()
            .expect("connection lease is occupied")
            .connection
    }
}

impl DerefMut for Lease {
    fn deref_mut(&mut self) -> &mut Connection {
        &mut self
            .cached
            .as_mut()
            .expect("connection lease is occupied")
            .connection
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let cached = self.cached.take().expect("connection lease is occupied");
        // Closing an unfinished transaction preserves rollback and releases its locks.
        if !cached.connection.is_autocommit() {
            return;
        }
        let mut idle = self
            .pool
            .idle
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if idle.len() < IDLE_LIMIT {
            idle.push(cached);
        }
    }
}

#[cfg(test)]
#[path = "tests/database.rs"]
mod tests;
