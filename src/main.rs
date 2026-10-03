mod listing;
mod reload;
mod server;

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::Request;
use axum::middleware::{self, Next};
use axum::response::Response;
use clap::Parser;
use tokio::net::TcpListener;

/// Serve a directory for local web development, with live reload.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Directory to serve
    #[arg(default_value = ".")]
    dir: PathBuf,

    /// Port to listen on (the next free port is used if it is taken)
    #[arg(short, long, default_value_t = 8080)]
    port: u16,

    /// Address to bind; use 127.0.0.1 to accept only local connections
    #[arg(long, default_value = "0.0.0.0")]
    host: String,

    /// Open the browser after starting
    #[arg(short, long)]
    open: bool,

    /// Serve /index.html for unknown extensionless paths (client-side routing)
    #[arg(long)]
    spa: bool,

    /// Disable file watching and live reload
    #[arg(long)]
    no_reload: bool,

    /// Disable directory listings
    #[arg(long)]
    no_listing: bool,

    /// Don't log requests
    #[arg(short, long)]
    quiet: bool,
}

/// How many ports above `--port` to try before giving up.
const PORT_ATTEMPTS: u16 = 10;

#[tokio::main]
async fn main() {
    if let Err(err) = run(Args::parse()).await {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

async fn run(args: Args) -> Result<(), String> {
    let root = args
        .dir
        .canonicalize()
        .map_err(|e| format!("cannot open {}: {e}", args.dir.display()))?;
    if !root.is_dir() {
        return Err(format!("{} is not a directory", root.display()));
    }

    // Keep the watcher alive for the life of the server.
    let (reload_tx, _watcher) = if args.no_reload {
        (None, None)
    } else {
        let (tx, watcher) = reload::watch(&root, !args.quiet)?;
        (Some(tx), Some(watcher))
    };

    let state = Arc::new(server::Config {
        root: root.clone(),
        spa: args.spa,
        listing: !args.no_listing,
        reload: reload_tx,
    });

    let mut app = server::router(state);
    if !args.quiet {
        app = app.layer(middleware::from_fn(log_request));
    }

    let listener = bind(&args.host, args.port).await?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let url = display_url(addr);

    println!("Serving {}", root.display());
    println!("  → {url}");
    if addr.ip().is_unspecified() {
        match network_ip() {
            Some(ip) => println!("  → http://{} (network)", SocketAddr::new(ip, addr.port())),
            None => println!("  (listening on all interfaces)"),
        }
    }
    if args.port != addr.port() {
        println!("  (port {} was busy)", args.port);
    }
    let features: Vec<&str> = [
        (!args.no_reload, "live reload"),
        (!args.no_listing, "directory listing"),
        (args.spa, "SPA fallback"),
    ]
    .into_iter()
    .filter_map(|(on, name)| on.then_some(name))
    .collect();
    if !features.is_empty() {
        println!("  {}", features.join(", "));
    }

    if args.open
        && let Err(e) = open::that(&url)
    {
        eprintln!("could not open browser: {e}");
    }

    axum::serve(listener, app).await.map_err(|e| e.to_string())
}

async fn bind(host: &str, port: u16) -> Result<TcpListener, String> {
    let mut last_err = None;
    for p in port..port.saturating_add(PORT_ATTEMPTS) {
        match try_bind(host, p).await {
            Ok(listener) => return Ok(listener),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => last_err = Some(e),
            Err(e) => return Err(format!("cannot bind {host}:{p}: {e}")),
        }
    }
    Err(format!(
        "ports {port}-{} are all in use ({})",
        port.saturating_add(PORT_ATTEMPTS - 1),
        last_err.map(|e| e.to_string()).unwrap_or_default()
    ))
}

/// Binds `host:port`. For a wildcard host, first checks that loopback is free on
/// that port: macOS lets the wildcard bind succeed even when another server holds
/// 127.0.0.1 there, and `localhost` would then reach that server instead of us.
async fn try_bind(host: &str, port: u16) -> std::io::Result<TcpListener> {
    if let Ok(ip) = host.parse::<IpAddr>()
        && ip.is_unspecified()
    {
        let loopback: IpAddr = match ip {
            IpAddr::V4(_) => Ipv4Addr::LOCALHOST.into(),
            IpAddr::V6(_) => Ipv6Addr::LOCALHOST.into(),
        };
        drop(TcpListener::bind((loopback, port)).await?);
    }
    TcpListener::bind((host, port)).await
}

/// The address other machines on the network can most likely reach us at: the
/// local end of the default route. Connecting a UDP socket sends no packets.
fn network_ip() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 80)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

fn display_url(addr: SocketAddr) -> String {
    let ip = addr.ip();
    if ip.is_unspecified() || ip.is_loopback() {
        format!("http://localhost:{}", addr.port())
    } else {
        format!("http://{addr}")
    }
}

async fn log_request(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let start = Instant::now();
    let res = next.run(req).await;
    if !path.starts_with(server::INTERNAL_PREFIX) {
        println!(
            "{} {method} {path} {:.1}ms",
            res.status().as_u16(),
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
    res
}
