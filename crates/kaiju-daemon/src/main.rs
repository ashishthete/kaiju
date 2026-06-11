use kaiju_daemon::server;
use std::net::SocketAddr;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,kaiju_daemon=debug")),
        )
        .init();

    let port: u16 = std::env::var("KAIJU_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(7800);

    let host = std::env::var("KAIJU_HOST").ok();
    let ip = kaiju_daemon::net::bind_ip(host.as_deref());
    let addr = SocketAddr::new(ip, port);

    if !ip.is_loopback() {
        tracing::warn!(
            "listening on {addr} — reachable from your local network. \
             Set KAIJU_TOKEN or pair devices; unpaired remote requests are rejected."
        );
    }

    if let Err(e) = server::run(addr).await {
        eprintln!("fatal: {e}");
        std::process::exit(1);
    }
}
