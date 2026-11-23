//! Daemon state and every operation on it: projects, sessions, approvals and
//! allow-always rules. Each state change is saved to the store, appended to
//! the event log and broadcast to app clients.
//!
//! Store writes are best effort: a failure is logged and the in-memory state
//! stays authoritative while the daemon runs.

use crate::claude;
use crate::paths::Paths;
use crate::pty::{self, PtySession, SpawnSpec};
use crate::server::{self, OutputBuffer};
use crate::session_path;
use crate::store::{AllowRule, NewEvent, Store};
use crate::worktree::{self, RepoInfo, WorktreeError};
use acc_protocol::{
    Approval, ApprovalState, DaemonEvent, Decision, EventRecord, Project, Session, SessionStatus, Snapshot, StatusChange,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
#[cfg(unix)]
use tokio::sync::OnceCell;
use tokio::sync::{broadcast, mpsc, oneshot, watch, Mutex};
use tokio::task::JoinHandle;
use uuid::Uuid;

/// How long a pending approval waits for a decision.
pub const DEFAULT_APPROVAL_TIMEOUT: Duration = Duration::from_secs(55 * 60);
pub const CLAUDE_NOT_FOUND: &str = "Claude Code was not found. Install it or add it to your PATH.";
/// How a user stops the daemon, for messages.
#[cfg(unix)]
pub const STOP_ACCD: &str = "pkill accd";
#[cfg(windows)]
pub const STOP_ACCD: &str = "taskkill /IM accd.exe /F";

const PTY_COLS: u16 = 120;
const PTY_ROWS: u16 = 32;
const TITLE_MAX_CHARS: usize = 80;
/// How long a restart waits for a run that is being stopped to exit.
const STOP_WAIT: Duration = Duration::from_secs(3);
/// An "Always allow" Bash rule never covers a command containing these.
const SHELL_OPERATORS: [&str; 8] = [";", "&", "|", "`", "$(", ">", "<", "\n"];
/// Nor one containing these: globs, braces, escapes, `~` and `$` let a
/// plain-looking command reach paths the user never saw.
const EXPANSION_CHARS: [char; 7] = ['*', '?', '[', '{', '\\', '~', '$'];
/// Split a summary into the tokens checked for protected paths, besides
/// whitespace. Shell punctuation is included, so `cat server.pem;` still
/// names `server.pem`, and so is `@`, after which curl reads a file.
const PATH_TOKEN_SEPARATORS: [char; 16] = ['"', '\'', '=', '>', '<', ';', '&', '|', '(', ')', '`', ',', ':', '{', '}', '@'];
/// Whose path rules apply. The rule checks take it as a parameter, so the
/// Windows rules are tested on every platform.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Platform {
    Unix,
    Windows,
}

/// The platform this daemon runs on.
const PLATFORM: Platform = if cfg!(windows) { Platform::Windows } else { Platform::Unix };

/// What separates the folders of a path. Windows takes a backslash as well
/// as a slash.
fn path_separators(platform: Platform) -> &'static [char] {
    match platform {
        Platform::Unix => &['/'],
        Platform::Windows => &['/', '\\'],
    }
}
/// Input fields that hold a path. Each is checked for protected paths, and a
/// rule without a prefix needs every one that is present inside the worktree.
const PATH_FIELDS: [&str; 3] = ["file_path", "notebook_path", "path"];
/// Credential folders and files, and Claude Code's own settings, hooks and
/// MCP servers (an agent that edits them can grant itself permissions). A
/// path to or through one of them is protected.
const PROTECTED_NAMES: [&str; 11] = [
    ".ssh", ".aws", ".gnupg", ".kube", ".docker", ".netrc", ".git-credentials", ".npmrc", ".pypirc", ".claude", ".mcp.json",
];
/// A file whose name starts with this is protected: `.env`, `.env.local`, `.envrc`.
const PROTECTED_PREFIX: &str = ".env";
/// A file whose name ends with one of these is protected: `server.pem`, `id.key`.
const PROTECTED_EXTENSIONS: [&str; 2] = [".pem", ".key"];
/// The folder that holds the [`CREDENTIAL_CONFIG_DIRS`].
const CONFIG_DIR: &str = ".config";
/// Credential folders under `.config`. A path through `.config/<name>` is protected.
const CREDENTIAL_CONFIG_DIRS: [&str; 2] = ["gh", "gcloud"];
/// Programs that run code given on their command line, with an argument
/// below. Every python and pypy counts too (`is_python`).
const INTERPRETERS: [&str; 19] = [
    "node", "deno", "bun", "tsx", "ts-node", "bash", "sh", "zsh", "fish", "dash", "ksh", "tcsh", "csh", "perl", "ruby",
    "php", "osascript", "ipython", "ipython3",
];
/// The arguments that give an interpreter its code, or a module to load
/// before the script (node, bun and deno). Long ones count abbreviated too
/// (`fish --comm`), and so do clusters of short flags (`bash -lc`, `perl
/// -lE`) and code attached to its flag (`python3 -c'print(1)'`).
const INLINE_CODE_ARGS: [&str; 14] = [
    "-c",
    "-e",
    "-E",
    "-p",
    "-r",
    "eval",
    "--eval",
    "--print",
    "--require",
    "--import",
    "--loader",
    "--experimental-loader",
    "--preload",
    "--command",
];
/// The short flags among the [`INLINE_CODE_ARGS`], which count in a cluster too.
const INLINE_CODE_LETTERS: [char; 5] = ['c', 'e', 'E', 'p', 'r'];
/// Commands that run the command given as their arguments.
const WRAPPERS: [&str; 18] = [
    "env", "xargs", "sudo", "doas", "nohup", "time", "nice", "timeout", "watch", "exec", "command", "builtin", "eval",
    "source", ".", "arch", "caffeinate", "xcrun",
];
/// On Windows also its shells, which run the command or code they are
/// given, WSL, and `start` and `runas`.
#[cfg(windows)]
const PLATFORM_WRAPPERS: &[&str] = &["cmd", "powershell", "pwsh", "wsl", "start", "runas"];
#[cfg(unix)]
const PLATFORM_WRAPPERS: &[&str] = &[];
/// A program and one of its subcommands that publish work or call a forge
/// API. They are asked about every time.
const PUBLISHING: [(&str, &str); 5] = [("git", "push"), ("gh", "pr"), ("gh", "api"), ("glab", "mr"), ("glab", "api")];
/// `git` options that set its configuration for one command, or for the new
/// repository (`clone --config`). An alias or a pager set this way runs any
/// command, so a `git --no-pager` rule would cover a push or `rm -rf`.
const GIT_CONFIG_ARGS: [&str; 3] = ["-c", "--config-env", "--config"];
/// `git` options before the subcommand that take the next word as their
/// value, as git 2.50 reads them (`--super-prefix` for older ones).
const GIT_GLOBAL_VALUE_ARGS: [&str; 9] =
    ["-C", "-c", "--git-dir", "--work-tree", "--namespace", "--attr-source", "--shallow-file", "--super-prefix", "--config-env"];
/// `git config` options that only read, or choose which file is read.
const GIT_CONFIG_READ_ARGS: [&str; 10] =
    ["--get", "--get-all", "--get-regexp", "--list", "-l", "--show-origin", "--global", "--system", "--local", "--worktree"];
/// Of those, the ones that make `git config` a read whatever words follow.
const GIT_CONFIG_READ_ACTIONS: [&str; 5] = ["--get", "--get-all", "--get-regexp", "--list", "-l"];
/// `git` options that run another program: one given to them (`rebase
/// --exec`, `difftool --extcmd`, `fetch --upload-pack`, `push
/// --receive-pack`, `grep --open-files-in-pager`), one the repository's
/// settings name (`diff --ext-diff`), or git's own commands from another
/// folder (`--exec-path=<dir>`).
const GIT_COMMAND_ARGS: [&str; 7] =
    ["--exec", "--extcmd", "--upload-pack", "--receive-pack", "--open-files-in-pager", "--ext-diff", "--exec-path"];
/// `git` subcommands that run a command given to them, as the words that
/// must all be in the command.
const GIT_COMMAND_SUBCOMMANDS: [&[&str]; 3] = [&["submodule", "foreach"], &["bisect", "run"], &["filter-branch"]];
/// `git` subcommands whose `-u` names the program to run as the upload pack.
const GIT_UPLOAD_PACK_SUBCOMMANDS: [&str; 3] = ["clone", "fetch", "ls-remote"];
/// Python modules that run code from their arguments: statements
/// (`timeit`), debugger commands (`pdb`), a console (`code`), or another
/// module or script (`runpy`, `trace`, `cProfile`, `profile`).
const CODE_MODULES: [&str; 7] = ["timeit", "pdb", "code", "runpy", "trace", "cProfile", "profile"];
/// Commands that run what their next word names: package runners that fetch
/// the package (`npx`, `pnpm dlx`, with the aliases `npm x`, `bun x` and `uv
/// tool run`), runners of a project's commands (`uv run`, `poetry run`, `pnpm
/// exec`, `yarn exec`, `bundle exec`, and those of `pipenv`, `pdm`, `hatch`,
/// `rye`, `pixi`, `conda`, `mise` and `deno run`), `go run` for a Go package,
/// and `python -m` (any python) and `coverage run -m` for a module. A rule for one names what it
/// runs; a flag in its place, or nothing, makes the request once-only.
const PACKAGE_RUNNERS: [&[&str]; 28] = [
    &["npx"],
    &["bunx"],
    &["pnpx"],
    &["uvx"],
    &["pnpm", "dlx"],
    &["pnpm", "exec"],
    &["yarn", "dlx"],
    &["yarn", "exec"],
    &["npm", "exec"],
    &["npm", "x"],
    &["bun", "x"],
    &["pipx", "run"],
    &["uv", "tool", "run"],
    &["uv", "run"],
    &["poetry", "run"],
    &["bundle", "exec"],
    &["go", "run"],
    &["python", "-m"],
    &["pipenv", "run"],
    &["pdm", "run"],
    &["hatch", "run"],
    &["rye", "run"],
    &["pixi", "run"],
    &["conda", "run"],
    &["mise", "exec"],
    &["mise", "x"],
    &["deno", "run"],
    &["coverage", "run", "-m"],
];
/// Runners that also run a command given after `--` or with `-c` or
/// `--command`, which a rule naming their next word would not name: `mise
/// exec node@20 -- node app.js`, `mise exec node@20 -c "<command>"`.
const COMMAND_RUNNERS: [&str; 1] = ["mise"];
/// The most words of a Bash command that are analysed, and the most runners
/// nested in one. A longer command gets no rule, as not a plain command,
/// which also bounds the work done under the state lock.
const MAX_ANALYSED_WORDS: usize = 256;
const MAX_RUNNER_DEPTH: usize = 8;

pub struct DaemonConfig {
    pub paths: Paths,
    /// `acc-hook`, written into every session's Claude Code settings.
    pub hook_bin: PathBuf,
    pub approval_timeout: Duration,
    /// Claude Code binary. `Some` is used as is, and a missing file counts as
    /// not found. `None` falls back to `claude::detect()`, which also looks
    /// on the login shell's PATH and in the usual install folders.
    pub claude_bin: Option<PathBuf>,
    /// The login shell asked for the PATH the agents get and `claude` is
    /// searched on, so an app opened from Finder or a Linux desktop finds
    /// what a terminal would. It is asked once, when the first session
    /// starts. `None` asks no shell, and the agents keep the daemon's PATH.
    /// Windows has none: the agents get the daemon's PATH there.
    pub login_shell: Option<PathBuf>,
}

impl DaemonConfig {
    /// A config with the user's login shell (`$SHELL`, else zsh, bash or sh),
    /// and none on Windows.
    pub fn new(paths: Paths, hook_bin: PathBuf) -> DaemonConfig {
        #[cfg(unix)]
        let login_shell = Some(session_path::login_shell());
        #[cfg(windows)]
        let login_shell = None;
        DaemonConfig { paths, hook_bin, approval_timeout: DEFAULT_APPROVAL_TIMEOUT, claude_bin: None, login_shell }
    }
}

/// A failed request. `code` is one of `bad_request`, `not_found`,
/// `not_a_repo`, `agent_not_found`, `hook_not_found`, `git_failed` and
/// `internal`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ApiError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl ApiError {
    pub(crate) fn bad_request(message: impl Into<String>) -> ApiError {
        ApiError { code: "bad_request", message: message.into() }
    }

    pub(crate) fn not_found(message: impl Into<String>) -> ApiError {
        ApiError { code: "not_found", message: message.into() }
    }

    fn agent_not_found() -> ApiError {
        ApiError { code: "agent_not_found", message: CLAUDE_NOT_FOUND.to_string() }
    }

    fn hook_not_found(path: &Path) -> ApiError {
        let message = format!("acc-hook was not found at {}. Run cargo build --workspace, then {STOP_ACCD} and reopen the app.", path.display());
        ApiError { code: "hook_not_found", message }
    }

    pub(crate) fn internal(e: impl std::fmt::Display) -> ApiError {
        ApiError { code: "internal", message: format!("{e:#}") }
    }
}

impl From<WorktreeError> for ApiError {
    fn from(e: WorktreeError) -> ApiError {
        let code = match e {
            WorktreeError::NotARepo(_) => "not_a_repo",
            WorktreeError::Git(_) => "git_failed",
        };
        ApiError { code, message: e.to_string() }
    }
}

fn session_not_found() -> ApiError {
    ApiError::not_found("Session not found.")
}

/// Shared by every task: the config, the event fan-out and the state.
pub(crate) struct Shared {
    pub(crate) cfg: DaemonConfig,
    pub(crate) events: broadcast::Sender<DaemonEvent>,
    pub(crate) state: Mutex<Daemon>,
    /// The login shell's PATH once it has been asked, `None` inside when it
    /// could not be read.
    #[cfg(unix)]
    login_path: OnceCell<Option<String>>,
}

/// A freshly spawned agent whose output pump has not started yet.
struct Spawned {
    live: Live,
    output: Arc<OutputBuffer>,
    exited: oneshot::Receiver<Option<i32>>,
}

impl Shared {
    pub(crate) fn new(cfg: DaemonConfig, events: broadcast::Sender<DaemonEvent>, daemon: Daemon) -> Shared {
        Shared {
            cfg,
            events,
            state: Mutex::new(daemon),
            #[cfg(unix)]
            login_path: OnceCell::new(),
        }
    }

    /// The PATH for an agent whose `claude` is `claude` (`None` while it is
    /// still being looked for): the login shell's PATH, then the folders of
    /// the daemon's own PATH it lacks, then the folder of `claude`. The login
    /// shell is asked once, the first time a session starts rather than at
    /// startup, so the socket is up at once and a daemon that never starts a
    /// session never runs the shell. `None` without a login shell in the
    /// config: the agents keep the daemon's PATH.
    #[cfg(unix)]
    async fn session_path(&self, claude: Option<&Path>) -> Option<String> {
        let shell = self.cfg.login_shell.clone()?;
        let login = self.login_path.get_or_init(|| async move {
            // A failed blocking task (the runtime shutting down) reads no PATH.
            blocking(move || session_path::login_path(&shell)).await.unwrap_or(None)
        });
        let login = login.await.as_deref();
        let inherited = std::env::var("PATH").ok();
        let claude_dir = claude.and_then(Path::parent);
        Some(session_path::session_path(login, inherited.as_deref(), dirs::home_dir().as_deref(), claude_dir))
    }

    /// The PATH for an agent whose `claude` is `claude` (`None` while it is
    /// still being looked for): the daemon's own PATH, then the folder of
    /// `claude`. Windows has no login shell to ask.
    #[cfg(windows)]
    async fn session_path(&self, claude: Option<&Path>) -> Option<String> {
        let inherited = std::env::var("PATH").ok();
        Some(session_path::session_path(inherited.as_deref(), claude.and_then(Path::parent)))
    }

    /// The Claude Code binary to launch, if there is an executable one.
    /// Without one in the config it is searched for, also on `search_path`.
    fn claude_program(&self, search_path: Option<&str>) -> Option<PathBuf> {
        let program = match &self.cfg.claude_bin {
            Some(path) => path.clone(),
            None => claude::detect(search_path)?.program,
        };
        claude::is_executable(&program).then_some(program)
    }

    /// Fails unless `acc-hook` is an executable file. Claude Code treats a
    /// hook that cannot run as a non-blocking error, so without this check
    /// its permission requests would go to its own prompt, unseen by the app.
    fn check_hook(&self) -> Result<(), ApiError> {
        match claude::is_executable(&self.cfg.hook_bin) {
            true => Ok(()),
            false => Err(ApiError::hook_not_found(&self.cfg.hook_bin)),
        }
    }

    pub(crate) async fn project_add(&self, path: &str) -> Result<Project, ApiError> {
        let dir = PathBuf::from(path);
        if !dir.is_absolute() {
            return Err(ApiError::bad_request("The project path must be absolute."));
        }
        let info = blocking(move || worktree::inspect_repo(&dir)).await??;
        Ok(self.state.lock().await.add_project(info))
    }

    pub(crate) async fn session_start(self: &Arc<Self>, project_id: &str, agent: &str, task: &str) -> Result<Session, ApiError> {
        if agent != "claude" {
            return Err(ApiError::bad_request(format!("Unknown agent {agent:?}.")));
        }
        let search_path = self.session_path(None).await;
        // Without the agent or the hook nothing is created: no worktree, settings or session.
        let program = self.claude_program(search_path.as_deref()).ok_or_else(ApiError::agent_not_found)?;
        self.check_hook()?;
        let session_path = self.session_path(Some(&program)).await;
        let project = self.state.lock().await.project(project_id).ok_or_else(|| ApiError::not_found("Project not found."))?;

        let id = Uuid::new_v4().to_string();
        let repo = RepoInfo {
            name: project.name.clone(),
            root: PathBuf::from(&project.path),
            host: project.host.clone(),
            base_branch: project.base_branch.clone(),
        };
        let slug = worktree::slugify(task);
        let session_dir = self.cfg.paths.session_dir(&id);
        // A run that cannot start leaves no worktree or settings behind.
        let planned = claude::launch_args(&claude::settings_path(&session_dir), &id, task);
        if let Some(why) = pty::launch_error(&program, &planned) {
            return Err(ApiError::bad_request(why));
        }
        let hook_bin = self.cfg.hook_bin.clone();
        let (worktree, branch, settings) = blocking(move || -> Result<_, ApiError> {
            let (worktree, branch) = worktree::create_worktree(&repo, &slug)?;
            let settings = claude::write_settings(&session_dir, &hook_bin).map_err(ApiError::internal)?;
            Ok((worktree, branch, settings))
        })
        .await??;

        let mut state = self.state.lock().await;
        let args = claude::launch_args(&settings, &id, task);
        let spawned = self.spawn_claude(&id, program, args, worktree.clone(), session_path)?;
        let now = now_ms();
        let session = Session {
            id: id.clone(),
            project_id: project.id,
            agent: agent.to_string(),
            title: session_title(task),
            branch,
            worktree: worktree.to_string_lossy().into_owned(),
            status: SessionStatus::Starting,
            status_since: now,
            started_at: now,
            ended_at: None,
            exit_code: None,
            history: vec![StatusChange { status: SessionStatus::Starting, at: now }],
        };
        state.add_session(session.clone(), spawned.live);
        let pump = server::spawn_output_pump(self.clone(), id.clone(), spawned.output, spawned.exited);
        state.set_pump(&id, pump);
        Ok(session)
    }

    pub(crate) async fn session_restart(self: &Arc<Self>, session_id: &str) -> Result<Session, ApiError> {
        // A run that is being stopped gets a moment to exit, waited for
        // outside the lock (its own statement, so the guard is dropped first).
        // The run's sender is dropped once it has exited.
        let stopping = self.state.lock().await.stopping_run(session_id)?;
        if let Some(mut ended) = stopping {
            let _ = tokio::time::timeout(STOP_WAIT, ended.changed()).await;
        }
        self.state.lock().await.restart_dir(session_id)?;
        let search_path = self.session_path(None).await;
        let program = self.claude_program(search_path.as_deref()).ok_or_else(ApiError::agent_not_found)?;
        self.check_hook()?;
        let session_path = self.session_path(Some(&program)).await;
        let settings = claude::write_settings(&self.cfg.paths.session_dir(session_id), &self.cfg.hook_bin)
            .map_err(ApiError::internal)?;

        let mut state = self.state.lock().await;
        // Checked again: another request may have restarted it in between.
        let worktree = state.restart_dir(session_id)?;
        let spawned = self.spawn_claude(session_id, program, claude::resume_args(&settings, session_id), worktree, session_path)?;
        let session = state.mark_restarted(session_id, spawned.live)?;
        let pump = server::spawn_output_pump(self.clone(), session_id.to_string(), spawned.output, spawned.exited);
        state.set_pump(session_id, pump);
        Ok(session)
    }

    pub(crate) async fn session_resize(&self, session_id: &str, cols: u16, rows: u16) -> Result<(), ApiError> {
        let pty = self.state.lock().await.pty(session_id)?;
        if let Some(pty) = pty {
            pty.resize(cols, rows).map_err(ApiError::internal)?;
        }
        Ok(())
    }

    /// Starts the agent on a PTY, with `path` as its PATH when there is one.
    /// The PTY threads hand output and the exit to an [`OutputBuffer`] and a
    /// channel; they never touch the state lock.
    fn spawn_claude(
        &self,
        session_id: &str,
        program: PathBuf,
        args: Vec<String>,
        cwd: PathBuf,
        path: Option<String>,
    ) -> Result<Spawned, ApiError> {
        let output = Arc::new(OutputBuffer::default());
        let (exit_tx, exited) = oneshot::channel();
        let sink = output.clone();
        let mut env = vec![
            ("ACC_SESSION_ID".to_string(), session_id.to_string()),
            ("ACC_SOCKET".to_string(), self.cfg.paths.socket.to_string_lossy().into_owned()),
            ("TERM".to_string(), "xterm-256color".to_string()),
        ];
        env.extend(path.map(|path| ("PATH".to_string(), path)));
        let spec = SpawnSpec {
            program,
            args,
            cwd,
            env,
            env_remove: claude::removed_env(),
            cols: PTY_COLS,
            rows: PTY_ROWS,
        };
        let pty = PtySession::spawn(
            spec,
            Box::new(move |chunk, offset| sink.push(chunk, offset)),
            Box::new(move |code| {
                let _ = exit_tx.send(code);
            }),
        )
        .map_err(|e| ApiError::internal(format!("Could not start Claude Code: {e:#}")))?;
        let pty = Arc::new(pty);
        let ended = watch::channel(()).0;
        let live = Live { input: spawn_input_writer(pty.clone()), pty, pump: None, stopping: false, ended };
        Ok(Spawned { live, output, exited })
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f).await.map_err(ApiError::internal)
}

/// Writes input to the PTY in arrival order, off the runtime threads
/// (`PtySession::write` blocks while the agent does not read).
fn spawn_input_writer(pty: Arc<PtySession>) -> mpsc::UnboundedSender<Vec<u8>> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        while let Some(data) = rx.recv().await {
            let pty = pty.clone();
            // A write error means the agent exited, which the output pump
            // records. Only a failed blocking task (runtime shutdown) stops this.
            if tokio::task::spawn_blocking(move || pty.write(&data)).await.is_err() {
                break;
            }
        }
    });
    tx
}

/// A running agent.
struct Live {
    pty: Arc<PtySession>,
    input: mpsc::UnboundedSender<Vec<u8>>,
    pump: Option<JoinHandle<()>>,
    /// The user stopped the session, so its exit keeps the Stopped status.
    stopping: bool,
    /// Never sent on. Dropped with the run when it has exited, which wakes
    /// every receiver (a restart waiting for a stop).
    ended: watch::Sender<()>,
}

struct SessionEntry {
    session: Session,
    live: Option<Live>,
    /// The last run's output and the stream position after it, kept after it exits.
    scrollback: Vec<u8>,
    scrollback_end: u64,
}

impl SessionEntry {
    fn new(session: Session, live: Option<Live>) -> SessionEntry {
        SessionEntry { session, live, scrollback: Vec::new(), scrollback_end: 0 }
    }
}

struct PendingApproval {
    approval: Approval,
    project_id: String,
    /// What Always allow adds (shown as `rule_preview`), or why it allows once.
    rule: Result<AllowRule, OnceOnly>,
    reply: oneshot::Sender<Decision>,
}

pub(crate) enum ApprovalOutcome {
    /// Unknown or finished session: Claude Code asks in its own terminal.
    Ignored,
    /// A project rule allowed it.
    Allowed,
    /// Waits for a decision, which arrives on `decision`.
    Pending { id: String, decision: oneshot::Receiver<Decision> },
}

pub(crate) enum Decided {
    Resolved(Approval),
    AlreadyResolved,
}

pub(crate) struct Daemon {
    store: Store,
    events: broadcast::Sender<DaemonEvent>,
    projects: Vec<Project>,
    sessions: HashMap<String, SessionEntry>,
    pending: HashMap<String, PendingApproval>,
    /// Approvals that left the inbox, so a late decision on one is a no-op.
    /// Rule-allowed ones never reach the inbox and are not kept.
    resolved: HashSet<String>,
    /// Set by shutdown: agent exits are no longer recorded.
    shutting_down: bool,
}

impl Daemon {
    /// Opens the store and loads it. Sessions that were live when the last
    /// daemon went away become Stopped and their pending approvals Cancelled.
    pub(crate) fn open(paths: &Paths, events: broadcast::Sender<DaemonEvent>) -> anyhow::Result<Daemon> {
        let store = Store::open(&paths.db)?;
        let (orphans, cancelled) = store.mark_orphans_stopped(now_ms())?;
        let projects = store.projects()?;
        let sessions = store
            .sessions()?
            .into_iter()
            .map(|session| (session.id.clone(), SessionEntry::new(session, None)))
            .collect();
        let mut daemon = Daemon {
            store,
            events,
            projects,
            sessions,
            pending: HashMap::new(),
            resolved: HashSet::new(),
            shutting_down: false,
        };
        for s in &orphans {
            daemon.session_event(s, "session.status", "system", json!({ "status": s.status }));
        }
        for a in cancelled {
            daemon.approval_event(&a, "approval.cancelled", "system", json!({}));
            daemon.resolved.insert(a.id);
        }
        Ok(daemon)
    }

    fn emit(&self, event: DaemonEvent) {
        // An error only means no app is connected.
        let _ = self.events.send(event);
    }

    /// Appends to the event log and broadcasts the record.
    fn record(&self, session_id: Option<&str>, project_id: Option<&str>, kind: &str, actor: &str, payload: Value) {
        let event = NewEvent {
            session_id: session_id.map(str::to_string),
            project_id: project_id.map(str::to_string),
            kind: kind.to_string(),
            actor: actor.to_string(),
            payload,
        };
        match self.store.append_event(event) {
            Ok(record) => self.emit(DaemonEvent::EventAppended(record)),
            Err(e) => eprintln!("accd: could not append {kind} to the event log: {e:#}"),
        }
    }

    fn session_event(&self, s: &Session, kind: &str, actor: &str, payload: Value) {
        self.record(Some(&s.id), Some(&s.project_id), kind, actor, payload);
    }

    /// Records an approval event; the payload names the approval, plus `extra`.
    fn approval_event(&self, a: &Approval, kind: &str, actor: &str, extra: Value) {
        let mut payload = json!({ "approval_id": a.id, "tool": a.tool, "summary": a.summary });
        if let (Some(fields), Value::Object(extra)) = (payload.as_object_mut(), extra) {
            fields.extend(extra);
        }
        let project_id = self.sessions.get(&a.session_id).map(|e| e.session.project_id.as_str());
        self.record(Some(&a.session_id), project_id, kind, actor, payload);
    }

    /// Saves the session and broadcasts it.
    fn publish(&self, s: &Session) {
        if let Err(e) = self.store.upsert_session(s) {
            eprintln!("accd: could not save session {}: {e:#}", s.id);
        }
        self.emit(DaemonEvent::SessionUpdated(s.clone()));
    }

    fn save_approval(&self, a: &Approval) {
        if let Err(e) = self.store.upsert_approval(a) {
            eprintln!("accd: could not save approval {}: {e:#}", a.id);
        }
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        let mut sessions: Vec<Session> = self.sessions.values().map(|e| e.session.clone()).collect();
        sessions.sort_by(|a, b| a.started_at.cmp(&b.started_at).then_with(|| a.id.cmp(&b.id)));
        let mut approvals: Vec<Approval> = self.pending.values().map(|p| p.approval.clone()).collect();
        approvals.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));
        Snapshot { projects: self.projects.clone(), sessions, approvals }
    }

    pub(crate) fn project(&self, id: &str) -> Option<Project> {
        self.projects.iter().find(|p| p.id == id).cloned()
    }

    /// Adds the repository, or returns the project that already has its root.
    pub(crate) fn add_project(&mut self, info: RepoInfo) -> Project {
        let path = info.root.to_string_lossy().into_owned();
        if let Some(existing) = self.projects.iter().find(|p| p.path == path) {
            return existing.clone();
        }
        let project = Project {
            id: Uuid::new_v4().to_string(),
            name: info.name,
            path,
            host: info.host,
            base_branch: info.base_branch,
            added_at: now_ms(),
        };
        if let Err(e) = self.store.upsert_project(&project) {
            eprintln!("accd: could not save project {}: {e:#}", project.id);
        }
        self.projects.push(project.clone());
        self.emit(DaemonEvent::ProjectAdded(project.clone()));
        self.record(None, Some(&project.id), "project.added", "user", json!({ "name": project.name, "path": project.path }));
        project
    }

    fn entry(&self, id: &str) -> Result<&SessionEntry, ApiError> {
        self.sessions.get(id).ok_or_else(session_not_found)
    }

    fn has_pending(&self, session_id: &str) -> bool {
        self.pending.values().any(|p| p.approval.session_id == session_id)
    }

    /// Registers a session whose agent was just spawned.
    fn add_session(&mut self, session: Session, live: Live) {
        self.publish(&session);
        self.session_event(&session, "session.started", "user", start_payload(&session, false));
        self.sessions.insert(session.id.clone(), SessionEntry::new(session, Some(live)));
    }

    fn set_pump(&mut self, session_id: &str, pump: JoinHandle<()>) {
        if let Some(live) = self.sessions.get_mut(session_id).and_then(|e| e.live.as_mut()) {
            live.pump = Some(pump);
        }
    }

    /// The worktree to respawn in, if the session exists and is not running.
    fn restart_dir(&self, session_id: &str) -> Result<PathBuf, ApiError> {
        let entry = self.entry(session_id)?;
        match &entry.live {
            Some(live) if live.stopping => Err(ApiError::bad_request("The session is still stopping. Try again in a moment.")),
            Some(_) => Err(ApiError::bad_request("The session is still running.")),
            None => Ok(PathBuf::from(&entry.session.worktree)),
        }
    }

    /// If the session's run is being stopped, a receiver whose `changed()`
    /// returns once the run has exited.
    fn stopping_run(&self, session_id: &str) -> Result<Option<watch::Receiver<()>>, ApiError> {
        let live = self.entry(session_id)?.live.as_ref();
        Ok(live.filter(|live| live.stopping).map(|live| live.ended.subscribe()))
    }

    fn mark_restarted(&mut self, session_id: &str, live: Live) -> Result<Session, ApiError> {
        let Some(entry) = self.sessions.get_mut(session_id) else {
            let _ = live.pty.kill();
            return Err(session_not_found());
        };
        let now = now_ms();
        change_status(&mut entry.session, SessionStatus::Starting, now);
        // A restart is a new run: its output offsets start at 0, and the UI
        // keys its terminal on `started_at`, so it remounts.
        entry.session.started_at = now;
        entry.session.ended_at = None;
        entry.session.exit_code = None;
        entry.live = Some(live);
        entry.scrollback.clear();
        entry.scrollback_end = 0;
        let session = entry.session.clone();
        self.publish(&session);
        self.session_event(&session, "session.started", "user", start_payload(&session, true));
        Ok(session)
    }

    /// Kills the agent and marks the session Stopped. A session that is not
    /// running is returned unchanged.
    pub(crate) fn stop_session(&mut self, session_id: &str) -> Result<Session, ApiError> {
        let entry = self.sessions.get_mut(session_id).ok_or_else(session_not_found)?;
        let Some(live) = entry.live.as_mut().filter(|live| !live.stopping) else {
            return Ok(entry.session.clone());
        };
        live.pty.kill().map_err(ApiError::internal)?;
        live.stopping = true;
        let now = now_ms();
        change_status(&mut entry.session, SessionStatus::Stopped, now);
        entry.session.ended_at = Some(now);
        let session = entry.session.clone();
        self.publish(&session);
        self.session_event(&session, "session.status", "user", json!({ "status": session.status }));
        Ok(session)
    }

    /// The current or last run's scrollback and the stream position just
    /// after it. Later `SessionOutput` bytes at or past that position are new.
    pub(crate) fn scrollback(&self, session_id: &str) -> Result<(Vec<u8>, u64), ApiError> {
        let entry = self.entry(session_id)?;
        Ok(match &entry.live {
            Some(live) => live.pty.scrollback_with_end(),
            None => (entry.scrollback.clone(), entry.scrollback_end),
        })
    }

    /// Queues input for the agent. Input to a session that is not running is dropped.
    pub(crate) fn queue_input(&self, session_id: &str, data: Vec<u8>) -> Result<(), ApiError> {
        if let Some(live) = &self.entry(session_id)?.live {
            let _ = live.input.send(data);
        }
        Ok(())
    }

    fn pty(&self, session_id: &str) -> Result<Option<Arc<PtySession>>, ApiError> {
        Ok(self.entry(session_id)?.live.as_ref().map(|live| live.pty.clone()))
    }

    /// Records the end of a run. Called after its last output was broadcast.
    pub(crate) fn pty_exited(&mut self, session_id: &str, code: Option<i32>) {
        let shutting_down = self.shutting_down;
        let Some(entry) = self.sessions.get_mut(session_id) else { return };
        let Some(live) = entry.live.take() else { return };
        (entry.scrollback, entry.scrollback_end) = live.pty.scrollback_with_end();
        if shutting_down {
            // As if the daemon died: the next one marks the session Stopped.
            return;
        }
        let now = now_ms();
        if !live.stopping {
            let status = if code == Some(0) { SessionStatus::Done } else { SessionStatus::Failed };
            change_status(&mut entry.session, status, now);
        }
        entry.session.exit_code = code;
        entry.session.ended_at.get_or_insert(now);
        let session = entry.session.clone();
        self.publish(&session);
        self.session_event(&session, "session.exited", "agent", json!({ "exit_code": code, "status": session.status }));

        // Nobody can act on a decision for a run that ended. A hook that is
        // still connected is closed without a reply, so it fails closed.
        let mut waiting: Vec<(i64, String)> = self
            .pending
            .values()
            .filter(|p| p.approval.session_id == session_id)
            .map(|p| (p.approval.created_at, p.approval.id.clone()))
            .collect();
        waiting.sort();
        for (_, approval_id) in waiting {
            self.cancel(&approval_id);
        }
    }

    fn set_status(&mut self, session_id: &str, status: SessionStatus, actor: &str) {
        let Some(entry) = self.sessions.get_mut(session_id) else { return };
        if entry.session.status == status {
            return;
        }
        change_status(&mut entry.session, status, now_ms());
        let session = entry.session.clone();
        self.publish(&session);
        self.session_event(&session, "session.status", actor, json!({ "status": session.status }));
    }

    /// A status reported by the agent's hooks. A finished session stays
    /// finished, and Needs you holds while an approval waits.
    pub(crate) fn hook_status(&mut self, session_id: &str, status: SessionStatus) {
        let Some(entry) = self.sessions.get(session_id) else { return };
        if is_finished(&entry.session.status) || self.has_pending(session_id) {
            return;
        }
        self.set_status(session_id, status, "agent");
    }

    pub(crate) fn tool_used(&self, session_id: &str, tool: &str, summary: &str) {
        if let Some(entry) = self.sessions.get(session_id) {
            self.session_event(&entry.session, "tool.used", "agent", json!({ "tool": tool, "summary": summary }));
        }
    }

    /// A permission request from the agent: allowed by a project rule, or
    /// held as a pending approval until someone decides.
    pub(crate) fn request_approval(&mut self, session_id: &str, tool: String, summary: String, input: Value) -> ApprovalOutcome {
        let Some(entry) = self.sessions.get(session_id) else { return ApprovalOutcome::Ignored };
        if is_finished(&entry.session.status) {
            return ApprovalOutcome::Ignored;
        }
        let project_id = entry.session.project_id.clone();
        let worktree = PathBuf::from(&entry.session.worktree);
        let rule = rule_for(&tool, &summary, &input, &worktree);
        let mut approval = Approval {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            tool,
            summary,
            input,
            created_at: now_ms(),
            state: ApprovalState::Pending,
            decided_by: None,
            protected: matches!(rule, Err(OnceOnly::Protected)),
            rule_preview: rule.as_ref().ok().map(rule_preview),
        };

        // A request that could never get a rule (a protected path, inline
        // code, a push, an unknown tool) is never allowed by one either, so
        // rules saved by an older daemon cannot cover it.
        if rule.is_ok() {
            let rules = self.store.rules(&project_id).unwrap_or_else(|e| {
                eprintln!("accd: could not read rules, asking instead: {e:#}");
                Vec::new()
            });
            if let Some(rule) = rules.iter().find(|r| rule_allows(r, &approval.tool, &approval.input, &worktree)) {
                approval.state = ApprovalState::Allowed;
                approval.decided_by = Some("policy".to_string());
                self.save_approval(&approval);
                let extra = json!({ "state": approval.state, "rule": rule_json(rule) });
                self.approval_event(&approval, "approval.decided", "policy", extra);
                return ApprovalOutcome::Allowed;
            }
        }

        self.save_approval(&approval);
        self.emit(DaemonEvent::ApprovalCreated(approval.clone()));
        self.approval_event(&approval, "approval.requested", "agent", json!({}));
        let id = approval.id.clone();
        let (reply, decision) = oneshot::channel();
        self.pending.insert(id.clone(), PendingApproval { approval, project_id, rule, reply });
        self.set_status(session_id, SessionStatus::NeedsYou, "agent");
        ApprovalOutcome::Pending { id, decision }
    }

    /// Moves a pending approval to its final state. `None` if it was not pending.
    fn take_pending(&mut self, id: &str, state: ApprovalState, decided_by: Option<&str>) -> Option<(Approval, PendingApproval)> {
        let pending = self.pending.remove(id)?;
        let mut approval = pending.approval.clone();
        approval.state = state;
        approval.decided_by = decided_by.map(str::to_string);
        self.resolved.insert(approval.id.clone());
        self.save_approval(&approval);
        self.emit(DaemonEvent::ApprovalResolved(approval.clone()));
        Some((approval, pending))
    }

    /// A session that no longer waits on any approval goes back to Working.
    fn after_resolution(&mut self, session_id: &str, actor: &str) {
        let waiting = self.sessions.get(session_id).is_some_and(|e| e.session.status == SessionStatus::NeedsYou);
        if waiting && !self.has_pending(session_id) {
            self.set_status(session_id, SessionStatus::Working, actor);
        }
    }

    pub(crate) fn decide(&mut self, approval_id: &str, decision: Decision) -> Result<Decided, ApiError> {
        let state = if decision == Decision::Deny { ApprovalState::Denied } else { ApprovalState::Allowed };
        let Some((approval, pending)) = self.take_pending(approval_id, state, Some("user")) else {
            return match self.resolved.contains(approval_id) {
                true => Ok(Decided::AlreadyResolved),
                false => Err(ApiError::not_found("Approval not found.")),
            };
        };
        // Always allow adds the rule its `rule_preview` described. Without one
        // it allows this request once, and the event says why.
        let (rule, once_only) = match (&decision, &pending.rule) {
            (Decision::AllowAlways, Ok(rule)) => (Some(rule), None),
            (Decision::AllowAlways, Err(why)) => (None, Some(why.reason())),
            _ => (None, None),
        };
        if let Some(rule) = rule {
            if let Err(e) = self.store.add_rule(&pending.project_id, rule) {
                eprintln!("accd: could not save rule for {}: {e:#}", rule.tool);
            }
        }
        let mut extra = json!({
            "decision": decision,
            "state": approval.state,
            "rule": rule.map(rule_json),
            "protected_path": approval.protected,
        });
        if let Some(reason) = once_only {
            extra["once_only"] = json!(reason);
        }
        self.approval_event(&approval, "approval.decided", "user", extra);
        // The hook may be gone already; then nobody waits for the reply.
        let _ = pending.reply.send(decision);
        self.after_resolution(&approval.session_id, "user");
        Ok(Decided::Resolved(approval))
    }

    /// Times out a pending approval. False if it was resolved first.
    pub(crate) fn expire(&mut self, approval_id: &str) -> bool {
        let Some((approval, _)) = self.take_pending(approval_id, ApprovalState::Expired, None) else { return false };
        self.approval_event(&approval, "approval.expired", "system", json!({}));
        self.after_resolution(&approval.session_id, "system");
        true
    }

    /// Cancels a pending approval: the hook that asked went away (Esc in
    /// Claude Code, or it exited), or the run it belongs to ended.
    pub(crate) fn cancel(&mut self, approval_id: &str) {
        if let Some((approval, _)) = self.take_pending(approval_id, ApprovalState::Cancelled, None) {
            self.approval_event(&approval, "approval.cancelled", "agent", json!({}));
            self.after_resolution(&approval.session_id, "agent");
        }
    }

    pub(crate) fn events(&self, session_id: Option<&str>, limit: u32) -> Result<Vec<EventRecord>, ApiError> {
        self.store.list_events(session_id, limit).map_err(ApiError::internal)
    }

    /// Kills every running agent without recording the exits, and returns
    /// their output pumps, which end once the agents have exited.
    pub(crate) fn kill_all(&mut self) -> Vec<JoinHandle<()>> {
        self.shutting_down = true;
        let mut pumps = Vec::new();
        for live in self.sessions.values_mut().filter_map(|e| e.live.as_mut()) {
            if let Err(e) = live.pty.kill() {
                eprintln!("accd: could not kill an agent: {e:#}");
            }
            pumps.extend(live.pump.take());
        }
        pumps
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn is_finished(status: &SessionStatus) -> bool {
    matches!(status, SessionStatus::Done | SessionStatus::Failed | SessionStatus::Stopped)
}

/// Sets the status and opens a new history entry, which closes the previous one.
fn change_status(s: &mut Session, status: SessionStatus, now: i64) {
    s.history.push(StatusChange { status: status.clone(), at: now });
    s.status = status;
    s.status_since = now;
}

fn start_payload(s: &Session, resume: bool) -> Value {
    json!({ "title": s.title, "agent": s.agent, "branch": s.branch, "worktree": s.worktree, "resume": resume })
}

fn rule_json(rule: &AllowRule) -> Value {
    json!({ "tool": rule.tool, "prefix": rule.prefix })
}

/// The task's first line, trimmed, at most 80 characters.
fn session_title(task: &str) -> String {
    let line = task.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    line.chars().take(TITLE_MAX_CHARS).collect::<String>().trim_end().to_string()
}

/// Why Always allow on a request allows it once and adds no rule.
#[derive(Debug, Clone, Copy, PartialEq)]
enum OnceOnly {
    /// It touches a protected path.
    Protected,
    /// A Bash command that runs code given inline: `python3 -c`, `node -e`.
    InlineCode,
    /// A Bash command that runs another command: `env`, `sudo`, `xargs`.
    Wrapper,
    /// A Bash command that starts with a variable assignment: `FOO=1 make`.
    Assignment,
    /// `git push`, a pull or merge request, or a GitHub or GitLab API call.
    Publish,
    /// `git` with `-c` or `--config-env`: an alias or a pager set there runs any command.
    GitConfig,
    /// `git` with an option or subcommand that runs another program:
    /// `rebase -x`, `submodule foreach`, `--upload-pack`, `diff --ext-diff`.
    GitRunsCommand,
    /// `git config` that changes a setting. `core.fsmonitor`, `core.pager`
    /// or an alias set this way makes later git commands run any command.
    GitConfigWrite,
    /// A python module that runs code from its arguments: `python3 -m timeit`.
    CodeModule,
    /// A Bash command with a URL (`://`) in it.
    Url,
    /// A runner (`npx`, `uv run`, `python3 -m`) whose next word is a flag, or
    /// missing, so a rule could not name what it runs.
    PackageRunner,
    /// No rule could match it: its first two words hold a shell operator, an
    /// expansion or a path outside the worktree.
    NotPlain,
    /// A Bash request without a command.
    NoCommand,
    /// A WebFetch request without an http or https origin.
    NoOrigin,
    /// A tool whose input the rules do not know, such as WebSearch or an MCP tool.
    UnknownTool,
    /// A PowerShell command. The command rules are written for Bash.
    PowerShell,
    /// On Windows, a path form that can name another file than its text
    /// says, or one the rules cannot place: a short (8.3) name, a name
    /// ending in a dot or a space, a device, a data stream, a drive letter
    /// in a command, or a `\\?\` or `\\.\` prefix.
    WindowsPath,
}

impl OnceOnly {
    /// The `once_only` reason in the `approval.decided` payload.
    fn reason(self) -> &'static str {
        match self {
            OnceOnly::Protected => "protected path",
            OnceOnly::InlineCode => "runs inline code",
            OnceOnly::Wrapper => "runs another command",
            OnceOnly::Assignment => "sets a variable for the command",
            OnceOnly::Publish => "push, pull request or API call",
            OnceOnly::GitConfig => "git -c or --config-env, which can run any command",
            OnceOnly::GitRunsCommand => "git option that runs another command",
            OnceOnly::GitConfigWrite => "git config change, which can make git run any command",
            OnceOnly::CodeModule => "python module that runs code from its arguments",
            OnceOnly::Url => "contains a URL",
            OnceOnly::PackageRunner => "runner with a flag or nothing to run, so a rule would cover anything it runs",
            OnceOnly::NotPlain => "not a plain command in this worktree",
            OnceOnly::NoCommand => "no command",
            OnceOnly::NoOrigin => "no http or https origin",
            OnceOnly::UnknownTool => "tool with no known input shape",
            OnceOnly::PowerShell => "PowerShell command, which rules do not cover",
            OnceOnly::WindowsPath => "Windows path form that the rules cannot check",
        }
    }
}

/// What a tool's rule is scoped by.
enum Scope {
    /// Bash: the command's first words.
    Command,
    /// The file tools: paths inside the session's worktree.
    Files,
    /// WebFetch: the URL's origin.
    Origin,
}

/// The tools whose input rules know. Every other tool gets no rule.
fn scope(tool: &str) -> Option<Scope> {
    match tool {
        "Bash" => Some(Scope::Command),
        "Edit" | "Write" | "MultiEdit" | "Read" | "NotebookEdit" | "Glob" | "Grep" => Some(Scope::Files),
        "WebFetch" => Some(Scope::Origin),
        _ => None,
    }
}

/// The rule Always allow adds for this request, or why it allows it once
/// only. A Bash rule holds the command's first two words (a runner's rule
/// the runner and what it runs), a WebFetch rule the URL's origin, and a
/// file tool's rule covers files in the worktree.
/// This one check decides which requests never get a rule, and
/// `request_approval` lets no rule allow those.
fn rule_for(tool: &str, summary: &str, input: &Value, worktree: &Path) -> Result<AllowRule, OnceOnly> {
    rule_for_on(tool, summary, input, worktree, PLATFORM)
}

/// [`rule_for`] by the path rules of `platform`.
fn rule_for_on(tool: &str, summary: &str, input: &Value, worktree: &Path, platform: Platform) -> Result<AllowRule, OnceOnly> {
    if request_is_protected(summary, input, worktree, platform) {
        return Err(OnceOnly::Protected);
    }
    if tool == "PowerShell" {
        return Err(OnceOnly::PowerShell);
    }
    let paths = PATH_FIELDS.iter().filter_map(|field| input.get(field).and_then(Value::as_str));
    if paths.into_iter().any(|path| names_another_file(path, worktree, platform)) {
        return Err(OnceOnly::WindowsPath);
    }
    let prefix = match scope(tool).ok_or(OnceOnly::UnknownTool)? {
        Scope::Files => None,
        Scope::Origin => {
            let origin = input.get("url").and_then(Value::as_str).and_then(web_origin);
            Some(origin.ok_or(OnceOnly::NoOrigin)?)
        }
        Scope::Command => {
            let command = input.get("command").and_then(Value::as_str).ok_or(OnceOnly::NoCommand)?;
            Some(command_prefix(command, worktree, platform)?)
        }
    };
    // On Windows any other short name, such as one in a URL, asks each time too.
    if request_has_short_name(summary, input, worktree, platform) {
        return Err(OnceOnly::WindowsPath);
    }
    Ok(AllowRule { tool: tool.to_string(), prefix })
}

/// A Bash rule's prefix: the command's first two words as written (a quoted
/// word stays whole), joined by one space. For a runner it is the runner and
/// what it runs: `npx vitest`, `uv run pytest`, `python3 -m pytest`.
fn command_prefix(command: &str, worktree: &Path, platform: Platform) -> Result<String, OnceOnly> {
    let words = shell_words(command).ok_or(OnceOnly::NotPlain)?;
    if words.is_empty() {
        return Err(OnceOnly::NoCommand);
    }
    let texts: Vec<&str> = words.iter().map(|(_, text)| text.as_str()).collect();
    if let Some(kind) = once_only_command(&texts) {
        return Err(kind);
    }
    // Anywhere in the command, not only in the words the rule keeps.
    if texts.iter().any(|word| is_windows_path_word(word, platform)) {
        return Err(OnceOnly::WindowsPath);
    }
    let prefix = words.iter().take(rule_length(&texts)).map(|(raw, _)| *raw).collect::<Vec<_>>().join(" ");
    match is_plain_command(&prefix, worktree, platform) {
        true => Ok(prefix),
        false => Err(OnceOnly::NotPlain),
    }
}

/// How many of the command's words its Bash rule holds: a runner's words and
/// what it runs, else the first two. What a runner runs is one word (the
/// command, module or package), but another runner adds its own words, and
/// an interpreter its script too: `uv run python -m pytest`, `poetry run
/// python manage.py`. A loop, not recursion, so nesting cannot exhaust the
/// stack.
fn rule_length(words: &[&str]) -> usize {
    let mut length = 0;
    let mut rest = words;
    while let Some(runner) = package_runner(rest) {
        length += runner;
        rest = &rest[runner..];
        if package_runner(rest).is_none() {
            length += match rest.first() {
                Some(first) if is_interpreter(first) => 2,
                _ => 1,
            };
            return length.min(words.len());
        }
    }
    2.min(words.len())
}

/// Commands whose prefix rule would cover more than the user saw: one that
/// starts with an assignment, a wrapper, a git command that sets config for
/// itself, runs another program or changes config, or a runner without
/// something to run next; or one that runs inline code, runs a module with
/// python's flags clustered around `-m`, publishes, runs a python module
/// that runs code from its arguments, or holds a URL. What a runner runs is
/// checked like a command of its own, so `uv run env` is a wrapper.
/// Interpreters, publishing commands and code modules count anywhere in the
/// command, so `uv run python -c`, `git -C . push` and `coverage run -m
/// timeit` are caught. A command of more than [`MAX_ANALYSED_WORDS`] words,
/// or with more than [`MAX_RUNNER_DEPTH`] runners, is not analysed.
fn once_only_command(words: &[&str]) -> Option<OnceOnly> {
    if words.len() > MAX_ANALYSED_WORDS {
        return Some(OnceOnly::NotPlain);
    }
    // The command, then what each runner in front of it runs.
    let mut command = words;
    let mut depth = 0;
    while let Some(first) = command.first() {
        if first.contains('=') {
            return Some(OnceOnly::Assignment);
        }
        // After a runner, `.` is a folder (`go run .`), not the shell's `source`.
        let runs_a_folder = *first == "." && command.len() < words.len();
        if WRAPPERS.iter().chain(PLATFORM_WRAPPERS).any(|wrapper| is_program(first, wrapper)) && !runs_a_folder {
            return Some(OnceOnly::Wrapper);
        }
        if is_program(first, "git") {
            if let Some(kind) = once_only_git(&command[1..]) {
                return Some(kind);
            }
        }
        if runner_after_flag(command) {
            return Some(OnceOnly::PackageRunner);
        }
        let Some(runner) = package_runner(command) else { break };
        depth += 1;
        if depth > MAX_RUNNER_DEPTH {
            return Some(OnceOnly::NotPlain);
        }
        let runs_a_command = COMMAND_RUNNERS.iter().any(|program| is_program(first, program))
            && command[runner..].iter().any(|arg| *arg == "--" || is_option(arg, "--command") || short_flag_letters(arg).contains('c'));
        match command.get(runner) {
            Some(target) if !target.starts_with('-') && !runs_a_command => command = &command[runner..],
            _ => return Some(OnceOnly::PackageRunner),
        }
    }
    if runs_code_module(words) {
        return Some(OnceOnly::CodeModule);
    }
    for (i, word) in words.iter().enumerate() {
        let rest = &words[i + 1..];
        if is_interpreter(word) && rest.iter().any(|arg| is_inline_code_arg(arg)) {
            return Some(OnceOnly::InlineCode);
        }
        // `-m` in a cluster of short flags runs a module: `python3 -Im timeit`,
        // `coverage run -Lm timeit`.
        let runs_modules = is_python(program_name(word)) || is_program(word, "coverage");
        if runs_modules && rest.iter().any(|arg| *arg != "-m" && short_flag_letters(arg).contains('m')) {
            return Some(OnceOnly::PackageRunner);
        }
        if PUBLISHING.iter().any(|(program, sub)| is_program(word, program) && rest.iter().any(|arg| arg.eq_ignore_ascii_case(sub))) {
            return Some(OnceOnly::Publish);
        }
    }
    // A URL is never inside the worktree, so a rule's preview would promise
    // what its match refuses. A `file://` URL into it would pass as a path.
    words.iter().any(|word| word.contains("://")).then_some(OnceOnly::Url)
}

/// A command's name without its folder: `/usr/bin/env` is `env`. On Windows
/// also without the extension that runs it: `git.exe` is `git`.
fn program_name(word: &str) -> &str {
    let name = word.rsplit(path_separators(PLATFORM)).next().unwrap_or(word);
    let ends_with = |ext: &&str| name.len() > ext.len() && name.get(name.len() - ext.len()..).is_some_and(|end| end.eq_ignore_ascii_case(ext));
    match [".exe", ".com", ".cmd", ".bat"].into_iter().find(ends_with) {
        Some(ext) if cfg!(windows) => &name[..name.len() - ext.len()],
        _ => name,
    }
}

/// Whether `arg` is `option`, alone or with a value after `=`. A long option
/// counts abbreviated too, down to one letter after the dashes: git,
/// getopt_long and argparse take any unambiguous prefix, so `git rebase
/// --ex` is `--exec`. Options keep their case: `-x` is not `-X`.
fn is_option(arg: &str, option: &str) -> bool {
    let name = arg.split('=').next().unwrap_or(arg);
    match option.starts_with("--") {
        true => name.len() > 2 && name.starts_with("--") && option.starts_with(name),
        false => name == option,
    }
}

/// Why a `git` command, given its arguments, gets no rule: it sets config
/// for itself (one of the [`GIT_CONFIG_ARGS`], or `clone` with `-c` in any
/// form), runs another program, or is a `git config` that changes a setting.
fn once_only_git(args: &[&str]) -> Option<OnceOnly> {
    // `git clone` reads `-c` stuck to its value and in a cluster too:
    // `-ccore.sshCommand=<command>`, `-qc`.
    let clone = args.iter().any(|arg| arg.eq_ignore_ascii_case("clone"));
    let clone_config = clone && args.iter().any(|arg| short_flag_letters(arg).contains('c'));
    if clone_config || args.iter().any(|arg| GIT_CONFIG_ARGS.iter().any(|option| is_option(arg, option))) {
        return Some(OnceOnly::GitConfig);
    }
    if git_runs_another_program(args) {
        return Some(OnceOnly::GitRunsCommand);
    }
    let sub = git_subcommand(args)?;
    (args[sub].eq_ignore_ascii_case("config") && !git_config_reads(&args[sub + 1..])).then_some(OnceOnly::GitConfigWrite)
}

/// Whether a `git` command, given its arguments, runs another program: one
/// of the [`GIT_COMMAND_ARGS`] (abbreviated too) or
/// [`GIT_COMMAND_SUBCOMMANDS`], a short `-x` (`rebase`, `difftool`) or `-O`
/// (`grep`), or `-u` with one of the [`GIT_UPLOAD_PACK_SUBCOMMANDS`]. They
/// count anywhere in the command, subcommands in any case (git runs a
/// script subcommand such as `FILTER-BRANCH` from a case-insensitive disk),
/// and short flags in a cluster too, so other subcommands' flags of the same
/// letter ask as well: `git clean -fdx`, `git cherry-pick -x`.
fn git_runs_another_program(args: &[&str]) -> bool {
    let has_word = |word: &&str| args.iter().any(|arg| arg.eq_ignore_ascii_case(word));
    let has_short_flag = |letter: char| args.iter().any(|arg| short_flag_letters(arg).contains(letter));
    args.iter().any(|arg| GIT_COMMAND_ARGS.iter().any(|option| is_option(arg, option)))
        || GIT_COMMAND_SUBCOMMANDS.iter().any(|words| words.iter().all(has_word))
        || has_short_flag('x')
        || has_short_flag('O')
        || (has_short_flag('u') && GIT_UPLOAD_PACK_SUBCOMMANDS.iter().any(has_word))
}

/// Where a git command's subcommand is among its arguments: the first word
/// that is neither a global option nor the value of one.
fn git_subcommand(args: &[&str]) -> Option<usize> {
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        if !arg.starts_with('-') {
            return Some(i);
        }
        i += if GIT_GLOBAL_VALUE_ARGS.contains(arg) { 2 } else { 1 };
    }
    None
}

/// Whether `git config <args>` only reads. Every option must be one of the
/// [`GIT_CONFIG_READ_ARGS`]. Then it reads with one of the
/// [`GIT_CONFIG_READ_ACTIONS`], with a single key and no value (`git config
/// user.email`; a key has a dot, so `git config edit` is not one), or as
/// `git config get <key>` or `git config list`.
fn git_config_reads(args: &[&str]) -> bool {
    let (options, words): (Vec<&str>, Vec<&str>) = args.iter().copied().partition(|arg| arg.starts_with('-'));
    if !options.iter().all(|option| GIT_CONFIG_READ_ARGS.contains(option)) {
        return false;
    }
    if options.iter().any(|option| GIT_CONFIG_READ_ACTIONS.contains(option)) {
        return true;
    }
    match words.as_slice() {
        [word] => word.contains('.') || word.eq_ignore_ascii_case("list"),
        [sub, key] => sub.eq_ignore_ascii_case("get") && key.contains('.'),
        _ => false,
    }
}

/// The letters of a cluster of short flags, up to the first other character:
/// `lc` for `-lc`, `c` for `-cprint(1)`. Empty for a long option, or for a
/// word that is not a flag.
fn short_flag_letters(arg: &str) -> &str {
    let Some(rest) = arg.strip_prefix('-') else { return "" };
    &rest[..rest.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(rest.len())]
}

/// How many words of one of the [`PACKAGE_RUNNERS`] the command starts with.
fn package_runner(words: &[&str]) -> Option<usize> {
    let (first, rest) = words.split_first()?;
    let runner = PACKAGE_RUNNERS.iter().find(|runner| is_program(first, runner[0]) && starts_with_words(rest, &runner[1..]))?;
    Some(runner.len())
}

/// Whether `words` starts with `expected`, in any case.
fn starts_with_words(words: &[&str], expected: &[&str]) -> bool {
    words.len() >= expected.len() && words.iter().zip(expected).all(|(word, expected)| word.eq_ignore_ascii_case(expected))
}

/// Whether a flag comes between a runner's words: `npm --prefix web exec`,
/// `pnpm -C web dlx`, `python3 -u -m`, `coverage run -L -m`. The rule would
/// end at the flag, which names nothing it runs. The runner's other words
/// count anywhere after the flag, since a flag may take a value.
fn runner_after_flag(words: &[&str]) -> bool {
    let Some(first) = words.first() else { return false };
    package_runner(words).is_none()
        && PACKAGE_RUNNERS.iter().any(|runner| {
            (1..runner.len()).any(|split| {
                words.len() > split
                    && is_program(first, runner[0])
                    && starts_with_words(&words[1..], &runner[1..split])
                    && words[split].starts_with('-')
                    && words[split + 1..].windows(runner.len() - split).any(|window| starts_with_words(window, &runner[split..]))
            })
        })
}

/// Whether `word` runs `program`, from any folder and in any case: macOS
/// file systems ignore case, so `GIT` runs git. `python` stands for every
/// python and pypy, such as `python3.12` and `pypy3`.
fn is_program(word: &str, program: &str) -> bool {
    let name = program_name(word);
    name.eq_ignore_ascii_case(program) || (program == "python" && is_python(name))
}

/// Whether a Bash rule's prefix may cover this command. The command must be
/// one that could get a rule of its own, so a rule saved earlier never
/// covers a once-only command (`git --no-pager -c alias.p=push p`). For a
/// runner the prefix must also hold the words of that rule, so it names what
/// the command runs, as every rule this daemon saves does. A rule for the
/// runner alone (`npx -y`, `pnpm dlx`, `python3 -m`, `uv run`, saved by an
/// older daemon) would cover anything it runs.
fn prefix_may_cover(command: &str, prefix: &str) -> bool {
    let Some(words) = shell_words(command) else { return false };
    let texts: Vec<&str> = words.iter().map(|(_, text)| text.as_str()).collect();
    if once_only_command(&texts).is_some() {
        return false;
    }
    package_runner(&texts).is_none()
        || shell_words(prefix).is_some_and(|prefix_words| prefix_words.len() >= rule_length(&texts))
}

/// Whether `word` runs one of the [`INTERPRETERS`] or a python, from any
/// folder and in any case.
fn is_interpreter(word: &str) -> bool {
    INTERPRETERS.iter().any(|interpreter| is_program(word, interpreter)) || is_python(program_name(word))
}

/// `python` or `pypy`, in any case, plain or a version or build of it:
/// `python3`, `python3.12`, `python3.13t`, `python3.12-dbg`, `pypy3`. On
/// Windows also `py`, the Python launcher.
fn is_python(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    if cfg!(windows) && name == "py" {
        return true;
    }
    ["python", "pypy"].iter().any(|base| {
        name.strip_prefix(base).is_some_and(|rest| rest.chars().all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c)))
    })
}

/// One of the [`INLINE_CODE_ARGS`] (a long one abbreviated too, `fish
/// --comm`), also with `=` (`--eval=...`), or short flags holding one of the
/// [`INLINE_CODE_LETTERS`] before anything else: a cluster (`-lc`, `-lE`) or
/// code attached to its flag (`-cprint(1)`, which is how the shell passes
/// `-c'print(1)'`).
fn is_inline_code_arg(arg: &str) -> bool {
    INLINE_CODE_ARGS.iter().any(|code_arg| is_option(arg, code_arg)) || short_flag_letters(arg).contains(INLINE_CODE_LETTERS)
}

/// Whether the command runs one of the [`CODE_MODULES`] as a module
/// anywhere, also through another module runner: `python3 -m timeit`,
/// `coverage run -m timeit`, `--module=pdb`, `-mtimeit`.
fn runs_code_module(words: &[&str]) -> bool {
    words.iter().enumerate().any(|(i, word)| {
        let module = if *word == "-m" || (is_option(word, "--module") && !word.contains('=')) {
            words.get(i + 1).copied()
        } else if is_option(word, "--module") {
            word.split_once('=').map(|(_, module)| module)
        } else {
            word.strip_prefix("-m").filter(|module| !module.is_empty())
        };
        module.is_some_and(|module| CODE_MODULES.iter().any(|code| module.eq_ignore_ascii_case(code)))
    })
}

/// What a rule covers, in words, for an approval's `rule_preview`.
fn rule_preview(rule: &AllowRule) -> String {
    match (scope(&rule.tool), rule.prefix.as_deref()) {
        (Some(Scope::Command), Some(prefix)) => format!("`{prefix}` commands"),
        (Some(Scope::Origin), Some(origin)) => format!("{} on {origin}", rule.tool),
        _ => format!("{} on files in this worktree", rule.tool),
    }
}

/// Whether `rule` covers this request without widening what the user approved.
fn rule_allows(rule: &AllowRule, tool: &str, input: &Value, worktree: &Path) -> bool {
    rule_allows_on(rule, tool, input, worktree, PLATFORM)
}

/// [`rule_allows`] by the path rules of `platform`.
fn rule_allows_on(rule: &AllowRule, tool: &str, input: &Value, worktree: &Path, platform: Platform) -> bool {
    if rule.tool != tool {
        return false;
    }
    match (scope(tool), rule.prefix.as_deref()) {
        (Some(Scope::Command), Some(prefix)) => {
            let Some(command) = input.get("command").and_then(Value::as_str) else { return false };
            let whole_words = command.strip_prefix(prefix).is_some_and(|rest| rest.is_empty() || rest.starts_with(' '));
            whole_words && prefix_may_cover(command, prefix) && is_plain_command(command, worktree, platform)
        }
        (Some(Scope::Origin), Some(origin)) => {
            input.get("url").and_then(Value::as_str).and_then(web_origin).is_some_and(|url| url == origin)
        }
        // Every path in the input must be inside the session's worktree.
        (Some(Scope::Files), None) => PATH_FIELDS.iter().all(|field| match input.get(field) {
            None => true,
            Some(Value::String(path)) => is_inside(Path::new(path), worktree, platform),
            Some(_) => false,
        }),
        // A Bash or WebFetch rule without a prefix would allow anything, and
        // a tool with an unknown input gets no rule at all.
        _ => false,
    }
}

/// The origin of an http or https URL: `scheme://host`, plus `:port` when it
/// is not the default, lowercased. `None` for another scheme, for user info,
/// and for a host with anything but ASCII letters, digits, dots and dashes
/// (or a bracketed IPv6 address), which a browser might read differently.
fn web_origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "http" => 80,
        "https" => 443,
        _ => return None,
    };
    // The host ends where a browser ends it, at a backslash too.
    let authority = rest[..rest.find(['/', '\\', '?', '#']).unwrap_or(rest.len())].to_ascii_lowercase();
    let (host, port) = if authority.starts_with('[') {
        let close = authority.find(']')?;
        let address = &authority[1..close];
        if address.is_empty() || !address.chars().all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.') {
            return None;
        }
        authority.split_at(close + 1)
    } else {
        let (host, port) = authority.split_at(authority.find(':').unwrap_or(authority.len()));
        if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
            return None;
        }
        (host, port)
    };
    let port = match port.strip_prefix(':') {
        None if port.is_empty() => default_port,
        Some("") => default_port,
        Some(digits) if digits.chars().all(|c| c.is_ascii_digit()) => digits.parse::<u16>().ok()?,
        _ => return None,
    };
    Some(match port == default_port {
        true => format!("{scheme}://{host}"),
        false => format!("{scheme}://{host}:{port}"),
    })
}

/// Lexically below `dir`: absolute, without `..`, not `dir` itself, and on
/// Windows with no path form below `dir` that can name another file.
fn is_inside(path: &Path, dir: &Path, platform: Platform) -> bool {
    path.is_absolute()
        && !path.components().any(|c| c == Component::ParentDir)
        && path.starts_with(dir)
        && path != dir
        && !names_another_file(&path.to_string_lossy(), dir, platform)
}

/// Whether a rule may cover this Bash command: no shell operators and no
/// expansions, no `..` step, every absolute path inside the worktree, and on
/// Windows no word that [`is_windows_path_word`]. Words also split on `=`,
/// `,` and `:`, so `--output=/etc/x` and `host:/etc/x` are checked, and so
/// is a path attached to a flag.
fn is_plain_command(command: &str, worktree: &Path, platform: Platform) -> bool {
    if SHELL_OPERATORS.iter().any(|op| command.contains(op)) || command.contains(EXPANSION_CHARS) {
        return false;
    }
    let Some(words) = shell_words(command) else { return false };
    if words.iter().any(|(_, text)| is_windows_path_word(text, platform)) {
        return false;
    }
    words.iter().flat_map(|(_, text)| text.split(['=', ',', ':'])).all(|part| stays_in_worktree(part, worktree, platform))
}

/// Whether a part of a word names nothing outside the worktree: no `..`
/// step, and any absolute path in it inside. A URL's `//host` is outside.
fn stays_in_worktree(part: &str, worktree: &Path, platform: Platform) -> bool {
    let climbs = part.contains("../") || part.ends_with("..");
    !climbs && absolute_path(part).is_none_or(|path| is_inside(Path::new(path), worktree, platform))
}

/// Whether, on Windows, `path` can name another file than its text says,
/// or one the rules cannot place: it has a `\\?\` or `\\.\` prefix, a `:`
/// other than after a leading drive letter (an alternate data stream such
/// as `a.rs::$DATA`), a drive letter with no folder after it (`D:notes` is
/// relative to drive D's current folder), or a name that
/// [`is_windows_alias_name`]. Either slash separates folders. The worktree's
/// own path is the daemon's, so only the names below it are checked when
/// `path` is in it. Never on Unix.
fn names_another_file(path: &str, worktree: &Path, platform: Platform) -> bool {
    if platform == Platform::Unix {
        return false;
    }
    let unified = path.replace('\\', "/");
    if ["//?/", "//./", "/??/"].iter().any(|prefix| unified.starts_with(prefix)) {
        return true;
    }
    let rest = match unified.as_bytes() {
        [letter, b':', b'/', ..] if letter.is_ascii_alphabetic() => &unified[2..],
        [letter, b':', ..] if letter.is_ascii_alphabetic() => return true,
        _ => unified.as_str(),
    };
    if rest.contains(':') {
        return true;
    }
    let below = Path::new(path).strip_prefix(worktree).ok().map(|below| below.to_string_lossy().replace('\\', "/"));
    below.as_deref().unwrap_or(rest).split('/').any(is_windows_alias_name)
}

/// Whether Windows reads a folder or file name as another one, or as a
/// device: a short (8.3) name, which has a `~`; a name that ends in a dot or
/// a space, which Windows drops (`.claude.` is `.claude`); or a device such
/// as `NUL`, `CON` or `COM1`, also with an extension (`nul.txt`). `.` and
/// `..` are not names.
fn is_windows_alias_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }
    let base = name.split('.').next().unwrap_or(name).trim_end_matches(' ').to_ascii_uppercase();
    let numbered = |port: &str| {
        base.strip_prefix(port).is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"))
    };
    let device = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$") || numbered("COM") || numbered("LPT");
    name.contains('~') || name.ends_with('.') || name.ends_with(' ') || device
}

/// Whether a word of a Bash command is, on Windows, a path the rules cannot
/// check: one with a `:` (a drive letter, as in `D:notes` or `C:/x`, or a
/// data stream), a `//?/` or `//./` prefix, or a name that
/// [`is_windows_alias_name`]. Never on Unix.
fn is_windows_path_word(word: &str, platform: Platform) -> bool {
    platform == Platform::Windows
        && (word.contains(':')
            || ["//?/", "//./"].iter().any(|prefix| word.starts_with(prefix))
            || word.split(['/', '\\', '=', ',']).any(is_windows_alias_name))
}

/// The absolute path in a part of a word: all of it when it starts with
/// `/`, else the text from its first `/`, unless what comes before is a
/// plain name. So `-C/Users/x` and `@/etc/x` hold one, and `src/a.rs` does not.
fn absolute_path(part: &str) -> Option<&str> {
    let slash = part.find('/')?;
    (!is_plain_name(&part[..slash])).then_some(&part[slash..])
}

/// A relative path's first step: `.`, or a name that is not a flag, has a
/// letter or digit, and holds only those, spaces and `._@+-`.
fn is_plain_name(name: &str) -> bool {
    name == "."
        || (!name.starts_with('-')
            && name.chars().any(char::is_alphanumeric)
            && name.chars().all(|c| c.is_alphanumeric() || " ._@+-".contains(c)))
}

/// Splits a command into words as the shell does when it has no escapes or
/// expansions: whitespace separates words, and quotes group. Each word comes
/// as written and as the program gets it, without the quotes. `None` for an
/// unterminated quote.
fn shell_words(command: &str) -> Option<Vec<(&str, String)>> {
    let mut words = Vec::new();
    let mut text = String::new();
    let mut start = None;
    let mut quote = None;
    for (i, c) in command.char_indices() {
        match quote {
            Some(open) if c == open => quote = None,
            Some(_) => text.push(c),
            None if c.is_whitespace() => {
                if let Some(start) = start.take() {
                    words.push((&command[start..i], std::mem::take(&mut text)));
                }
            }
            None => {
                start.get_or_insert(i);
                match c {
                    '"' | '\'' => quote = Some(c),
                    _ => text.push(c),
                }
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if let Some(start) = start {
        words.push((&command[start..], text));
    }
    Some(words)
}

/// Whether `summary` names a protected path: a file whose name starts with
/// `.env` or ends with `.pem` or `.key`, a path to or through one of the
/// [`PROTECTED_NAMES`], or one through `.config/gh` or `.config/gcloud`.
/// Case-insensitive. On Windows a backslash separates folders too, a name
/// counts without the dots and spaces that Windows drops from its end
/// (`.claude.` is `.claude`), and a short (8.3) name that can stand for a
/// protected name ([`is_protected_short_name`]) counts too, unless it is in
/// the worktree's own path. The UI flags these paths too, but this check is
/// the one that is enforced, so it must not be looser.
fn touches_protected_path(summary: &str, worktree: &Path, platform: Platform) -> bool {
    let lower = summary.to_lowercase();
    let names = spellings(&lower, platform).iter().any(|text| path_tokens(text).any(|token| is_protected_token(token, platform)));
    names
        || (platform == Platform::Windows
            && spellings(&without_worktree(&lower, worktree), platform)
                .iter()
                .any(|text| path_tokens(text).any(|token| has_protected_short_name(token, platform))))
}

/// The words of a text that are checked for protected paths: the text split
/// at whitespace and at the [`PATH_TOKEN_SEPARATORS`].
fn path_tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| c.is_whitespace() || PATH_TOKEN_SEPARATORS.contains(&c))
}

/// The texts a summary is checked as. Quotes and escapes can split a name
/// (`.s"s"h`, `.s\sh`), so it is checked with them removed too. On Windows
/// a backslash is also a folder separator, so there it is checked with only
/// the quotes removed as well.
fn spellings(text: &str, platform: Platform) -> Vec<String> {
    let mut texts = vec![text.to_string(), text.replace(['"', '\'', '\\'], "")];
    if platform == Platform::Windows {
        texts.push(text.replace(['"', '\''], ""));
    }
    texts
}

fn is_protected_token(token: &str, platform: Platform) -> bool {
    let parts: Vec<&str> = token.split(path_separators(platform)).map(|part| opened_name(part, platform)).collect();
    let name = parts.iter().rev().find(|part| !part.is_empty()).copied().unwrap_or_default();
    name.starts_with(PROTECTED_PREFIX)
        || PROTECTED_EXTENSIONS.iter().any(|extension| name.ends_with(extension))
        || parts.iter().any(|part| PROTECTED_NAMES.contains(part))
        || parts.windows(2).any(|pair| pair[0] == CONFIG_DIR && CREDENTIAL_CONFIG_DIRS.contains(&pair[1]))
}

/// Whether a token holds, on Windows, a short (8.3) name that can stand for
/// a protected name. Unix has no short names.
fn has_protected_short_name(token: &str, platform: Platform) -> bool {
    platform == Platform::Windows
        && token.split(path_separators(platform)).any(|part| is_protected_short_name(opened_name(part, platform)))
}

/// Whether `name`, a lowercase folder or file name, is a short (8.3) name
/// that can stand for a protected name. Windows makes a short name from the
/// start of the long one, and after four names with the same start it keeps
/// only the first two characters and adds a hash (`CL3F2A~1` for `.claude`).
/// So a short name counts when its first two characters are those of a
/// protected name's short base ([`protected_short_starts`]), or when its
/// extension is that of a protected file (`SERVER~1.PEM`, `SETTIN~1.JSO`).
/// Any other one, such as the git revision `HEAD~1`, does not.
fn is_protected_short_name(name: &str) -> bool {
    let Some(tilde) = short_name_tilde(name) else { return false };
    let after_number = name[tilde + 1..].trim_start_matches(|c: char| c.is_ascii_digit());
    let extension: String = after_number.strip_prefix('.').unwrap_or_default().chars().take(3).collect();
    protected_short_starts().iter().any(|start| name.starts_with(start.as_str()))
        || (!extension.is_empty() && protected_short_extensions().contains(&extension))
}

/// The short (8.3) form Windows makes of a long name, lowercase and without
/// its `~1`: a base and an extension. The base is the name without its
/// leading dots and its extension, and without other dots and spaces, cut
/// to 6 characters. The extension is what follows the last dot that does
/// not lead the name, cut to 3. `.claude` is `claude`, `.mcp.json` is `mcp`
/// with the extension `jso`.
fn short_form(name: &str) -> (String, String) {
    let name = name.trim_start_matches('.');
    let (stem, extension) = name.rsplit_once('.').unwrap_or((name, ""));
    let kept = |text: &str, length| text.chars().filter(|c| !matches!(c, '.' | ' ')).take(length).collect();
    (kept(stem, 6), kept(extension, 3))
}

/// The first two characters of the short base of each protected name: the
/// [`PROTECTED_NAMES`], the [`PROTECTED_PREFIX`] and the [`CONFIG_DIR`].
/// The folders inside `.config` are left out: a path to one goes through
/// `.config`, and `gh` and `gcloud` are short names already, so Windows
/// makes no other.
fn protected_short_starts() -> Vec<String> {
    PROTECTED_NAMES
        .iter()
        .chain([&PROTECTED_PREFIX, &CONFIG_DIR])
        .map(|name| short_form(name).0.chars().take(2).collect::<String>())
        .filter(|start| !start.is_empty())
        .collect()
}

/// The extensions a short name of a protected file can have: those of the
/// [`PROTECTED_NAMES`] that have one (`.mcp.json`), and the
/// [`PROTECTED_EXTENSIONS`], each cut to 3 characters.
fn protected_short_extensions() -> Vec<String> {
    let of_names = PROTECTED_NAMES.iter().map(|name| short_form(name).1);
    let extensions = PROTECTED_EXTENSIONS.iter().map(|extension| extension.trim_start_matches('.').chars().take(3).collect());
    of_names.chain(extensions).filter(|extension| !extension.is_empty()).collect()
}

/// The name that a part of a path opens. On Windows that is the part without
/// the dots and spaces at its end, which Windows drops. `.` and `..` stay.
fn opened_name(part: &str, platform: Platform) -> &str {
    match platform {
        Platform::Windows if part != "." && part != ".." => part.trim_end_matches(['.', ' ']),
        _ => part,
    }
}

/// Whether `text` holds a short (8.3) name: a `~` followed by a digit, as in
/// `CLAUDE~1` or `SETTIN~1.JSO`. A `~` before anything else, as in `~/x`, is
/// not one.
fn has_short_name(text: &str) -> bool {
    short_name_tilde(text).is_some()
}

/// Where the `~` of the first short (8.3) name in `text` is: the first `~`
/// followed by a digit.
fn short_name_tilde(text: &str) -> Option<usize> {
    text.as_bytes().windows(2).position(|pair| pair[0] == b'~' && pair[1].is_ascii_digit())
}

/// `text`, which is lowercase, without the worktree's own path. That path is
/// the daemon's, so a short name in it (a temp folder such as
/// `C:\Users\RUNNER~1\...`) is not one the agent chose. The path is taken
/// out where it is written whole, with either slash, and a folder or the end
/// of a word comes next.
fn without_worktree(text: &str, worktree: &Path) -> String {
    let worktree = worktree.to_string_lossy().to_lowercase();
    let worktree = worktree.trim_end_matches(['/', '\\']);
    if worktree.is_empty() {
        return text.to_string();
    }
    [worktree.to_string(), worktree.replace('\\', "/"), worktree.replace('/', "\\")]
        .iter()
        .fold(text.to_string(), |text, spelling| without_whole_path(&text, spelling))
}

/// `text` without each `path` in it that a slash, a backslash, whitespace,
/// one of the [`PATH_TOKEN_SEPARATORS`] or the end of the text follows.
fn without_whole_path(text: &str, path: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(path) {
        let after = &rest[at + path.len()..];
        let whole = after
            .chars()
            .next()
            .is_none_or(|c| c == '/' || c == '\\' || c.is_whitespace() || PATH_TOKEN_SEPARATORS.contains(&c));
        kept.push_str(&rest[..at]);
        if !whole {
            kept.push_str(path);
        }
        rest = after;
    }
    kept.push_str(rest);
    kept
}

/// Whether a request touches a protected path, in its summary or in a path
/// field of its input. Such a request is never allowed by a rule.
fn request_is_protected(summary: &str, input: &Value, worktree: &Path, platform: Platform) -> bool {
    request_texts(summary, input).any(|text| touches_protected_path(text, worktree, platform))
}

/// Whether, on Windows, a request holds a short (8.3) name outside the
/// worktree's own path, in its summary or in a path field of its input.
/// One that is not a protected name can still name another file than its
/// text says, so such a request gets no rule either.
fn request_has_short_name(summary: &str, input: &Value, worktree: &Path, platform: Platform) -> bool {
    platform == Platform::Windows
        && request_texts(summary, input).any(|text| {
            spellings(&without_worktree(&text.to_lowercase(), worktree), platform).iter().any(|text| has_short_name(text))
        })
}

/// A request's summary and the path fields of its input.
fn request_texts<'a>(summary: &'a str, input: &'a Value) -> impl Iterator<Item = &'a str> {
    let paths = PATH_FIELDS.iter().filter_map(|field| input.get(field).and_then(Value::as_str));
    std::iter::once(summary).chain(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bash_rule(prefix: &str) -> AllowRule {
        AllowRule { tool: "Bash".into(), prefix: Some(prefix.into()) }
    }

    fn allows_command(rule: &AllowRule, command: &str) -> bool {
        rule_allows(rule, "Bash", &json!({ "command": command }), Path::new("/w"))
    }

    fn allows_file(path: Value) -> bool {
        let rule = AllowRule { tool: "Edit".into(), prefix: None };
        rule_allows(&rule, "Edit", &json!({ "file_path": path }), Path::new(W))
    }

    /// The session's worktree in these tests.
    #[cfg(unix)]
    const W: &str = "/repo/.worktrees/fix";
    #[cfg(windows)]
    const W: &str = r"C:\repo\.worktrees\fix";

    /// The rule Always allow would add for a request in worktree `W`.
    fn rule_of(tool: &str, input: Value) -> Result<AllowRule, OnceOnly> {
        let summary = claude::summarize_tool(tool, &input);
        rule_for(tool, &summary, &input, Path::new(W))
    }

    fn command_rule(command: &str) -> Result<AllowRule, OnceOnly> {
        rule_of("Bash", json!({ "command": command }))
    }

    /// Whether a Bash rule for `prefix` covers `command` in worktree `W`.
    fn rule_covers(prefix: &str, command: &str) -> bool {
        rule_allows(&bash_rule(prefix), "Bash", &json!({ "command": command }), Path::new(W))
    }

    /// The rule for a command with a word Windows can read as another path,
    /// such as one with a `:` (a drive or a stream) or a name ending in a
    /// dot: `rule` on macOS and Linux, none on Windows.
    fn unix_only_rule(rule: AllowRule) -> Result<AllowRule, OnceOnly> {
        match cfg!(windows) {
            true => Err(OnceOnly::WindowsPath),
            false => Ok(rule),
        }
    }

    #[test]
    fn bash_rules_use_the_first_two_words() {
        assert_eq!(command_rule("npm  install x"), Ok(bash_rule("npm install")));
        assert_eq!(command_rule("ls"), Ok(bash_rule("ls")));
        // A quoted word stays whole.
        assert_eq!(command_rule("cat \"my file.txt\" -n"), Ok(bash_rule("cat \"my file.txt\"")));
        assert_eq!(command_rule("  "), Err(OnceOnly::NoCommand));
        assert_eq!(rule_of("Bash", json!({})), Err(OnceOnly::NoCommand));
        assert_eq!(rule_of("Bash", json!({"command": 5})), Err(OnceOnly::NoCommand));
        assert_eq!(rule_of("Edit", json!({"file_path": "/a"})), Ok(AllowRule { tool: "Edit".into(), prefix: None }));
    }

    #[test]
    fn inline_code_gets_no_rule() {
        for command in [
            "python3 -c \"print(1)\"",
            "python -c 'import os'",
            "/usr/bin/python3.12 -c x",
            "node -e \"require('fs')\"",
            "node -p 1+1",
            "node -r ./hook.js app.js",
            "node --eval=1",
            "deno eval \"Deno.exit()\"",
            "bun -e 1",
            "bash -c \"rm -rf build\"",
            "bash -lc make",
            "sh -c make",
            "zsh -c make",
            "fish -c make",
            "perl -pe s/a/b/ x.txt",
            "ruby -e 'puts 1'",
            "php -r 'echo 1;'",
            "osascript -e beep",
            // Through a runner too.
            "uv run python -c x",
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::InlineCode), "{command:?}");
        }
        // A script or a module is a normal command, and a module's rule names it.
        assert_eq!(command_rule("python3 scripts/build.py --fast"), Ok(bash_rule("python3 scripts/build.py")));
        assert_eq!(command_rule("python3 -m pytest"), Ok(bash_rule("python3 -m pytest")));
        assert_eq!(command_rule("bash scripts/test.sh"), Ok(bash_rule("bash scripts/test.sh")));
    }

    #[test]
    fn inline_code_attached_to_its_flag_gets_no_rule() {
        for command in [
            // The shell passes `-c'code'` as one word, `-ccode`.
            "python3 -u -c'import os'",
            "python3 -c\"print(1)\"",
            "python3 -uc'print(1)'",
            "python3 -c1",
            "perl -w -e'print 1'",
            "ruby -e\"puts 1\"",
            "bash -c'make'",
            // Modules that node, bun and deno load before the script.
            "node --import ./hook.mjs app.js",
            "node --import=./hook.mjs app.js",
            "node --loader ./hook.mjs app.js",
            "node --experimental-loader=./hook.mjs app.js",
            "node --require ./hook.js app.js",
            "node -r ./hook.js app.js",
            "node --eval 1",
            "bun --preload ./hook.ts app.ts",
            "bun -r ./hook.ts app.ts",
            "deno eval 1",
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::InlineCode), "{command:?}");
        }
        // Flags that give no code keep their rules.
        for (command, prefix) in [
            ("python3 -u scripts/build.py", "python3 -u"),
            ("perl -w scripts/x.pl", "perl -w"),
            ("node --version", "node --version"),
            ("node app.js --port 3000", "node app.js"),
        ] {
            assert_eq!(command_rule(command), Ok(bash_rule(prefix)), "{command:?}");
        }
        // A rule saved for the program and a harmless flag does not cover them.
        assert!(!rule_covers("python3 -u", "python3 -u -c'import os'"));
        assert!(!rule_covers("node app.js", "node app.js --import ./hook.mjs"));
    }

    #[test]
    fn git_config_options_get_no_rule() {
        for command in [
            // An alias set for one command runs a push, or any command.
            "git --no-pager -c alias.p=push p origin main",
            "git -c 'alias.x=!rm -rf src' x",
            "git -c core.pager=less log",
            "/usr/bin/git -c core.editor=vim commit",
            "git --config-env=core.pager=PAGER log",
            "git --config-env core.pager=PAGER log",
            // Anywhere in the command.
            "git log -c",
            "git --no-pager log --config-env=core.pager=PAGER",
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::GitConfig), "{command:?}");
        }
        assert_eq!(OnceOnly::GitConfig.reason(), "git -c or --config-env, which can run any command");
        // No git rule covers them, so one saved earlier cannot either.
        for (prefix, command) in [
            ("git --no-pager", "git --no-pager -c alias.p=push p origin main"),
            ("git --no-pager", "git --no-pager -c 'alias.x=!rm -rf src' x"),
            ("git log", "git log -c"),
        ] {
            assert!(!rule_covers(prefix, command), "{prefix:?} covers {command:?}");
        }
        // Other git commands keep their rules.
        for (command, prefix) in [
            ("git --no-pager log -n 5", "git --no-pager"),
            ("git commit -m 'fix -c handling'", "git commit"),
            ("git checkout -b fix-config", "git checkout"),
        ] {
            assert_eq!(command_rule(command), Ok(bash_rule(prefix)), "{command:?}");
        }
        assert!(rule_covers("git --no-pager", "git --no-pager log -n 5"));
    }

    #[test]
    fn git_options_that_run_another_program_get_no_rule() {
        // Each asks, also under the rule its prefix would match.
        for (prefix, command) in [
            ("git rebase", "git rebase -x 'make test' main"),
            ("git rebase", "git rebase --exec 'make test' main"),
            ("git rebase", "git rebase --exec=make main"),
            ("git rebase", "git rebase -ix make main"),
            ("git difftool", "git difftool -x vimdiff main"),
            ("git difftool", "git difftool --extcmd=vimdiff main"),
            ("git submodule", "git submodule foreach make"),
            ("git submodule", "git submodule --quiet foreach --recursive make"),
            ("git bisect", "git bisect run make test"),
            ("git pull", "git pull --upload-pack='touch pwned' ."),
            ("git fetch", "git fetch --upload-pack=touch ."),
            ("git send-pack", "git send-pack --receive-pack=touch . main"),
            ("git clone", "git clone -u 'touch pwned' . copy"),
            ("git fetch", "git fetch -u ."),
            ("git ls-remote", "git ls-remote -u touch ."),
            ("git grep", "git grep -O TODO"),
            ("git grep", "git grep -Ovim TODO"),
            ("git grep", "git grep --open-files-in-pager=vim TODO"),
            ("git filter-branch", "git filter-branch --tree-filter make HEAD"),
            ("git diff", "git diff --ext-diff"),
            ("git log", "git log -p --ext-diff"),
            ("git --exec-path=bin", "git --exec-path=bin lfs pull"),
            ("git --no-pager", "git --no-pager rebase -x make main"),
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::GitRunsCommand), "{command:?}");
            assert!(!rule_covers(prefix, command), "{prefix:?} covers {command:?}");
        }
        // Git from any folder, and through a runner.
        assert_eq!(command_rule("/usr/bin/git rebase -x make main"), Err(OnceOnly::GitRunsCommand));
        assert_eq!(command_rule("uv run git submodule foreach make"), Err(OnceOnly::GitRunsCommand));
        assert_eq!(OnceOnly::GitRunsCommand.reason(), "git option that runs another command");
        // Accepted false positives: other subcommands' `-x` and `-O`.
        for command in ["git clean -x", "git clean -fdx", "git cherry-pick -x abc123", "git diff -Oorder.txt"] {
            assert_eq!(command_rule(command), Err(OnceOnly::GitRunsCommand), "{command:?}");
        }
        // Other git commands keep their rules.
        for (command, prefix) in [
            ("git rebase main", "git rebase"),
            ("git rebase -i main", "git rebase"),
            ("git rebase -X theirs main", "git rebase"),
            ("git add -u", "git add"),
            ("git status -uall", "git status"),
            ("git fetch origin", "git fetch"),
        ] {
            assert_eq!(command_rule(command), Ok(bash_rule(prefix)), "{command:?}");
        }
        assert_eq!(command_rule("git clone git@github.com:x/y.git"), unix_only_rule(bash_rule("git clone")));
        assert!(rule_covers("git rebase", "git rebase origin/main"));
    }

    #[test]
    fn python_modules_that_run_code_get_no_rule() {
        // Each asks, also under the rule its prefix would match.
        for (prefix, command) in [
            ("python3 -m timeit", "python3 -m timeit -n 3 pass"),
            ("python -m pdb", "python -m pdb scripts/x.py"),
            ("python3 -m code", "python3 -m code"),
            ("python3 -m runpy", "python3 -m runpy http.server"),
            ("python3 -m trace", "python3 -m trace --trace scripts/x.py"),
            ("python3 -m cProfile", "python3 -m cProfile scripts/x.py"),
            ("python3 -m profile", "python3 -m profile scripts/x.py"),
            ("python3.12 -m timeit", "python3.12 -m timeit pass"),
            ("uv run python -m timeit", "uv run python -m timeit pass"),
            ("poetry run python3 -m pdb", "poetry run python3 -m pdb scripts/x.py"),
            ("uv run python3.12 -m cProfile", "uv run python3.12 -m cProfile scripts/x.py"),
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::CodeModule), "{command:?}");
            assert!(!rule_covers(prefix, command), "{prefix:?} covers {command:?}");
        }
        // An absolute python is once-only before its prefix is looked at.
        assert_eq!(command_rule("/usr/bin/python3 -m timeit pass"), Err(OnceOnly::CodeModule));
        assert_eq!(OnceOnly::CodeModule.reason(), "python module that runs code from its arguments");
        // Other modules keep their rules.
        assert_eq!(command_rule("python3 -m pytest"), Ok(bash_rule("python3 -m pytest")));
        assert_eq!(command_rule("uv run python -m pytest -q"), Ok(bash_rule("uv run python -m pytest")));
        assert_eq!(command_rule("python3 -m pip install requests"), Ok(bash_rule("python3 -m pip")));
    }

    #[test]
    fn abbreviated_git_options_get_no_rule() {
        // Git takes any unambiguous prefix of a long option: `--ex` is `--exec`.
        for (prefix, command) in [
            ("git rebase", "git rebase --exe \"git push -f origin HEAD:main\" main"),
            ("git rebase", "git rebase --ex make main"),
            ("git fetch", "git fetch --upl=touch ."),
            ("git pull", "git pull --uplo=touch ."),
            ("git clone", "git clone --upload=touch . copy"),
            ("git ls-remote", "git ls-remote --upl=touch ."),
            ("git grep", "git grep --open-files=vim TODO"),
            ("git difftool", "git difftool --extc=vimdiff main"),
            ("git diff", "git diff --ext-d"),
            ("git send-pack", "git send-pack --rec=touch . main"),
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::GitRunsCommand), "{command:?}");
            assert!(!rule_covers(prefix, command), "{prefix:?} covers {command:?}");
        }
        // `git clone --config` sets config like `-c`, abbreviated too.
        for command in [
            "git clone --config core.fsmonitor=touch . copy",
            "git clone --config=core.fsmonitor=touch . copy",
            "git clone --conf core.fsmonitor=touch . copy",
            "git --config-e core.pager=PAGER log",
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::GitConfig), "{command:?}");
            assert!(!rule_covers("git clone", command), "{command:?}");
        }
        // Long options that are not a prefix of those keep their rules.
        for (command, prefix) in [
            ("git log --oneline", "git log"),
            ("git log --exclude=refs/stash --all", "git log"),
            ("git fetch --prune", "git fetch"),
            ("git commit --amend --no-edit", "git commit"),
            ("git log --recursive", "git log"),
        ] {
            assert_eq!(command_rule(command), Ok(bash_rule(prefix)), "{command:?}");
        }
    }

    #[test]
    fn python_clusters_variants_and_nested_module_runners_get_no_rule() {
        // `-m` inside a cluster of flags runs the module: `-Im` is `-I -m`.
        for (prefix, command) in [
            ("python3 -Im", "python3 -Im timeit pass"),
            ("python3 -um", "python3 -um timeit pass"),
            ("python3 -Bm", "python3 -Bm pytest"),
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::PackageRunner), "{command:?}");
            assert!(!rule_covers(prefix, command), "{prefix:?} covers {command:?}");
        }
        // Free-threaded, debug and pypy builds are pythons too.
        for command in ["python3.13t -m timeit pass", "python3.12-dbg -m pdb x.py", "pypy3 -m timeit pass"] {
            assert_eq!(command_rule(command), Err(OnceOnly::CodeModule), "{command:?}");
        }
        for command in ["python3.13t -c x", "pypy3 -c x", "pypy -c x"] {
            assert_eq!(command_rule(command), Err(OnceOnly::InlineCode), "{command:?}");
        }
        // A code module reached through another module runner.
        for (prefix, command) in [
            ("python3 -m coverage", "python3 -m coverage run -m timeit pass"),
            ("coverage run", "coverage run -m timeit pass"),
            ("coverage run", "coverage run --module=pdb x.py"),
            ("coverage run", "coverage run -mtimeit pass"),
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::CodeModule), "{command:?}");
            assert!(!rule_covers(prefix, command), "{prefix:?} covers {command:?}");
        }
        // Ordinary modules keep their rules.
        for (command, prefix) in [
            ("python3 -m coverage run -m pytest", "python3 -m coverage run -m pytest"),
            ("python3.13t -m pytest -q", "python3.13t -m pytest"),
            ("pypy3 -m pytest", "pypy3 -m pytest"),
        ] {
            assert_eq!(command_rule(command), Ok(bash_rule(prefix)), "{command:?}");
        }
    }

    #[test]
    fn huge_or_deeply_nested_commands_get_no_rule_quickly() {
        let started = std::time::Instant::now();
        // About 120 KB of nested runners once overflowed the stack.
        let huge = format!("{}x", "npx ".repeat(30_000));
        assert_eq!(command_rule(&huge), Err(OnceOnly::NotPlain));
        assert!(!rule_covers("npx npx", &huge));
        let words = format!("echo{}", " word".repeat(300));
        assert_eq!(command_rule(&words), Err(OnceOnly::NotPlain));
        assert!(started.elapsed() < Duration::from_secs(2), "took {:?}", started.elapsed());
        // At most 8 runners deep, and at most 256 words.
        assert_eq!(command_rule(&format!("{}vitest", "npx ".repeat(9))), Err(OnceOnly::NotPlain));
        let deep = format!("{}vitest", "npx ".repeat(8));
        assert_eq!(command_rule(&deep), Ok(bash_rule(deep.as_str())));
        assert_eq!(command_rule(&format!("echo{}", " word".repeat(255))), Ok(bash_rule("echo word")));
    }

    #[test]
    fn git_config_writes_get_no_rule() {
        for command in [
            "git config core.fsmonitor \"touch pwned #\"",
            "git config --global core.pager less",
            "git config --add alias.x '!rm -rf src'",
            "git config --unset user.name",
            "git config --replace-all core.pager less",
            "git config -e",
            "git config --edit",
            "git config edit",
            "git config set core.pager less",
            "git config --file .git/config core.fsmonitor x",
            "git -C . config core.fsmonitor x",
            "git --git-dir .git config core.fsmonitor x",
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::GitConfigWrite), "{command:?}");
            assert!(!rule_covers("git config", command), "{command:?}");
        }
        assert_eq!(OnceOnly::GitConfigWrite.reason(), "git config change, which can make git run any command");
        // Reads keep the `git config` rule.
        for command in [
            "git config user.email",
            "git config --get user.email",
            "git config --get-all remote.origin.fetch",
            "git config --get-regexp alias",
            "git config --list",
            "git config -l",
            "git config --list --show-origin",
            "git config --show-origin user.email",
            "git config --global user.email",
            "git config get user.email",
            "git config list",
        ] {
            assert_eq!(command_rule(command), Ok(bash_rule("git config")), "{command:?}");
            assert!(rule_covers("git config", command), "{command:?}");
        }
    }

    #[test]
    fn more_runners_name_what_they_run() {
        for (command, prefix) in [
            ("pipenv run pytest -q", "pipenv run pytest"),
            ("pdm run pytest", "pdm run pytest"),
            ("hatch run test", "hatch run test"),
            ("rye run pytest", "rye run pytest"),
            ("pixi run test", "pixi run test"),
            ("conda run pytest", "conda run pytest"),
            ("deno run main.ts", "deno run main.ts"),
        ] {
            assert_eq!(command_rule(command), Ok(bash_rule(prefix)), "{command:?}");
        }
        for command in [
            "pipenv run --verbose pytest",
            "conda run -n base pytest",
            "deno run -A main.ts",
            "pdm --verbose run pytest",
            "mise exec -- node app.js",
            "mise exec node@20 -- node app.js",
            "mise x node@20 -- node app.js",
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::PackageRunner), "{command:?}");
        }
        for (prefix, command) in [
            ("pipenv run", "pipenv run evil"),
            ("deno run", "deno run evil.ts"),
            ("mise exec node@20", "mise exec node@20 -- rm -rf src"),
        ] {
            assert!(!rule_covers(prefix, command), "{prefix:?} covers {command:?}");
        }
    }

    #[test]
    fn more_wrappers_interpreters_and_code_flags_get_no_rule() {
        for command in ["arch -arm64 make", "caffeinate -i make", "xcrun swift build"] {
            assert_eq!(command_rule(command), Err(OnceOnly::Wrapper), "{command:?}");
        }
        for command in [
            "tsx -e x",
            "ts-node -e x",
            "ipython -c x",
            "dash -c make",
            "ksh -c make",
            "tcsh -c make",
            "csh -c make",
            "perl -E 'say 1'",
            "perl -lE 'say 1'",
            "fish --command make",
            "fish --comm make",
            "fish -c make",
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::InlineCode), "{command:?}");
        }
        assert_eq!(command_rule("tsx scripts/build.ts"), Ok(bash_rule("tsx scripts/build.ts")));
    }

    #[test]
    fn program_names_match_in_any_case() {
        // macOS file systems ignore case, so `GIT` runs git.
        for (command, kind) in [
            ("GIT -c alias.x=!make x", OnceOnly::GitConfig),
            ("ENV X=1 make", OnceOnly::Wrapper),
            ("Sudo make", OnceOnly::Wrapper),
            ("Git rebase -x make main", OnceOnly::GitRunsCommand),
            ("git FILTER-BRANCH --tree-filter make HEAD", OnceOnly::GitRunsCommand),
            ("git Submodule Foreach make", OnceOnly::GitRunsCommand),
            ("NPX -y x", OnceOnly::PackageRunner),
            ("PYTHON3 -c x", OnceOnly::InlineCode),
            ("Python3 -m TimeIt pass", OnceOnly::CodeModule),
            ("GIT PUSH origin main", OnceOnly::Publish),
            ("Git Config core.fsmonitor x", OnceOnly::GitConfigWrite),
        ] {
            assert_eq!(command_rule(command), Err(kind), "{command:?}");
        }
        assert!(!rule_covers("ENV X=1", "ENV X=1 rm -rf src"));
    }

    #[test]
    fn legitimate_rules_still_work() {
        for (command, prefix) in [
            ("npm install x", "npm install"),
            ("git rebase main", "git rebase"),
            ("python3 -m pytest", "python3 -m pytest"),
            ("uv run pytest", "uv run pytest"),
            ("touch approval-test.txt", "touch approval-test.txt"),
            ("git config user.email", "git config"),
            ("coverage run -m pytest", "coverage run -m pytest"),
        ] {
            assert_eq!(command_rule(command), Ok(bash_rule(prefix)), "{command:?}");
        }
        assert_eq!(command_rule("git clone git@github.com:x/y.git"), unix_only_rule(bash_rule("git clone")));
        assert!(rule_covers("npm install", "npm install y"));
        assert!(rule_covers("git rebase", "git rebase origin/main"));
        assert!(rule_covers("python3 -m pytest", "python3 -m pytest tests"));
        assert!(rule_covers("uv run pytest", "uv run pytest -q"));
        assert!(rule_covers("touch approval-test.txt", "touch approval-test.txt"));
        assert!(rule_covers("git config", "git config user.name"));
        assert_eq!(rule_covers("git clone", "git clone -q git@github.com:x/z.git"), cfg!(unix));
        assert!(rule_covers("coverage run -m pytest", "coverage run -m pytest -x"));
        // As before, `mise exec` with `--` asks every time.
        assert_eq!(command_rule("mise exec node@20 -- node x.js"), Err(OnceOnly::PackageRunner));
    }

    #[test]
    fn git_clone_with_a_stuck_or_clustered_c_is_git_config() {
        // `git clone` reads `-c` stuck to its value and in a cluster.
        for command in [
            "git clone -ccore.sshCommand=touch x:y d",
            "git clone -qc core.sshCommand=touch x:y d",
            "git clone -nc core.sshCommand=touch x:y d",
            "git clone -cprotocol.ext.allow=always x:y d",
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::GitConfig), "{command:?}");
            assert!(!rule_covers("git clone", command), "{command:?}");
        }
        // Other clone flags keep the rule.
        assert_eq!(command_rule("git clone -q --depth 1 git@github.com:x/y.git"), unix_only_rule(bash_rule("git clone")));
    }

    #[test]
    fn coverage_run_is_a_module_runner() {
        // The rule names the module, as for python.
        assert_eq!(command_rule("coverage run -m pytest -q"), Ok(bash_rule("coverage run -m pytest")));
        assert_eq!(command_rule("coverage run scripts/x.py"), Ok(bash_rule("coverage run")));
        assert_eq!(command_rule("coverage report -m"), Ok(bash_rule("coverage report")));
        // `m` in a cluster, a flag before `-m`, or nothing to run: once-only.
        for (prefix, command) in [
            ("coverage run", "coverage run -Lm timeit pass"),
            ("coverage run", "coverage run -pm timeit pass"),
            ("uv run coverage", "uv run coverage run -Lm timeit pass"),
            ("coverage run", "coverage run -L -m pip install evil"),
            ("coverage run", "coverage run -m"),
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::PackageRunner), "{command:?}");
            assert!(!rule_covers(prefix, command), "{prefix:?} covers {command:?}");
        }
        // A rule for `coverage run` alone covers no module.
        assert!(!rule_covers("coverage run", "coverage run -m pip install evil"));
        assert_eq!(command_rule("coverage run -m timeit pass"), Err(OnceOnly::CodeModule));
    }

    #[test]
    fn mise_exec_with_a_command_flag_gets_no_rule() {
        for command in [
            "mise exec node@20 -c 'touch pwned'",
            "mise exec node@20 --command 'touch pwned'",
            "mise exec node@20 --comm 'touch pwned'",
            "mise exec node@20 -qc make",
            "mise x node@20 -c make",
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::PackageRunner), "{command:?}");
            assert!(!rule_covers("mise exec node@20", command), "{command:?}");
            assert!(!rule_covers("mise x node@20", command), "{command:?}");
        }
    }

    #[test]
    fn commands_with_a_url_get_no_rule() {
        for command in [
            "git clone https://github.com/x/y",
            "git clone \"https://github.com/x/y\" y",
            "git clone https:'//'github.com/x/y",
            "curl -s https://example.com",
            "npm install https://example.com/pkg.tgz",
            "pip install git+https://github.com/x/y",
            "open http://localhost:3000",
            "cat file:///etc/hosts",
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::Url), "{command:?}");
        }
        assert_eq!(OnceOnly::Url.reason(), "contains a URL");
        // No rule covers one, so a request never shows a rule that would not match it.
        assert!(!rule_covers("git clone", "git clone https://github.com/x/y"));
        assert!(!rule_covers("cat README.md", "cat README.md file:///repo/.worktrees/fix/a.rs"));
        // A host without a scheme is not a URL.
        assert_eq!(command_rule("git clone git@github.com:x/y.git"), unix_only_rule(bash_rule("git clone")));
        assert_eq!(command_rule("curl -s localhost:3000/api"), unix_only_rule(bash_rule("curl -s")));
    }

    #[test]
    fn wrappers_and_assignments_get_no_rule() {
        for command in [
            "env FOO=1 make",
            "/usr/bin/env make",
            "xargs rm",
            "sudo make install",
            "doas make",
            "nohup make",
            "time make",
            "nice make",
            "timeout 5 make",
            "watch make",
            "exec make",
            "command make",
            "builtin cd",
            "eval make",
            "source .venv/bin/activate",
            ". .venv/bin/activate",
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::Wrapper), "{command:?}");
        }
        for command in ["FOO=1 make", "NODE_ENV=production npm run build", "A=1 B=2 ls"] {
            assert_eq!(command_rule(command), Err(OnceOnly::Assignment), "{command:?}");
        }
        // A wrapper's name later in the command is an argument.
        assert_eq!(command_rule("npm run watch"), Ok(bash_rule("npm run")));
    }

    #[test]
    fn push_pull_requests_and_api_calls_get_no_rule() {
        for command in [
            "git push origin agent/x",
            "git push --force origin main",
            "git -C . push",
            "gh pr create --fill",
            "gh api repos/x/y",
            "glab mr create",
            "glab api projects",
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::Publish), "{command:?}");
        }
        assert_eq!(command_rule("git pull origin main"), Ok(bash_rule("git pull")));
        assert_eq!(command_rule("gh issue list"), Ok(bash_rule("gh issue")));
    }

    #[test]
    fn package_runners_with_a_flag_get_no_rule() {
        for command in [
            "npx -y create-vite",
            "npx --yes cowsay hi",
            "npx -p typescript tsc --init",
            "/usr/local/bin/npx -y x",
            "bunx --bun vite",
            "pnpx -y create-x",
            "uvx --from httpie http",
            "pnpm dlx -y create-vite",
            "yarn dlx -q create-x",
            "npm exec -- vitest",
            "npm exec --package=x y",
            "pipx run --spec black black .",
            // The aliases npm x, bun x and uv tool run.
            "npm x -y x",
            "bun x --bun vite",
            "uv tool run --from x y",
            // No package at all: the rule would be the runner alone.
            "npx",
            "pnpm dlx",
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::PackageRunner), "{command:?}");
        }
        assert_eq!(OnceOnly::PackageRunner.reason(), "runner with a flag or nothing to run, so a rule would cover anything it runs");
        // Otherwise the rule names the package, so it covers that package only.
        for (command, prefix) in [
            ("npx vitest run", "npx vitest"),
            ("npx @playwright/test install", "npx @playwright/test"),
            ("uvx ruff check .", "uvx ruff"),
            ("pnpm dlx create-vite my-app", "pnpm dlx create-vite"),
            ("yarn dlx create-react-app x", "yarn dlx create-react-app"),
            ("npm exec vitest run", "npm exec vitest"),
            ("pipx run black .", "pipx run black"),
            ("uv tool run ruff check", "uv tool run ruff"),
            // Other npm and uv commands keep their two words.
            ("npm install x", "npm install"),
            ("uv tool list", "uv tool"),
        ] {
            assert_eq!(command_rule(command), Ok(bash_rule(prefix)), "{command:?}");
        }
        let preview = command_rule("pnpm dlx create-vite my-app").map(|rule| rule_preview(&rule));
        assert_eq!(preview.as_deref(), Ok("`pnpm dlx create-vite` commands"));
    }

    #[test]
    fn package_runner_rules_must_name_the_package() {
        let covers = |prefix: &str, command: &str| rule_allows(&bash_rule(prefix), "Bash", &json!({ "command": command }), Path::new(W));
        assert!(covers("npx vitest", "npx vitest run --watch"));
        assert!(covers("pnpm dlx create-vite", "pnpm dlx create-vite other-app"));
        assert!(covers("npm install", "npm install x"));
        // Rules for a runner alone, as an older daemon saved them, cover no package.
        for (prefix, command) in [
            ("npx -y", "npx -y evil"),
            ("npx", "npx vitest"),
            ("pnpm dlx", "pnpm dlx create-vite"),
            ("npm exec", "npm exec evil"),
            ("uv tool", "uv tool run evil"),
        ] {
            assert!(!covers(prefix, command), "{prefix:?} covers {command:?}");
        }
    }

    #[test]
    fn runners_name_what_they_run() {
        // The rule names the module, command or package, so it covers that one only.
        for (command, prefix) in [
            ("python3 -m pytest -q tests", "python3 -m pytest"),
            ("python -m pip install requests", "python -m pip"),
            ("python3.12 -m http.server 8000", "python3.12 -m http.server"),
            ("uv run pytest -q", "uv run pytest"),
            ("poetry run pytest tests/test_a.py", "poetry run pytest"),
            ("pnpm exec vitest run", "pnpm exec vitest"),
            ("yarn exec tsc --noEmit", "yarn exec tsc"),
            ("bundle exec rspec spec/a_spec.rb", "bundle exec rspec"),
            ("go run ./cmd/server --port 8080", "go run ./cmd/server"),
            ("go run .", "go run ."),
            // A runner through a runner, and an interpreter's script.
            ("uv run python -m pytest -q", "uv run python -m pytest"),
            ("poetry run python manage.py test", "poetry run python manage.py"),
            ("uv run npx vitest", "uv run npx vitest"),
            // Other python, uv and go commands keep their two words.
            ("python3 scripts/build.py --fast", "python3 scripts/build.py"),
            ("uv sync --frozen", "uv sync"),
        ] {
            assert_eq!(command_rule(command), Ok(bash_rule(prefix)), "{command:?}");
        }
        assert_eq!(command_rule("go test ./..."), unix_only_rule(bash_rule("go test")));
        let preview = command_rule("python3 -m pytest -q").map(|rule| rule_preview(&rule));
        assert_eq!(preview.as_deref(), Ok("`python3 -m pytest` commands"));
        // A flag in place of what it runs, or nothing, makes it once-only.
        for command in [
            "python3 -m",
            "python3 -m -X",
            "uv run",
            "uv run --with httpx pytest",
            "poetry run",
            "poetry run -- pytest",
            "pnpm exec",
            "pnpm exec -- vitest",
            "yarn exec",
            "bundle exec",
            "bundle exec --keep-file-descriptors rake",
            "go run",
            "go run -race .",
            "uv run python -m",
            "uv run npx -y create-vite",
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::PackageRunner), "{command:?}");
        }
        // What a runner runs is checked like a command of its own.
        assert_eq!(command_rule("uv run env FOO=1 pytest"), Err(OnceOnly::Wrapper));
        assert_eq!(command_rule("bundle exec sudo rake"), Err(OnceOnly::Wrapper));
        assert_eq!(command_rule("uv run git -c core.pager=less log"), Err(OnceOnly::GitConfig));
    }

    #[test]
    fn runner_rules_cover_only_what_they_name() {
        assert!(rule_covers("python3 -m pytest", "python3 -m pytest tests/test_a.py -x"));
        assert!(rule_covers("uv run pytest", "uv run pytest -q"));
        assert!(rule_covers("uv run python -m pytest", "uv run python -m pytest -q"));
        assert!(rule_covers("go run ./cmd/server", "go run ./cmd/server --port 9090"));
        for (prefix, command) in [
            ("python3 -m pytest", "python3 -m timeit -n 3 pass"),
            ("uv run pytest", "uv run python -c x"),
            ("uv run pytest", "uv run ruff check"),
            // Rules for a runner alone, as an older daemon saved them, cover nothing.
            ("python3 -m", "python3 -m timeit -n 3 pass"),
            ("python3 -m", "python3 -m pytest"),
            ("uv run", "uv run pytest"),
            ("poetry run", "poetry run pytest"),
            ("pnpm exec", "pnpm exec vitest"),
            ("yarn exec", "yarn exec tsc"),
            ("bundle exec", "bundle exec rspec"),
            ("go run", "go run ."),
            // Nor does one that names a runner but not what it runs.
            ("uv run python", "uv run python -m http.server"),
            ("uv run python", "uv run python evil.py"),
        ] {
            assert!(!rule_covers(prefix, command), "{prefix:?} covers {command:?}");
        }
    }

    #[test]
    fn runners_after_a_leading_flag_get_no_rule() {
        for command in [
            "npm --prefix web exec -y x",
            "npm --prefix web exec vitest",
            "npm -w web x vitest",
            "pnpm -C web dlx create-vite",
            "pnpm --filter web exec vitest",
            "yarn --cwd web dlx create-x",
            "yarn --cwd web exec tsc",
            "bun --cwd web x vite",
            "uv --directory web run pytest",
            "uv -q tool run ruff",
            "poetry -C web run pytest",
            "go -C web run .",
            "python3 -u -m timeit pass",
            "python3 -W error -m pytest",
        ] {
            assert_eq!(command_rule(command), Err(OnceOnly::PackageRunner), "{command:?}");
        }
        // Without a runner after the flag they keep their two words.
        for (command, prefix) in [
            ("npm --prefix web install", "npm --prefix"),
            ("pnpm -C web build", "pnpm -C"),
            ("uv --directory web sync", "uv --directory"),
            ("uv -q tool list", "uv -q"),
            ("python3 -u scripts/build.py", "python3 -u"),
            // A module's own `-m` is not python's.
            ("python3 -m pytest -m slow", "python3 -m pytest"),
        ] {
            assert_eq!(command_rule(command), Ok(bash_rule(prefix)), "{command:?}");
        }
        // A rule saved for the program and its flag covers them no more.
        assert!(!rule_covers("npm --prefix", "npm --prefix web exec -y x"));
        assert!(!rule_covers("pnpm -C", "pnpm -C web dlx x"));
        assert!(!rule_covers("python3 -u", "python3 -u -m timeit pass"));
    }

    #[test]
    fn commands_no_rule_could_match_get_no_rule() {
        for command in ["cat /etc/hosts", "ls|wc", "git status; ls", "cat \"unterminated"] {
            assert_eq!(command_rule(command), Err(OnceOnly::NotPlain), "{command:?}");
        }
        // On Windows a `~` can also be part of a short (8.3) name.
        let tilde = if cfg!(windows) { OnceOnly::WindowsPath } else { OnceOnly::NotPlain };
        assert_eq!(command_rule("cat ~/.zshrc"), Err(tilde));
        // Only the rule's own words must be plain.
        assert_eq!(command_rule("npm install x && npm test"), Ok(bash_rule("npm install")));
    }

    #[test]
    fn tools_with_no_known_input_shape_get_no_rule() {
        for tool in ["WebSearch", "mcp__github__create_issue", "Task", "unknown"] {
            assert_eq!(rule_of(tool, json!({"query": "x"})), Err(OnceOnly::UnknownTool), "{tool}");
        }
    }

    #[test]
    fn protected_requests_get_no_rule() {
        assert_eq!(rule_of("Edit", json!({"file_path": format!("{W}/.claude/settings.json")})), Err(OnceOnly::Protected));
        assert_eq!(rule_of("Write", json!({"file_path": format!("{W}/.mcp.json")})), Err(OnceOnly::Protected));
        assert_eq!(rule_of("Grep", json!({"pattern": "x", "path": "/Users/x/.ssh"})), Err(OnceOnly::Protected));
        assert_eq!(command_rule("cat .env"), Err(OnceOnly::Protected));
    }

    #[test]
    fn file_and_web_rules() {
        assert_eq!(rule_of("Edit", json!({"file_path": format!("{W}/src/a.rs")})), Ok(AllowRule { tool: "Edit".into(), prefix: None }));
        assert_eq!(rule_of("Grep", json!({"pattern": "x"})), Ok(AllowRule { tool: "Grep".into(), prefix: None }));
        let fetch = rule_of("WebFetch", json!({"url": "https://Docs.Example.com:443/guide?x=1", "prompt": "x"}));
        assert_eq!(fetch, Ok(AllowRule { tool: "WebFetch".into(), prefix: Some("https://docs.example.com".into()) }));
        for url in ["file:///etc/passwd", "docs.example.com", "https://user@docs.example.com/", ""] {
            assert_eq!(rule_of("WebFetch", json!({"url": url})), Err(OnceOnly::NoOrigin), "{url:?}");
        }
        assert_eq!(rule_of("WebFetch", json!({"prompt": "x"})), Err(OnceOnly::NoOrigin));
    }

    #[test]
    fn web_origins() {
        assert_eq!(web_origin("https://docs.example.com/a"), Some("https://docs.example.com".into()));
        assert_eq!(web_origin("HTTP://Example.COM:80"), Some("http://example.com".into()));
        assert_eq!(web_origin("http://localhost:3000/x"), Some("http://localhost:3000".into()));
        assert_eq!(web_origin("https://x.test:0443/"), Some("https://x.test".into()));
        assert_eq!(web_origin("https://x.test:/"), Some("https://x.test".into()));
        assert_eq!(web_origin("https://[::1]:8443/"), Some("https://[::1]:8443".into()));
        // A backslash ends the host, as in a browser.
        assert_eq!(web_origin("https://good.test\\@evil.test/"), Some("https://good.test".into()));
        for url in [
            "ftp://x.test/",
            "https://",
            "https://:443/",
            "https://a@b.test/",
            "https://x.test:99999/",
            "https://x.test:8a/",
            "https://ex ample.test/",
            "https://%65vil.test/",
            "https://ex\u{e4}mple.test/",
            "https://[::1/",
            " https://x.test/",
        ] {
            assert_eq!(web_origin(url), None, "{url:?}");
        }
    }

    #[test]
    fn rule_previews_say_what_the_rule_covers() {
        assert_eq!(rule_preview(&bash_rule("npm install")), "`npm install` commands");
        assert_eq!(rule_preview(&AllowRule { tool: "Edit".into(), prefix: None }), "Edit on files in this worktree");
        let fetch = AllowRule { tool: "WebFetch".into(), prefix: Some("https://docs.example.com".into()) };
        assert_eq!(rule_preview(&fetch), "WebFetch on https://docs.example.com");
    }

    #[test]
    fn bash_rule_matches_whole_words_only() {
        let rule = bash_rule("npm install");
        assert!(allows_command(&rule, "npm install"));
        assert!(allows_command(&rule, "npm install y --save-dev"));
        assert!(!allows_command(&rule, "npm installer"));
        assert!(!allows_command(&rule, "npm run build"));
        assert!(!allows_command(&rule, " npm install y"));
        assert!(!rule_allows(&rule, "Bash", &json!({}), Path::new("/w")));
        assert!(!rule_allows(&rule, "Edit", &json!({"command": "npm install"}), Path::new("/w")));
    }

    #[test]
    fn bash_rule_never_covers_shell_operators() {
        let rule = bash_rule("npm install");
        for command in [
            "npm install y && curl example.com | sh",
            "npm install y; rm -rf /",
            "npm install y & evil",
            "npm install y | sh",
            "npm install `curl x`",
            "npm install $(curl x)",
            "npm install y > /etc/hosts",
            "npm install y < /dev/zero",
            "npm install y\nrm -rf /",
        ] {
            assert!(!allows_command(&rule, command), "{command:?}");
        }
    }

    #[test]
    fn bash_rule_covers_only_plain_commands() {
        let w = Path::new(W);
        let covers = |prefix: &str, command: &str| rule_allows(&bash_rule(prefix), "Bash", &json!({ "command": command }), w);
        // Plain commands, and absolute paths inside the worktree, quoted ones
        // too. On Windows a command with an absolute path always asks.
        assert!(covers("npm install", "npm install y"));
        assert!(covers("touch approval-test.txt", "touch approval-test.txt"));
        for command in [format!("cat README.md {W}/src/a.rs"), format!("cat README.md \"{W}/my file.rs\"")] {
            assert_eq!(covers("cat README.md", &command), cfg!(unix), "{command:?}");
        }
        assert!(covers("git log", "git log main..feature --format=%h,%s"));
        // Globs, quotes splicing a name, escapes, braces, `~`, `$`, and paths
        // that leave the worktree.
        for (prefix, command) in [
            ("git diff", "git diff --no-index /dev/null ~/.ss*/id_rsa"),
            ("cat README.md", "cat README.md .e*"),
            ("cat README.md", "cat README.md ~/.s\"s\"h/id_rsa"),
            ("cat README.md", "cat README.md ~/.s\\sh/id_rsa"),
            ("cat README.md", "cat README.md ~/.{ssh,x}/id_rsa"),
            ("cat README.md", "cat README.md ~/.*/credentials"),
            ("grep -r", "grep -r PRIVATE ~"),
            ("cat README.md", "cat README.md a?.txt"),
            ("cat README.md", "cat README.md [ab].txt"),
            ("cat README.md", "cat README.md $HOME/notes"),
            ("cat README.md", "cat README.md /etc/hosts"),
            ("cat README.md", "cat README.md \"/etc/hosts\""),
            ("git diff", "git diff --output=/etc/x"),
            ("cat README.md", "cat README.md ../../etc/hosts"),
            ("cat README.md", "cat README.md \"unterminated"),
        ] {
            assert!(!covers(prefix, command), "{command:?}");
        }
    }

    #[test]
    fn bash_rule_never_covers_attached_outside_paths() {
        let w = Path::new(W);
        let covers = |prefix: &str, command: &str| rule_allows(&bash_rule(prefix), "Bash", &json!({ "command": command }), w);
        // A path attached to a flag or after punctuation is still a path.
        for (prefix, command) in [
            ("tar xf", "tar xf a.tar -C/Users/x"),
            ("cat README.md", "cat README.md -f/etc/x"),
            ("tar xf", "tar xf a.tar -C.."),
            ("tar xf", "tar xf a.tar -C../other"),
            ("cat README.md", "cat README.md x@../y"),
            ("curl -d", "curl -d @/etc/passwd localhost:8080"),
            ("scp -r", "scp -r host:/etc/passwd ."),
            ("docker run", "docker run -v src/x:/etc/passwd img"),
            // A URL names no worktree path, so it always asks.
            ("curl -s", "curl -s https://example.com"),
        ] {
            assert!(!covers(prefix, command), "{command:?}");
        }
        // Relative paths, and absolute ones inside the worktree, stay covered
        // (on Windows a command with an absolute path, or a `:`, asks).
        assert_eq!(covers("tar xf", &format!("tar xf a.tar -C{W}/out")), cfg!(unix));
        assert_eq!(covers("curl -s", "curl -s localhost:3000/api"), cfg!(unix));
        for (prefix, command) in [
            ("git add", "git add .github/workflows/ci.yml"),
            ("git add", "git add ./src/a.rs src/b.rs"),
            ("npm install", "npm install @types/node"),
            ("cat README.md", "cat README.md \"my dir/a.rs\""),
            ("git log", "git log origin/main..HEAD"),
        ] {
            assert!(covers(prefix, command), "{command:?}");
        }
    }

    #[test]
    fn bash_rule_without_prefix_allows_nothing() {
        let rule = AllowRule { tool: "Bash".into(), prefix: None };
        assert!(!allows_command(&rule, "ls"));
    }

    #[test]
    fn file_rule_stays_inside_the_worktree() {
        assert!(allows_file(json!(format!("{W}/src/a.rs"))));
        assert!(allows_file(json!(format!("{W}/./a.rs"))));
        assert!(!allows_file(json!(W)));
        assert!(!allows_file(json!(format!("{W}/../other/a.rs"))));
        assert!(!allows_file(json!(format!("{W}-2/a.rs"))));
        assert!(!allows_file(json!("/etc/hosts")));
        assert!(!allows_file(json!("src/a.rs")));
        assert!(!allows_file(json!(5)));
        assert!(!allows_file(Value::Null));
    }

    #[test]
    fn webfetch_rule_covers_its_origin_only() {
        let w = Path::new("/w");
        let fetch = |url: &str| json!({"url": url, "prompt": "Summarize"});
        let rule = AllowRule { tool: "WebFetch".into(), prefix: Some("https://docs.example.com".into()) };
        for url in ["https://docs.example.com", "https://docs.example.com/a/b?q=1#x", "HTTPS://DOCS.example.com:443/"] {
            assert!(rule_allows(&rule, "WebFetch", &fetch(url), w), "{url}");
        }
        for url in [
            "https://example.com/",
            "http://docs.example.com/",
            "https://docs.example.com:8443/",
            "https://docs.example.com.evil.test/",
            "https://evil.test/https://docs.example.com",
            "https://docs.example.com@evil.test/",
            "file:///etc/passwd",
            "docs.example.com",
        ] {
            assert!(!rule_allows(&rule, "WebFetch", &fetch(url), w), "{url}");
        }
        assert!(!rule_allows(&rule, "WebFetch", &json!({"prompt": "x"}), w));
        assert!(!rule_allows(&rule, "Edit", &fetch("https://docs.example.com/"), w));
        // A WebFetch rule without an origin, as older daemons saved, allows nothing.
        let any_url = AllowRule { tool: "WebFetch".into(), prefix: None };
        assert!(!rule_allows(&any_url, "WebFetch", &fetch("https://example.com"), w));
    }

    #[test]
    fn rules_for_tools_with_no_known_input_shape_allow_nothing() {
        for tool in ["WebSearch", "mcp__github__create_issue", "Task", "unknown"] {
            let rule = AllowRule { tool: tool.into(), prefix: None };
            assert!(!rule_allows(&rule, tool, &json!({"query": "x"}), Path::new("/w")), "{tool}");
        }
    }

    #[test]
    fn search_and_notebook_paths_stay_inside_the_worktree() {
        let w = Path::new(W);
        let rule = |tool: &str| AllowRule { tool: tool.into(), prefix: None };
        assert!(rule_allows(&rule("Grep"), "Grep", &json!({"pattern": "x", "path": format!("{W}/src")}), w));
        assert!(rule_allows(&rule("Grep"), "Grep", &json!({"pattern": "x"}), w));
        assert!(!rule_allows(&rule("Grep"), "Grep", &json!({"pattern": "x", "path": "/etc"}), w));
        assert!(!rule_allows(&rule("Glob"), "Glob", &json!({"pattern": "*", "path": 5}), w));
        let notebook = json!({"notebook_path": "/tmp/n.ipynb", "new_source": "x"});
        assert!(!rule_allows(&rule("NotebookEdit"), "NotebookEdit", &notebook, w));
        // Every path field must be inside, not just one.
        let mixed = json!({"file_path": format!("{W}/a.rs"), "path": "/etc"});
        assert!(!rule_allows(&rule("Edit"), "Edit", &mixed, w));
    }

    #[test]
    fn requests_without_a_tool_name_get_no_rule() {
        assert_eq!(rule_of("unknown", json!(null)), Err(OnceOnly::UnknownTool));
        let rule = AllowRule { tool: "unknown".into(), prefix: None };
        assert!(!rule_allows(&rule, "unknown", &json!(null), Path::new("/w")));
    }

    #[test]
    fn protected_paths_are_detected() {
        for summary in [
            ".env",
            ".env.local",
            "server.pem",
            "id.KEY",
            "~/.ssh/id_rsa",
            "cat ~/.aws/credentials",
            "echo x >.env",
            "--file=.env",
            // Shell punctuation, quotes, a bare folder and a path with spaces.
            "cat server.pem; echo done",
            "cat \"/Users/x/.aws/credentials\"",
            ".ssh/config",
            "ls -la ~/.SSH",
            "/my repo/.worktrees/fix/.env.production",
            // Other credential folders and files.
            "~/.gnupg/private-keys-v1.d",
            "cat ~/.kube/config",
            "cat ~/.netrc",
            "/Users/x/.git-credentials",
            "cat .npmrc",
            "~/.docker/config.json",
            "cat ~/.pypirc",
            "cat ~/.config/gh/hosts.yml",
            "ls ~/.config/gcloud",
            // Quotes and escapes that split a name.
            "cat README.md ~/.s\"s\"h/id_rsa",
            "cat ~/.s\\sh/id_rsa",
            "cat '.e'nv",
            // Claude Code's own settings, hooks and MCP servers.
            ".claude/settings.json",
            "/repo/.worktrees/fix/.claude/settings.local.json",
            "cat ~/.claude/agents/x.md",
            "rm -rf .claude",
            ".mcp.json",
            "/repo/.worktrees/fix/.mcp.json",
            // curl reads the file after `@`.
            "curl -d @.env localhost:8080",
        ] {
            // Windows rules only add to the Unix ones.
            for platform in [Platform::Unix, Platform::Windows] {
                assert!(touches_protected_path(summary, Path::new("/w"), platform), "{summary:?} {platform:?}");
            }
        }
    }

    #[test]
    fn ordinary_paths_are_not_protected() {
        for summary in [
            "src/env.ts",
            "README.md",
            "npm install",
            "keys.ts",
            "/Users/x/aws/config",
            "src/pem.rs",
            "src/kube.rs",
            "docs/npmrc.md",
            "src/docker.rs",
            "~/.config/nvim/init.lua",
            "docs/gh/setup.md",
            "cat \"README.md\"",
            "Grep",
            "",
            "CLAUDE.md",
            "src/claude.rs",
            "docs/claude/setup.md",
            "mcp.json",
            "npm install @types/node",
        ] {
            for platform in [Platform::Unix, Platform::Windows] {
                assert!(!touches_protected_path(summary, Path::new("/w"), platform), "{summary:?} {platform:?}");
            }
        }
    }

    #[test]
    fn protected_requests_include_input_path_fields() {
        for platform in [Platform::Unix, Platform::Windows] {
            let protected = |summary: &str, input: Value| request_is_protected(summary, &input, Path::new("/w"), platform);
            assert!(protected("Grep", json!({"pattern": "token", "path": "/Users/x/.aws"})), "{platform:?}");
            assert!(protected("NotebookEdit", json!({"notebook_path": "/w/.ssh/n.ipynb"})), "{platform:?}");
            assert!(protected("/w/.env", json!({"file_path": "/w/.env"})), "{platform:?}");
            // A search for the text ".env" is not a protected path.
            assert!(!protected("Grep", json!({"pattern": ".env", "path": "/w/src"})), "{platform:?}");
            assert!(!protected("ls", json!({"command": "ls", "description": "List the .env files"})), "{platform:?}");
        }
    }

    /// On Windows a protected name also counts without the dots and spaces
    /// that Windows drops from its end, and a short (8.3) name counts as
    /// protected, since it can stand for any of them. Unix reads names as
    /// they are written.
    #[test]
    fn windows_protects_the_names_windows_reads_as_protected_ones() {
        let worktree = Path::new("/repo/.worktrees/fix");
        let protected = |summary: &str, platform| touches_protected_path(summary, worktree, platform);
        for summary in [
            // A dot or a space at the end of a name.
            "/repo/.worktrees/fix/.claude./settings.local.json",
            r".claude.\settings.local.json",
            "cat .claude../settings.json",
            ".mcp.json.",
            "cert/server.pem.",
            "id.key. .",
            "~/.ssh./id_rsa",
            "~/.config./gh/hosts.yml",
            "~/.config/gcloud./credentials.db",
            r"C:\Users\x\.aws.\credentials",
            // Short names, in any case, quoted or escaped.
            "/repo/.worktrees/fix/CLAUDE~1/settings.local.json",
            r"C:\Users\x\CLAUDE~1\settings.json",
            "cat SETTIN~1.JSO",
            "MCP~1.JSO",
            "type env~1",
            "claude~12/x",
            "CLAUDE~\"1\"",
            r"CLAUDE~\1",
        ] {
            assert!(protected(summary, Platform::Windows), "{summary:?}");
            assert!(!protected(summary, Platform::Unix), "{summary:?}");
        }
        // A space ends a word, so `.env ` is protected everywhere.
        for summary in [".env ", "\".ssh /id_rsa\""] {
            assert!(protected(summary, Platform::Windows) && protected(summary, Platform::Unix), "{summary:?}");
        }
        for summary in [
            // A `~` before anything but a digit is not a short name.
            "~/.zshrc",
            "cat ~/notes.txt",
            "ls ~user/x",
            "a~b.txt",
            "~",
            // Ordinary names with dots, and `.` and `..`.
            "src/a.rs.",
            "docs/claude./x",
            "./src/../README.md",
            "...",
            // A data stream or a drive asks each time, but is not protected.
            "src/a.rs::$DATA",
            "notes.txt:hidden",
            "D:notes",
            r"C:src\a.rs",
        ] {
            assert!(!protected(summary, Platform::Windows), "{summary:?}");
            assert!(!protected(summary, Platform::Unix), "{summary:?}");
        }
        // The worktree's own path is the daemon's: a short name in it is not
        // the agent's, even one that could stand for a protected name, as the
        // user folder `CLAUDE~1` of a long user name can. Below it, beside it
        // and elsewhere one is.
        let short = Path::new("/Users/CLAUDE~1/repo/.worktrees/fix");
        let in_short = |summary: &str| touches_protected_path(summary, short, Platform::Windows);
        assert!(!in_short("/Users/CLAUDE~1/repo/.worktrees/fix/src/a.rs"));
        assert!(!in_short("/Users/CLAUDE~1/repo/.worktrees/fix"));
        assert!(!in_short(r"cat \Users\claude~1\repo\.worktrees\fix\src\a.rs"));
        assert!(!in_short("cat \"/users/claude~1/repo/.worktrees/fix/src/a.rs\" README.md"));
        assert!(in_short("/Users/CLAUDE~1/repo/.worktrees/fix/CLAUDE~1/x"));
        assert!(in_short("/Users/CLAUDE~1/repo/.worktrees/fix2/src/a.rs"));
        assert!(in_short("/Users/CLAUDE~1/repo/src/a.rs"));
        let drive = Path::new(r"C:\Users\CLAUDE~1\repo\.worktrees\fix");
        assert!(!touches_protected_path(r"C:\Users\CLAUDE~1\repo\.worktrees\fix\src\a.rs", drive, Platform::Windows));
        assert!(!touches_protected_path("C:/Users/CLAUDE~1/repo/.worktrees/fix/src/a.rs", drive, Platform::Windows));
        assert!(!touches_protected_path(r"\\?\C:\Users\CLAUDE~1\repo\.worktrees\fix\src\a.rs", drive, Platform::Windows));
        assert!(touches_protected_path(r"C:\Users\CLAUDE~1\repo\.worktrees\fix\.claude.\x", drive, Platform::Windows));

        // These requests are protected, so Always allow says so, and the rest
        // keep the reason they had.
        let rule = |summary: &str, platform| rule_for_on("Edit", summary, &json!({ "file_path": summary }), worktree, platform);
        for path in ["/repo/.worktrees/fix/CLAUDE~1/settings.local.json", "/repo/.worktrees/fix/.mcp.json."] {
            assert_eq!(rule(path, Platform::Windows), Err(OnceOnly::Protected), "{path:?}");
        }
        for path in ["/repo/.worktrees/fix/src/a.rs::$DATA", "/repo/.worktrees/fix/NUL", "D:notes.md"] {
            assert_eq!(rule(path, Platform::Windows), Err(OnceOnly::WindowsPath), "{path:?}");
        }
        let command = |command: &str| rule_for_on("Bash", command, &json!({ "command": command }), worktree, Platform::Windows);
        assert_eq!(command("cp README.md .claude./settings.local.json"), Err(OnceOnly::Protected));
        assert_eq!(command("cat CLAUDE~1/settings.local.json"), Err(OnceOnly::Protected));
        assert_eq!(command("cat ~/.zshrc"), Err(OnceOnly::WindowsPath));
        assert_eq!(command("cp README.md D:notes"), Err(OnceOnly::WindowsPath));
    }

    /// On Windows a short (8.3) name is a protected path only when it can
    /// stand for a protected name. Any other one, such as a git revision, is
    /// not, but still asks each time and gets no rule. Unix has no short names.
    #[test]
    fn windows_short_names_are_protected_only_when_they_can_be_protected_names() {
        let worktree = Path::new("/repo/.worktrees/fix");
        let protected = |summary: &str, platform| touches_protected_path(summary, worktree, platform);
        let rule = |command: &str, platform| rule_for_on("Bash", command, &json!({ "command": command }), worktree, platform);
        for name in [
            // `.claude`, also with the hash Windows uses once four names share a start.
            "CLAUDE~1",
            "CL3F2A~1",
            "claude~12",
            // `.claude/settings.json`, `.mcp.json`, `.env`, `.env.local`, `.ssh`, `.config`.
            "SETTIN~1.JSO",
            "MCP~1.JSO",
            "ENV~1",
            "ENV~1.LOC",
            "SSH~1",
            "CONFIG~1",
            "GIT-CR~1",
            "DOCKER~1",
        ] {
            for summary in [
                format!("type {name}"),
                format!(r"C:\Users\x\{name}\x"),
                format!("/repo/.worktrees/fix/{name}/x"),
                format!("cat {name}."),
                format!("--file={name}"),
            ] {
                assert!(protected(&summary, Platform::Windows), "{summary:?}");
                assert!(!protected(&summary, Platform::Unix), "{summary:?}");
                assert_eq!(rule(&summary, Platform::Windows), Err(OnceOnly::Protected), "{summary:?}");
            }
        }
        // A `.pem` or `.key` file is protected by its name everywhere.
        for summary in ["type SERVER~1.PEM", r"type C:\x\PRIVAT~1.KEY"] {
            assert!(protected(summary, Platform::Windows) && protected(summary, Platform::Unix), "{summary:?}");
        }
        for command in [
            "git diff HEAD~1",
            "git rebase -i @~2",
            "git log main~3",
            "git diff origin/main~1",
            "git show HEAD~1:src/a.rs",
            "git diff HEAD~1..HEAD",
            "git log a1b2c3d~2 --oneline",
            "git checkout -b fix HEAD~10",
            // Short names that no protected name has.
            "type RUNNER~1",
            r"type C:\Users\RUNNER~1\notes.txt",
            "type PROGRA~1",
            "type NOTES~1.TXT",
        ] {
            for platform in [Platform::Unix, Platform::Windows] {
                assert!(!protected(command, platform), "{command:?} {platform:?}");
            }
            assert_eq!(rule(command, Platform::Windows), Err(OnceOnly::WindowsPath), "{command:?}");
        }
        // A protected name next to a revision still counts.
        assert_eq!(rule("git diff HEAD~1 -- .env", Platform::Windows), Err(OnceOnly::Protected));
        // A short name in a URL gets no rule on Windows either. Unix keeps its rule.
        let url = "https://example.com/docs/HEAD~1";
        let fetch = |platform| rule_for_on("WebFetch", url, &json!({ "url": url }), worktree, platform);
        assert_eq!(fetch(Platform::Windows), Err(OnceOnly::WindowsPath));
        assert_eq!(fetch(Platform::Unix), Ok(AllowRule { tool: "WebFetch".into(), prefix: Some("https://example.com".into()) }));
        // The worktree's own short name is the daemon's.
        let short = Path::new("/Users/RUNNER~1/repo/.worktrees/fix");
        let path = "/Users/RUNNER~1/repo/.worktrees/fix/src/a.rs";
        let input = json!({ "file_path": path, "content": "x" });
        assert_eq!(rule_for_on("Write", path, &input, short, Platform::Windows), Ok(AllowRule { tool: "Write".into(), prefix: None }));
    }

    /// The short forms that count come from the protected names, made the
    /// way Windows makes them.
    #[test]
    fn short_forms_of_protected_names_come_from_the_names() {
        let form = |base: &str, extension: &str| (base.to_string(), extension.to_string());
        assert_eq!(short_form(".claude"), form("claude", ""));
        assert_eq!(short_form(".mcp.json"), form("mcp", "jso"));
        assert_eq!(short_form(".git-credentials"), form("git-cr", ""));
        assert_eq!(short_form(".env"), form("env", ""));
        assert_eq!(short_form("settings.local.json"), form("settin", "jso"));
        assert_eq!(short_form("my file.tar.gz"), form("myfile", "gz"));
        assert_eq!(protected_short_starts(), ["ss", "aw", "gn", "ku", "do", "ne", "gi", "np", "py", "cl", "mc", "en", "co"]);
        assert_eq!(protected_short_extensions(), ["jso", "pem", "key"]);
    }

    /// The check reads text only, so the Windows rules run on every platform.
    #[test]
    fn windows_paths_are_checked_with_backslashes_in_any_case() {
        let w = Path::new(r"C:\Users\x\repo\.worktrees\fix");
        let protected = |summary: &str| touches_protected_path(summary, w, Platform::Windows);
        for summary in [
            r"C:\Users\x\.ssh\id_rsa",
            r"type C:\Users\x\.aws\credentials",
            r"C:\Users\x\repo\.worktrees\fix\.env.local",
            r"C:\Users\X\.SSH\config",
            r"C:\Users\x\.config\gh\hosts.yml",
            r"C:\Users\x\repo\.claude\settings.json",
            r"C:\Users\x\.s's'h\id_rsa",
            r#"C:\Users\x\.s"s"h\id_rsa"#,
            r"cert\server.PEM",
            r"C:/Users/x/.kube/config",
            r"C:\Users\x\repo\.worktrees\fix\CLAUDE~1\settings.local.json",
            r"C:\Users\x\repo\.worktrees\fix\.claude.\settings.local.json",
        ] {
            assert!(protected(summary), "{summary:?}");
        }
        for summary in [r"C:\Users\x\repo\src\env.ts", r"C:\Users\x\repo\README.md", r"src\claude.rs", r"docs\gh\setup.md"] {
            assert!(!protected(summary), "{summary:?}");
        }
        assert!(request_is_protected("Edit", &json!({"file_path": r"C:\w\.mcp.json"}), w, Platform::Windows));
        let ssh = r"C:\Users\x\.ssh\config";
        assert_eq!(rule_for_on("Edit", ssh, &json!({ "file_path": ssh }), w, Platform::Windows), Err(OnceOnly::Protected));
    }

    #[cfg(windows)]
    #[test]
    fn windows_shells_and_program_extensions_get_no_rule() {
        for (command, kind) in [
            ("git.exe push origin main", OnceOnly::Publish),
            ("GIT.EXE -c alias.x=!make x", OnceOnly::GitConfig),
            ("python.exe -c \"print(1)\"", OnceOnly::InlineCode),
            ("py -c \"print(1)\"", OnceOnly::InlineCode),
            ("py -m timeit pass", OnceOnly::CodeModule),
            ("node.exe -e x", OnceOnly::InlineCode),
            ("env.exe FOO=1 make", OnceOnly::Wrapper),
            ("powershell -Command Remove-Item src", OnceOnly::Wrapper),
            ("pwsh -c Get-ChildItem", OnceOnly::Wrapper),
            ("PowerShell.exe -EncodedCommand ZQBjAGgAbwA=", OnceOnly::Wrapper),
            ("cmd.exe /c dir", OnceOnly::Wrapper),
            ("wsl rm -rf build", OnceOnly::Wrapper),
            ("start notepad", OnceOnly::Wrapper),
            ("runas /user:admin cmd", OnceOnly::Wrapper),
            ("npx.cmd -y create-vite", OnceOnly::PackageRunner),
        ] {
            assert_eq!(command_rule(command), Err(kind), "{command:?}");
        }
        // The Python launcher runs modules like python does.
        assert_eq!(command_rule("py -m pytest -q"), Ok(bash_rule("py -m pytest")));
        // Rules an older daemon could save do not cover them either.
        assert!(!rule_covers("powershell -Command", "powershell -Command Remove-Item src"));
        assert!(!rule_covers("git.exe push", "git.exe push origin main"));
        assert!(!rule_covers("py -c", "py -c \"print(2)\""));
    }

    #[test]
    fn windows_reads_some_names_as_other_ones() {
        for name in [
            "CLAUDE~1",
            "SETTIN~1.JSO",
            ".claude.",
            ".mcp.json.",
            ".ssh ",
            "...",
            "NUL",
            "nul.txt",
            "Con",
            "aux.c",
            "PRN",
            "COM1.log",
            "lpt9",
            "COM\u{b9}",
            "CONOUT$",
            "nul .txt",
        ] {
            assert!(is_windows_alias_name(name), "{name:?}");
        }
        for name in ["", ".", "..", "src", ".claude", ".env", "a.rs", "COM", "COM10", "LPT0", "console", "nullable.rs", "auxiliary", "README.md"] {
            assert!(!is_windows_alias_name(name), "{name:?}");
        }
    }

    /// The Windows path rules, checked on every platform with paths that
    /// use slashes. Windows paths with backslashes are checked on Windows.
    #[test]
    fn windows_path_forms_never_get_or_match_a_rule() {
        let w = "/repo/.worktrees/fix";
        let worktree = Path::new(w);
        let forms = [
            // Short (8.3) names, which can be `.claude`, `.env` or `.mcp.json`.
            format!("{w}/CLAUDE~1/settings.local.json"),
            format!("{w}/MCPJSO~1.JSO"),
            // Names ending in a dot or a space, which Windows drops.
            format!("{w}/.claude./settings.local.json"),
            format!("{w}/.claude /settings.local.json"),
            format!("{w}/.mcp.json."),
            // Data streams.
            format!("{w}/src/a.rs::$DATA"),
            format!("{w}/src/a.rs:hidden"),
            // Devices.
            format!("{w}/src/NUL"),
            format!("{w}/COM1.log"),
            format!("{w}/con.txt"),
            // Prefixes that skip the usual path rules, with either slash.
            r"\\?\C:\repo\.worktrees\fix\src\a.rs".to_string(),
            "//?/C:/repo/.worktrees/fix/src/a.rs".to_string(),
            r"\\.\C:\repo\.worktrees\fix\src\a.rs".to_string(),
            r"\??\C:\repo\.worktrees\fix\src\a.rs".to_string(),
            // A drive with no folder after it: relative to that drive's current folder.
            "D:notes".to_string(),
            "C:src/a.rs".to_string(),
            // Mixed slashes.
            format!("{w}\\CLAUDE~1/settings.local.json"),
            format!("{w}/src\\NUL"),
        ];
        let files = AllowRule { tool: "Write".into(), prefix: None };
        for path in &forms {
            let input = json!({ "file_path": path, "content": "x" });
            assert!(names_another_file(path, worktree, Platform::Windows), "{path:?}");
            // Allowed once only; a path that also reads as a protected one says so.
            let once = rule_for_on("Write", path, &input, worktree, Platform::Windows);
            assert!(matches!(once, Err(OnceOnly::WindowsPath | OnceOnly::Protected)), "{path:?}: {once:?}");
            assert!(!rule_allows_on(&files, "Write", &input, worktree, Platform::Windows), "{path:?}");
            // Unix reads them as they are written.
            assert!(!names_another_file(path, worktree, Platform::Unix), "{path:?}");
        }
        // Ordinary paths keep their rule, and the worktree's own path is not
        // checked. Whether `/repo/...` is absolute depends on the system the
        // test runs on, so only Unix checks that the rule covers them.
        for path in [format!("{w}/src/a.rs"), format!("{w}/./src/a.rs"), format!("{w}/src/a.b.rs")] {
            let input = json!({ "file_path": path, "content": "x" });
            assert_eq!(rule_for_on("Write", &path, &input, worktree, Platform::Windows), Ok(files.clone()), "{path:?}");
            assert_eq!(rule_allows_on(&files, "Write", &input, worktree, Platform::Windows), cfg!(unix), "{path:?}");
        }
        let short = Path::new("/Users/RUNNER~1/repo/.worktrees/fix");
        assert!(!names_another_file("/Users/RUNNER~1/repo/.worktrees/fix/src/a.rs", short, Platform::Windows));
        assert!(names_another_file("/Users/RUNNER~1/repo/.worktrees/fix/CLAUDE~1/x", short, Platform::Windows));
        assert!(names_another_file("C:/x/a.rs:s", worktree, Platform::Windows), "a stream after a drive");
        assert!(!names_another_file("C:/x/a.rs", worktree, Platform::Windows), "a drive with a folder");
    }

    #[test]
    fn windows_command_words_with_a_drive_or_an_alias_get_no_rule() {
        let worktree = Path::new("/repo/.worktrees/fix");
        let rule = bash_rule("cp README.md");
        let covers = |command: &str, platform| rule_allows_on(&rule, "Bash", &json!({ "command": command }), worktree, platform);
        let rule_of = |command: &str, platform| rule_for_on("Bash", command, &json!({ "command": command }), worktree, platform);
        for command in [
            // A drive-relative path is outside the worktree.
            "cp README.md D:notes",
            "cp README.md --target-directory=D:notes",
            "cp README.md C:/repo/.worktrees/fix/x",
            // Streams, names Windows changes, devices and prefixes.
            "cp README.md notes.txt:hidden",
            "cp README.md docs./notes.md",
            "cp README.md notes.md.",
            "cp README.md \"docs /x\"",
            "cp README.md NUL",
            "cp README.md docs/com1.txt",
            "cp README.md //?/C:/repo/x",
        ] {
            assert!(!covers(command, Platform::Windows), "{command:?}");
            assert_eq!(rule_of(command, Platform::Windows), Err(OnceOnly::WindowsPath), "{command:?}");
            // On Unix these are plain names in the worktree.
            if !command.contains("C:/") && !command.contains("//?/") {
                assert!(covers(command, Platform::Unix), "{command:?}");
                assert_eq!(rule_of(command, Platform::Unix), Ok(rule.clone()), "{command:?}");
            }
        }
        // Ordinary commands keep their rule on Windows.
        for command in ["cp README.md docs/notes.txt", "cp README.md ./docs/a.b.txt", "cp README.md \"my dir/a.txt\""] {
            assert!(covers(command, Platform::Windows), "{command:?}");
            assert_eq!(rule_of(command, Platform::Windows), Ok(rule.clone()), "{command:?}");
        }
        // A word Windows reads as another path anywhere in the command, not
        // only in the words the rule keeps, gets no rule.
        assert_eq!(rule_of("cp README.md docs/x README.md.", Platform::Windows), Err(OnceOnly::WindowsPath));
        // One that Windows reads as a protected name is a protected path there.
        for command in ["cp README.md .claude./settings.local.json", "cp README.md .mcp.json.", "cp README.md CLAUDE~1/x"] {
            assert!(!covers(command, Platform::Windows), "{command:?}");
            assert_eq!(rule_of(command, Platform::Windows), Err(OnceOnly::Protected), "{command:?}");
        }
        for command in ["cp README.md .claude./settings.local.json", "cp README.md .mcp.json."] {
            assert!(covers(command, Platform::Unix), "{command:?}");
            assert_eq!(rule_of(command, Platform::Unix), Ok(rule.clone()), "{command:?}");
        }
    }

    /// The same checks with real Windows paths, as the daemon runs them.
    #[cfg(windows)]
    #[test]
    fn windows_paths_with_backslashes_never_get_or_match_a_rule() {
        let files = AllowRule { tool: "Edit".into(), prefix: None };
        for path in [
            format!(r"{W}\CLAUDE~1\settings.local.json"),
            format!(r"{W}\.claude.\settings.local.json"),
            format!(r"{W}\.mcp.json "),
            format!(r"{W}\src\a.rs::$DATA"),
            format!(r"\\?\{W}\src\a.rs"),
            format!(r"{W}\src\CON"),
            format!(r"{W}/CLAUDE~1\settings.local.json"),
            r"C:src\a.rs".to_string(),
        ] {
            let input = json!({ "file_path": path, "old_string": "a", "new_string": "b" });
            let once = rule_of("Edit", input.clone());
            assert!(matches!(once, Err(OnceOnly::WindowsPath | OnceOnly::Protected)), "{path:?}: {once:?}");
            assert!(!rule_allows(&files, "Edit", &input, Path::new(W)), "{path:?}");
        }
        for path in [format!(r"{W}\src\a.rs"), format!("{W}/src/a.rs"), format!(r"{W}\.\src\a.rs")] {
            let input = json!({ "file_path": path, "old_string": "a", "new_string": "b" });
            assert_eq!(rule_of("Edit", input.clone()), Ok(files.clone()), "{path:?}");
            assert!(rule_allows(&files, "Edit", &input, Path::new(W)), "{path:?}");
        }
        assert!(!rule_covers("cp README.md", "cp README.md D:notes"));
        assert_eq!(command_rule("cp README.md D:notes"), Err(OnceOnly::WindowsPath));
    }

    #[test]
    fn powershell_commands_get_no_rule() {
        let rule = rule_of("PowerShell", json!({"command": "Get-ChildItem"}));
        assert_eq!(rule, Err(OnceOnly::PowerShell));
        // A protected path is still said to be one.
        assert_eq!(rule_of("PowerShell", json!({"command": "Get-Content .env"})), Err(OnceOnly::Protected));
        // And no saved rule covers one.
        let saved = AllowRule { tool: "PowerShell".into(), prefix: Some("Get-ChildItem".into()) };
        assert!(!rule_allows(&saved, "PowerShell", &json!({"command": "Get-ChildItem"}), Path::new("/w")));
    }

    #[test]
    fn title_is_the_first_line_trimmed_and_capped() {
        assert_eq!(session_title("  Fix the bug  \nwith details"), "Fix the bug");
        assert_eq!(session_title("\n\n  Second line wins\n"), "Second line wins");
        assert_eq!(session_title(&"é".repeat(100)).chars().count(), 80);
        assert_eq!(session_title(""), "");
    }
}
