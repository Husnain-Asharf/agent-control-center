//! PTY sessions: spawn a program on a pseudo-terminal, stream its output,
//! keep a capped scrollback, accept input and resizes, and stop it.

use anyhow::{anyhow, Context};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

/// Scrollback kept in memory per session.
pub const SCROLLBACK_CAP: usize = 1024 * 1024;

/// How long the waiter waits for the reader to drain after the child exits.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(1);
/// How long kill() waits after SIGHUP before sending SIGKILL.
const KILL_GRACE: Duration = Duration::from_secs(1);

pub struct SpawnSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Added on top of the inherited environment.
    pub env: Vec<(String, String)>,
    /// Removed from the inherited environment.
    pub env_remove: Vec<String>,
    pub cols: u16,
    pub rows: u16,
}

/// The child's environment: `inherited` without the names in
/// `spec.env_remove`, plus `spec.env`.
fn child_env(spec: &SpawnSpec, inherited: impl IntoIterator<Item = (OsString, OsString)>) -> BTreeMap<OsString, OsString> {
    let removed = |name: &OsString| spec.env_remove.iter().any(|r| name.as_os_str() == OsStr::new(r));
    let mut env: BTreeMap<OsString, OsString> = inherited.into_iter().filter(|(name, _)| !removed(name)).collect();
    env.extend(spec.env.iter().map(|(name, value)| (OsString::from(name), OsString::from(value))));
    env
}

/// Called with each chunk of output and its start offset: the number of
/// bytes this PTY produced before it.
pub type OutputFn = Box<dyn Fn(&[u8], u64) + Send + Sync>;
/// Called once with `Some(code)` on a normal exit, `None` if the child was
/// killed by a signal or waiting failed.
pub type ExitFn = Box<dyn FnOnce(Option<i32>) + Send>;

pub struct PtySession {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    scrollback: Arc<Mutex<Scrollback>>,
    pid: Option<u32>,
    exited: Arc<AtomicBool>,
}

#[derive(Default)]
struct Scrollback {
    /// The last `SCROLLBACK_CAP` bytes.
    bytes: Vec<u8>,
    /// Bytes ever appended, so the stream position just after `bytes`.
    total: u64,
}

pub(crate) fn append_capped(buf: &mut Vec<u8>, chunk: &[u8]) {
    if chunk.len() >= SCROLLBACK_CAP {
        buf.clear();
        buf.extend_from_slice(&chunk[chunk.len() - SCROLLBACK_CAP..]);
        return;
    }
    buf.extend_from_slice(chunk);
    if buf.len() > SCROLLBACK_CAP {
        let excess = buf.len() - SCROLLBACK_CAP;
        buf.drain(..excess);
    }
}

impl PtySession {
    pub fn spawn(spec: SpawnSpec, on_output: OutputFn, on_exit: ExitFn) -> anyhow::Result<PtySession> {
        let pair = native_pty_system()
            .openpty(PtySize { rows: spec.rows, cols: spec.cols, pixel_width: 0, pixel_height: 0 })
            .context("open pty")?;

        let mut cmd = CommandBuilder::new(&spec.program);
        cmd.args(&spec.args);
        cmd.cwd(&spec.cwd);
        cmd.env_clear();
        for (k, v) in child_env(&spec, std::env::vars_os()) {
            cmd.env(k, v);
        }

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .with_context(|| format!("spawn {}", spec.program.display()))?;
        // Drop our slave handle so the reader sees EOF once the child side closes.
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().context("clone pty reader")?;
        let writer = pair.master.take_writer().context("take pty writer")?;
        let pid = child.process_id();

        let scrollback = Arc::new(Mutex::new(Scrollback::default()));
        let exited = Arc::new(AtomicBool::new(false));
        let (done_tx, done_rx) = mpsc::channel::<()>();

        let sb = scrollback.clone();
        std::thread::Builder::new()
            .name("pty-reader".into())
            .spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let offset = {
                                let mut sb = sb.lock().unwrap();
                                let offset = sb.total;
                                append_capped(&mut sb.bytes, &buf[..n]);
                                sb.total += n as u64;
                                offset
                            };
                            on_output(&buf[..n], offset);
                        }
                    }
                }
                let _ = done_tx.send(());
            })
            .context("spawn reader thread")?;

        let flag = exited.clone();
        std::thread::Builder::new()
            .name("pty-waiter".into())
            .spawn(move || {
                let code = match child.wait() {
                    Ok(status) if status.signal().is_none() => Some(status.exit_code() as i32),
                    _ => None,
                };
                flag.store(true, Ordering::SeqCst);
                // A grandchild can hold the slave open, so do not wait forever.
                let _ = done_rx.recv_timeout(DRAIN_TIMEOUT);
                on_exit(code);
            })
            .context("spawn waiter thread")?;

        Ok(PtySession {
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            scrollback,
            pid,
            exited,
        })
    }

    pub fn write(&self, data: &[u8]) -> anyhow::Result<()> {
        let mut w = self.writer.lock().unwrap();
        w.write_all(data).context("write to pty")?;
        w.flush().context("flush pty")
    }

    pub fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.master
            .lock()
            .unwrap()
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .context("resize pty")
    }

    pub fn scrollback(&self) -> Vec<u8> {
        self.scrollback.lock().unwrap().bytes.clone()
    }

    /// The scrollback and the stream position just after its last byte,
    /// read together so output passed to `on_output` later starts at or
    /// after that position.
    pub fn scrollback_with_end(&self) -> (Vec<u8>, u64) {
        let sb = self.scrollback.lock().unwrap();
        (sb.bytes.clone(), sb.total)
    }

    /// Sends SIGHUP to the child's process group, then SIGKILL after a grace
    /// period if it is still running. Returns without waiting for the grace
    /// period. Ok if the process already exited.
    pub fn kill(&self) -> anyhow::Result<()> {
        if self.exited.load(Ordering::SeqCst) {
            return Ok(());
        }
        let pid = self.pid.ok_or_else(|| anyhow!("child has no pid"))? as libc::pid_t;
        // The PTY child is a session leader, so its pgid is its pid.
        if unsafe { libc::killpg(pid, libc::SIGHUP) } != 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ESRCH) {
                return Ok(());
            }
            return Err(anyhow!("killpg SIGHUP: {err}"));
        }
        let exited = self.exited.clone();
        std::thread::Builder::new()
            .name("pty-kill".into())
            .spawn(move || {
                std::thread::sleep(KILL_GRACE);
                if !exited.load(Ordering::SeqCst) {
                    unsafe { libc::killpg(pid, libc::SIGKILL) };
                }
            })
            .context("spawn kill thread")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn spec(program: &str, args: &[&str]) -> SpawnSpec {
        SpawnSpec {
            program: PathBuf::from(program),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: std::env::temp_dir(),
            env: vec![("TERM".into(), "xterm-256color".into())],
            env_remove: vec![],
            cols: 120,
            rows: 32,
        }
    }

    fn start(spec: SpawnSpec) -> (PtySession, mpsc::Receiver<Option<i32>>) {
        let (tx, rx) = mpsc::channel();
        let s = PtySession::spawn(
            spec,
            Box::new(|_, _| {}),
            Box::new(move |code| {
                let _ = tx.send(code);
            }),
        )
        .unwrap();
        (s, rx)
    }

    fn wait_for(s: &PtySession, needle: &[u8]) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if s.scrollback().windows(needle.len()).any(|w| w == needle) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("output never contained {:?}", String::from_utf8_lossy(needle));
    }

    #[test]
    fn session_is_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<PtySession>();
    }

    #[test]
    fn runs_command_and_captures_output() {
        let (s, rx) = start(spec("/bin/sh", &["-c", "printf hello"]));
        let code = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(code, Some(0));
        let out = String::from_utf8_lossy(&s.scrollback()).to_string();
        assert!(out.contains("hello"), "got {out:?}");
    }

    #[test]
    fn exit_code_is_reported() {
        let (_s, rx) = start(spec("/bin/sh", &["-c", "exit 3"]));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Some(3));
    }

    #[test]
    fn on_output_sees_the_chunk_after_it_is_in_scrollback() {
        let (tx, rx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        let holder: Arc<std::sync::OnceLock<Arc<PtySession>>> = Arc::new(std::sync::OnceLock::new());
        let h = holder.clone();
        let s = PtySession::spawn(
            spec("/bin/sh", &["-c", "printf hello"]),
            Box::new(move |chunk, offset| {
                // Reading the scrollback inside the callback must not deadlock.
                if let Some(s) = h.get() {
                    let _ = tx.send((offset + chunk.len() as u64, s.scrollback_with_end().1));
                }
            }),
            Box::new(move |c| {
                let _ = etx.send(c);
            }),
        )
        .unwrap();
        let s = Arc::new(s);
        let _ = holder.set(s.clone());
        erx.recv_timeout(Duration::from_secs(5)).unwrap();
        // Callbacks that ran after `holder` was set saw a scrollback that already ended past the chunk.
        while let Ok((chunk_end, end)) = rx.try_recv() {
            assert!(end >= chunk_end, "chunk ends at {chunk_end}, scrollback at {end}");
        }
    }

    #[test]
    fn output_offsets_are_contiguous_and_end_the_scrollback() {
        let (tx, rx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        let s = PtySession::spawn(
            spec("/bin/sh", &["-c", "printf abc; sleep 0.1; printf defgh"]),
            Box::new(move |chunk, offset| {
                let _ = tx.send((offset, chunk.to_vec()));
            }),
            Box::new(move |c| {
                let _ = etx.send(c);
            }),
        )
        .unwrap();
        erx.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut end = 0;
        let mut all = Vec::new();
        let mut chunks = 0;
        while let Ok((offset, chunk)) = rx.try_recv() {
            assert_eq!(offset, end, "each chunk starts where the last one ended");
            end += chunk.len() as u64;
            all.extend(chunk);
            chunks += 1;
        }
        assert!(chunks >= 2, "the pause splits the output");
        assert_eq!(all, b"abcdefgh");
        assert_eq!(s.scrollback_with_end(), (all, end));
    }

    #[test]
    fn child_env_drops_the_removed_names_and_adds_the_extras() {
        let inherited = [
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_ENTRYPOINT", "cli"),
            ("CLAUDE_CODE_SSE_PORT", "5000"),
            // Turns transcript saving off, so a restart could not resume.
            ("CLAUDE_CODE_CHILD_SESSION", "1"),
            // The parent agent session's own channels; its token must never reach another agent.
            ("CLAUDE_CODE_MESSAGING_SOCKET", "/tmp/parent.sock"),
            ("CLAUDE_CODE_MESSAGING_TOKEN", "parent-token"),
            ("CLAUDE_CODE_CHILD_NOTES_DIR", "/tmp/parent-notes"),
            ("CLAUDE_CODE_MAX_OUTPUT_TOKENS", "8000"),
            ("PATH", "/usr/bin"),
            ("TERM", "dumb"),
        ]
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let mut s = spec("/bin/true", &[]);
        s.env_remove = crate::claude::removed_env();
        let env = child_env(&s, inherited);
        let names: Vec<&str> = env.keys().map(|k| k.to_str().unwrap()).collect();
        // A deny list: the user's own settings, such as the output token limit, stay.
        assert_eq!(names, ["CLAUDE_CODE_MAX_OUTPUT_TOKENS", "PATH", "TERM"]);
        assert_eq!(env[OsStr::new("TERM")], "xterm-256color", "an extra replaces the inherited value");
    }

    #[test]
    fn removed_variables_do_not_reach_the_child() {
        assert!(std::env::var_os("HOME").is_some(), "the test process has HOME");
        let mut s = spec("/bin/sh", &["-c", "echo \"HOME=${HOME-unset} TERM=$TERM\""]);
        s.env_remove = vec!["HOME".into()];
        let (session, rx) = start(s);
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let out = String::from_utf8_lossy(&session.scrollback()).to_string();
        assert!(out.contains("HOME=unset TERM=xterm-256color"), "{out:?}");
    }

    #[test]
    fn input_reaches_process() {
        let (s, _rx) = start(spec("/bin/cat", &[]));
        s.write(b"abc\r").unwrap();
        wait_for(&s, b"abc");
        s.kill().unwrap();
    }

    #[test]
    fn resize_does_not_fail() {
        let (s, _rx) = start(spec("/bin/sleep", &["30"]));
        s.resize(100, 40).unwrap();
        s.kill().unwrap();
    }

    #[test]
    fn kill_stops_process() {
        let (s, rx) = start(spec("/bin/sleep", &["30"]));
        s.kill().unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
    }

    #[test]
    fn kill_after_exit_is_ok() {
        let (s, rx) = start(spec("/bin/sh", &["-c", "true"]));
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        s.kill().unwrap();
    }

    #[test]
    fn kill_stops_child_processes() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let script = format!("sleep 30 & echo $! > '{}'; wait", pidfile.display());
        let (s, _rx) = start(spec("/bin/sh", &["-c", &script]));
        let deadline = Instant::now() + Duration::from_secs(5);
        let pid: i32 = loop {
            if let Ok(t) = std::fs::read_to_string(&pidfile) {
                if let Ok(p) = t.trim().parse() {
                    break p;
                }
            }
            assert!(Instant::now() < deadline, "pid file never appeared");
            std::thread::sleep(Duration::from_millis(20));
        };
        s.kill().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if unsafe { libc::kill(pid, 0) } != 0 {
                break;
            }
            assert!(Instant::now() < deadline, "grandchild {pid} still alive");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn kill_escalates_to_sigkill() {
        let (s, rx) = start(spec("/bin/sh", &["-c", "trap '' HUP; sleep 30"]));
        std::thread::sleep(Duration::from_millis(300));
        s.kill().unwrap();
        let code = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(code, None);
    }

    #[test]
    fn scrollback_is_capped() {
        let (s, rx) = start(spec(
            "/bin/sh",
            &["-c", "head -c 3145728 /dev/zero | tr '\\0' a"],
        ));
        rx.recv_timeout(Duration::from_secs(20)).unwrap();
        let sb = s.scrollback();
        assert!(sb.len() <= SCROLLBACK_CAP, "len {}", sb.len());
        assert!(sb.len() > 1024, "len {}", sb.len());
        assert_eq!(sb.iter().rev().find(|b| **b != b'\r' && **b != b'\n'), Some(&b'a'));
        // The end keeps counting past the cap.
        let (capped, end) = s.scrollback_with_end();
        assert_eq!(capped, sb);
        assert_eq!(end, 3 * 1024 * 1024);
    }
}
