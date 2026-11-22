use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use tokio::net::{UnixListener, UnixStream};

/// The daemon's end of a connection.
pub type ServerStream = UnixStream;
/// The app's end of a connection.
pub type ClientStream = UnixStream;
/// A connection for blocking reads and writes, as the hook makes.
pub type BlockingStream = std::os::unix::net::UnixStream;

/// The daemon's socket.
pub struct Listener(UnixListener);

impl Listener {
    /// Listens on the socket `endpoint`, mode 0600 so only this user can
    /// connect. Fails if the file exists.
    pub fn bind(endpoint: &Path) -> io::Result<Listener> {
        let listener = UnixListener::bind(endpoint)?;
        std::fs::set_permissions(endpoint, std::fs::Permissions::from_mode(0o600))?;
        Ok(Listener(listener))
    }

    /// The next connection.
    pub async fn accept(&mut self) -> io::Result<ServerStream> {
        Ok(self.0.accept().await?.0)
    }
}

/// Connects to the daemon at `endpoint`.
pub async fn connect(endpoint: &Path) -> io::Result<ClientStream> {
    UnixStream::connect(endpoint).await
}

/// Connects to the daemon at `endpoint`, for blocking reads and writes.
pub fn connect_blocking(endpoint: &Path) -> io::Result<BlockingStream> {
    BlockingStream::connect(endpoint)
}

/// The process id of the daemon at the other end.
pub fn server_pid(stream: &ClientStream) -> io::Result<u32> {
    let pid = stream.peer_cred()?.pid().ok_or_else(|| io::Error::other("the socket has no peer pid"))?;
    u32::try_from(pid).map_err(io::Error::other)
}
