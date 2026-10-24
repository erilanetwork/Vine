mod duration;
mod store;

pub use duration::{format_duration, parse_duration};

use std::net::IpAddr;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use store::JsonStore;

pub const WHITELIST_FILE: &str = "whitelist.json";
pub const BANNED_PLAYERS_FILE: &str = "banned-players.json";
pub const BANNED_IPS_FILE: &str = "banned-ips.json";

/// Seconds since the Unix epoch, which is what the ban files store
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// The proxy's bans and whitelist, each kept in its own JSON file
pub struct Moderation {
    pub bans: BanList,
    pub whitelist: Whitelist,
}

impl Moderation {
    /// Loads the lists from `dir`, starting empty for files that don't exist yet
    pub fn load(dir: &Path) -> Result<Self, String> {
        Ok(Self {
            bans: BanList {
                players: JsonStore::load(&dir.join(BANNED_PLAYERS_FILE))?,
                ips: JsonStore::load(&dir.join(BANNED_IPS_FILE))?,
            },
            whitelist: Whitelist {
                entries: JsonStore::load(&dir.join(WHITELIST_FILE))?,
            },
        })
    }

    /// Lists that are never written anywhere, for tests and tools
    pub fn in_memory() -> Self {
        Self {
            bans: BanList {
                players: JsonStore::in_memory(),
                ips: JsonStore::in_memory(),
            },
            whitelist: Whitelist {
                entries: JsonStore::in_memory(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ban {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Who placed the ban, a player name or `CONSOLE`
    pub source: String,
    pub created: u64,
    /// When the ban runs out, as seconds since the Unix epoch; none means permanent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<u64>,
}

impl Ban {
    pub fn is_expired(&self, now: u64) -> bool {
        self.expires.is_some_and(|expires| expires <= now)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerBan {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<Uuid>,
    #[serde(flatten)]
    pub ban: Ban,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IpBan {
    pub ip: IpAddr,
    #[serde(flatten)]
    pub ban: Ban,
}

pub struct BanList {
    players: JsonStore<PlayerBan>,
    ips: JsonStore<IpBan>,
}

impl BanList {
    /// Bans the player, replacing an earlier ban of the same name or UUID
    pub fn ban_player(&self, name: &str, uuid: Option<Uuid>, ban: Ban) -> Result<(), String> {
        let mut players = self.players.lock();
        players.retain(|entry| !matches_player(entry, name, uuid));
        players.push(PlayerBan {
            name: name.to_string(),
            uuid,
            ban,
        });
        self.players.save(&players)
    }

    /// Returns false if the player wasn't banned
    pub fn unban_player(&self, name: &str) -> Result<bool, String> {
        let mut players = self.players.lock();
        let before = players.len();
        players.retain(|entry| !entry.name.eq_ignore_ascii_case(name));
        if players.len() == before {
            return Ok(false);
        }
        self.players.save(&players).map(|()| true)
    }

    /// The ban on this player, matched by UUID or by name. Expired bans are dropped on the way.
    pub fn player_ban(&self, name: &str, uuid: Option<Uuid>) -> Option<Ban> {
        let mut players = self.players.lock();
        let now = now();
        let before = players.len();
        players.retain(|entry| !entry.ban.is_expired(now));
        if players.len() != before {
            let _ = self.players.save(&players);
        }
        players
            .iter()
            .find(|entry| matches_player(entry, name, uuid))
            .map(|entry| entry.ban.clone())
    }

    pub fn players(&self) -> Vec<PlayerBan> {
        let now = now();
        let players = self.players.lock();
        players
            .iter()
            .filter(|entry| !entry.ban.is_expired(now))
            .cloned()
            .collect()
    }

    pub fn ban_ip(&self, ip: IpAddr, ban: Ban) -> Result<(), String> {
        let mut ips = self.ips.lock();
        ips.retain(|entry| entry.ip != ip);
        ips.push(IpBan { ip, ban });
        self.ips.save(&ips)
    }

    pub fn unban_ip(&self, ip: IpAddr) -> Result<bool, String> {
        let mut ips = self.ips.lock();
        let before = ips.len();
        ips.retain(|entry| entry.ip != ip);
        if ips.len() == before {
            return Ok(false);
        }
        self.ips.save(&ips).map(|()| true)
    }

    pub fn ip_ban(&self, ip: IpAddr) -> Option<Ban> {
        let now = now();
        let ips = self.ips.lock();
        ips.iter()
            .find(|entry| entry.ip == ip && !entry.ban.is_expired(now))
            .map(|entry| entry.ban.clone())
    }

    pub fn ips(&self) -> Vec<IpBan> {
        let now = now();
        let ips = self.ips.lock();
        ips.iter()
            .filter(|entry| !entry.ban.is_expired(now))
            .cloned()
            .collect()
    }
}

fn matches_player(entry: &PlayerBan, name: &str, uuid: Option<Uuid>) -> bool {
    entry.name.eq_ignore_ascii_case(name) || (uuid.is_some() && entry.uuid == uuid)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WhitelistEntry {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<Uuid>,
}

pub struct Whitelist {
    entries: JsonStore<WhitelistEntry>,
}

impl Whitelist {
    /// Returns false if the player was already on it
    pub fn add(&self, name: &str, uuid: Option<Uuid>) -> Result<bool, String> {
        let mut entries = self.entries.lock();
        if let Some(entry) = entries
            .iter_mut()
            .find(|entry| entry.name.eq_ignore_ascii_case(name))
        {
            // hmm, an entry added by name picks up the UUID once we know it
            if entry.uuid.is_none() && uuid.is_some() {
                entry.uuid = uuid;
                self.entries.save(&entries)?;
            }
            return Ok(false);
        }
        entries.push(WhitelistEntry {
            name: name.to_string(),
            uuid,
        });
        self.entries.save(&entries).map(|()| true)
    }

    pub fn remove(&self, name: &str) -> Result<bool, String> {
        let mut entries = self.entries.lock();
        let before = entries.len();
        entries.retain(|entry| !entry.name.eq_ignore_ascii_case(name));
        if entries.len() == before {
            return Ok(false);
        }
        self.entries.save(&entries).map(|()| true)
    }

    pub fn contains(&self, name: &str, uuid: Uuid) -> bool {
        self.entries.lock().iter().any(|entry| {
            entry.uuid == Some(uuid)
                || (entry.uuid.is_none() && entry.name.eq_ignore_ascii_case(name))
        })
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .entries
            .lock()
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        names.sort_by_key(|name| name.to_ascii_lowercase());
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ban(expires: Option<u64>) -> Ban {
        Ban {
            reason: Some("griefing".to_string()),
            source: "CONSOLE".to_string(),
            created: now(),
            expires,
        }
    }

    #[test]
    fn test_bans_survive_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let uuid = Uuid::from_u128(7);
        {
            let moderation = Moderation::load(dir.path()).unwrap();
            moderation
                .bans
                .ban_player("Steve", Some(uuid), ban(None))
                .unwrap();
            moderation
                .bans
                .ban_ip("10.0.0.1".parse().unwrap(), ban(None))
                .unwrap();
        }

        let moderation = Moderation::load(dir.path()).unwrap();
        // a renamed account is still caught by its UUID
        assert!(moderation.bans.player_ban("Renamed", Some(uuid)).is_some());
        assert!(moderation.bans.player_ban("steve", None).is_some());
        assert!(
            moderation
                .bans
                .ip_ban("10.0.0.1".parse().unwrap())
                .is_some()
        );
        assert!(
            moderation
                .bans
                .ip_ban("10.0.0.2".parse().unwrap())
                .is_none()
        );

        assert!(moderation.bans.unban_player("STEVE").unwrap());
        assert!(!moderation.bans.unban_player("Steve").unwrap());
        assert!(moderation.bans.player_ban("Steve", Some(uuid)).is_none());
    }

    #[test]
    fn test_expired_bans_are_ignored() {
        let moderation = Moderation::in_memory();
        moderation
            .bans
            .ban_player("Alex", None, ban(Some(now() - 1)))
            .unwrap();
        assert!(moderation.bans.player_ban("Alex", None).is_none());
        assert!(moderation.bans.players().is_empty());
    }

    #[test]
    fn test_whitelist_matches_by_uuid_once_known() {
        let moderation = Moderation::in_memory();
        let whitelist = &moderation.whitelist;
        assert!(whitelist.add("Alex", None).unwrap());
        assert!(whitelist.contains("alex", Uuid::from_u128(1)));

        assert!(!whitelist.add("Alex", Some(Uuid::from_u128(1))).unwrap());
        // the name alone no longer gets someone else in
        assert!(!whitelist.contains("Alex", Uuid::from_u128(2)));
        assert!(whitelist.contains("NewName", Uuid::from_u128(1)));

        assert!(whitelist.remove("ALEX").unwrap());
        assert!(whitelist.names().is_empty());
    }
}
