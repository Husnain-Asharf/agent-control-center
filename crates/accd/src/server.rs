//! The socket server (a named pipe on Windows). Newline-delimited JSON, one
//! task per connection. An app connection that says hello receives every
//! daemon event; a hook connection gets exactly one response. PTY output is
//! coalesced here.

use crate::claude::{self, HookEffect};
use crate::daemon::{Answer, ApiError, ApprovalOutcome, Daemon, DaemonConfig, Decided, PreToolOutcome, Shared};
use crate::pty;
use acc_protocol::{
    encode_line, permission_reply, pre_tool_reply, ClientKind, DaemonEvent, Decision, HookReply, Request, RequestEnvelope,
    ServerMsg, SessionMode, SessionStatus, PROTOCOL_VERSION,
};
use anyhow::Context;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::Serialize;
use serde_json::{json, Value};
use acc_transport::{Listener, ServerStream};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};
use tokio::sync::{broadcast, oneshot, Notify};
use tokio::task::{JoinHandle, JoinSet};

pub const APPROVAL_EXPIRED: &str = "No decision in Agent Control Center within 55 minutes.";
pub const APPROVAL_DENIED: &str = "Denied in Agent Control Center.";

/// Events an app client may fall behind by before it is closed.
const EVENT_CAPACITY: usize = 4096;
/// A session's output is broadcast at most this often.
const FLUSH_INTERVAL: Duration = Duration::from_millis(16);
/// How long shutdown waits for the killed agents to exit.
const EXIT_WAIT: Duration = Duration::from_secs(5);
/// How long `run_server` keeps trying the data dir's lock.
const LOCK_WAIT: Duration = Duration::from_millis(500);

/// The read side of a connection, and its write side.
type Reader = BufReader<ReadHalf<ServerStream>>;
type Writer = WriteHalf<ServerStream>;

/// `run_server` fails with this when another daemon already serves the data dir.
#[derive(Debug, thiserror::Error)]
#[error("accd already running")]
pub struct AlreadyRunning;

pub struct ServerHandle {
    stop: oneshot::Sender<()>,
    accept: JoinHandle<()>,
    shared: Arc<Shared>,
    /// The data dir's lock, held until shutdown or drop.
    _lock: File,
}

/// Opens the store, marks sessions left by a previous daemon as Stopped,
/// binds the socket (user-only; a named pipe on Windows) and starts serving.
/// Fails with [`AlreadyRunning`] when another daemon serves the same data dir.
pub async fn run_server(cfg: DaemonConfig) -> anyhow::Result<ServerHandle> {
    cfg.paths.create_dirs().with_context(|| format!("create {}", cfg.paths.data.display()))?;
    // Taken before anything else is touched, so a second daemon can neither
    // mark the first one's sessions Stopped nor unlink its socket.
    let lock = lock_data_dir(&cfg.paths.lock).await?;
    let (events, _) = broadcast::channel(EVENT_CAPACITY);
    let daemon = Daemon::open(&cfg.paths, events.clone()).context("open the store")?;
    let socket = &cfg.paths.socket;
    // With the lock held, a socket file can only be left by a daemon that
    // went away. A named pipe goes away with its daemon.
    #[cfg(unix)]
    let _ = std::fs::remove_file(socket);
    let listener = Listener::bind(socket).with_context(|| format!("bind {}", socket.display()))?;
    let shared = Arc::new(Shared::new(cfg, events, daemon));
    let (stop, stopped) = oneshot::channel();
    let accept = tokio::spawn(accept_loop(listener, shared.clone(), stopped));
    tokio::spawn(watch_branches(Arc::downgrade(&shared)));
    Ok(ServerHandle { stop, accept, shared, _lock: lock })
}

/// How often the branches of running sessions are reread.
const BRANCH_POLL: Duration = Duration::from_secs(2);

/// Rereads the running sessions' branches every [`BRANCH_POLL`], until the
/// daemon is dropped. Reading a branch is a small file read, not a git run.
async fn watch_branches(shared: std::sync::Weak<Shared>) {
    loop {
        tokio::time::sleep(BRANCH_POLL).await;
        let Some(shared) = shared.upgrade() else { return };
        shared.state.lock().await.refresh_branches();
    }
}

/// Takes the data dir's lock, an exclusive non-blocking lock on the file
/// (flock, or LockFileEx on Windows). It lasts while the returned file is
/// open. Files are opened close-on-exec (not inherited on Windows), so the
/// agents and git do not keep it. A process that is between fork and exec
/// does share it, though, so a lock that was just released can look held for
/// a moment. The attempt is therefore repeated for up to [`LOCK_WAIT`].
async fn lock_data_dir(path: &Path) -> anyhow::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options.open(path).with_context(|| format!("open {}", path.display()))?;
    let deadline = tokio::time::Instant::now() + LOCK_WAIT;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(TryLockError::WouldBlock) => return Err(AlreadyRunning.into()),
            Err(TryLockError::Error(e)) => return Err(anyhow::Error::new(e).context(format!("lock {}", path.display()))),
        }
    }
}

impl ServerHandle {
    /// Stops accepting, closes every connection and kills every running
    /// agent without recording the exits, as if the daemon went away. The
    /// next `run_server` on the same dir marks those sessions Stopped.
    /// Returns once the agents have exited.
    pub async fn shutdown(self) {
        let _ = self.stop.send(());
        let _ = self.accept.await;
        let pumps = self.shared.state.lock().await.kill_all();
        let deadline = tokio::time::Instant::now() + EXIT_WAIT;
        for pump in pumps {
            let _ = tokio::time::timeout_at(deadline, pump).await;
        }
    }
}

async fn accept_loop(mut listener: Listener, shared: Arc<Shared>, mut stopped: oneshot::Receiver<()>) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            _ = &mut stopped => break,
            accepted = listener.accept() => match accepted {
                Ok(stream) => {
                    connections.spawn(serve(stream, shared.clone()));
                }
                Err(e) => {
                    eprintln!("accd: accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
            // Reap finished connections so the set does not grow.
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
    // Dropping a connection task closes its socket and writes nothing to the
    // store, so a pending approval stays pending for the next daemon.
    connections.shutdown().await;
}

enum Outcome {
    Reply(ServerMsg),
    /// Reply, then close the connection.
    Last(ServerMsg),
    Close,
}

async fn serve(stream: ServerStream, shared: Arc<Shared>) {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);
    let mut subscription = None;
    let mut line = Vec::new();
    loop {
        let read = tokio::select! {
            // read_until keeps partial input in `line` when the other branch wins.
            read = reader.read_until(b'\n', &mut line) => read,
            event = next_event(&mut subscription) => {
                match event {
                    Ok(event) => {
                        if send(&mut writer, &ServerMsg::Event { event }).await.is_err() {
                            return;
                        }
                    }
                    // A client that lags is closed. It reconnects and re-reads the snapshot.
                    Err(_) => return,
                }
                continue;
            }
        };
        if !matches!(read, Ok(n) if n > 0) {
            return;
        }
        let request = std::mem::take(&mut line);
        if request.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match handle_line(&shared, &request, &mut subscription, &mut reader).await {
            Outcome::Reply(msg) => {
                if send(&mut writer, &msg).await.is_err() {
                    return;
                }
            }
            Outcome::Last(msg) => {
                let _ = send(&mut writer, &msg).await;
                return;
            }
            Outcome::Close => return,
        }
    }
}

async fn next_event(subscription: &mut Option<broadcast::Receiver<DaemonEvent>>) -> Result<DaemonEvent, broadcast::error::RecvError> {
    match subscription {
        Some(events) => events.recv().await,
        None => std::future::pending().await,
    }
}

async fn send(writer: &mut Writer, msg: &ServerMsg) -> std::io::Result<()> {
    writer.write_all(&encode_line(msg)).await
}

async fn handle_line(
    shared: &Arc<Shared>,
    line: &[u8],
    subscription: &mut Option<broadcast::Receiver<DaemonEvent>>,
    conn: &mut Reader,
) -> Outcome {
    let envelope: RequestEnvelope = match serde_json::from_slice(line) {
        Ok(envelope) => envelope,
        Err(e) => return Outcome::Reply(error(request_id(line), ApiError::bad_request(format!("Bad request: {e}")))),
    };
    let id = envelope.id;
    match envelope.request {
        // acc-hook sends one request without hello and reads one line.
        Request::Hook { session_id, payload } => match hook(shared, &session_id, &payload, conn).await {
            Some(reply) => Outcome::Last(ServerMsg::Response { id, result: to_json(&reply) }),
            None => Outcome::Close,
        },
        request => match dispatch(shared, request, subscription).await {
            Ok(result) => Outcome::Reply(ServerMsg::Response { id, result }),
            Err(e) => Outcome::Reply(error(id, e)),
        },
    }
}

/// The id of a request that did not parse, if it has one.
fn request_id(line: &[u8]) -> u64 {
    serde_json::from_slice::<Value>(line).ok().and_then(|v| v.get("id")?.as_u64()).unwrap_or(0)
}

fn error(id: u64, e: ApiError) -> ServerMsg {
    ServerMsg::Error { id, code: e.code.to_string(), message: e.message }
}

fn to_json<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("protocol types serialize")
}

async fn dispatch(
    shared: &Arc<Shared>,
    request: Request,
    subscription: &mut Option<broadcast::Receiver<DaemonEvent>>,
) -> Result<Value, ApiError> {
    Ok(match request {
        Request::Hello { client, version } => {
            if version != PROTOCOL_VERSION {
                return Err(ApiError::bad_request(format!(
                    "Protocol version {version} is not supported. This daemon speaks version {PROTOCOL_VERSION}."
                )));
            }
            if client == ClientKind::App && subscription.is_none() {
                *subscription = Some(shared.events.subscribe());
            }
            json!({ "version": PROTOCOL_VERSION })
        }
        Request::Snapshot => to_json(&shared.state.lock().await.snapshot()),
        Request::ProjectAdd { path } => to_json(&shared.project_add(&path).await?),
        Request::SessionStart { project_id, agent, task, mode } => {
            to_json(&shared.session_start(&project_id, &agent, &task, mode.unwrap_or(SessionMode::Project)).await?)
        }
        Request::SessionInput { session_id, data_b64 } => {
            let data = B64.decode(data_b64).map_err(|e| ApiError::bad_request(format!("data_b64 is not base64: {e}")))?;
            shared.state.lock().await.queue_input(&session_id, data)?;
            json!({})
        }
        Request::SessionResize { session_id, cols, rows } => {
            shared.session_resize(&session_id, cols, rows).await?;
            json!({})
        }
        Request::SessionAttach { session_id } => {
            let (scrollback, end) = shared.state.lock().await.scrollback(&session_id)?;
            json!({ "scrollback_b64": B64.encode(scrollback), "end": end })
        }
        Request::SessionStop { session_id } => to_json(&shared.state.lock().await.stop_session(&session_id)?),
        Request::SessionRestart { session_id } => to_json(&shared.session_restart(&session_id).await?),
        Request::ApprovalDecide { approval_id, decision } => match shared.approval_decide(&approval_id, decision).await? {
            Decided::Resolved(approval) => to_json(&approval),
            Decided::AlreadyResolved => json!({ "already_resolved": true }),
        },
        Request::EventsList { session_id, limit } => to_json(&shared.state.lock().await.events(session_id.as_deref(), limit)?),
        Request::Hook { .. } => return Err(ApiError::internal("hook requests are served by handle_line")),
    })
}

enum Waited {
    Answered(Answer),
    Expired,
}

/// Waits for the answer to a pending approval without holding the state
/// lock. `None` closes the connection without a reply: the hook went away
/// (the approval is cancelled), or the daemon is going away.
async fn wait_for_answer(shared: &Shared, approval_id: &str, mut answer: oneshot::Receiver<Answer>, conn: &mut Reader) -> Option<Waited> {
    enum Woke {
        Answered(Result<Answer, oneshot::error::RecvError>),
        Expired,
        HookClosed,
    }
    let woke = tokio::select! {
        answered = &mut answer => Woke::Answered(answered),
        _ = tokio::time::sleep(shared.cfg.approval_timeout) => Woke::Expired,
        _ = wait_closed(conn) => Woke::HookClosed,
    };
    match woke {
        Woke::Answered(answered) => answered.ok().map(Waited::Answered),
        Woke::Expired => {
            if shared.state.lock().await.expire(approval_id) {
                Some(Waited::Expired)
            } else {
                // An answer landed just before the timer fired.
                answer.try_recv().ok().map(Waited::Answered)
            }
        }
        Woke::HookClosed => {
            shared.state.lock().await.cancel(approval_id);
            None
        }
    }
}

/// Serves one hook call. `None` closes the connection without a reply,
/// because the hook went away or the daemon is going away.
async fn hook(shared: &Shared, session_id: &str, payload: &Value, conn: &mut Reader) -> Option<HookReply> {
    let silent = HookReply { stdout: None, exit_code: 0 };
    let (tool, summary, input) = match claude::interpret_hook(payload) {
        HookEffect::Approval { tool, summary, input } => (tool, summary, input),
        HookEffect::Status(status) => {
            shared.state.lock().await.hook_status(session_id, status);
            return Some(silent);
        }
        HookEffect::Activity(summary) => {
            let tool = payload.get("tool_name").and_then(Value::as_str).unwrap_or("unknown");
            shared.state.lock().await.tool_used(session_id, tool, &summary);
            return Some(silent);
        }
        HookEffect::PreTool { tool, summary, input } => {
            let outcome = {
                let mut state = shared.state.lock().await;
                state.hook_status(session_id, SessionStatus::Working);
                state.pre_tool_check(session_id, &tool, &summary, &input)
            };
            let (approval_id, answer, protected) = match outcome {
                PreToolOutcome::Proceed => return Some(silent),
                PreToolOutcome::Protected => return protected_approval(shared, session_id, tool, summary, input, conn).await,
                PreToolOutcome::Branch { id, answer, protected } => (id, answer, protected),
            };
            let yes = match wait_for_answer(shared, &approval_id, answer, conn).await? {
                Waited::Answered(Answer::Decided(Decision::Deny)) => return Some(pre_tool("deny", Some(APPROVAL_DENIED))),
                Waited::Expired => return Some(pre_tool("deny", Some(APPROVAL_EXPIRED))),
                Waited::Answered(Answer::Decided(_)) => true,
                Waited::Answered(Answer::Proceed) => false,
            };
            // A protected path still waits for its own approval.
            if protected {
                return protected_approval(shared, session_id, tool, summary, input, conn).await;
            }
            return Some(if yes { pre_tool("allow", None) } else { silent });
        }
        HookEffect::Nothing => return Some(silent),
    };
    let outcome = shared.state.lock().await.request_approval(session_id, tool, summary, input);
    let (approval_id, decision) = match outcome {
        ApprovalOutcome::Ignored => return Some(silent),
        ApprovalOutcome::Allowed => return Some(permission(true, None)),
        ApprovalOutcome::Pending { id, decision } => (id, decision),
    };
    match wait_for_answer(shared, &approval_id, decision, conn).await? {
        Waited::Answered(Answer::Decided(decision)) => Some(decision_reply(decision)),
        Waited::Answered(Answer::Proceed) => Some(silent),
        Waited::Expired => Some(permission(false, Some(APPROVAL_EXPIRED))),
    }
}

fn pre_tool(decision: &str, reason: Option<&str>) -> HookReply {
    HookReply { stdout: Some(pre_tool_reply(decision, reason)), exit_code: 0 }
}

/// Holds a tool call on a protected path, from a session in the project
/// folder, for an approval in the inbox, and answers the PreToolUse hook
/// with the decision. Claude Code's own "ask" answer cannot be used: it
/// fails the call instead of asking (Claude Code 2.1.323). An allow here
/// skips Claude Code's permission prompt, so the call is asked once.
async fn protected_approval(
    shared: &Shared,
    session_id: &str,
    tool: String,
    summary: String,
    input: Value,
    conn: &mut Reader,
) -> Option<HookReply> {
    let outcome = shared.state.lock().await.request_approval(session_id, tool, summary, input);
    let (approval_id, decision) = match outcome {
        ApprovalOutcome::Ignored => return Some(HookReply { stdout: None, exit_code: 0 }),
        ApprovalOutcome::Allowed => return Some(pre_tool("allow", None)),
        ApprovalOutcome::Pending { id, decision } => (id, decision),
    };
    Some(match wait_for_answer(shared, &approval_id, decision, conn).await? {
        Waited::Answered(Answer::Decided(Decision::Deny)) => pre_tool("deny", Some(APPROVAL_DENIED)),
        Waited::Answered(_) => pre_tool("allow", None),
        Waited::Expired => pre_tool("deny", Some(APPROVAL_EXPIRED)),
    })
}

/// Resolves once the peer closed its end. A hook sends nothing after its request.
async fn wait_closed(conn: &mut Reader) {
    let mut buf = [0u8; 512];
    while matches!(conn.read(&mut buf).await, Ok(n) if n > 0) {}
}

fn permission(allow: bool, message: Option<&str>) -> HookReply {
    HookReply { stdout: Some(permission_reply(allow, message)), exit_code: 0 }
}

fn decision_reply(decision: Decision) -> HookReply {
    match decision {
        Decision::AllowOnce | Decision::AllowAlways | Decision::MakeBranch | Decision::StayOnBase => permission(true, None),
        Decision::Deny => permission(false, Some(APPROVAL_DENIED)),
    }
}

/// A session's output waiting to be broadcast. The PTY reader thread fills it.
#[derive(Default)]
pub(crate) struct OutputBuffer {
    pending: std::sync::Mutex<Pending>,
    ready: Notify,
}

#[derive(Default)]
struct Pending {
    bytes: Vec<u8>,
    /// Stream position just after the last pending byte.
    end: u64,
}

impl OutputBuffer {
    /// Adds a chunk that starts at stream position `offset`. Chunks arrive in
    /// order and contiguous. Keeps at most the scrollback cap, so a flood
    /// cannot grow memory without bound.
    pub(crate) fn push(&self, chunk: &[u8], offset: u64) {
        {
            let mut pending = self.pending.lock().unwrap();
            pty::append_capped(&mut pending.bytes, chunk);
            pending.end = offset + chunk.len() as u64;
        }
        self.ready.notify_one();
    }

    /// The pending bytes and the stream position of the first one. That is
    /// the first chunk's offset, unless the cap dropped older bytes.
    fn take(&self) -> (Vec<u8>, u64) {
        let mut pending = self.pending.lock().unwrap();
        let bytes = std::mem::take(&mut pending.bytes);
        let offset = pending.end - bytes.len() as u64;
        (bytes, offset)
    }
}

/// Broadcasts the session's output as one `SessionOutput` at most every
/// [`FLUSH_INTERVAL`]. When the agent exits, flushes the rest and then
/// records the exit, so the last output reaches clients first.
pub(crate) fn spawn_output_pump(
    shared: Arc<Shared>,
    session_id: String,
    output: Arc<OutputBuffer>,
    mut exited: oneshot::Receiver<Option<i32>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let code = loop {
            tokio::select! {
                biased;
                // The exit is polled first, so output that never stops (a
                // leftover background job) cannot keep a finished run open.
                code = &mut exited => break code.ok().flatten(),
                _ = output.ready.notified() => {
                    flush(&shared.events, &session_id, &output);
                    tokio::time::sleep(FLUSH_INTERVAL).await;
                }
            }
        };
        flush(&shared.events, &session_id, &output);
        shared.state.lock().await.pty_exited(&session_id, code);
    })
}

fn flush(events: &broadcast::Sender<DaemonEvent>, session_id: &str, output: &OutputBuffer) {
    let (data, offset) = output.take();
    if !data.is_empty() {
        let event = DaemonEvent::SessionOutput { session_id: session_id.to_string(), data_b64: B64.encode(data), offset };
        let _ = events.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Paths;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    /// Daemon state on a temp dir, and a receiver for its events.
    fn shared() -> (tempfile::TempDir, Arc<Shared>, broadcast::Receiver<DaemonEvent>) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::in_dir(dir.path());
        let (events, rx) = broadcast::channel(EVENT_CAPACITY);
        let daemon = Daemon::open(&paths, events.clone()).unwrap();
        let mut cfg = DaemonConfig::new(paths, "/bin/true".into());
        cfg.login_shell = None;
        (dir, Arc::new(Shared::new(cfg, events, daemon)), rx)
    }

    #[tokio::test]
    async fn exit_is_not_held_back_by_output() {
        let (_dir, shared, _rx) = shared();
        let output = Arc::new(OutputBuffer::default());
        let (exit_tx, exited) = oneshot::channel();
        let pump = spawn_output_pump(shared, "s1".into(), output.clone(), exited);

        // Like a background job that keeps writing after the agent exited.
        let writing = Arc::new(AtomicBool::new(true));
        let still_writing = writing.clone();
        let writer = std::thread::spawn(move || {
            let mut offset = 0;
            while still_writing.load(Ordering::SeqCst) {
                output.push(b"x\n", offset);
                offset += 2;
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        exit_tx.send(Some(0)).unwrap();
        let ended = tokio::time::timeout(Duration::from_secs(1), pump).await;
        writing.store(false, Ordering::SeqCst);
        tokio::task::spawn_blocking(move || writer.join().unwrap()).await.unwrap();
        assert!(ended.is_ok(), "the exit waited for the output to stop");
    }

    #[tokio::test]
    async fn output_is_flushed_at_most_every_16_ms() {
        let (_dir, shared, mut rx) = shared();
        let output = Arc::new(OutputBuffer::default());
        let (exit_tx, exited) = oneshot::channel();
        let pump = spawn_output_pump(shared, "s1".into(), output.clone(), exited);

        // Like the PTY reader thread: many small chunks for 200 ms.
        let writer = std::thread::spawn(move || {
            let start = Instant::now();
            let mut n = 0;
            let mut offset = 0;
            while start.elapsed() < Duration::from_millis(200) {
                let chunk = format!("{n}\n");
                output.push(chunk.as_bytes(), offset);
                offset += chunk.len() as u64;
                n += 1;
                std::thread::sleep(Duration::from_micros(100));
            }
            (n, start.elapsed())
        });
        let (pushes, took) = tokio::task::spawn_blocking(move || writer.join().unwrap()).await.unwrap();
        exit_tx.send(Some(0)).unwrap();
        tokio::time::timeout(Duration::from_secs(5), pump).await.unwrap().unwrap();

        let mut flushes = 0;
        let mut data = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let DaemonEvent::SessionOutput { data_b64, offset, .. } = event {
                assert_eq!(offset, data.len() as u64, "each flush starts where the last one ended");
                flushes += 1;
                data.extend(B64.decode(data_b64).unwrap());
            }
        }
        let most = took.as_millis() as usize / 16 + 2;
        assert!(flushes <= most, "{flushes} flushes for {pushes} chunks in {took:?}");
        let expected: String = (0..pushes).map(|n| format!("{n}\n")).collect();
        assert_eq!(String::from_utf8(data).unwrap(), expected, "every chunk, in order");
    }

    #[test]
    fn flushed_offset_is_the_first_pending_byte() {
        let output = OutputBuffer::default();
        output.push(b"abc", 7);
        output.push(b"de", 10);
        assert_eq!(output.take(), (b"abcde".to_vec(), 7));
        // A flood past the cap drops the oldest bytes, and the offset moves with them.
        output.push(&vec![b'x'; pty::SCROLLBACK_CAP + 10], 12);
        let (bytes, offset) = output.take();
        assert_eq!((bytes.len(), offset), (pty::SCROLLBACK_CAP, 22));
    }
}
