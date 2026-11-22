//! The connection between the app, the daemon and the hook, which all speak
//! newline-delimited JSON over it. The daemon names the endpoint
//! (`acc_protocol::socket_path`).
//!
//! On macOS and Linux it is a Unix socket in the data dir, mode 0600. On
//! Windows it is a named pipe that only the current user can open and remote
//! clients cannot reach. Named pipes share one namespace for all users, so a
//! client there also checks that the pipe is served by a process of its own
//! user, and the daemon refuses to start on a name that is already taken.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::{connect, connect_blocking, server_pid, BlockingStream, ClientStream, Listener, ServerStream};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{connect, connect_blocking, server_pid, BlockingStream, ClientStream, Listener, ServerStream};

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    fn endpoint(dir: &Path) -> PathBuf {
        acc_protocol::socket_path(dir)
    }

    /// Answers each line on `stream` with `echo: <line>` until it closes.
    async fn echo(stream: ServerStream) {
        let (reader, mut writer) = tokio::io::split(stream);
        let mut lines = tokio::io::BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            writer.write_all(format!("echo: {line}\n").as_bytes()).await.unwrap();
        }
    }

    #[tokio::test]
    async fn a_client_talks_to_the_listener() {
        let dir = tempfile::tempdir().unwrap();
        let mut listener = Listener::bind(&endpoint(dir.path())).unwrap();
        let client = connect(&endpoint(dir.path())).await.unwrap();
        echo_once(&mut listener, client).await;
    }

    async fn echo_once(listener: &mut Listener, client: ClientStream) {
        let served = tokio::spawn({
            let stream = listener.accept().await.unwrap();
            echo(stream)
        });
        let (reader, mut writer) = tokio::io::split(client);
        writer.write_all(b"hello\n").await.unwrap();
        let mut lines = tokio::io::BufReader::new(reader).lines();
        assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("echo: hello"));
        drop(writer);
        drop(lines);
        tokio::time::timeout(Duration::from_secs(10), served).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn a_blocking_client_talks_to_the_listener() {
        let dir = tempfile::tempdir().unwrap();
        let path = endpoint(dir.path());
        let mut listener = Listener::bind(&path).unwrap();
        let client = tokio::task::spawn_blocking(move || {
            let mut stream = connect_blocking(&path).unwrap();
            stream.write_all(b"hi\n").unwrap();
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line).unwrap();
            line
        });
        echo(listener.accept().await.unwrap()).await;
        assert_eq!(client.await.unwrap(), "echo: hi\n");
    }

    #[tokio::test]
    async fn many_clients_at_once_all_get_through() {
        let dir = tempfile::tempdir().unwrap();
        let path = endpoint(dir.path());
        let mut listener = Listener::bind(&path).unwrap();
        // Like a burst of hooks: each one connects, asks once and waits.
        let clients: Vec<_> = (0..20)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let mut stream = connect_blocking(&path).unwrap();
                    stream.write_all(format!("{i}\n").as_bytes()).unwrap();
                    let mut line = String::new();
                    BufReader::new(stream).read_line(&mut line).unwrap();
                    assert_eq!(line, format!("echo: {i}\n"));
                })
            })
            .collect();
        let served = tokio::spawn(async move {
            for _ in 0..20 {
                tokio::spawn(echo(listener.accept().await.unwrap()));
            }
        });
        for client in clients {
            tokio::task::spawn_blocking(move || client.join().unwrap()).await.unwrap();
        }
        served.await.unwrap();
    }

    #[tokio::test]
    async fn a_taken_endpoint_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = endpoint(dir.path());
        let mut first = Listener::bind(&path).unwrap();
        assert!(Listener::bind(&path).is_err(), "a second listener on the same endpoint");
        // The first one still serves.
        let client = connect(&path).await.unwrap();
        echo_once(&mut first, client).await;
    }

    #[tokio::test]
    async fn nothing_listening_fails_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = endpoint(&dir.path().join("none"));
        let started = std::time::Instant::now();
        assert!(connect(&path).await.is_err());
        assert!(connect_blocking(&path).is_err());
        assert!(started.elapsed() < Duration::from_secs(2), "took {:?}", started.elapsed());
    }

    #[tokio::test]
    async fn the_client_learns_the_server_pid() {
        let dir = tempfile::tempdir().unwrap();
        let path = endpoint(dir.path());
        let _listener = Listener::bind(&path).unwrap();
        let client = connect(&path).await.unwrap();
        assert_eq!(server_pid(&client).unwrap(), std::process::id());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_socket_is_for_this_user_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = endpoint(dir.path());
        let _listener = Listener::bind(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
