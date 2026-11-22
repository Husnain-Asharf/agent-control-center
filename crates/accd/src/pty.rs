//! PTY sessions: spawn a program on a pseudo-terminal, stream its output,
//! keep a capped scrollback, accept input and resizes, and stop it.
//!
//! On Windows the pseudo-terminal is ConPTY. Each session's processes are in
//! a job object, so stopping a session ends all of them, and a batch file
//! (an npm `claude.cmd`) runs through `cmd.exe /c`.

#[cfg(unix)]
use anyhow::anyhow;
use anyhow::Context;
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
#[cfg(unix)]
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
/// `spec.env_remove`, plus `spec.env`. Windows ignores the case of names, so
/// there a name is removed or replaced in any case (`Path` by `PATH`).
fn child_env(spec: &SpawnSpec, inherited: impl IntoIterator<Item = (OsString, OsString)>) -> BTreeMap<OsString, OsString> {
    let removed = |name: &OsStr| spec.env_remove.iter().chain(spec.env.iter().map(|(name, _)| name)).any(|r| same_name(name, r));
    let mut env: BTreeMap<OsString, OsString> = inherited.into_iter().filter(|(name, _)| !removed(name)).collect();
    env.extend(spec.env.iter().map(|(name, value)| (OsString::from(name), OsString::from(value))));
    env
}

fn same_name(name: &OsStr, other: &str) -> bool {
    match cfg!(windows) {
        true => name.to_str().is_some_and(|name| name.eq_ignore_ascii_case(other)),
        false => name == OsStr::new(other),
    }
}

/// Called with each chunk of output and its start offset: the number of
/// bytes this PTY produced before it.
pub type OutputFn = Box<dyn Fn(&[u8], u64) + Send + Sync>;
/// Called once with `Some(code)` on a normal exit, `None` if the child was
/// killed by a signal or waiting failed.
pub type ExitFn = Box<dyn FnOnce(Option<i32>) + Send>;

pub struct PtySession {
    /// The session's processes. First, so that on drop they end before the
    /// pseudo console closes. `None` when the child had exited already.
    #[cfg(windows)]
    job: Option<crate::conpty::Job>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    scrollback: Arc<Mutex<Scrollback>>,
    #[cfg(unix)]
    pid: Option<u32>,
    exited: Arc<AtomicBool>,
    /// Set by kill(): the exit then reports no code, as a signal does on Unix.
    #[cfg(windows)]
    killed: Arc<AtomicBool>,
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

        let program = spec.program.clone();
        let spec = launch_spec(spec);
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
            .with_context(|| format!("spawn {}", program.display()))?;
        // Drop our slave handle so the reader sees EOF once the child side closes.
        drop(pair.slave);
        #[cfg(windows)]
        let job = match child.as_raw_handle().map(crate::conpty::Job::holding) {
            Some(Ok(job)) => Some(job),
            // It has exited already, so there is nothing to stop.
            _ if matches!(child.try_wait(), Ok(Some(_))) => None,
            Some(Err(e)) => {
                let _ = child.kill();
                return Err(anyhow::Error::new(e).context("put the session in a job object"));
            }
            None => {
                let _ = child.kill();
                anyhow::bail!("the session's process has no handle");
            }
        };

        let mut reader = pair.master.try_clone_reader().context("clone pty reader")?;
        let writer = Arc::new(Mutex::new(pair.master.take_writer().context("take pty writer")?));
        #[cfg(unix)]
        let pid = child.process_id();

        let scrollback = Arc::new(Mutex::new(Scrollback::default()));
        let exited = Arc::new(AtomicBool::new(false));
        let (done_tx, done_rx) = mpsc::channel::<()>();

        let sb = scrollback.clone();
        let mut startup = startup_output(writer.clone());
        std::thread::Builder::new()
            .name("pty-reader".into())
            .spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let replaced = startup(&buf[..n]);
                            let chunk = replaced.as_deref().unwrap_or(&buf[..n]);
                            if chunk.is_empty() {
                                continue;
                            }
                            let offset = {
                                let mut sb = sb.lock().unwrap();
                                let offset = sb.total;
                                append_capped(&mut sb.bytes, chunk);
                                sb.total += chunk.len() as u64;
                                offset
                            };
                            on_output(chunk, offset);
                        }
                    }
                }
                let _ = done_tx.send(());
            })
            .context("spawn reader thread")?;

        let flag = exited.clone();
        #[cfg(windows)]
        let killed = Arc::new(AtomicBool::new(false));
        #[cfg(windows)]
        let was_killed = killed.clone();
        std::thread::Builder::new()
            .name("pty-waiter".into())
            .spawn(move || {
                let code = match child.wait() {
                    Ok(status) if status.signal().is_none() => Some(status.exit_code() as i32),
                    _ => None,
                };
                #[cfg(windows)]
                let code = code.filter(|_| !was_killed.load(Ordering::SeqCst));
                flag.store(true, Ordering::SeqCst);
                // A grandchild can hold the slave open, so do not wait forever.
                // ConPTY keeps its output open until the pseudo console closes.
                let _ = done_rx.recv_timeout(DRAIN_TIMEOUT);
                on_exit(code);
            })
            .context("spawn waiter thread")?;

        Ok(PtySession {
            #[cfg(windows)]
            job,
            master: Mutex::new(pair.master),
            writer,
            scrollback,
            #[cfg(unix)]
            pid,
            exited,
            #[cfg(windows)]
            killed,
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
    #[cfg(unix)]
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

    /// Ends the child and every process it started, at once: Windows has no
    /// hangup signal to give them a moment first. Ok if the child already
    /// exited.
    #[cfg(windows)]
    pub fn kill(&self) -> anyhow::Result<()> {
        if self.exited.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.killed.store(true, Ordering::SeqCst);
        match &self.job {
            Some(job) => job.terminate().context("end the session's processes"),
            None => Ok(()),
        }
    }
}

/// The spec as it is spawned. On Windows a batch file runs through cmd.exe.
#[cfg(unix)]
fn launch_spec(spec: SpawnSpec) -> SpawnSpec {
    spec
}

#[cfg(windows)]
fn launch_spec(mut spec: SpawnSpec) -> SpawnSpec {
    if crate::conpty::is_batch_file(&spec.program) {
        let (cmd, args, variable) = crate::conpty::batch_launch(&spec.program, &spec.args);
        spec.program = cmd;
        spec.args = args;
        spec.env.push(variable);
    }
    spec
}

/// Replaces a chunk of the first output when it needs to: ConPTY's cursor
/// request is answered and taken out. `None` keeps the chunk.
type StartupFn = Box<dyn FnMut(&[u8]) -> Option<Vec<u8>> + Send>;

#[cfg(unix)]
fn startup_output(_writer: Arc<Mutex<Box<dyn Write + Send>>>) -> StartupFn {
    Box::new(|_| None)
}

#[cfg(windows)]
fn startup_output(writer: Arc<Mutex<Box<dyn Write + Send>>>) -> StartupFn {
    let mut request = crate::conpty::CursorRequest::new();
    Box::new(move |chunk| {
        let rest = request.remove_from(chunk)?;
        let mut writer = writer.lock().unwrap();
        let _ = writer.write_all(crate::conpty::CURSOR_AT_TOP_LEFT).and_then(|()| writer.flush());
        Some(rest)
    })
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

    /// `command` run by the system shell.
    fn shell(command: &str) -> SpawnSpec {
        match cfg!(windows) {
            true => spec("cmd.exe", &["/d", "/c", command]),
            false => spec("/bin/sh", &["-c", command]),
        }
    }

    /// A program that runs for 30 s and prints nothing.
    fn sleeper() -> SpawnSpec {
        match cfg!(windows) {
            true => spec("ping.exe", &["-n", "30", "127.0.0.1"]),
            false => spec("/bin/sleep", &["30"]),
        }
    }

    /// Prints `hello` and exits.
    fn hello() -> SpawnSpec {
        shell(if cfg!(windows) { "echo hello" } else { "printf hello" })
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
        let (s, rx) = start(hello());
        let code = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(code, Some(0));
        let out = String::from_utf8_lossy(&s.scrollback()).to_string();
        assert!(out.contains("hello"), "got {out:?}");
    }

    #[test]
    fn exit_code_is_reported() {
        let (_s, rx) = start(shell("exit 3"));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Some(3));
    }

    #[test]
    fn on_output_sees_the_chunk_after_it_is_in_scrollback() {
        let (tx, rx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        let holder: Arc<std::sync::OnceLock<Arc<PtySession>>> = Arc::new(std::sync::OnceLock::new());
        let h = holder.clone();
        let s = PtySession::spawn(
            hello(),
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
        let command = match cfg!(windows) {
            true => "echo abc & ping -n 2 127.0.0.1 >nul & echo defgh",
            false => "printf abc; sleep 0.1; printf defgh",
        };
        let s = PtySession::spawn(
            shell(command),
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
        if cfg!(windows) {
            // ConPTY draws the screen, so there is more than the text.
            let text = String::from_utf8_lossy(&all);
            assert!(text.contains("abc") && text.contains("defgh"), "{text:?}");
        } else {
            assert_eq!(all, b"abcdefgh");
        }
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

    #[cfg(windows)]
    #[test]
    fn child_env_names_match_in_any_case_on_windows() {
        let inherited = [("Path", r"C:\Windows"), ("ClaudeCode", "1"), ("Term", "dumb")]
            .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let mut s = spec("cmd.exe", &[]);
        s.env = vec![("PATH".into(), r"C:\Windows;C:\bin".into()), ("TERM".into(), "xterm-256color".into())];
        s.env_remove = crate::claude::removed_env();
        let env = child_env(&s, inherited);
        let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.to_str().unwrap(), v.to_str().unwrap())).collect();
        assert_eq!(pairs, [("PATH", r"C:\Windows;C:\bin"), ("TERM", "xterm-256color")]);
    }

    #[test]
    fn removed_variables_do_not_reach_the_child() {
        // A variable every test process has, removed for the child.
        let (name, command) = match cfg!(windows) {
            true => ("COMPUTERNAME", "echo X=%COMPUTERNAME% TERM=%TERM%"),
            false => ("HOME", "echo \"X=${HOME-%HOME%} TERM=$TERM\""),
        };
        assert!(std::env::var_os(name).is_some(), "the test process has {name}");
        let mut s = shell(command);
        s.env_remove = vec![name.into()];
        let (session, rx) = start(s);
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let out = String::from_utf8_lossy(&session.scrollback()).to_string();
        // Unset, sh prints the default and cmd leaves the reference as it is.
        assert!(out.contains(&format!("X=%{name}% TERM=xterm-256color")), "{out:?}");
    }

    #[test]
    fn input_reaches_process() {
        // cat, or cmd reading commands, echo what is typed.
        let reader = match cfg!(windows) {
            true => spec("cmd.exe", &["/d", "/q"]),
            false => spec("/bin/cat", &[]),
        };
        let (s, _rx) = start(reader);
        s.write(b"echo abc\r").unwrap();
        wait_for(&s, b"abc");
        s.kill().unwrap();
    }

    #[test]
    fn resize_does_not_fail() {
        let (s, _rx) = start(sleeper());
        s.resize(100, 40).unwrap();
        s.kill().unwrap();
    }

    #[test]
    fn kill_stops_process() {
        let (s, rx) = start(sleeper());
        s.kill().unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
    }

    #[test]
    fn kill_after_exit_is_ok() {
        let (s, rx) = start(shell("exit 0"));
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        s.kill().unwrap();
    }

    #[cfg(unix)]
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

    #[cfg(windows)]
    #[test]
    fn kill_stops_child_processes() {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
        // cmd.exe runs ping.exe, so the session has two processes.
        let (s, rx) = start(shell("ping -n 30 127.0.0.1"));
        let job = s.job.as_ref().expect("a job for a running session");
        let deadline = Instant::now() + Duration::from_secs(5);
        let pids = loop {
            let pids = job.process_ids();
            if pids.len() >= 2 {
                break pids;
            }
            assert!(Instant::now() < deadline, "ping never started: {pids:?}");
            std::thread::sleep(Duration::from_millis(20));
        };
        s.kill().unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), None, "a killed session has no exit code");
        for pid in pids {
            // SAFETY: plain calls; a null handle means the process is gone.
            let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid as u32) };
            if !process.is_null() {
                let waited = unsafe { WaitForSingleObject(process, 2000) };
                unsafe { CloseHandle(process) };
                assert_eq!(waited, WAIT_OBJECT_0, "process {pid} still runs");
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn the_cursor_request_does_not_reach_the_output() {
        let (s, rx) = start(hello());
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let out = s.scrollback();
        assert!(out.windows(5).any(|w| w == b"hello"), "{:?}", String::from_utf8_lossy(&out));
        assert!(!out.windows(4).any(|w| w == b"\x1b[6n"), "{:?}", String::from_utf8_lossy(&out));
    }

    #[cfg(unix)]
    #[test]
    fn kill_escalates_to_sigkill() {
        let (s, rx) = start(spec("/bin/sh", &["-c", "trap '' HUP; sleep 30"]));
        std::thread::sleep(Duration::from_millis(300));
        s.kill().unwrap();
        let code = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(code, None);
    }

    /// ConPTY draws a screen rather than passing every byte through, so the
    /// cap is checked where output arrives in full.
    #[cfg(unix)]
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
