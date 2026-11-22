use acc_protocol::SessionStatus;
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub struct ClaudeAdapter {
    pub program: PathBuf,
}

/// `$ACC_CLAUDE_BIN` if set and the file exists, else `claude` on the
/// daemon's PATH, then on `search_path` (the session PATH, led by the login
/// shell's, which an app opened from Finder does not have), then in one of
/// the [`install_locations`]. Only an executable file counts there.
pub fn detect(search_path: Option<&str>) -> Option<ClaudeAdapter> {
    if let Ok(p) = std::env::var("ACC_CLAUDE_BIN") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(ClaudeAdapter { program: p });
        }
    }
    let program = which::which("claude")
        .ok()
        .or_else(|| find_claude(search_path, dirs::home_dir().as_deref(), is_executable))?;
    Some(ClaudeAdapter { program })
}

/// Where Claude Code's installers put `claude`, in the order they are tried:
/// `~/.local/bin` (the native installer), `~/.claude/local` (the older local
/// install), then Homebrew's folders on Apple Silicon and on Intel.
pub fn install_locations(home: Option<&Path>) -> Vec<PathBuf> {
    let in_home = home.into_iter().flat_map(|home| [home.join(".local/bin/claude"), home.join(".claude/local/claude")]);
    in_home.chain(["/opt/homebrew/bin/claude", "/usr/local/bin/claude"].map(PathBuf::from)).collect()
}

/// `claude` in a folder of `search_path` (absolute folders only), else in
/// one of the [`install_locations`]: the first that `is_executable` accepts.
pub fn find_claude(search_path: Option<&str>, home: Option<&Path>, is_executable: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let on_path = search_path.into_iter().flat_map(|path| path.split(':')).filter(|dir| dir.starts_with('/'));
    on_path.map(|dir| Path::new(dir).join("claude")).chain(install_locations(home)).find(|candidate| is_executable(candidate))
}

/// Whether `path` is a file with an execute bit.
pub fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Set by a running Claude Code in the environment of the commands it runs.
/// An agent that inherits them (the app was started from a Claude Code
/// session) runs as a session nested in that one, so they are removed from
/// its environment. `CLAUDE_CODE_CHILD_SESSION` turns transcript saving off,
/// so `--resume` would not find the session. Apart from these and
/// [`PARENT_SESSION_ENV`], everything is passed on, such as the user's own
/// `CLAUDE_*` settings.
pub const NESTED_SESSION_MARKERS: [&str; 4] =
    ["CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_SSE_PORT", "CLAUDE_CODE_CHILD_SESSION"];

/// The parent agent session's own channels: its messaging socket and token,
/// and its notes folder. They belong to that session, and its token must
/// never reach another agent.
pub const PARENT_SESSION_ENV: [&str; 3] =
    ["CLAUDE_CODE_MESSAGING_SOCKET", "CLAUDE_CODE_MESSAGING_TOKEN", "CLAUDE_CODE_CHILD_NOTES_DIR"];

/// Every variable removed from the agent's inherited environment.
pub fn removed_env() -> Vec<String> {
    NESTED_SESSION_MARKERS.iter().chain(&PARENT_SESSION_ENV).map(|name| name.to_string()).collect()
}

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

const HOOK_EVENTS: [&str; 8] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "Notification",
    "Stop",
    "SessionEnd",
];

/// Writes `dir/claude-settings.json` (creating `dir`) and returns its path.
pub fn write_settings(dir: &Path, hook_bin: &Path) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let command = shell_quote(&hook_bin.to_string_lossy());
    let mut hooks = serde_json::Map::new();
    for ev in HOOK_EVENTS {
        let mut entry = json!({"type": "command", "command": command});
        if ev == "PermissionRequest" {
            entry["timeout"] = json!(3600);
        }
        hooks.insert(ev.to_string(), json!([{ "hooks": [entry] }]));
    }
    let path = dir.join("claude-settings.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&json!({ "hooks": hooks }))?)?;
    Ok(path)
}

pub fn launch_args(settings: &Path, session_id: &str, task: &str) -> Vec<String> {
    vec![
        "--settings".into(),
        settings.to_string_lossy().into_owned(),
        "--session-id".into(),
        session_id.into(),
        task.into(),
    ]
}

pub fn resume_args(settings: &Path, session_id: &str) -> Vec<String> {
    vec![
        "--settings".into(),
        settings.to_string_lossy().into_owned(),
        "--resume".into(),
        session_id.into(),
    ]
}

#[derive(Debug, Clone, PartialEq)]
pub enum HookEffect {
    Status(SessionStatus),
    Approval { tool: String, summary: String, input: Value },
    Activity(String),
    Nothing,
}

pub fn interpret_hook(payload: &Value) -> HookEffect {
    let Some(event) = payload.get("hook_event_name").and_then(Value::as_str) else {
        return HookEffect::Nothing;
    };
    let tool = payload.get("tool_name").and_then(Value::as_str);
    let input = payload.get("tool_input").cloned().unwrap_or(Value::Null);
    match event {
        "UserPromptSubmit" | "PreToolUse" => HookEffect::Status(SessionStatus::Working),
        "PermissionRequest" => {
            let tool = tool.unwrap_or("unknown");
            HookEffect::Approval {
                tool: tool.to_string(),
                summary: summarize_tool(tool, &input),
                input,
            }
        }
        "PostToolUse" => HookEffect::Activity(summarize_tool(tool.unwrap_or("unknown"), &input)),
        "Stop" => HookEffect::Status(SessionStatus::Idle),
        "Notification"
            if payload.get("notification_type").and_then(Value::as_str) == Some("idle_prompt") =>
        {
            HookEffect::Status(SessionStatus::Idle)
        }
        _ => HookEffect::Nothing,
    }
}

pub fn summarize_tool(tool: &str, input: &Value) -> String {
    let field = match tool {
        "Bash" => "command",
        "Edit" | "Write" | "Read" | "MultiEdit" => "file_path",
        "WebFetch" => "url",
        _ => return tool.to_string(),
    };
    input.get(field).and_then(Value::as_str).unwrap_or(tool).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use acc_protocol::SessionStatus;
    use serde_json::json;
    use std::path::Path;

    #[test]
    fn launch_args_order() {
        assert_eq!(
            launch_args(Path::new("/d/s.json"), "abc", "Fix the bug"),
            vec!["--settings", "/d/s.json", "--session-id", "abc", "Fix the bug"]
        );
    }

    #[test]
    fn resume_args_order() {
        assert_eq!(
            resume_args(Path::new("/d/s.json"), "abc"),
            vec!["--settings", "/d/s.json", "--resume", "abc"]
        );
    }

    #[test]
    fn settings_have_all_hooks_and_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_settings(&dir.path().join("a/b"), Path::new("/bin/acc-hook")).unwrap();
        assert_eq!(p.file_name().unwrap(), "claude-settings.json");
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        for ev in ["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "PermissionRequest", "Notification", "Stop", "SessionEnd"] {
            assert_eq!(v["hooks"][ev][0]["hooks"][0]["type"], "command", "{ev}");
            assert_eq!(v["hooks"][ev][0]["hooks"][0]["command"], "'/bin/acc-hook'", "{ev}");
        }
        assert_eq!(v["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"], 3600);
        assert!(v["hooks"]["Stop"][0]["hooks"][0].get("timeout").is_none());
    }

    #[test]
    fn settings_quote_hook_path_with_spaces() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_settings(dir.path(), Path::new("/a b/acc-hook")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], "'/a b/acc-hook'");
    }

    #[test]
    fn shell_quote_escapes_single_quote() {
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn interpret_permission_request() {
        let p = json!({"hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"npm install x"}});
        assert_eq!(
            interpret_hook(&p),
            HookEffect::Approval {
                tool: "Bash".into(),
                summary: "npm install x".into(),
                input: json!({"command":"npm install x"})
            }
        );
    }

    #[test]
    fn interpret_stop_and_idle() {
        assert_eq!(interpret_hook(&json!({"hook_event_name":"Stop"})), HookEffect::Status(SessionStatus::Idle));
        assert_eq!(
            interpret_hook(&json!({"hook_event_name":"Notification","notification_type":"idle_prompt"})),
            HookEffect::Status(SessionStatus::Idle)
        );
        assert_eq!(
            interpret_hook(&json!({"hook_event_name":"Notification","notification_type":"other"})),
            HookEffect::Nothing
        );
        assert_eq!(interpret_hook(&json!({"hook_event_name":"UserPromptSubmit"})), HookEffect::Status(SessionStatus::Working));
        assert_eq!(interpret_hook(&json!({"hook_event_name":"PreToolUse"})), HookEffect::Status(SessionStatus::Working));
    }

    #[test]
    fn interpret_post_tool_use_is_activity() {
        let p = json!({"hook_event_name":"PostToolUse","tool_name":"Read","tool_input":{"file_path":"/x.rs"}});
        assert_eq!(interpret_hook(&p), HookEffect::Activity("/x.rs".into()));
    }

    #[test]
    fn interpret_unknown_is_nothing() {
        assert_eq!(interpret_hook(&json!({"hook_event_name":"SessionStart"})), HookEffect::Nothing);
        assert_eq!(interpret_hook(&json!({"hook_event_name":"Whatever"})), HookEffect::Nothing);
    }

    #[test]
    fn malformed_payloads_do_not_panic() {
        assert_eq!(interpret_hook(&json!(null)), HookEffect::Nothing);
        assert_eq!(interpret_hook(&json!({})), HookEffect::Nothing);
        assert_eq!(interpret_hook(&json!({"hook_event_name":5})), HookEffect::Nothing);
        assert_eq!(
            interpret_hook(&json!({"hook_event_name":"PermissionRequest"})),
            HookEffect::Approval { tool: "unknown".into(), summary: "unknown".into(), input: serde_json::Value::Null }
        );
        assert_eq!(summarize_tool("Bash", &json!({"command":5})), "Bash");
        assert_eq!(summarize_tool("Edit", &json!(null)), "Edit");
    }

    #[test]
    fn install_locations_come_in_order() {
        let locations = ["/h/.local/bin/claude", "/h/.claude/local/claude", "/opt/homebrew/bin/claude", "/usr/local/bin/claude"];
        assert_eq!(install_locations(Some(Path::new("/h"))), locations.map(PathBuf::from));
        // Without a home folder, only the Homebrew ones.
        assert_eq!(install_locations(None), locations[2..].iter().map(PathBuf::from).collect::<Vec<_>>());
    }

    #[test]
    fn find_claude_tries_the_path_then_the_install_locations() {
        let home = Some(Path::new("/h"));
        // Nothing is executable: every place is tried once, in order. Relative
        // and empty PATH entries are skipped.
        let tried = std::cell::RefCell::new(Vec::new());
        let found = find_claude(Some("/a:relative::/b/"), home, |path| {
            tried.borrow_mut().push(path.to_path_buf());
            false
        });
        assert_eq!(found, None);
        let expected =
            ["/a/claude", "/b/claude", "/h/.local/bin/claude", "/h/.claude/local/claude", "/opt/homebrew/bin/claude", "/usr/local/bin/claude"];
        assert_eq!(tried.into_inner(), expected.map(PathBuf::from));
        // The first executable one wins.
        let only = |wanted: &'static [&'static str]| move |path: &Path| wanted.iter().any(|w| path == Path::new(w));
        assert_eq!(find_claude(Some("/a:/b"), home, only(&["/b/claude", "/opt/homebrew/bin/claude"])), Some("/b/claude".into()));
        assert_eq!(find_claude(Some("/a"), home, only(&["/h/.claude/local/claude"])), Some("/h/.claude/local/claude".into()));
        assert_eq!(find_claude(None, None, only(&["/usr/local/bin/claude"])), Some("/usr/local/bin/claude".into()));
    }

    #[test]
    fn find_claude_needs_an_executable_file() {
        let home = tempfile::tempdir().unwrap();
        let write = |path: &Path, mode: u32| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        // On the PATH but not executable, then a folder in ~/.local/bin, then
        // an executable file in ~/.claude/local, so the real system folders
        // after it are never looked at.
        let bin = home.path().join("bin");
        write(&bin.join("claude"), 0o644);
        std::fs::create_dir_all(home.path().join(".local/bin/claude")).unwrap();
        let installed = home.path().join(".claude/local/claude");
        write(&installed, 0o755);
        assert_eq!(find_claude(bin.to_str(), Some(home.path()), is_executable), Some(installed));
    }

    #[test]
    fn executable_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x");
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!is_executable(&file));
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(is_executable(&file));
        assert!(!is_executable(dir.path()));
        assert!(!is_executable(&dir.path().join("missing")));
    }

    #[test]
    fn summarize_edit_uses_file_path() {
        assert_eq!(summarize_tool("Edit", &json!({"file_path":"/a.rs"})), "/a.rs");
        assert_eq!(summarize_tool("WebFetch", &json!({"url":"http://x"})), "http://x");
        assert_eq!(summarize_tool("Grep", &json!({"pattern":"x"})), "Grep");
    }
}
