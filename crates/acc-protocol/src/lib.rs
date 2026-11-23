use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus { Starting, Working, NeedsYou, Idle, Done, Failed, Stopped }

/// Where a session works. `Project`: in the project's own folder, on its
/// current branch. `Worktree`: in its own worktree on a new `agent/<slug>`
/// branch. Sessions saved before modes existed are `Worktree`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    Project,
    #[default]
    Worktree,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct StatusChange { pub status: SessionStatus, pub at: i64 }

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    pub host: Option<String>,
    pub base_branch: String,
    pub added_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct Session {
    pub id: String,
    pub project_id: String,
    pub agent: String,
    pub title: String,
    /// The folder's branch when the session reported last (`Project` mode),
    /// or the session's own branch (`Worktree` mode).
    pub branch: String,
    /// The folder the session works in: the project folder or its worktree.
    pub worktree: String,
    #[serde(default)]
    pub mode: SessionMode,
    /// The user chose to let this session change files on the base branch,
    /// so it is not asked to make a branch again.
    #[serde(default)]
    pub stay_on_base: bool,
    /// What Claude Code last reported to its status line. `None` until then.
    #[serde(default)]
    pub usage: Option<SessionUsage>,
    pub status: SessionStatus,
    pub status_since: i64,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub exit_code: Option<i32>,
    pub history: Vec<StatusChange>,
}

/// A session's use as Claude Code reports it to its status line.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct SessionUsage {
    /// What the session's use would cost on the API, in US dollars. On a
    /// Claude plan it is an estimate, not a charge.
    pub cost_usd: f64,
    /// How full the context window is, 0 to 100.
    pub context_used_percentage: Option<f64>,
    pub context_window_size: Option<u64>,
    pub lines_added: u64,
    pub lines_removed: u64,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub updated_at: i64,
}

/// One limit window of the Claude plan.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PlanLimit {
    /// 0 to 100.
    pub used_percentage: f64,
    /// When the window resets, in seconds since 1970. `None` when not given.
    pub resets_at: Option<i64>,
}

/// The Claude plan's limits, the same for every session of the account.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct PlanUsage {
    pub five_hour: Option<PlanLimit>,
    pub seven_day: Option<PlanLimit>,
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState { Pending, Allowed, Denied, Cancelled, Expired }

/// `Permission`: Claude Code asks to use a tool. `Branch`: a session in the
/// project folder wants to change something while the folder is on the base
/// branch, and the user can make a branch first.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    #[default]
    Permission,
    Branch,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct Approval {
    pub id: String,
    pub session_id: String,
    pub tool: String,
    pub summary: String,
    pub input: serde_json::Value,
    pub created_at: i64,
    pub state: ApprovalState,
    /// "user" or "policy"
    pub decided_by: Option<String>,
    /// The request touches a protected path, so Always allow allows it once only.
    #[serde(default)]
    pub protected: bool,
    /// What Always allow would add a rule for, such as `` `npm install` commands ``.
    /// `None` when Always allow would allow this request once and add no rule.
    #[serde(default)]
    pub rule_preview: Option<String>,
    #[serde(default)]
    pub kind: ApprovalKind,
    /// For a `Branch` approval: the branch that Make branch would create.
    #[serde(default)]
    pub branch: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    AllowOnce,
    AllowAlways,
    Deny,
    /// A `Branch` approval: make the branch, then allow the request.
    MakeBranch,
    /// A `Branch` approval: allow the request on the base branch, and do not
    /// ask this session again.
    StayOnBase,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct EventRecord {
    pub seq: i64,
    pub ts: i64,
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub kind: String,
    pub actor: String,
    pub payload: serde_json::Value,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind { App, Hook }

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Request {
    Hello { client: ClientKind, version: u32 },
    Snapshot,
    ProjectAdd { path: String },
    SessionStart {
        project_id: String,
        agent: String,
        task: String,
        /// `Project` when left out.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<SessionMode>,
    },
    SessionInput { session_id: String, data_b64: String },
    SessionResize { session_id: String, cols: u16, rows: u16 },
    SessionAttach { session_id: String },
    SessionStop { session_id: String },
    SessionRestart { session_id: String },
    /// Removes a session that is not running from the app. With
    /// `remove_worktree`, a worktree session's worktree and branch go too,
    /// but only when git can remove them without losing work.
    SessionRemove {
        session_id: String,
        #[serde(default)]
        remove_worktree: bool,
    },
    /// Removes a project with no running session from the app, with its
    /// sessions and rules. Its folder stays as it is.
    ProjectRemove { project_id: String },
    /// Memory and disk use of the sessions, and of the app's data.
    Stats,
    ApprovalDecide { approval_id: String, decision: Decision },
    EventsList { session_id: Option<String>, limit: u32 },
    Hook { session_id: String, payload: serde_json::Value },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RequestEnvelope {
    pub id: u64,
    #[serde(flatten)]
    pub request: Request,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum DaemonEvent {
    SessionUpdated(Session),
    /// `offset` is the stream position of the first byte in this run of the
    /// session (each run starts at 0). `session_attach` returns the matching `end`.
    SessionOutput { session_id: String, data_b64: String, offset: u64 },
    ApprovalCreated(Approval),
    ApprovalResolved(Approval),
    ProjectAdded(Project),
    EventAppended(EventRecord),
    PlanUsageUpdated(PlanUsage),
    SessionRemoved { session_id: String },
    ProjectRemoved { project_id: String },
}

/// What removing a session did.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct SessionRemoved {
    pub worktree_removed: bool,
    pub branch_removed: bool,
    /// Why the worktree or the branch was kept, in git's words.
    pub kept: Option<String>,
}

/// One session's use of memory and disk. `None` where it cannot be measured
/// (memory on Windows, a folder that is not the session's own).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct SessionStats {
    pub session_id: String,
    /// Resident memory of Claude Code and everything it started.
    pub memory_bytes: Option<u64>,
    pub processes: u32,
    /// Terminal output the daemon keeps in memory.
    pub scrollback_bytes: u64,
    /// Claude Code's conversation file.
    pub transcript_bytes: Option<u64>,
    /// The session's own worktree. `None` for a session in the project folder.
    pub folder_bytes: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Stats {
    /// `acc.db` with its write-ahead log.
    pub db_bytes: u64,
    /// The whole data folder, the database included.
    pub data_bytes: u64,
    pub sessions: Vec<SessionStats>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Response { id: u64, result: serde_json::Value },
    Error { id: u64, code: String, message: String },
    Event { event: DaemonEvent },
}

/// Approvals hold pending ones only.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub projects: Vec<Project>,
    pub sessions: Vec<Session>,
    pub approvals: Vec<Approval>,
    /// `None` until a session has reported the plan's limits.
    #[serde(default)]
    pub plan_usage: Option<PlanUsage>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HookReply {
    pub stdout: Option<String>,
    pub exit_code: i32,
}

/// Compact JSON followed by a newline.
pub fn encode_line<T: Serialize>(msg: &T) -> Vec<u8> {
    let mut v = serde_json::to_vec(msg).expect("protocol types serialize");
    v.push(b'\n');
    v
}

/// Claude Code PermissionRequest hook reply (stdout of acc-hook).
pub fn permission_reply(allow: bool, message: Option<&str>) -> String {
    let decision = if allow {
        serde_json::json!({"behavior": "allow"})
    } else {
        match message {
            Some(m) => serde_json::json!({"behavior": "deny", "message": m}),
            None => serde_json::json!({"behavior": "deny"}),
        }
    };
    serde_json::json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": decision}}).to_string()
}

/// Claude Code PreToolUse hook reply (stdout of acc-hook): `allow` skips the
/// permission prompt for this call, `deny` blocks it with `reason`, and `ask`
/// makes Claude Code ask, which reaches the inbox as a PermissionRequest.
pub fn pre_tool_reply(decision: &str, reason: Option<&str>) -> String {
    let mut output = serde_json::json!({"hookEventName": "PreToolUse", "permissionDecision": decision});
    if let Some(reason) = reason {
        output["permissionDecisionReason"] = serde_json::json!(reason);
    }
    serde_json::json!({"hookSpecificOutput": output}).to_string()
}

/// `env_value` (`$ACC_DATA_DIR`) if set, else `AgentControlCenter` in the
/// platform's data folder: `~/Library/Application Support` on macOS,
/// `~/.local/share` on Linux and `%LOCALAPPDATA%` on Windows, which stays on
/// this machine (the roaming one is copied between machines).
pub fn data_dir_from(env_value: Option<OsString>) -> PathBuf {
    let base = if cfg!(windows) { dirs::data_local_dir() } else { dirs::data_dir() };
    match env_value {
        Some(v) => PathBuf::from(v),
        None => base.unwrap_or_else(std::env::temp_dir).join("AgentControlCenter"),
    }
}

pub fn data_dir() -> PathBuf { data_dir_from(std::env::var_os("ACC_DATA_DIR")) }

/// Where the daemon serving `data_dir` listens: `accd.sock` in it, a Unix
/// socket.
#[cfg(unix)]
pub fn socket_path(data_dir: &Path) -> PathBuf { data_dir.join("accd.sock") }

/// Where the daemon serving `data_dir` listens: a named pipe whose name is
/// made from the data dir, so each data dir has its own. Windows ignores case
/// and takes either slash in a path, so the name does too.
#[cfg(windows)]
pub fn socket_path(data_dir: &Path) -> PathBuf {
    let dir = data_dir.to_string_lossy().replace('/', "\\").to_lowercase();
    // FNV-1a, which gives the same name in every build.
    let hash = dir.trim_end_matches('\\').bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    PathBuf::from(format!(r"\\.\pipe\agent-control-center-{hash:016x}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test] fn request_envelope_shape() {
        let r = RequestEnvelope { id: 7, request: Request::SessionStart { project_id: "p1".into(), agent: "claude".into(), task: "Fix bug".into(), mode: None } };
        assert_eq!(serde_json::to_value(&r).unwrap(), serde_json::json!({"id":7,"method":"session_start","params":{"project_id":"p1","agent":"claude","task":"Fix bug"}}));
    }
    #[test] fn unit_request_has_no_params() {
        let r: RequestEnvelope = serde_json::from_str(r#"{"id":1,"method":"snapshot"}"#).unwrap();
        assert_eq!(r.request, Request::Snapshot);
    }
    #[test] fn needs_you_is_snake_case() { assert_eq!(serde_json::to_string(&SessionStatus::NeedsYou).unwrap(), "\"needs_you\""); }
    #[test] fn event_message_shape() {
        let m = ServerMsg::Event { event: DaemonEvent::SessionOutput { session_id: "s".into(), data_b64: "aGk=".into(), offset: 4096 } };
        assert_eq!(serde_json::to_value(&m).unwrap(), serde_json::json!({"type":"event","event":{"kind":"session_output","data":{"session_id":"s","data_b64":"aGk=","offset":4096}}}));
    }
    #[test] fn encode_line_ends_with_newline() { assert!(encode_line(&Request::Snapshot).ends_with(b"\n")); }
    #[test] fn data_dir_prefers_env_value() {
        assert_eq!(data_dir_from(Some("/tmp/acc test".into())), PathBuf::from("/tmp/acc test"));
        assert!(data_dir_from(None).ends_with("AgentControlCenter"));
    }
    #[cfg(unix)]
    #[test] fn socket_is_in_the_data_dir() {
        assert_eq!(socket_path(Path::new("/d")), PathBuf::from("/d/accd.sock"));
    }
    #[cfg(windows)]
    #[test] fn data_dir_is_in_local_app_data() {
        assert_eq!(data_dir_from(None), dirs::data_local_dir().unwrap().join("AgentControlCenter"));
    }
    #[cfg(windows)]
    #[test] fn each_data_dir_has_its_own_pipe() {
        let pipe = socket_path(Path::new(r"C:\Users\x\AppData\Local\AgentControlCenter"));
        let name = pipe.to_str().unwrap();
        assert!(name.starts_with(r"\\.\pipe\agent-control-center-"), "{name}");
        assert_eq!(name.len(), r"\\.\pipe\agent-control-center-".len() + 16, "{name}");
        // The same folder written another way is the same pipe.
        for same in [r"c:\users\x\appdata\local\agentcontrolcenter", "C:/Users/x/AppData/Local/AgentControlCenter/"] {
            assert_eq!(socket_path(Path::new(same)), pipe, "{same}");
        }
        assert_ne!(socket_path(Path::new(r"C:\Users\y\AppData\Local\AgentControlCenter")), pipe);
    }
    #[test] fn approval_carries_protected_and_rule_preview() {
        let a = Approval {
            id: "a1".into(), session_id: "s1".into(), tool: "Bash".into(), summary: "npm install x".into(),
            input: serde_json::json!({"command": "npm install x"}), created_at: 5, state: ApprovalState::Pending,
            decided_by: None, protected: false, rule_preview: Some("`npm install` commands".into()),
            kind: ApprovalKind::Permission, branch: None,
        };
        let v = serde_json::to_value(&a).unwrap();
        assert_eq!((&v["protected"], &v["rule_preview"]), (&serde_json::json!(false), &serde_json::json!("`npm install` commands")));
        let once = Approval { protected: true, rule_preview: None, ..a };
        assert_eq!(serde_json::to_value(&once).unwrap()["rule_preview"], serde_json::Value::Null);
    }
    #[test] fn approval_from_an_older_daemon_decodes_with_defaults() {
        let old = r#"{"id":"a1","session_id":"s1","tool":"Bash","summary":"ls","input":{"command":"ls"},"created_at":5,"state":"pending","decided_by":null}"#;
        let a: Approval = serde_json::from_str(old).unwrap();
        assert_eq!((a.protected, a.rule_preview), (false, None));
        assert_eq!((a.kind, a.branch), (ApprovalKind::Permission, None));
    }
    #[test] fn session_from_an_older_daemon_is_a_worktree_session() {
        let old = r#"{"id":"s1","project_id":"p1","agent":"claude","title":"t","branch":"agent/t","worktree":"/r/.worktrees/t","status":"idle","status_since":1,"started_at":1,"ended_at":null,"exit_code":null,"history":[]}"#;
        let s: Session = serde_json::from_str(old).unwrap();
        assert_eq!((s.mode, s.stay_on_base), (SessionMode::Worktree, false));
    }
    #[test] fn session_start_mode_is_optional() {
        let r: RequestEnvelope = serde_json::from_str(r#"{"id":1,"method":"session_start","params":{"project_id":"p","agent":"claude","task":"t"}}"#).unwrap();
        assert_eq!(r.request, Request::SessionStart { project_id: "p".into(), agent: "claude".into(), task: "t".into(), mode: None });
        let r: RequestEnvelope = serde_json::from_str(r#"{"id":1,"method":"session_start","params":{"project_id":"p","agent":"claude","task":"t","mode":"project"}}"#).unwrap();
        assert!(matches!(r.request, Request::SessionStart { mode: Some(SessionMode::Project), .. }));
    }
    #[test] fn branch_decisions_are_snake_case() {
        assert_eq!(serde_json::to_string(&Decision::MakeBranch).unwrap(), "\"make_branch\"");
        assert_eq!(serde_json::to_string(&Decision::StayOnBase).unwrap(), "\"stay_on_base\"");
        assert_eq!(serde_json::to_string(&ApprovalKind::Branch).unwrap(), "\"branch\"");
    }
    #[test] fn pre_tool_reply_shapes() {
        assert_eq!(pre_tool_reply("allow", None), r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow"}}"#);
        assert_eq!(
            pre_tool_reply("deny", Some("no")),
            r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"no"}}"#
        );
    }
    #[test] fn permission_reply_allow_and_deny() {
        assert_eq!(permission_reply(true, None), r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#);
        assert_eq!(permission_reply(false, Some("no")), r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"no"}}}"#);
    }
}
