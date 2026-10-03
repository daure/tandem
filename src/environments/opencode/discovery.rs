use std::collections::{BTreeMap, BTreeSet};

use super::{Observer, Presence, transport::local_server};
use crate::store::opencode::{Activity, Session};

impl Observer {
    pub(super) fn inventory_with_sessions(
        &self,
        sessions: &[Session],
    ) -> (Vec<Presence>, BTreeMap<String, BTreeSet<String>>) {
        let (_, mut servers) = self.inventory();
        let presences = self.observed_presences();
        let listening = listening_ports();
        let required = presences
            .iter()
            .map(|presence| (&presence.server, &presence.directory))
            .chain(
                sessions
                    .iter()
                    .filter(|session| {
                        !self.excluded.sessions.contains(&session.id)
                            && (session.activity != Activity::Idle
                                || session.approval_pending == Some(true))
                    })
                    .map(|session| (&session.server, &session.directory)),
            )
            .filter_map(|(server, directory)| {
                local_server(server).map(|server| (server, directory.clone()))
            })
            .collect::<BTreeSet<_>>();
        let observable = |server: &str, directory: &str| {
            // Only definite absence retires a receipt; inaccessible paths remain uncertain.
            let key = (server.to_owned(), directory.to_owned());
            !self.excluded.servers.contains(server)
                && (!self.excluded.directories.contains(&key)
                    || required.contains(&key)
                    || sessions.iter().any(|session| {
                        !self.excluded.sessions.contains(&session.id)
                            && session.server == server
                            && session.directory == directory
                    }))
                && (!matches!(std::path::Path::new(directory).try_exists(), Ok(false))
                    || required.contains(&key))
        };
        for (server, directories) in &mut servers {
            directories.retain(|directory| observable(server, directory));
        }
        for session in sessions {
            if let Some(server) = local_server(&session.server)
                && !self.excluded.sessions.contains(&session.id)
                && observable(&server, &session.directory)
                && (servers.contains_key(&server)
                    || reqwest::Url::parse(&server)
                        .ok()
                        .and_then(|url| url.port())
                        .is_some_and(|port| listening.contains(&port))
                    || session.activity != Activity::Idle
                    || session.approval_pending == Some(true))
            {
                servers
                    .entry(server)
                    .or_default()
                    .insert(session.directory.clone());
            }
        }
        servers.retain(|server, directories| {
            !self.excluded.servers.contains(server) && !directories.is_empty()
        });
        (presences, servers)
    }
}

pub(super) fn listening_ports() -> BTreeSet<u16> {
    // Daemon URLs use IPv4 loopback. Kernel evidence avoids waking dormant servers with HTTP probes.
    std::fs::read_to_string("/proc/net/tcp")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let local = fields.nth(1)?;
            if fields.nth(1)? != "0A" {
                return None;
            }
            let (address, port) = local.split_once(':')?;
            if !matches!(address, "0100007F" | "00000000") {
                return None;
            }
            u16::from_str_radix(port, 16).ok()
        })
        .collect()
}
