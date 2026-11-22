//! The bridge between the webview and accd.
//!
//! It finds or starts the daemon (never a second one while the one it started
//! still runs), keeps one connection to it, matches replies to requests by id,
//! forwards daemon events to the webview unchanged and reports whether it is
//! connected, or why it could not connect. After a disconnect it tries again
//! every second.

use acc_protocol::{encode_line, socket_path, ClientKind, Request, RequestEnvelope, PROTOCOL_VERSION};
use anyhow::{anyhow, bail, Context};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Display;
use std::fs::{DirBuilder, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Runtime, State};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{sleep, timeout, Instant};

const RETRY_INTERVAL: Duration = Duration::from_secs(1);
const START_POLL_INTERVAL: Duration = Duration::from_millis(100);
const START_TIMEOUT: Duration = Duration::from_secs(5);
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const LOG_FILE: &str = "accd.log";

/// Tauri command: sends one request to accd and resolves with its `result`.
/// Rejects with the JSON string `{"code","message"}`.
#[tauri::command]
pub async fn daemon_request(bridge: State<'_, Bridge>, method: String, params: Option<Value>) -> Result<Value, String> {
    bridge.request(&method, params).await.map_err(|e| e.to_json())
}

/// Tauri command: whether the bridge is connected to accd right now.
#[tauri::command]
pub fn daemon_connected(bridge: State<'_, Bridge>) -> bool {
    bridge.is_connected()
}

/// Why a request failed. Daemon errors keep their own code; bridge failures use `disconnected`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BridgeError {
    pub code: String,
    pub message: String,
}

impl BridgeError {
    fn disconnected() -> BridgeError {
        BridgeError { code: "disconnected".into(), message: "Not connected to the daemon.".into() }
    }

    /// The string `daemon_request` rejects with: `{"code":"...","message":"..."}`.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("an error serializes")
    }
}

type Reply = Result<Value, BridgeError>;

/// A line from accd. It has the shape of `acc_protocol::ServerMsg`, but the event
/// stays raw JSON, so a field or kind this build does not know still reaches the
/// webview as accd sent it.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Incoming {
    Response { id: u64, result: Value },
    Error { id: u64, code: String, message: String },
    Event { event: Value },
}

/// The `daemon://status` payload: `{"connected": bool, "error"?: string}`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DaemonStatus {
    pub connected: bool,
    /// Why the last attempt to start or reach accd failed. Only while disconnected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl DaemonStatus {
    fn connected() -> DaemonStatus {
        DaemonStatus { connected: true, error: None }
    }

    fn disconnected(error: Option<String>) -> DaemonStatus {
        DaemonStatus { connected: false, error }
    }
}

/// Where the bridge reports connection status and daemon events.
pub trait Frontend: Send + Sync + 'static {
    fn status(&self, status: &DaemonStatus);
    /// One daemon event (`{kind, data}`), exactly as accd sent it.
    fn event(&self, event: Value);
}

/// The app reports to the webview.
impl<R: Runtime> Frontend for AppHandle<R> {
    fn status(&self, status: &DaemonStatus) {
        let _ = self.emit("daemon://status", status);
    }

    fn event(&self, event: Value) {
        let _ = self.emit("daemon://event", event);
    }
}

/// The app's side of the daemon connection. Clones share one connection.
#[derive(Clone, Default)]
pub struct Bridge {
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    next_id: AtomicU64,
    /// Set while connected, which is from the hello reply until the connection ends.
    link: Mutex<Option<Link>>,
}

struct Link {
    /// Request lines for the writer task.
    outgoing: mpsc::UnboundedSender<String>,
    /// Requests waiting for their reply, by id.
    pending: HashMap<u64, oneshot::Sender<Reply>>,
}

impl Bridge {
    pub fn is_connected(&self) -> bool {
        self.link().is_some()
    }

    /// Sends one request and waits for its reply. Without a connection it fails at once.
    pub async fn request(&self, method: &str, params: Option<Value>) -> Reply {
        let id = self.next_id();
        let line = request_line(id, method, params.as_ref());
        let reply = {
            let mut link = self.link();
            let link = link.as_mut().ok_or_else(BridgeError::disconnected)?;
            link.outgoing.send(line).map_err(|_| BridgeError::disconnected())?;
            let (tx, rx) = oneshot::channel();
            link.pending.insert(id, tx);
            rx
        };
        // The sender is dropped when the connection ends before the reply.
        reply.await.unwrap_or_else(|_| Err(BridgeError::disconnected()))
    }

    /// Keeps the app connected for as long as it runs. Every attempt goes through
    /// `ensure_daemon`, so a daemon that died is started again, and one that is
    /// still starting is waited for. Each failed attempt reports its error in the
    /// status, so a webview that subscribes late still learns why.
    pub async fn run(self, frontend: impl Frontend, accd: PathBuf, data_dir: PathBuf) {
        let mut started = None;
        loop {
            let attempt = match ensure_daemon(&accd, &data_dir, &mut started).await {
                Ok(stream) => self.serve(stream, &frontend).await,
                Err(e) => Err(e),
            };
            if let Err(e) = attempt {
                let error = format!("{e:#}");
                log(&error);
                frontend.status(&DaemonStatus::disconnected(Some(error)));
            }
            sleep(RETRY_INTERVAL).await;
        }
    }

    /// Serves one connection until it ends. An error means it never got
    /// connected (hello failed); how a connection ended is only logged.
    async fn serve(&self, stream: UnixStream, frontend: &impl Frontend) -> anyhow::Result<()> {
        let (reader, mut writer) = stream.into_split();
        let mut lines = BufReader::new(reader).lines();
        timeout(HELLO_TIMEOUT, hello(self.next_id(), &mut lines, &mut writer, frontend))
            .await
            .map_err(|_| anyhow!("accd did not answer hello"))??;

        let (outgoing, queued) = mpsc::unbounded_channel();
        let mut writing = tokio::spawn(write_lines(writer, queued));
        *self.link() = Some(Link { outgoing, pending: HashMap::new() });
        frontend.status(&DaemonStatus::connected());

        let ended = loop {
            tokio::select! {
                line = lines.next_line() => match line {
                    Ok(Some(line)) => self.dispatch(&line, frontend),
                    Ok(None) => break Ok(()),
                    Err(e) => break Err(anyhow::Error::new(e).context("reading from accd")),
                },
                written = &mut writing => break Err(match written {
                    Ok(Err(e)) => anyhow::Error::new(e).context("writing to accd"),
                    _ => anyhow!("the writer to accd stopped"),
                }),
            }
        };

        writing.abort();
        // Dropping the link rejects every pending request with `disconnected`.
        *self.link() = None;
        frontend.status(&DaemonStatus::disconnected(None));
        if let Err(e) = ended {
            log(format_args!("{e:#}"));
        }
        Ok(())
    }

    /// Hands one line from accd to the request waiting for it, or to the frontend.
    fn dispatch(&self, line: &str, frontend: &impl Frontend) {
        match serde_json::from_str::<Incoming>(line) {
            Ok(Incoming::Response { id, result }) => self.resolve(id, Ok(result)),
            Ok(Incoming::Error { id, code, message }) => self.resolve(id, Err(BridgeError { code, message })),
            Ok(Incoming::Event { event }) => frontend.event(event),
            Err(e) => log(format_args!("skipped a line from accd: {e}")),
        }
    }

    fn resolve(&self, id: u64, reply: Reply) {
        let waiting = self.link().as_mut().and_then(|link| link.pending.remove(&id));
        if let Some(waiting) = waiting {
            let _ = waiting.send(reply);
        }
    }

    fn next_id(&self) -> u64 {
        self.shared.next_id.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn link(&self) -> MutexGuard<'_, Option<Link>> {
        self.shared.link.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Says hello, which subscribes this connection to every daemon event, and waits for the reply.
/// Events that arrive before it are passed on.
async fn hello(
    id: u64,
    lines: &mut Lines<BufReader<OwnedReadHalf>>,
    writer: &mut OwnedWriteHalf,
    frontend: &impl Frontend,
) -> anyhow::Result<()> {
    let hello = RequestEnvelope { id, request: Request::Hello { client: ClientKind::App, version: PROTOCOL_VERSION } };
    writer.write_all(&encode_line(&hello)).await.context("sending hello to accd")?;
    loop {
        let line = lines
            .next_line()
            .await
            .context("reading from accd")?
            .context("accd closed the connection before answering hello")?;
        match serde_json::from_str::<Incoming>(&line) {
            Ok(Incoming::Response { id: answered, .. }) if answered == id => return Ok(()),
            Ok(Incoming::Error { id: answered, code, message }) if answered == id => {
                bail!("accd refused hello: {code}: {message}")
            }
            Ok(Incoming::Event { event }) => frontend.event(event),
            _ => {}
        }
    }
}

/// Writes queued request lines until the link is dropped or a write fails.
async fn write_lines(mut writer: OwnedWriteHalf, mut queued: mpsc::UnboundedReceiver<String>) -> std::io::Result<()> {
    while let Some(line) = queued.recv().await {
        writer.write_all(line.as_bytes()).await?;
    }
    Ok(())
}

/// Connects to the accd serving `data_dir`. When nothing answers on its socket it
/// starts `accd`, unless the accd in `started` (from an earlier attempt) still
/// runs: a slow or hung accd is waited for, never given a sibling. Then it polls
/// the socket every 100 ms for up to 5 s, and fails at once if that accd exits.
pub async fn ensure_daemon(
    accd: &Path,
    data_dir: &Path,
    started: &mut Option<StartedAccd>,
) -> anyhow::Result<UnixStream> {
    let socket = socket_path(data_dir);
    if let Ok(stream) = UnixStream::connect(&socket).await {
        return Ok(stream);
    }
    if started.as_ref().is_none_or(|earlier| earlier.ended().is_some()) {
        let (path, dir) = (accd.to_path_buf(), data_dir.to_path_buf());
        let new = tokio::task::spawn_blocking(move || start_accd(&path, &dir)).await.context("starting accd")??;
        *started = Some(new);
    }
    let waiting_for = started.as_ref().expect("an accd was started");
    let log_path = data_dir.join(LOG_FILE);
    let deadline = Instant::now() + START_TIMEOUT;
    while Instant::now() < deadline {
        sleep(START_POLL_INTERVAL).await;
        if let Ok(stream) = UnixStream::connect(&socket).await {
            return Ok(stream);
        }
        if let Some(how) = waiting_for.ended() {
            bail!(
                "{} exited ({how}) before it listened on {}; its output is in {}",
                accd.display(),
                socket.display(),
                log_path.display()
            );
        }
    }
    bail!(
        "{} did not listen on {} within 5 s; its output is in {}",
        accd.display(),
        socket.display(),
        log_path.display()
    )
}

/// An accd this app started.
pub struct StartedAccd {
    /// Set by its reaper when it exits, for example to `exit status: 1`.
    ended: Arc<OnceLock<String>>,
}

impl StartedAccd {
    fn ended(&self) -> Option<&str> {
        self.ended.get().map(String::as_str)
    }
}

/// Starts accd detached in its own session, with stdin from /dev/null and stdout
/// and stderr appended to `<data dir>/accd.log`, so it outlives the app and never
/// writes to a closed terminal. It blocks (fork and exec), so async code runs it
/// with `spawn_blocking`.
fn start_accd(accd: &Path, data_dir: &Path) -> anyhow::Result<StartedAccd> {
    let executable = std::fs::metadata(accd).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
    if !executable {
        bail!("accd was not found at {}. Run cargo build --workspace.", accd.display());
    }
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(data_dir)
        .with_context(|| format!("creating {}", data_dir.display()))?;
    let log_path = data_dir.join(LOG_FILE);
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log_path)
        .with_context(|| format!("opening {}", log_path.display()))?;
    let mut command = Command::new(accd);
    command.arg("--data-dir").arg(data_dir).stdin(Stdio::null()).stdout(log_file.try_clone()?).stderr(log_file);
    // SAFETY: setsid is async-signal-safe, and the closure touches nothing else.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().with_context(|| format!("starting {}", accd.display()))?;
    let pid = child.id();
    let ended = Arc::new(OnceLock::new());
    let reaper_ended = ended.clone();
    // Reap it when it exits, so a daemon that dies while the app runs leaves no
    // zombie, and record how it ended so a crash leaves a trace.
    std::thread::spawn(move || {
        let how = match child.wait() {
            Ok(status) => status.to_string(),
            Err(e) => format!("unknown status ({e})"),
        };
        let _ = reaper_ended.set(how.clone());
        log(format_args!("accd (pid {pid}) exited: {how}"));
    });
    Ok(StartedAccd { ended })
}

/// The accd next to the app's executable, else the workspace build used during development.
pub fn accd_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name("accd"))
        .filter(|sibling| sibling.exists())
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/debug/accd"))
}

/// A line on the app's stderr. The app ignores a failed write instead of
/// panicking; test builds use `eprintln!`, which the test harness captures.
fn log(message: impl Display) {
    if cfg!(test) {
        eprintln!("accd bridge: {message}");
    } else {
        let _ = writeln!(std::io::stderr(), "accd bridge: {message}");
    }
}

/// One request as a protocol line: compact JSON and a newline. `params` is left out when `None`.
pub fn request_line(id: u64, method: &str, params: Option<&Value>) -> String {
    #[derive(Serialize)]
    struct Line<'a> {
        id: u64,
        method: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        params: Option<&'a Value>,
    }
    let mut line = serde_json::to_string(&Line { id, method, params }).expect("a request serializes");
    line.push('\n');
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use acc_protocol::{ServerMsg, Snapshot};
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;
    use tokio::net::UnixListener;
    use tokio::task::JoinHandle;

    /// Upper bound for anything a test waits on. Generous, because process starts
    /// can stall for seconds on a busy Mac; waits end as soon as their condition holds.
    const WAIT: Duration = Duration::from_secs(10);

    fn disconnected() -> BridgeError {
        BridgeError { code: "disconnected".into(), message: "Not connected to the daemon.".into() }
    }

    /// What the bridge told the frontend, in order. A status is the exact
    /// `daemon://status` payload.
    #[derive(Debug, PartialEq)]
    enum Notice {
        Status(Value),
        Event(Value),
    }

    fn connected() -> Notice {
        Notice::Status(json!({"connected": true}))
    }

    fn disconnected_notice() -> Notice {
        Notice::Status(json!({"connected": false}))
    }

    struct Recorder(mpsc::UnboundedSender<Notice>);

    impl Frontend for Recorder {
        fn status(&self, status: &DaemonStatus) {
            let _ = self.0.send(Notice::Status(serde_json::to_value(status).unwrap()));
        }

        fn event(&self, event: Value) {
            let _ = self.0.send(Notice::Event(event));
        }
    }

    /// A bridge running against a data dir, as the app runs it.
    struct Running {
        bridge: Bridge,
        notices: mpsc::UnboundedReceiver<Notice>,
        task: JoinHandle<()>,
    }

    impl Running {
        fn start(accd: PathBuf, data_dir: &Path) -> Running {
            let bridge = Bridge::default();
            let (tx, notices) = mpsc::unbounded_channel();
            let task = tokio::spawn(bridge.clone().run(Recorder(tx), accd, data_dir.to_path_buf()));
            Running { bridge, notices, task }
        }

        async fn notice(&mut self) -> Notice {
            timeout(WAIT, self.notices.recv()).await.expect("a notice in time").expect("the bridge is running")
        }

        /// Sends a request from its own task, so the test can answer it as the daemon.
        fn request(&self, method: &str, params: Option<Value>) -> JoinHandle<Reply> {
            let bridge = self.bridge.clone();
            let method = method.to_string();
            tokio::spawn(async move { bridge.request(&method, params).await })
        }
    }

    impl Drop for Running {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn reply(request: JoinHandle<Reply>) -> Reply {
        timeout(WAIT, request).await.expect("a reply in time").unwrap()
    }

    /// The daemon's side of an app connection.
    struct Peer {
        lines: Lines<BufReader<OwnedReadHalf>>,
        writer: OwnedWriteHalf,
    }

    impl Peer {
        async fn accept(listener: &UnixListener) -> Peer {
            let (stream, _) = timeout(WAIT, listener.accept()).await.expect("the bridge connects").unwrap();
            let (reader, writer) = stream.into_split();
            Peer { lines: BufReader::new(reader).lines(), writer }
        }

        /// Accepts a connection and answers its hello, as accd does.
        async fn accept_app(listener: &UnixListener) -> Peer {
            let mut peer = Peer::accept(listener).await;
            let hello = peer.recv().await;
            assert_eq!(hello.request, Request::Hello { client: ClientKind::App, version: 1 });
            peer.send(ServerMsg::Response { id: hello.id, result: json!({"version": 1}) }).await;
            peer
        }

        async fn recv(&mut self) -> RequestEnvelope {
            let line = timeout(WAIT, self.lines.next_line()).await.expect("a request in time").unwrap().expect("a line");
            serde_json::from_str(&line).unwrap()
        }

        /// Sends one line: a `ServerMsg` as accd builds it, or raw JSON.
        async fn send(&mut self, msg: impl Serialize) {
            self.writer.write_all(&encode_line(&msg)).await.unwrap();
        }
    }

    /// A shell script standing in for accd.
    fn fake_accd(dir: &Path, script: &str) -> PathBuf {
        let path = dir.join("fake-accd");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// An accd (or stand-in) a test started, by pid. Dropping it stops the process,
    /// so a failed test leaves none behind.
    struct TestAccd(libc::pid_t);

    impl TestAccd {
        fn is_running(&self) -> bool {
            unsafe { libc::kill(self.0, 0) == 0 }
        }
    }

    impl Drop for TestAccd {
        fn drop(&mut self) {
            unsafe { libc::kill(self.0, libc::SIGTERM) };
            let deadline = std::time::Instant::now() + WAIT;
            while self.is_running() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            if self.is_running() {
                unsafe { libc::kill(self.0, libc::SIGKILL) };
            }
        }
    }

    /// The stand-ins that wrote `pid: <pid>` to the log, each stopped when dropped.
    fn logged_accds(log: &str) -> Vec<TestAccd> {
        log.lines().filter_map(|l| l.strip_prefix("pid: ")).map(|pid| TestAccd(pid.parse().unwrap())).collect()
    }

    /// Waits until `<data dir>/accd.log` contains `needle` `count` times and returns the log.
    async fn wait_for_log(data_dir: &Path, needle: &str, count: usize) -> String {
        let deadline = Instant::now() + WAIT;
        loop {
            let log = std::fs::read_to_string(data_dir.join("accd.log")).unwrap_or_default();
            if log.matches(needle).count() >= count {
                return log;
            }
            assert!(Instant::now() < deadline, "accd.log never had {count} x {needle:?}: {log:?}");
            sleep(Duration::from_millis(20)).await;
        }
    }

    #[test]
    fn request_line_with_params() {
        let line = request_line(7, "project_add", Some(&json!({"path": "/tmp/my repo"})));
        assert_eq!(line, "{\"id\":7,\"method\":\"project_add\",\"params\":{\"path\":\"/tmp/my repo\"}}\n");
        let parsed: RequestEnvelope = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed, RequestEnvelope { id: 7, request: Request::ProjectAdd { path: "/tmp/my repo".into() } });
    }

    #[test]
    fn request_line_without_params() {
        assert_eq!(request_line(3, "snapshot", None), "{\"id\":3,\"method\":\"snapshot\"}\n");
    }

    #[tokio::test]
    async fn request_while_disconnected_rejects_at_once() {
        let bridge = Bridge::default();
        assert!(!bridge.is_connected());
        let err = timeout(Duration::from_millis(200), bridge.request("snapshot", None))
            .await
            .expect("no waiting")
            .unwrap_err();
        let seen_by_ui: Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(seen_by_ui, json!({"code": "disconnected", "message": "Not connected to the daemon."}));
    }

    #[tokio::test]
    async fn connected_only_after_hello_is_answered() {
        let dir = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(socket_path(dir.path())).unwrap();
        let mut app = Running::start(dir.path().join("no-accd"), dir.path());
        let mut peer = Peer::accept(&listener).await;
        let hello = peer.recv().await;
        assert_eq!(hello.request, Request::Hello { client: ClientKind::App, version: 1 });
        sleep(Duration::from_millis(100)).await;
        assert!(!app.bridge.is_connected());
        assert!(app.notices.try_recv().is_err(), "no status before the hello reply");

        peer.send(ServerMsg::Response { id: hello.id, result: json!({"version": 1}) }).await;
        assert_eq!(app.notice().await, connected());
        assert!(app.bridge.is_connected());
    }

    #[tokio::test]
    async fn replies_match_requests_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(socket_path(dir.path())).unwrap();
        let mut app = Running::start(dir.path().join("no-accd"), dir.path());
        let mut peer = Peer::accept_app(&listener).await;
        assert_eq!(app.notice().await, connected());

        let first = app.request("snapshot", None);
        let first_seen = peer.recv().await;
        let second = app.request("snapshot", None);
        let second_seen = peer.recv().await;
        assert_eq!(first_seen.request, Request::Snapshot);
        assert_eq!(second_seen.request, Request::Snapshot);
        assert_ne!(first_seen.id, second_seen.id);

        // Answer the later request first.
        peer.send(ServerMsg::Response { id: second_seen.id, result: json!({"reply": "second"}) }).await;
        peer.send(ServerMsg::Response { id: first_seen.id, result: json!({"reply": "first"}) }).await;
        assert_eq!(reply(second).await, Ok(json!({"reply": "second"})));
        assert_eq!(reply(first).await, Ok(json!({"reply": "first"})));
    }

    #[tokio::test]
    async fn daemon_errors_keep_their_code() {
        let dir = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(socket_path(dir.path())).unwrap();
        let mut app = Running::start(dir.path().join("no-accd"), dir.path());
        let mut peer = Peer::accept_app(&listener).await;
        assert_eq!(app.notice().await, connected());

        let add = app.request("project_add", Some(json!({"path": "/tmp/plain dir"})));
        let seen = peer.recv().await;
        assert_eq!(seen.request, Request::ProjectAdd { path: "/tmp/plain dir".into() });
        let message = "/tmp/plain dir is not a git repository.";
        peer.send(ServerMsg::Error { id: seen.id, code: "not_a_repo".into(), message: message.into() }).await;

        let err = reply(add).await.unwrap_err();
        let seen_by_ui: Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(seen_by_ui, json!({"code": "not_a_repo", "message": message}));
    }

    #[tokio::test]
    async fn daemon_events_reach_the_frontend_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(socket_path(dir.path())).unwrap();
        let mut app = Running::start(dir.path().join("no-accd"), dir.path());
        let mut peer = Peer::accept_app(&listener).await;
        assert_eq!(app.notice().await, connected());

        // A field this build's protocol does not have, and a kind it does not know.
        let output = json!({"kind": "session_output", "data": {"session_id": "s1", "data_b64": "aGk=", "offset": 42}});
        let unknown = json!({"kind": "something_new", "data": {"items": [1, 2]}});
        peer.send(json!({"type": "event", "event": output.clone()})).await;
        peer.send(json!({"type": "event", "event": unknown.clone()})).await;
        assert_eq!(app.notice().await, Notice::Event(output));
        assert_eq!(app.notice().await, Notice::Event(unknown));
    }

    #[tokio::test]
    async fn losing_the_daemon_rejects_pending_requests() {
        let dir = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(socket_path(dir.path())).unwrap();
        let mut app = Running::start(dir.path().join("no-accd"), dir.path());
        let mut peer = Peer::accept_app(&listener).await;
        assert_eq!(app.notice().await, connected());

        let pending = app.request("snapshot", None);
        peer.recv().await;
        drop(peer);
        assert_eq!(reply(pending).await, Err(disconnected()));
        assert_eq!(app.notice().await, disconnected_notice());
        assert!(!app.bridge.is_connected());
        let later = timeout(Duration::from_millis(200), app.bridge.request("snapshot", None)).await.expect("no waiting");
        assert_eq!(later, Err(disconnected()));
    }

    #[tokio::test]
    async fn status_reports_why_the_daemon_is_not_reachable() {
        let dir = tempfile::tempdir().unwrap();
        let accd = dir.path().join("target/debug/accd");
        let mut app = Running::start(accd.clone(), dir.path());
        let missing = format!("accd was not found at {}. Run cargo build --workspace.", accd.display());
        assert_eq!(app.notice().await, Notice::Status(json!({"connected": false, "error": missing})));
        // Every failed attempt reports it, so a UI that subscribes late still sees it.
        assert_eq!(app.notice().await, Notice::Status(json!({"connected": false, "error": missing})));

        // Once a daemon answers, the error is gone.
        let listener = UnixListener::bind(socket_path(dir.path())).unwrap();
        let peer = Peer::accept_app(&listener).await;
        assert_eq!(app.notice().await, connected());
        drop(peer);
        assert_eq!(app.notice().await, disconnected_notice());
    }

    #[test]
    fn status_payload_has_an_error_only_while_disconnected() {
        let status = |connected, error: Option<&str>| {
            serde_json::to_value(DaemonStatus { connected, error: error.map(str::to_string) }).unwrap()
        };
        assert_eq!(status(true, None), json!({"connected": true}));
        assert_eq!(status(false, None), json!({"connected": false}));
        assert_eq!(status(false, Some("accd did not answer hello")), json!({"connected": false, "error": "accd did not answer hello"}));
    }

    #[tokio::test]
    async fn reconnect_starts_accd_again() {
        let dir = tempfile::tempdir().unwrap();
        let socket = socket_path(dir.path());
        let accd = fake_accd(dir.path(), "echo \"pid: $$\"\nexec sleep 30");
        let listener = UnixListener::bind(&socket).unwrap();
        let mut app = Running::start(accd, dir.path());
        let peer = Peer::accept_app(&listener).await;
        assert_eq!(app.notice().await, connected());
        assert!(!dir.path().join("accd.log").exists(), "accd was answering, so none was started");

        // accd dies, and its socket goes with it.
        drop(listener);
        std::fs::remove_file(&socket).unwrap();
        drop(peer);
        assert_eq!(app.notice().await, disconnected_notice());
        let lost = Instant::now();
        let restarted = logged_accds(&wait_for_log(dir.path(), "pid: ", 1).await);
        assert!(lost.elapsed() >= Duration::from_millis(900), "the retry waits about a second");
        assert_eq!(restarted.len(), 1);

        // The restarted accd listens; the test plays its socket.
        let listener = UnixListener::bind(&socket).unwrap();
        let _peer = Peer::accept_app(&listener).await;
        assert_eq!(app.notice().await, connected());
    }

    #[tokio::test]
    async fn accd_starts_in_its_own_session_with_the_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data dir");
        let accd = fake_accd(dir.path(), "echo \"args: $# $*\"\necho \"pid: $$\"\nexec sleep 30");
        start_accd(&accd, &data_dir).unwrap();
        let log = wait_for_log(&data_dir, "pid: ", 1).await;
        let accds = logged_accds(&log);
        let pid = accds[0].0;
        assert!(log.contains(&format!("args: 2 --data-dir {}\n", data_dir.display())), "{log}");
        assert_eq!(unsafe { libc::getsid(pid) }, pid, "accd leads its own session");
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&data_dir), 0o700);
        assert_eq!(mode(&data_dir.join("accd.log")), 0o600);
    }

    #[tokio::test]
    async fn accd_output_is_appended_to_its_log() {
        let dir = tempfile::tempdir().unwrap();
        let accd = fake_accd(dir.path(), "echo to stdout\necho to stderr >&2");
        start_accd(&accd, dir.path()).unwrap();
        wait_for_log(dir.path(), "to stderr", 1).await;
        start_accd(&accd, dir.path()).unwrap();
        let log = wait_for_log(dir.path(), "to stderr", 2).await;
        assert_eq!(log, "to stdout\nto stderr\nto stdout\nto stderr\n");
    }

    #[tokio::test]
    async fn a_running_accd_gets_no_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let accd = fake_accd(dir.path(), "echo \"pid: $$\"\nexec sleep 30");
        let mut started = None;
        let first = ensure_daemon(&accd, dir.path(), &mut started).await.unwrap_err();
        assert!(first.to_string().contains("did not listen"), "{first:#}");

        // The next attempt waits for the accd that is still running. Its full wait
        // is another 5 s; half a second shows whether it started a second accd.
        let second = timeout(Duration::from_millis(500), ensure_daemon(&accd, dir.path(), &mut started)).await;
        assert!(second.is_err(), "still waiting for the first accd");
        let log = std::fs::read_to_string(dir.path().join("accd.log")).unwrap();
        let accds = logged_accds(&log);
        assert_eq!(accds.len(), 1, "one accd at a time: {log:?}");
        assert!(accds[0].is_running());
    }

    #[tokio::test]
    async fn an_accd_that_exits_fails_the_attempt_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let accd = fake_accd(dir.path(), "echo started\nsleep 0.5\nexit 3");
        // An earlier attempt started this accd. Starting it is not timed: starting a
        // process has been seen to stall for over 2 s on a busy Mac.
        let mut started = Some(start_accd(&accd, dir.path()).unwrap());
        let began = Instant::now();
        let err = ensure_daemon(&accd, dir.path(), &mut started).await.unwrap_err();
        assert!(err.to_string().contains("exit status: 3"), "{err:#}");
        assert!(began.elapsed() < Duration::from_secs(4), "no 5 s wait once that accd is gone");

        // It is gone, so the next attempt starts a new one.
        let starts = || std::fs::read_to_string(dir.path().join("accd.log")).unwrap().matches("started").count();
        let before = starts();
        ensure_daemon(&accd, dir.path(), &mut started).await.unwrap_err();
        assert_eq!(starts(), before + 1);
    }

    /// Needs the real daemon: `cargo build -p accd --offline` first.
    #[tokio::test]
    async fn ensure_daemon_starts_accd_and_the_bridge_gets_a_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let accd = accd_path();
        let probe = ensure_daemon(&accd, dir.path(), &mut None).await.expect("accd listens on its socket");
        let started = TestAccd(probe.peer_cred().unwrap().pid().expect("the pid of accd"));
        drop(probe);

        let mut app = Running::start(accd, dir.path());
        assert_eq!(app.notice().await, connected());
        let snapshot = app.bridge.request("snapshot", None).await.unwrap();
        let snapshot: Snapshot = serde_json::from_value(snapshot).unwrap();
        assert_eq!(snapshot, Snapshot { projects: vec![], sessions: vec![], approvals: vec![] });

        drop(app);
        assert!(UnixStream::connect(socket_path(dir.path())).await.is_ok(), "accd keeps running without the app");
        let pid = started.0;
        drop(started);
        assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "accd has stopped");
    }
}
