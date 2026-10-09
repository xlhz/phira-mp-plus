mod console;
mod ip_access;
pub use ip_access::*;

mod l10n;

mod room;
pub use room::*;

mod server;
pub use server::*;

mod session;
pub use session::*;

mod traffic;
pub use traffic::*;

mod maintenance;
pub use maintenance::*;

use anyhow::Result;
use clap::Parser;
use std::{
    collections::{
        HashMap,
        hash_map::{Entry, VacantEntry},
    },
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::sync::RwLock;
use tracing_appender::non_blocking::WorkerGuard;
use uuid::Uuid;

pub type SafeMap<K, V> = RwLock<HashMap<K, V>>;
pub type IdMap<V> = SafeMap<Uuid, V>;

fn vacant_entry<V>(map: &mut HashMap<Uuid, V>) -> VacantEntry<'_, Uuid, V> {
    let mut id = Uuid::new_v4();
    while map.contains_key(&id) {
        id = Uuid::new_v4();
    }
    match map.entry(id) {
        Entry::Vacant(entry) => entry,
        _ => unreachable!(),
    }
}

struct TuiLogWriter {
    buffer: Arc<Mutex<Vec<String>>>,
}

impl std::io::Write for TuiLogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let s = String::from_utf8_lossy(buf);
        if let Ok(mut buffer) = self.buffer.lock() {
            for line in s.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    buffer.push(trimmed.to_string());
                }
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for TuiLogWriter {
    type Writer = TuiLogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        TuiLogWriter {
            buffer: Arc::clone(&self.buffer),
        }
    }
}

pub fn init_log(file: &str, tui_buffer: Arc<Mutex<Vec<String>>>) -> Result<WorkerGuard> {
    use tracing::{Level, metadata::LevelFilter};
    use tracing_log::LogTracer;
    use tracing_subscriber::{filter, fmt, prelude::*};

    let log_dir = Path::new("log");
    if log_dir.exists() {
        if !log_dir.is_dir() {
            panic!("log exists and is not a folder");
        }
    } else {
        std::fs::create_dir(log_dir).expect("failed to create log folder");
    }

    LogTracer::init()?;

    let (non_blocking, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::hourly(log_dir, file));

    let tui_writer = TuiLogWriter { buffer: tui_buffer };

    let subscriber = tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_writer(non_blocking)
                .with_filter(LevelFilter::DEBUG),
        )
        .with(
            fmt::layer()
                .with_writer(tui_writer)
                .with_filter(LevelFilter::INFO),
        )
        .with(
            filter::Targets::new()
                .with_target("hyper", Level::INFO)
                .with_target("rustls", Level::INFO)
                .with_target("isahc", Level::INFO)
                .with_default(Level::TRACE),
        );

    tracing::subscriber::set_global_default(subscriber).expect("unable to set global subscriber");
    Ok(guard)
}

/// Command line arguments
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(
        short,
        long,
        default_value_t = 12346,
        help = "Specify the port number to use for the server"
    )]
    port: u16,
    #[clap(
        short,
        long,
        default_value = "v4",
        value_parser = ["v4", "v6", "both"],
        help = "IP version to use: v4, v6, or both"
    )]
    ip_version: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let log_buffer: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let _guard = init_log("phira-mp", Arc::clone(&log_buffer))?;

    let args = Args::parse();
    let port = args.port;

    let addrs: Vec<SocketAddr> = match args.ip_version.as_str() {
        "v4" => {
            vec![SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), port)]
        }
        "v6" => {
            vec![SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), port)]
        }
        "both" => {
            vec![
                SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), port),
                SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), port),
            ]
        }
        _ => {
            vec![SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), port)]
        }
    };

    println!("Listening on: {:?}", addrs);

    let server = Server::bind_all(&addrs).await?;
    let state = server.state();

    console::run_console(state, log_buffer);

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
    }
}
