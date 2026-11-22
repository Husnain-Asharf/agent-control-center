use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus { Starting, Working, NeedsYou, Idle, Done, Failed, Stopped }

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
    pub branch: String,
    pub worktree: String,
    pub status: SessionStatus,
    pub status_since: i64,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub exit_code: Option<i32>,
    pub history: Vec<StatusChange>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState { Pending, Allowed, Denied, Cancelled, Expired }

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
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Decision { AllowOnce, AllowAlways, Deny }

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
    SessionStart { project_id: String, agent: String, task: String },
    SessionInput { session_id: String, data_b64: String },
    SessionResize { session_id: String, cols: u16, rows: u16 },
    SessionAttach { session_id: String },
    SessionStop { session_id: String },
    SessionRestart { session_id: String },
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
        let r = RequestEnvelope { id: 7, request: Request::SessionStart { project_id: "p1".into(), agent: "claude".into(), task: "Fix bug".into() } };
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
    }
    #[test] fn permission_reply_allow_and_deny() {
        assert_eq!(permission_reply(true, None), r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#);
        assert_eq!(permission_reply(false, Some("no")), r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"no"}}}"#);
    }
}
