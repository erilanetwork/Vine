use std::collections::BTreeMap;
use std::fmt::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tracing::{debug, info};

use vine_plugin::metrics::Metrics;
use vine_plugin::plugin::PluginManager;
use vine_plugin::services::ProxyServices;
use vine_plugin::session::SessionManager;

const REQUEST_LIMIT: usize = 8 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

struct Scrape {
    services: Arc<ProxyServices>,
    sessions: Arc<SessionManager>,
    plugins: Arc<PluginManager>,
    started: Instant,
}

/// Serves Prometheus metrics at `http://<address>/metrics`. A tiny hand rolled HTTP server is
/// plenty for one scraper hitting one path, so this doesn't pull in a web framework.
pub async fn serve(
    address: &str,
    services: Arc<ProxyServices>,
    sessions: Arc<SessionManager>,
    plugins: Arc<PluginManager>,
) -> std::io::Result<JoinHandle<()>> {
    let listener = TcpListener::bind(address).await?;
    info!(
        "Serving metrics on http://{}/metrics",
        listener.local_addr()?
    );
    Ok(spawn(listener, services, sessions, plugins))
}

fn spawn(
    listener: TcpListener,
    services: Arc<ProxyServices>,
    sessions: Arc<SessionManager>,
    plugins: Arc<PluginManager>,
) -> JoinHandle<()> {
    let scrape = Arc::new(Scrape {
        services,
        sessions,
        plugins,
        started: Instant::now(),
    });
    tokio::spawn(async move {
        loop {
            let Ok((stream, peer)) = listener.accept().await else {
                continue;
            };
            let scrape = scrape.clone();
            tokio::spawn(async move {
                if let Err(e) = tokio::time::timeout(REQUEST_TIMEOUT, answer(stream, &scrape)).await
                {
                    debug!("[{peer}] Metrics request timed out: {e}");
                }
            });
        }
    })
}

async fn answer(mut stream: TcpStream, scrape: &Scrape) -> std::io::Result<()> {
    let mut request = Vec::new();
    let mut buf = [0u8; 1024];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut buf).await?;
        if n == 0 || request.len() + n > REQUEST_LIMIT {
            return Ok(());
        }
        request.extend_from_slice(&buf[..n]);
    }
    let line = request.split(|b| *b == b'\n').next().unwrap_or_default();
    let line = String::from_utf8_lossy(line);
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next(), parts.next().unwrap_or_default());
    let path = path.split('?').next().unwrap_or_default();

    let response = if method != Some("GET") {
        reply("405 Method Not Allowed", "text/plain", "GET only\n")
    } else if path == "/metrics" {
        let body = render(scrape).await;
        reply("200 OK", "text/plain; version=0.0.4; charset=utf-8", &body)
    } else {
        reply("404 Not Found", "text/plain", "try /metrics\n")
    };
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await
}

fn reply(status: &str, content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Prometheus label values only need backslashes, quotes and newlines escaped
fn label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn metric(out: &mut String, name: &str, kind: &str, help: &str) {
    let _ = writeln!(out, "# HELP {name} {help}");
    let _ = writeln!(out, "# TYPE {name} {kind}");
}

async fn render(scrape: &Scrape) -> String {
    let services = &scrape.services;
    let config = services.config();
    let sessions = scrape.sessions.list_sessions();
    let health = services.health.snapshot();
    let queues = services.queue.lengths();
    let counters = &services.metrics;
    let mut out = String::new();

    metric(&mut out, "vine_info", "gauge", "Vine version");
    let _ = writeln!(
        out,
        "vine_info{{version=\"{}\"}} 1",
        env!("CARGO_PKG_VERSION")
    );
    metric(
        &mut out,
        "vine_uptime_seconds",
        "gauge",
        "Seconds since the proxy started",
    );
    let _ = writeln!(
        out,
        "vine_uptime_seconds {}",
        scrape.started.elapsed().as_secs()
    );

    metric(
        &mut out,
        "vine_players_online",
        "gauge",
        "Players connected to the proxy",
    );
    let _ = writeln!(out, "vine_players_online {}", sessions.len());

    let mut per_server: BTreeMap<&str, usize> = config
        .servers
        .keys()
        .map(|name| (name.as_str(), 0))
        .collect();
    for session in &sessions {
        if let Some(count) = per_server.get_mut(session.current_server.as_str()) {
            *count += 1;
        }
    }
    metric(
        &mut out,
        "vine_server_players",
        "gauge",
        "Players on each backend, as the proxy counts them",
    );
    for (server, count) in &per_server {
        let _ = writeln!(
            out,
            "vine_server_players{{server=\"{}\"}} {count}",
            label(server)
        );
    }
    metric(
        &mut out,
        "vine_server_up",
        "gauge",
        "Whether the backend answers health checks",
    );
    for server in per_server.keys() {
        let up = health.get(*server).is_none_or(|h| h.online);
        let _ = writeln!(
            out,
            "vine_server_up{{server=\"{}\"}} {}",
            label(server),
            u8::from(up)
        );
    }
    metric(
        &mut out,
        "vine_server_ping_seconds",
        "gauge",
        "How long the last health check took",
    );
    for (server, h) in &health {
        if let Some(latency) = h.latency {
            let _ = writeln!(
                out,
                "vine_server_ping_seconds{{server=\"{}\"}} {:.4}",
                label(server),
                latency.as_secs_f64()
            );
        }
    }
    metric(
        &mut out,
        "vine_server_reported_players",
        "gauge",
        "Players the backend itself reports",
    );
    for (server, h) in &health {
        if let Some((online, _)) = h.players {
            let _ = writeln!(
                out,
                "vine_server_reported_players{{server=\"{}\"}} {online}",
                label(server)
            );
        }
    }
    metric(
        &mut out,
        "vine_queue_length",
        "gauge",
        "Players waiting in each server's queue",
    );
    for server in per_server.keys() {
        let waiting = queues.get(*server).copied().unwrap_or_default();
        let _ = writeln!(
            out,
            "vine_queue_length{{server=\"{}\"}} {waiting}",
            label(server)
        );
    }

    let plugins = scrape.plugins.plugin_list().await;
    metric(&mut out, "vine_plugins", "gauge", "Loaded plugins");
    let _ = writeln!(
        out,
        "vine_plugins{{state=\"enabled\"}} {}",
        plugins.iter().filter(|(_, d)| !d).count()
    );
    let _ = writeln!(
        out,
        "vine_plugins{{state=\"disabled\"}} {}",
        plugins.iter().filter(|(_, d)| *d).count()
    );

    for (name, help, counter) in [
        (
            "vine_connections_total",
            "TCP connections accepted",
            &counters.connections,
        ),
        (
            "vine_connections_rate_limited_total",
            "Connections dropped by the rate limiter",
            &counters.rate_limited,
        ),
        (
            "vine_status_pings_total",
            "Server list pings answered",
            &counters.status_pings,
        ),
        (
            "vine_logins_total",
            "Players that made it onto a backend",
            &counters.logins,
        ),
        (
            "vine_logins_rejected_total",
            "Logins refused by bans, maintenance or the whitelist",
            &counters.logins_rejected,
        ),
        (
            "vine_server_switches_total",
            "Finished server switches",
            &counters.server_switches,
        ),
    ] {
        metric(&mut out, name, "counter", help);
        let _ = writeln!(out, "{name} {}", Metrics::get(counter));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_metrics_endpoint() {
        let services = ProxyServices::from_config(vine_config::Config::default());
        Metrics::inc(&services.metrics.logins);
        services.health.record_failure("vanilla", 1);
        let plugins = PluginManager::new(
            tempfile::tempdir().unwrap().path().to_path_buf(),
            (&vine_config::PluginConfig::default()).into(),
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = spawn(listener, services, Arc::new(SessionManager::new()), plugins);

        let get = |path: &'static str| async move {
            let mut stream = TcpStream::connect(address).await.unwrap();
            let request = format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n");
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            response
        };

        let response = get("/metrics").await;
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("vine_logins_total 1"));
        assert!(response.contains("vine_server_up{server=\"vanilla\"} 0"));
        assert!(response.contains("vine_server_up{server=\"lobby\"} 1"));
        assert!(get("/").await.starts_with("HTTP/1.1 404"));
        task.abort();
    }

    #[test]
    fn test_label_escaping() {
        assert_eq!(label("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
    }
}
