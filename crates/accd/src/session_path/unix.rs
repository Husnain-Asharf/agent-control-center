//! The login shell's PATH, and the session PATH built from it, on macOS
//! and Linux.

use crate::claude::is_executable;
use std::ffi::OsString;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long the interactive login shell gets to print its PATH. It also
/// reads `~/.zshrc` or `~/.bashrc`, and nvm set up there can take a second
/// or two.
pub const INTERACTIVE_SHELL_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the login shell gets to print its PATH.
pub const LOGIN_SHELL_TIMEOUT: Duration = Duration::from_secs(3);
/// The most of the login shell's output that is read.
const MAX_OUTPUT: usize = 64 * 1024;
/// launchd's PATH, the base when the daemon has none.
const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";
/// Homebrew's folders on Apple Silicon and on Intel.
const HOMEBREW_BINS: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];

/// The shells tried, in order, when `$SHELL` is unset: zsh is the macOS
/// default, bash the usual one on Linux, and sh is always there.
const DEFAULT_SHELLS: [&str; 3] = ["/bin/zsh", "/bin/bash", "/bin/sh"];

/// The user's login shell: `$SHELL`, else the first of [`DEFAULT_SHELLS`]
/// that is an executable file.
pub fn login_shell() -> PathBuf {
    shell_or_default(std::env::var_os("SHELL"), is_executable)
}

fn shell_or_default(shell: Option<OsString>, is_executable: impl Fn(&Path) -> bool) -> PathBuf {
    if let Some(shell) = shell.filter(|shell| !shell.is_empty()) {
        return PathBuf::from(shell);
    }
    let last = DEFAULT_SHELLS[DEFAULT_SHELLS.len() - 1];
    let found = DEFAULT_SHELLS.into_iter().find(|shell| is_executable(Path::new(shell)));
    PathBuf::from(found.unwrap_or(last))
}

/// The PATH of `shell`, or `None` when it cannot be read or holds no
/// absolute folder (nushell prints `"$PATH"` as it is). The shell is asked
/// as an interactive login shell first, which also reads `~/.zshrc`, where
/// nvm and the like are often set up. When that gives no PATH, it is asked
/// as a login shell only. Each reason goes to stderr, which is `accd.log`.
/// Blocks for up to [`INTERACTIVE_SHELL_TIMEOUT`] plus
/// [`LOGIN_SHELL_TIMEOUT`].
pub fn login_path(shell: &Path) -> Option<String> {
    login_path_within(shell, INTERACTIVE_SHELL_TIMEOUT, LOGIN_SHELL_TIMEOUT)
}

/// [`login_path`] with the time each of the two tries gets.
fn login_path_within(shell: &Path, interactive_timeout: Duration, login_timeout: Duration) -> Option<String> {
    let reason = match with_absolute_dir(ask_interactive_shell(shell, interactive_timeout)) {
        Ok(path) => return Some(path),
        Err(reason) => reason,
    };
    let next = "Asking it as a login shell only.";
    eprintln!("accd: could not read the PATH of the interactive login shell {}: {reason}. {next}", shell.display());
    let reason = match with_absolute_dir(ask_login_shell(shell, login_timeout)) {
        Ok(path) => return Some(path),
        Err(reason) => reason,
    };
    let fallback = "Sessions get the daemon's PATH with Homebrew's folders, ~/.local/bin and the folder of claude added.";
    eprintln!("accd: could not read the PATH of the login shell {}: {reason}. {fallback}", shell.display());
    None
}

/// The PATH a shell printed, unless it holds no absolute folder.
fn with_absolute_dir(printed: Result<String, String>) -> Result<String, String> {
    match printed {
        Ok(path) if !path.split(':').any(is_absolute_dir) => Err(format!("it printed {path:?}, which holds no absolute folder")),
        printed => printed,
    }
}

/// Whether a PATH entry is an absolute folder.
fn is_absolute_dir(entry: &str) -> bool {
    entry.starts_with('/')
}

/// [`ask_shell`] with `-l -i`: as an interactive login shell, which also
/// reads `~/.zshrc`, and `~/.bashrc` where `~/.bash_profile` or `~/.profile`
/// reads it.
pub fn ask_interactive_shell(shell: &Path, timeout: Duration) -> Result<String, String> {
    ask_shell(shell, &["-l", "-i"], timeout)
}

/// [`ask_shell`] with `-l`: as a login shell only, which reads the login
/// files such as `~/.zprofile` and `~/.profile`.
pub fn ask_login_shell(shell: &Path, timeout: Duration) -> Result<String, String> {
    ask_shell(shell, &["-l"], timeout)
}

/// Runs `shell` with `flags`, then `-c` and a command that prints the PATH
/// between two marker lines made unique for this call, and returns the PATH
/// between them. A banner, a prompt or escape codes from the startup files,
/// output after the PATH (`.zlogout`), or a job that holds the output open
/// cannot hide it. The shell gets a null stdin, so a startup script that
/// asks a question does not wait for an answer. It runs in a session of its
/// own, with no terminal: an interactive shell would stop when it tried to
/// take over the terminal of a daemon started from one, and without a
/// terminal it does no job control, so the programs it starts stay in its
/// process group. Once the PATH is in, or after `timeout`, the shell is
/// killed with that whole group. The error says why there is no PATH.
fn ask_shell(shell: &Path, flags: &[&str], timeout: Duration) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    let id = Uuid::new_v4().simple().to_string();
    let (begin, end) = (format!("ACC-PATH-BEGIN-{id}"), format!("ACC-PATH-END-{id}"));
    let print_path = format!("printf '\\n%s\\n%s\\n%s\\n' '{begin}' \"$PATH\" '{end}'");
    let mut command = Command::new(shell);
    command.args(flags).arg("-c").arg(&print_path).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe, and the closure touches nothing else.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|e| format!("it could not start ({e})"))?;
    let Some(mut stdout) = child.stdout.take() else {
        kill_group(&mut child);
        return Err("it has no output".into());
    };
    let (sender, chunks) = mpsc::channel::<Vec<u8>>();
    // Ends when every process in the group has closed the pipe, or at the cap.
    let reader = std::thread::Builder::new().name("login-shell-path".into()).spawn(move || {
        let mut buf = [0u8; 4096];
        let mut read = 0;
        while read < MAX_OUTPUT {
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) if sender.send(buf[..n].to_vec()).is_ok() => read += n,
                Ok(_) => break,
            }
        }
    });
    if let Err(e) = reader {
        kill_group(&mut child);
        return Err(format!("its output could not be read ({e})"));
    }
    let mut output = Vec::new();
    let found = loop {
        if let Some(path) = path_between_markers(&String::from_utf8_lossy(&output), &begin, &end) {
            break Some(path.to_string());
        }
        match chunks.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(chunk) => output.extend(chunk),
            Err(RecvTimeoutError::Timeout) => {
                kill_group(&mut child);
                return Err(format!("it did not print its PATH within {timeout:?}"));
            }
            Err(RecvTimeoutError::Disconnected) => break None,
        }
    };
    let status = kill_group(&mut child);
    match found {
        Some(path) if path.is_empty() => Err("it printed an empty PATH".into()),
        Some(path) => Ok(path),
        None => Err(format!("it exited without printing its PATH ({status})")),
    }
}

/// Kills the process group that `child` leads, then reaps `child` and says
/// how it ended.
fn kill_group(child: &mut Child) -> String {
    // SAFETY: a plain syscall. `child` leads the group and is not reaped
    // yet, so the group id cannot belong to anything else.
    unsafe { libc::killpg(child.id() as libc::pid_t, libc::SIGKILL) };
    match child.wait() {
        Ok(status) => status.to_string(),
        Err(e) => e.to_string(),
    }
}

/// The line between the line `begin` and the line `end` in a login shell's
/// output, once `end` has arrived.
fn path_between_markers<'a>(output: &'a str, begin: &str, end: &str) -> Option<&'a str> {
    let mut lines = output.split('\n').map(|line| line.trim_end_matches('\r'));
    lines.find(|line| *line == begin)?;
    let path = lines.next()?;
    (lines.next()? == end).then_some(path)
}

/// The PATH for agent sessions: the absolute folders of the login shell's
/// PATH, then every folder of the inherited PATH it lacks, then
/// `claude_dir`, the folder of the `claude` that was found. Each folder once
/// (a trailing slash aside), and no empty entries. Without a login PATH, or
/// with no absolute folder in it, whichever of `/opt/homebrew/bin`,
/// `/usr/local/bin` and `~/.local/bin` the inherited PATH lacks go first. The
/// inherited PATH is launchd's when there is none.
pub fn session_path(login: Option<&str>, inherited: Option<&str>, home: Option<&Path>, claude_dir: Option<&Path>) -> String {
    let inherited = inherited.filter(|path| !path.is_empty()).unwrap_or(SYSTEM_PATH);
    let login_dirs = login.into_iter().flat_map(|path| path.split(':'));
    let mut first: Vec<String> = login_dirs.filter(|dir| is_absolute_dir(dir)).map(String::from).collect();
    if first.is_empty() {
        let local_bin = home.map(|home| home.join(".local/bin").to_string_lossy().into_owned());
        first = HOMEBREW_BINS
            .map(String::from)
            .into_iter()
            .chain(local_bin)
            .filter(|dir| !inherited.split(':').any(|entry| same_dir(entry, dir)))
            .collect();
    }
    let claude_dir = claude_dir.map(|dir| dir.to_string_lossy().into_owned());
    let mut dirs: Vec<String> = Vec::new();
    for dir in first.into_iter().chain(inherited.split(':').map(String::from)).chain(claude_dir) {
        if !dir.is_empty() && !dirs.iter().any(|seen| same_dir(seen, &dir)) {
            dirs.push(dir);
        }
    }
    dirs.join(":")
}

/// Whether two PATH entries name the same folder, a trailing slash aside.
fn same_dir(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    fn home() -> Option<&'static Path> {
        Some(Path::new("/Users/x"))
    }

    /// A stand-in for a login shell. It gets `-l -i -c <command>` or
    /// `-l -c <command>`, and exits with 64 on anything else. The body finds
    /// the command in `$cmd`, and `interactive` or `login` in `$mode`. It has
    /// run once already: macOS checks a new executable the first time it
    /// runs, which can take longer than a test's timeout when many tests
    /// start new scripts at once. And Linux does not start a file that is
    /// open for writing (ETXTBSY): a process another test forks while the
    /// script is written keeps it open until that process execs.
    fn fake_shell(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        let args = "if [ $# = 4 ] && [ \"$1 $2 $3\" = '-l -i -c' ]; then mode=interactive cmd=$4\n\
                    elif [ $# = 3 ] && [ \"$1 $2\" = '-l -c' ]; then mode=login cmd=$3\n\
                    else exit 64; fi";
        std::fs::write(&path, format!("#!/bin/sh\n[ -n \"$WARM_UP\" ] && exit 0\n{args}\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match Command::new(&path).env("WARM_UP", "1").status() {
                Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                warmed_up => break assert!(warmed_up.unwrap().success()),
            }
        }
        path
    }

    /// Waits up to 2 s for process `pid` to be gone.
    fn gone(pid: i32) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while unsafe { libc::kill(pid, 0) } == 0 {
            assert!(Instant::now() < deadline, "{pid} still runs");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn the_login_path_comes_first_then_inherited_folders_it_lacks_then_claude() {
        // nvm from ~/.zshrc when only the login shell (`-l`) could be read: an
        // app started from a terminal inherits it, and the agent must find node.
        let login = "/opt/homebrew/bin:/usr/bin:/bin";
        let inherited = "/Users/x/.nvm/versions/node/v23/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin";
        assert_eq!(
            session_path(Some(login), Some(inherited), home(), Some(Path::new("/Users/x/.local/bin"))),
            "/opt/homebrew/bin:/usr/bin:/bin:/Users/x/.nvm/versions/node/v23/bin:/usr/sbin:/Users/x/.local/bin"
        );
        // Each folder once, with or without a trailing slash, and no empty entries.
        assert_eq!(session_path(Some("/a:/b/:/a::/b"), Some("/b:/c/"), home(), Some(Path::new("/c"))), "/a:/b/:/c/");
        // A claude already on the PATH adds nothing; without an inherited PATH, launchd's is used.
        assert_eq!(
            session_path(Some(login), None, home(), Some(Path::new("/usr/bin"))),
            "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        );
    }

    #[test]
    fn login_path_entries_that_are_not_absolute_are_ignored() {
        let finder = "/usr/bin:/bin:/usr/sbin:/sbin";
        let fallback = "/opt/homebrew/bin:/usr/local/bin:/Users/x/.local/bin:/usr/bin:/bin:/usr/sbin:/sbin";
        // A shell such as nushell printing `$PATH` as it is counts as no login PATH.
        assert_eq!(session_path(Some("$PATH"), Some(finder), home(), None), fallback);
        assert_eq!(session_path(Some(".:relative/bin"), Some(finder), home(), None), fallback);
        // Absolute entries stay, in order.
        assert_eq!(session_path(Some(".:/login/bin:relative"), Some("/usr/bin"), home(), None), "/login/bin:/usr/bin");
    }

    #[test]
    fn a_login_shell_that_prints_no_absolute_folder_gives_no_login_path() {
        let dir = tempfile::tempdir().unwrap();
        // Like nushell, where `"$PATH"` is a plain string.
        let shell = fake_shell(dir.path(), "nu-like", "PATH='$PATH'\nexport PATH\nexec /bin/sh -c \"$cmd\"");
        assert_eq!(ask_login_shell(&shell, LOGIN_SHELL_TIMEOUT), Ok("$PATH".into()));
        assert_eq!(login_path(&shell), None);
    }

    #[test]
    fn without_the_login_path_the_missing_install_folders_go_first() {
        let finder = "/usr/bin:/bin:/usr/sbin:/sbin";
        let expected = "/opt/homebrew/bin:/usr/local/bin:/Users/x/.local/bin:/usr/bin:/bin:/usr/sbin:/sbin";
        assert_eq!(session_path(None, Some(finder), home(), None), expected);
        // An empty login PATH counts as none.
        assert_eq!(session_path(Some(""), Some(finder), home(), None), expected);
        // Folders already there, with or without a trailing slash, are not added again.
        assert_eq!(
            session_path(None, Some("/usr/bin:/usr/local/bin/:/opt/homebrew/bin"), home(), None),
            "/Users/x/.local/bin:/usr/bin:/usr/local/bin/:/opt/homebrew/bin"
        );
        let full = "/Users/x/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin";
        assert_eq!(session_path(None, Some(full), home(), None), full);
        // Without a home folder there is no ~/.local/bin.
        assert_eq!(session_path(None, Some(finder), None, None), "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin");
        // Without an inherited PATH launchd's is the base.
        assert_eq!(session_path(None, None, home(), None), expected);
        assert_eq!(session_path(None, Some(""), home(), None), expected);
        // The folder of claude comes last.
        assert_eq!(
            session_path(None, Some(finder), None, Some(Path::new("/Applications/Tools/bin"))),
            "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:/Applications/Tools/bin"
        );
    }

    #[test]
    fn the_path_is_read_between_the_markers() {
        fn between(output: &str) -> Option<&str> {
            path_between_markers(output, "BEGIN-1", "END-1")
        }
        // A banner before it and output after it, from `.zlogout` or a job, do not matter.
        assert_eq!(between("Last login: today\nBEGIN-1\n/usr/bin:/bin\nEND-1\nSaving session...\n"), Some("/usr/bin:/bin"));
        assert_eq!(between("\nBEGIN-1\n/a b/bin\r\nEND-1\r\n"), Some("/a b/bin"));
        assert_eq!(between("BEGIN-1\n/usr/bin\nEND-1"), Some("/usr/bin"));
        assert_eq!(between("BEGIN-1\n\nEND-1\n"), Some(""));
        // Not yet, or not at all.
        assert_eq!(between("BEGIN-1\n/usr/bin\n"), None);
        assert_eq!(between("BEGIN-1\n/usr/bin\nEND"), None);
        assert_eq!(between("/usr/bin\nEND-1\n"), None);
        assert_eq!(between("xBEGIN-1\n/usr/bin\nEND-1\n"), None);
        assert_eq!(between(""), None);
    }

    #[test]
    fn the_shell_is_shell_else_zsh_then_bash_then_sh() {
        let all = |_: &Path| true;
        let only = |wanted: &'static str| move |path: &Path| path == Path::new(wanted);
        assert_eq!(shell_or_default(Some("/opt/homebrew/bin/fish".into()), all), PathBuf::from("/opt/homebrew/bin/fish"));
        // $SHELL unset or empty: zsh where it is there (macOS), else bash
        // (most Linux systems), else sh.
        assert_eq!(shell_or_default(None, all), PathBuf::from("/bin/zsh"));
        assert_eq!(shell_or_default(Some("".into()), all), PathBuf::from("/bin/zsh"));
        assert_eq!(shell_or_default(None, only("/bin/bash")), PathBuf::from("/bin/bash"));
        assert_eq!(shell_or_default(Some("".into()), only("/bin/sh")), PathBuf::from("/bin/sh"));
        assert_eq!(shell_or_default(None, |_| false), PathBuf::from("/bin/sh"));
        // This machine has one of them.
        assert!(is_executable(&shell_or_default(None, is_executable)));
    }

    #[test]
    fn asks_the_shell_for_its_path() {
        let dir = tempfile::tempdir().unwrap();
        // Like a login shell whose profile prints a banner and sets the PATH.
        let body = "echo 'Last login: today'\nPATH='/login/bin:/usr/bin:/bin'\nexport PATH\nexec /bin/sh -c \"$cmd\"";
        let shell = fake_shell(dir.path(), "login-shell", body);
        assert_eq!(ask_login_shell(&shell, LOGIN_SHELL_TIMEOUT), Ok("/login/bin:/usr/bin:/bin".into()));
        assert_eq!(login_path(&shell), Some("/login/bin:/usr/bin:/bin".into()));
    }

    #[test]
    fn trailing_output_and_a_job_holding_the_output_open_do_not_hide_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let job = dir.path().join("job");
        // A profile that leaves a job holding the output open, and a `.zlogout` that prints.
        let body = format!(
            "sleep 30 &\necho $! > '{}'\nPATH='/login/bin:/usr/bin:/bin'\nexport PATH\n/bin/sh -c \"$cmd\"\n{}",
            job.display(),
            "echo 'Saving session...completed.'\nwait"
        );
        let shell = fake_shell(dir.path(), "busy-shell", &body);
        let asked = Instant::now();
        assert_eq!(ask_login_shell(&shell, Duration::from_secs(10)), Ok("/login/bin:/usr/bin:/bin".into()));
        assert!(asked.elapsed() < Duration::from_secs(5), "took {:?}", asked.elapsed());
        // The job was stopped with the shell.
        gone(std::fs::read_to_string(&job).unwrap().trim().parse().unwrap());
    }

    #[test]
    fn a_shell_that_fails_says_why() {
        let dir = tempfile::tempdir().unwrap();
        for (i, (body, reason)) in [
            ("exit 1", "without printing its PATH (exit status: 1)"),
            ("printf 'not a path'", "without printing its PATH (exit status: 0)"),
            ("PATH=''\nexport PATH\nexec /bin/sh -c \"$cmd\"", "it printed an empty PATH"),
        ]
        .into_iter()
        .enumerate()
        {
            let shell = fake_shell(dir.path(), &format!("shell-{i}"), body);
            let err = ask_login_shell(&shell, LOGIN_SHELL_TIMEOUT).unwrap_err();
            assert!(err.contains(reason), "{body}: {err}");
        }
        let missing = dir.path().join("missing");
        assert!(ask_login_shell(&missing, LOGIN_SHELL_TIMEOUT).unwrap_err().contains("it could not start"));
        // The daemon logs the reason and goes on without the login PATH.
        assert_eq!(login_path(&missing), None);
    }

    #[test]
    fn a_slow_shell_is_stopped_with_everything_it_started() {
        let dir = tempfile::tempdir().unwrap();
        let pids = dir.path().join("pids");
        // A profile that starts a job holding the output open, then hangs.
        let body = format!("sleep 30 &\necho $! > '{0}'\necho $$ >> '{0}'\nwait", pids.display());
        let shell = fake_shell(dir.path(), "slow-shell", &body);
        let asked = Instant::now();
        let err = ask_login_shell(&shell, Duration::from_millis(500)).unwrap_err();
        assert!(err.contains("did not print its PATH within 500ms"), "{err}");
        assert!(asked.elapsed() < Duration::from_secs(2), "took {:?}", asked.elapsed());
        for pid in std::fs::read_to_string(&pids).unwrap().lines() {
            gone(pid.parse().unwrap());
        }
    }

    #[test]
    fn the_interactive_login_shell_is_asked_first_and_its_path_wins() {
        let dir = tempfile::tempdir().unwrap();
        let runs = dir.path().join("runs");
        // Like zsh with nvm set up in ~/.zshrc, which only an interactive shell reads.
        let body = format!(
            "echo \"$mode\" >> '{}'\nPATH='/login/bin:/usr/bin:/bin'\n[ \"$mode\" = interactive ] && PATH=\"/rc/nvm/bin:$PATH\"\nexport PATH\nexec /bin/sh -c \"$cmd\"",
            runs.display()
        );
        let shell = fake_shell(dir.path(), "rc-shell", &body);
        assert_eq!(ask_interactive_shell(&shell, INTERACTIVE_SHELL_TIMEOUT), Ok("/rc/nvm/bin:/login/bin:/usr/bin:/bin".into()));
        assert_eq!(ask_login_shell(&shell, LOGIN_SHELL_TIMEOUT), Ok("/login/bin:/usr/bin:/bin".into()));
        std::fs::remove_file(&runs).unwrap();
        // With a PATH from the interactive login shell, the login shell is not asked.
        assert_eq!(login_path(&shell), Some("/rc/nvm/bin:/login/bin:/usr/bin:/bin".into()));
        assert_eq!(std::fs::read_to_string(&runs).unwrap(), "interactive\n");
    }

    #[test]
    fn the_path_is_read_through_the_noise_of_an_interactive_shell() {
        let dir = tempfile::tempdir().unwrap();
        // A ~/.zshrc with a colored banner, a window title, bracketed paste and
        // a prompt with no newline, and more of them after the PATH.
        let noise = "printf '\\033[1;32mWelcome back\\033[0m\\n\\033]0;~\\007\\033[?2004h%% '";
        let body = format!(
            "{noise}\nPATH='/rc/bin:/usr/bin:/bin'\nexport PATH\n/bin/sh -c \"$cmd\"\n{noise}\nprintf '\\033[?2004l\\r\\033[K\\r\\n'"
        );
        let shell = fake_shell(dir.path(), "noisy-shell", &body);
        assert_eq!(ask_interactive_shell(&shell, INTERACTIVE_SHELL_TIMEOUT), Ok("/rc/bin:/usr/bin:/bin".into()));
        assert_eq!(login_path(&shell), Some("/rc/bin:/usr/bin:/bin".into()));
    }

    #[test]
    fn without_a_path_from_the_interactive_shell_the_login_shell_is_asked() {
        let dir = tempfile::tempdir().unwrap();
        let (rc, runs, pids) = (dir.path().join("rc"), dir.path().join("runs"), dir.path().join("pids"));
        // The interactive shell also runs `rc`, as zsh runs ~/.zshrc.
        let body = format!(
            "echo \"$mode\" >> '{}'\nPATH='/login/bin:/usr/bin:/bin'\n[ \"$mode\" = interactive ] && . '{}'\nexport PATH\nexec /bin/sh -c \"$cmd\"",
            runs.display(),
            rc.display()
        );
        let shell = fake_shell(dir.path(), "rc-shell", &body);
        let hang = format!("sleep 30 &\necho $! > '{0}'\necho $$ >> '{0}'\nwait", pids.display());
        for script in [
            // It fails, or exits before the command runs.
            "exit 1",
            "exit 0",
            // An empty PATH, or one with no absolute folder.
            "PATH=''",
            "PATH='$PATH'",
            // It starts a job holding the output open, then hangs.
            hang.as_str(),
        ] {
            std::fs::write(&rc, script).unwrap();
            let _ = std::fs::remove_file(&runs);
            let asked = Instant::now();
            let path = login_path_within(&shell, Duration::from_millis(500), LOGIN_SHELL_TIMEOUT);
            assert_eq!(path, Some("/login/bin:/usr/bin:/bin".into()), "{script}");
            assert!(asked.elapsed() < Duration::from_secs(3), "{script}: took {:?}", asked.elapsed());
            assert_eq!(std::fs::read_to_string(&runs).unwrap(), "interactive\nlogin\n", "{script}");
        }
        // The shell that hung was stopped with the job it started.
        for pid in std::fs::read_to_string(&pids).unwrap().lines() {
            gone(pid.parse().unwrap());
        }
    }

    #[test]
    fn a_startup_script_that_asks_a_question_does_not_wait() {
        let dir = tempfile::tempdir().unwrap();
        // Like an update prompt in ~/.zshrc that asks without checking for a
        // terminal, on stdin and on the terminal. Stdin is null and the shell
        // has no terminal, also when the tests run in one, so both reads end
        // at once.
        let body = "printf 'Update now? [Y/n] '\nread answer\nread answer < /dev/tty\nPATH='/rc/bin:/usr/bin:/bin'\nexport PATH\nexec /bin/sh -c \"$cmd\"";
        let shell = fake_shell(dir.path(), "asking-shell", body);
        let asked = Instant::now();
        assert_eq!(ask_interactive_shell(&shell, Duration::from_secs(10)), Ok("/rc/bin:/usr/bin:/bin".into()));
        assert!(asked.elapsed() < Duration::from_secs(5), "took {:?}", asked.elapsed());
    }
}
