use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;
use uuid::Uuid;
use vine_config::Config;
use vine_i18n::translate;
use vine_protocol::text::parse_message;

use crate::services::ProxyServices;
use crate::session::{PlayerAction, PlayerSession, SessionManager, send_reliably};

pub const BYPASS_PERMISSION: &str = "vine.queue.bypass";
pub const PRIORITY_PERMISSION: &str = "vine.queue.priority";

// A player we sent to a server holds a slot until they show up there, or until this runs out
// because the switch failed
const SLOT_HOLD: Duration = Duration::from_secs(15);
const POSITION_EVERY: Duration = Duration::from_secs(15);
const TICK: Duration = Duration::from_secs(1);

#[derive(Clone, Debug)]
struct Waiting {
    uuid: Uuid,
    priority: bool,
}

#[derive(Default)]
struct State {
    queues: HashMap<String, VecDeque<Waiting>>,
    // players on their way to a server, keyed by UUID
    sending: HashMap<Uuid, (String, Instant)>,
}

/// Queues for backends that have a `max_players` limit. Players join one by asking for a full
/// server and leave it when they get in, disconnect or ask for another server.
#[derive(Default)]
pub struct QueueManager {
    state: Mutex<State>,
}

impl QueueManager {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether a new player would go over the server's `max_players`, counting players already
    /// on their way there
    pub fn is_full(&self, config: &Config, server: &str, sessions: &[PlayerSession]) -> bool {
        let Some(max) = config.servers.get(server).and_then(|s| s.max_players) else {
            return false;
        };
        let state = self.lock();
        occupied(&state, server, sessions) >= max as usize
    }

    /// Puts the player in the server's queue, leaving any other queue they were in. Players
    /// with priority go ahead of everyone without it. Returns their position, starting at 1.
    pub fn join(&self, server: &str, uuid: Uuid, priority: bool) -> usize {
        let mut state = self.lock();
        for queue in state.queues.values_mut() {
            queue.retain(|w| w.uuid != uuid);
        }
        let queue = state.queues.entry(server.to_string()).or_default();
        let at = if priority {
            queue
                .iter()
                .position(|w| !w.priority)
                .unwrap_or(queue.len())
        } else {
            queue.len()
        };
        queue.insert(at, Waiting { uuid, priority });
        at + 1
    }

    /// Returns the server the player was queued for, if any
    pub fn leave(&self, uuid: Uuid) -> Option<String> {
        let mut state = self.lock();
        let mut left = None;
        for (server, queue) in state.queues.iter_mut() {
            let before = queue.len();
            queue.retain(|w| w.uuid != uuid);
            if queue.len() != before {
                left = Some(server.clone());
            }
        }
        left
    }

    /// The server the player waits for and their place in line
    pub fn position(&self, uuid: Uuid) -> Option<(String, usize)> {
        let state = self.lock();
        state.queues.iter().find_map(|(server, queue)| {
            queue
                .iter()
                .position(|w| w.uuid == uuid)
                .map(|i| (server.clone(), i + 1))
        })
    }

    pub fn len(&self, server: &str) -> usize {
        self.lock().queues.get(server).map_or(0, VecDeque::len)
    }

    pub fn lengths(&self) -> HashMap<String, usize> {
        self.lock()
            .queues
            .iter()
            .map(|(server, queue)| (server.clone(), queue.len()))
            .collect()
    }

    /// Lets in as many players as each server has room for. Returns who to send where.
    fn admit(&self, services: &ProxyServices, sessions: &[PlayerSession]) -> Vec<(Uuid, String)> {
        let config = services.config();
        let mut state = self.lock();
        let now = Instant::now();
        state.sending.retain(|uuid, (server, sent)| {
            now.duration_since(*sent) < SLOT_HOLD
                && sessions
                    .iter()
                    .any(|s| s.uuid == *uuid && s.current_server != *server)
        });

        let mut admitted = Vec::new();
        let servers: Vec<String> = state.queues.keys().cloned().collect();
        for server in servers {
            let Some(backend) = config.servers.get(&server) else {
                state.queues.remove(&server);
                continue;
            };
            // gone players and players already there drop out first
            if let Some(queue) = state.queues.get_mut(&server) {
                queue.retain(|w| {
                    sessions
                        .iter()
                        .any(|s| s.uuid == w.uuid && s.current_server != server)
                });
            }
            if services.health.is_down(&server) {
                continue;
            }
            let room = match backend.max_players {
                Some(max) => (max as usize).saturating_sub(occupied(&state, &server, sessions)),
                None => usize::MAX,
            };
            for _ in 0..room {
                let Some(next) = state.queues.get_mut(&server).and_then(VecDeque::pop_front) else {
                    break;
                };
                state.sending.insert(next.uuid, (server.clone(), now));
                admitted.push((next.uuid, server.clone()));
            }
        }
        state.queues.retain(|_, queue| !queue.is_empty());
        admitted
    }

    fn waiting(&self) -> Vec<(Uuid, String, usize)> {
        let state = self.lock();
        state
            .queues
            .iter()
            .flat_map(|(server, queue)| {
                queue
                    .iter()
                    .enumerate()
                    .map(move |(i, w)| (w.uuid, server.clone(), i + 1))
            })
            .collect()
    }
}

fn occupied(state: &State, server: &str, sessions: &[PlayerSession]) -> usize {
    let on_server = sessions
        .iter()
        .filter(|s| s.current_server == server)
        .count();
    let on_the_way = state.sending.values().filter(|(s, _)| s == server).count();
    on_server + on_the_way
}

/// Moves queued players onto their server as slots open up, and reminds them of their place
/// every now and then
pub fn spawn_queue(
    services: Arc<ProxyServices>,
    session_manager: Arc<SessionManager>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut last_reminder = Instant::now();
        loop {
            tokio::time::sleep(TICK).await;
            let sessions = session_manager.list_sessions();
            for (uuid, server) in services.queue.admit(&services, &sessions) {
                if let Some(session) = sessions.iter().find(|s| s.uuid == uuid) {
                    let message = translate(
                        session.effective_locale(),
                        "vine.queue.admitted",
                        &[&server],
                    );
                    send_reliably(
                        &session.action_tx,
                        PlayerAction::Message(parse_message(&message)),
                    );
                    send_reliably(
                        &session.action_tx,
                        PlayerAction::Connect {
                            server_name: server,
                        },
                    );
                }
            }

            if last_reminder.elapsed() >= POSITION_EVERY {
                last_reminder = Instant::now();
                for (uuid, server, position) in services.queue.waiting() {
                    if let Some(session) = sessions.iter().find(|s| s.uuid == uuid) {
                        let message = translate(
                            session.effective_locale(),
                            "vine.queue.position",
                            &[&position, &server],
                        );
                        let _ = session
                            .action_tx
                            .try_send(PlayerAction::Message(parse_message(&message)));
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn session(uuid: u128, server: &str) -> PlayerSession {
        let (tx, _rx) = mpsc::channel(8);
        PlayerSession {
            username: format!("player{uuid}"),
            uuid: Uuid::from_u128(uuid),
            current_server: server.to_string(),
            action_tx: tx,
            client_ip: "127.0.0.1".to_string(),
            protocol_version: 769,
            locale: None,
            locale_override: None,
            plugin_waiting: Default::default(),
        }
    }

    fn services(max: u32) -> Arc<ProxyServices> {
        let mut config = Config::default();
        config.servers.get_mut("survival").unwrap().max_players = Some(max);
        ProxyServices::from_config(config)
    }

    #[test]
    fn test_priority_players_go_first() {
        let queue = QueueManager::default();
        assert_eq!(queue.join("survival", Uuid::from_u128(1), false), 1);
        assert_eq!(queue.join("survival", Uuid::from_u128(2), false), 2);
        assert_eq!(queue.join("survival", Uuid::from_u128(3), true), 1);
        assert_eq!(
            queue.position(Uuid::from_u128(1)),
            Some(("survival".to_string(), 2))
        );

        // joining another queue leaves the first one
        queue.join("lobby", Uuid::from_u128(1), false);
        assert_eq!(queue.len("survival"), 2);
        assert_eq!(queue.leave(Uuid::from_u128(1)), Some("lobby".to_string()));
    }

    #[test]
    fn test_admits_only_as_many_as_fit() {
        let services = services(2);
        let sessions = vec![
            session(1, "survival"),
            session(2, "lobby"),
            session(3, "lobby"),
            session(4, "lobby"),
        ];
        let config = services.config();
        assert!(!services.queue.is_full(&config, "survival", &sessions));
        assert!(!services.queue.is_full(&config, "lobby", &sessions));

        for uuid in [2, 3, 4] {
            services
                .queue
                .join("survival", Uuid::from_u128(uuid), false);
        }
        let admitted = services.queue.admit(&services, &sessions);
        assert_eq!(admitted, vec![(Uuid::from_u128(2), "survival".to_string())]);
        // player 2 holds the last slot while on the way
        assert!(services.queue.is_full(&config, "survival", &sessions));
        assert!(services.queue.admit(&services, &sessions).is_empty());
        assert_eq!(services.queue.len("survival"), 2);

        // a queued player who logs off drops out
        let sessions: Vec<_> = sessions
            .into_iter()
            .filter(|s| s.uuid != Uuid::from_u128(4))
            .collect();
        services.queue.admit(&services, &sessions);
        assert_eq!(services.queue.len("survival"), 1);
    }
}
