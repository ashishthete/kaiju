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

    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    if let Err(e) = server::run(addr).await {
        eprintln!("fatal: {e}");
        std::process::exit(1);
    }
}
