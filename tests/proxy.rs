//! Runs a real proxy against a fake backend and talks to it like a 1.8 client would.
//!
//! The backend only knows the handful of packets these tests need: status for the health
//! checks, and a login that ends with LoginSuccess plus one plugin message to forward.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout;

use vine::config::{Config, ForwardingMode};
use vine::console::Console;
use vine::protocol::packets::handshake::{Handshake, HandshakeIntent};
use vine::protocol::packets::login::{LoginDisconnect, LoginStart};
use vine::protocol::packets::play::ClientboundPluginMessage;
use vine::protocol::packets::status::StatusResponse;
use vine::protocol::wire::WriteExt;
use vine::protocol::{
    PacketKind, PacketReader, PacketWriter, ProtocolVersion, decode_packet, encode_packet,
};
use vine::proxy::ProxyServer;

const V1_8: ProtocolVersion = ProtocolVersion::V1_8;
const WAIT: Duration = Duration::from_secs(10);

type Reader = PacketReader<OwnedReadHalf>;
type Writer = PacketWriter<OwnedWriteHalf>;

fn split(stream: TcpStream) -> (Reader, Writer) {
    let (read, write) = stream.into_split();
    (PacketReader::new(read), PacketWriter::new(write))
}

/// A backend that answers pings and logs players in. Every username that logs in is sent
/// down the returned channel.
async fn fake_backend() -> (SocketAddr, mpsc::UnboundedReceiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (logins_tx, logins) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let logins_tx = logins_tx.clone();
            tokio::spawn(async move {
                let (mut reader, mut writer) = split(stream);
                let Ok(handshake) = reader.read_packet().await else {
                    return;
                };
                let handshake: Handshake = decode_packet(&handshake.payload, V1_8).unwrap();
                let version = ProtocolVersion::from_handshake(handshake.protocol_version);
                match handshake.intent {
                    HandshakeIntent::Status => {
                        let _ = reader.read_packet().await;
                        let status = StatusResponse {
                            json: r#"{"version":{"name":"fake","protocol":47},"players":{"online":0,"max":20},"description":"fake"}"#.to_string(),
                        };
                        let _ = writer.send(encode_packet(&status, V1_8).unwrap()).await;
                    }
                    HandshakeIntent::Login => {
                        let start = reader.read_packet().await.unwrap();
                        let start: LoginStart = decode_packet(&start.payload, version).unwrap();
                        let _ = logins_tx.send(start.name.clone());

                        let mut success = Vec::new();
                        success.write_varint(PacketKind::LoginSuccess.id(version).unwrap());
                        success.write_string("00000000-0000-3000-8000-000000000001");
                        success.write_string(&start.name);
                        writer.send(Bytes::from(success)).await.unwrap();

                        let hello = ClientboundPluginMessage {
                            channel: "vine:test".to_string(),
                            data: b"hello from the backend".to_vec(),
                        };
                        writer
                            .send(encode_packet(&hello, version).unwrap())
                            .await
                            .unwrap();
                        // stay connected until the proxy hangs up
                        while reader.read_packet().await.is_ok() {}
                    }
                    HandshakeIntent::Transfer => {}
                }
            });
        }
    });
    (address, logins)
}

async fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

struct Proxy {
    address: SocketAddr,
    shutdown: broadcast::Sender<()>,
    _dir: tempfile::TempDir,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = self.shutdown.send(());
    }
}

/// Starts a proxy whose only backend is `backend`. `prepare` can change the config and drop
/// files into the proxy's folder before it starts.
async fn start_proxy(backend: SocketAddr, prepare: impl FnOnce(&mut Config, &Path)) -> Proxy {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port().await;

    let mut config = Config::default();
    config.server.bind_address = format!("127.0.0.1:{port}");
    config.server.online_mode = false;
    config.server.compression_threshold = -1;
    config.security.rate_limit_enabled = false;
    config.plugins.enabled = false;
    config.routing.forced_hosts.clear();
    config.routing.fallback_server = None;
    config.servers.retain(|name, _| name == "lobby");
    let lobby = config.servers.get_mut("lobby").unwrap();
    lobby.address = backend.to_string();
    lobby.forwarding = ForwardingMode::None;
    prepare(&mut config, dir.path());

    let path = dir.path().join("vine.toml");
    std::fs::write(&path, toml::to_string_pretty(&config).unwrap()).unwrap();
    let config = Config::load(&path).unwrap();
    let server = ProxyServer::new(config, &path).unwrap();
    let (shutdown, _) = broadcast::channel(1);
    let shutdown_tx = shutdown.clone();
    tokio::spawn(async move {
        let _ = server.run(shutdown_tx, Console::headless()).await;
    });

    let address: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    timeout(WAIT, async {
        while TcpStream::connect(address).await.is_err() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the proxy never started listening");
    Proxy {
        address,
        shutdown,
        _dir: dir,
    }
}

async fn handshake(proxy: &Proxy, intent: HandshakeIntent) -> (Reader, Writer) {
    let (reader, mut writer) = split(TcpStream::connect(proxy.address).await.unwrap());
    let handshake = Handshake {
        protocol_version: V1_8.0 as i32,
        server_address: "localhost".to_string(),
        server_port: proxy.address.port(),
        intent,
    };
    writer
        .send(encode_packet(&handshake, V1_8).unwrap())
        .await
        .unwrap();
    (reader, writer)
}

async fn status(proxy: &Proxy) -> serde_json::Value {
    let (mut reader, mut writer) = handshake(proxy, HandshakeIntent::Status).await;
    writer.send(Bytes::from_static(&[0x00])).await.unwrap();
    let packet = timeout(WAIT, reader.read_packet()).await.unwrap().unwrap();
    let status: StatusResponse = decode_packet(&packet.payload, V1_8).unwrap();
    serde_json::from_str(&status.json).unwrap()
}

/// Logs in as `name` and returns the first packet the proxy answers with
async fn login(proxy: &Proxy, name: &str) -> (Reader, Writer, vine::protocol::RawPacket) {
    let (mut reader, mut writer) = handshake(proxy, HandshakeIntent::Login).await;
    let start = LoginStart {
        name: name.to_string(),
        uuid: None,
        profile_key: None,
    };
    writer
        .send(encode_packet(&start, V1_8).unwrap())
        .await
        .unwrap();
    let packet = timeout(WAIT, reader.read_packet()).await.unwrap().unwrap();
    (reader, writer, packet)
}

async fn kick_reason(proxy: &Proxy, name: &str) -> String {
    let (_, _, packet) = login(proxy, name).await;
    assert!(
        PacketKind::LoginDisconnect.matches(packet.id, V1_8),
        "expected a kick, got packet {:#04x}",
        packet.id
    );
    decode_packet::<LoginDisconnect>(&packet.payload, V1_8)
        .unwrap()
        .reason_json
}

#[tokio::test]
async fn test_status_ping_shows_the_motd() {
    let (backend, _) = fake_backend().await;
    let proxy = start_proxy(backend, |_, _| {}).await;

    let status = status(&proxy).await;
    assert_eq!(status["version"]["protocol"], 47);
    assert!(status["description"].to_string().contains("A Vine Server"));
}

#[tokio::test]
async fn test_maintenance_changes_the_server_list_and_kicks_players() {
    let (backend, mut logins) = fake_backend().await;
    let proxy = start_proxy(backend, |config, _| config.maintenance.enabled = true).await;

    let status = status(&proxy).await;
    assert_eq!(status["version"]["protocol"], -1);
    assert_eq!(status["version"]["name"], "Maintenance");
    assert!(status["description"].to_string().contains("maintenance"));

    assert!(kick_reason(&proxy, "Steve").await.contains("maintenance"));
    assert!(logins.try_recv().is_err(), "the backend saw a login");
}

#[tokio::test]
async fn test_login_reaches_the_backend_and_traffic_flows_back() {
    let (backend, mut logins) = fake_backend().await;
    let metrics_port = free_port().await;
    let proxy = start_proxy(backend, |config, _| {
        config.metrics.enabled = true;
        config.metrics.bind_address = format!("127.0.0.1:{metrics_port}");
    })
    .await;

    let (mut reader, _writer, success) = login(&proxy, "Steve").await;
    assert!(PacketKind::LoginSuccess.matches(success.id, V1_8));
    assert_eq!(
        timeout(WAIT, logins.recv()).await.unwrap().unwrap(),
        "Steve"
    );

    let packet = timeout(WAIT, reader.read_packet()).await.unwrap().unwrap();
    let message: ClientboundPluginMessage = decode_packet(&packet.payload, V1_8).unwrap();
    assert_eq!(message.channel, "vine:test");
    assert_eq!(message.data, b"hello from the backend");

    let mut stream = TcpStream::connect(("127.0.0.1", metrics_port))
        .await
        .unwrap();
    stream
        .write_all(b"GET /metrics HTTP/1.1\r\n\r\n")
        .await
        .unwrap();
    let mut metrics = String::new();
    stream.read_to_string(&mut metrics).await.unwrap();
    assert!(metrics.contains("vine_logins_total 1"), "{metrics}");
    assert!(
        metrics.contains("vine_server_players{server=\"lobby\"} 1"),
        "{metrics}"
    );
}

#[tokio::test]
async fn test_banned_players_and_addresses_are_kicked() {
    let (backend, mut logins) = fake_backend().await;
    let proxy = start_proxy(backend, |_, dir| {
        std::fs::write(
            dir.join("banned-players.json"),
            r#"[{"name":"Griefer","reason":"broke the spawn","source":"CONSOLE","created":0}]"#,
        )
        .unwrap();
    })
    .await;

    let reason = kick_reason(&proxy, "Griefer").await;
    assert!(reason.contains("banned"), "{reason}");
    assert!(reason.contains("broke the spawn"), "{reason}");

    // someone else still gets in
    let (_, _, packet) = login(&proxy, "Steve").await;
    assert!(PacketKind::LoginSuccess.matches(packet.id, V1_8));
    assert_eq!(
        timeout(WAIT, logins.recv()).await.unwrap().unwrap(),
        "Steve"
    );

    let (backend, _) = fake_backend().await;
    let proxy = start_proxy(backend, |_, dir| {
        std::fs::write(
            dir.join("banned-ips.json"),
            r#"[{"ip":"127.0.0.1","source":"CONSOLE","created":0}]"#,
        )
        .unwrap();
    })
    .await;
    assert!(kick_reason(&proxy, "Steve").await.contains("banned"));
}

#[tokio::test]
async fn test_whitelist_lets_only_listed_players_in() {
    let (backend, mut logins) = fake_backend().await;
    let proxy = start_proxy(backend, |config, dir| {
        config.whitelist.enabled = true;
        std::fs::write(dir.join("whitelist.json"), r#"[{"name":"Alex"}]"#).unwrap();
    })
    .await;

    assert!(kick_reason(&proxy, "Steve").await.contains("whitelisted"));
    let (_, _, packet) = login(&proxy, "alex").await;
    assert!(PacketKind::LoginSuccess.matches(packet.id, V1_8));
    assert_eq!(timeout(WAIT, logins.recv()).await.unwrap().unwrap(), "alex");
}
