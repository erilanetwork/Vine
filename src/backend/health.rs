use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::{debug, info, warn};

use vine_plugin::services::ProxyServices;
use vine_protocol::packets::handshake::{Handshake, HandshakeIntent};
use vine_protocol::packets::status::StatusResponse;
use vine_protocol::{
    PacketKind, PacketReader, PacketWriter, ProtocolVersion, decode_packet, encode_packet,
};

// A status response with a favicon is ~100 KB at most, anything bigger isn't a status response
const STATUS_LIMIT: usize = 256 * 1024;

/// Pings every backend on `health.interval_secs` and records the result in the health
/// registry. Reads the config fresh every round, so reloads apply on the next one.
pub fn spawn_health_checks(services: Arc<ProxyServices>) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let config = services.config();
            let interval = Duration::from_secs(config.health.interval_secs.max(1));
            if !config.health.enabled {
                tokio::time::sleep(interval).await;
                continue;
            }
            services
                .health
                .retain(|name| config.servers.contains_key(name));

            let check_timeout = Duration::from_secs(config.health.timeout_secs.max(1));
            let protocol = config.motd.protocol_version;
            let checks: Vec<_> = config
                .servers
                .iter()
                .map(|(name, server)| {
                    let (name, address) = (name.clone(), server.address.clone());
                    let check = tokio::spawn(async move {
                        timeout(check_timeout, ping(&address, protocol)).await
                    });
                    (name, check)
                })
                .collect();

            for (name, check) in checks {
                let result = match check.await {
                    Ok(Ok(result)) => result,
                    Ok(Err(_)) => Err("timed out".to_string()),
                    Err(e) => Err(e.to_string()),
                };
                match result {
                    Ok((latency, players)) => {
                        let was_down = services.health.is_down(&name);
                        services.health.record_up(&name, latency, players);
                        if was_down {
                            info!("Server '{name}' answers health checks again");
                        }
                    }
                    Err(reason) => {
                        debug!("Health check of '{name}' failed: {reason}");
                        if services
                            .health
                            .record_failure(&name, config.health.unhealthy_after)
                        {
                            warn!(
                                "Server '{name}' stopped answering health checks ({reason}); new players go elsewhere until it is back"
                            );
                        }
                    }
                }
            }

            tokio::time::sleep(interval).await;
        }
    })
}

/// A server list ping, the same one a client sends. Returns how long the answer took and the
/// player counts the backend reported.
async fn ping(address: &str, protocol: i32) -> Result<(Duration, Option<(u32, u32)>), String> {
    let started = Instant::now();
    let stream = TcpStream::connect(address)
        .await
        .map_err(|e| e.to_string())?;
    let _ = stream.set_nodelay(true);
    let (read, write) = stream.into_split();
    let mut reader = PacketReader::new(read);
    reader.set_limits(STATUS_LIMIT, STATUS_LIMIT);
    let mut writer = PacketWriter::new(write);

    let (host, port) = match address.rsplit_once(':') {
        Some((host, port)) => (host, port.parse().unwrap_or(25565)),
        None => (address, 25565),
    };
    let handshake = Handshake {
        protocol_version: protocol,
        server_address: host.trim_matches(['[', ']']).to_string(),
        server_port: port,
        intent: HandshakeIntent::Status,
    };
    // the handshake layout never changed, and the status request is an empty packet with id 0
    let request_id = PacketKind::StatusRequest
        .id(ProtocolVersion::from_handshake(protocol))
        .unwrap_or(0);
    let send = async {
        writer
            .send(encode_packet(&handshake, ProtocolVersion::V1_8)?)
            .await?;
        writer.send(Bytes::from(vec![request_id as u8])).await
    };
    send.await.map_err(|e| e.to_string())?;

    let packet = reader.read_packet().await.map_err(|e| e.to_string())?;
    if !PacketKind::StatusResponse.matches(packet.id, ProtocolVersion::V1_8) {
        return Err(format!("unexpected packet {:#04x}", packet.id));
    }
    let latency = started.elapsed();
    let status: StatusResponse =
        decode_packet(&packet.payload, ProtocolVersion::V1_8).map_err(|e| e.to_string())?;
    Ok((latency, player_counts(&status.json)))
}

fn player_counts(json: &str) -> Option<(u32, u32)> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let players = value.get("players")?;
    let count = |key| {
        players
            .get(key)?
            .as_u64()
            .map(|n| n.min(u32::MAX as u64) as u32)
    };
    Some((count("online")?, count("max")?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_player_counts() {
        let json = r#"{"version":{"name":"x","protocol":1},"players":{"online":4,"max":20}}"#;
        assert_eq!(player_counts(json), Some((4, 20)));
        assert_eq!(player_counts(r#"{"players":{"online":4}}"#), None);
        assert_eq!(player_counts("nope"), None);
    }

    #[tokio::test]
    async fn test_ping_reads_a_status_response() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, write) = stream.into_split();
            let mut reader = PacketReader::new(read);
            let mut writer = PacketWriter::new(write);
            let handshake = reader.read_packet().await.unwrap();
            let handshake: Handshake =
                decode_packet(&handshake.payload, ProtocolVersion::V1_8).unwrap();
            assert_eq!(handshake.intent, HandshakeIntent::Status);
            assert_eq!(reader.read_packet().await.unwrap().id, 0);
            let response = StatusResponse {
                json: r#"{"players":{"online":1,"max":10}}"#.to_string(),
            };
            writer
                .send(encode_packet(&response, ProtocolVersion::V1_8).unwrap())
                .await
                .unwrap();
        });

        let (_, players) = ping(&address, 776).await.unwrap();
        assert_eq!(players, Some((1, 10)));
        assert!(ping("127.0.0.1:1", 776).await.is_err());
    }
}
