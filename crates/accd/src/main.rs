//! accd: the Agent Control Center daemon.
//!
//! Usage: `accd [--data-dir <dir>]`. Exits at once (status 0) if a daemon
//! already serves the data dir.

use accd::daemon::DaemonConfig;
use accd::paths::Paths;
use accd::server::{run_server, AlreadyRunning};
use std::path::PathBuf;

const USAGE: &str = "usage: accd [--data-dir <dir>]";

fn data_dir_arg() -> Result<Option<PathBuf>, &'static str> {
    let mut args = std::env::args_os().skip(1);
    match (args.next(), args.next(), args.next()) {
        (None, _, _) => Ok(None),
        (Some(flag), Some(dir), None) if flag == "--data-dir" => Ok(Some(PathBuf::from(dir))),
        _ => Err(USAGE),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let paths = match data_dir_arg() {
        Ok(Some(dir)) => Paths::in_dir(&dir),
        Ok(None) => Paths::resolve()?,
        Err(usage) => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    };
    let hook_bin = std::env::current_exe()?.with_file_name("acc-hook");
    let server = match run_server(DaemonConfig::new(paths, hook_bin)).await {
        Ok(server) => server,
        Err(e) if e.is::<AlreadyRunning>() => {
            println!("accd already running");
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    tokio::signal::ctrl_c().await?;
    server.shutdown().await;
    Ok(())
}
