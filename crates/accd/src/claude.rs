use acc_protocol::{PlanLimit, PlanUsage, SessionMode, SessionStatus, SessionUsage};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub struct ClaudeAdapter {
    pub program: PathBuf,
}

/// `$ACC_CLAUDE_BIN` if set and the file exists, else `claude` on the
/// daemon's PATH, then on `search_path` (the session PATH, led by the login
/// shell's, which an app opened from Finder or a Linux desktop may not
/// have), then in one of the [`install_locations`]. Only an executable file
/// counts there.
#[cfg(unix)]
pub fn detect(search_path: Option<&str>) -> Option<ClaudeAdapter> {
    if let Some(program) = claude_bin_env() {
        return Some(ClaudeAdapter { program });
    }
    let program = which::which("claude")
        .ok()
        .or_else(|| find_claude(search_path, dirs::home_dir().as_deref(), is_executable))?;
    Some(ClaudeAdapter { program })
}

/// `$ACC_CLAUDE_BIN` if set and the file exists, else `claude` with an
/// extension of `PATHEXT` in a folder of `search_path` (the session PATH,
/// which is the daemon's), then in one of the [`install_locations`].
#[cfg(windows)]
pub fn detect(search_path: Option<&str>) -> Option<ClaudeAdapter> {
    if let Some(program) = claude_bin_env() {
        return Some(ClaudeAdapter { program });
    }
    let inherited = std::env::var("PATH").ok();
    let names = claude_names(std::env::var("PATHEXT").ok().as_deref());
    let app_data = std::env::var_os("APPDATA").map(PathBuf::from);
    let home = dirs::home_dir();
    let program = find_claude(search_path.or(inherited.as_deref()), &names, home.as_deref(), app_data.as_deref(), is_executable)?;
    Some(ClaudeAdapter { program })
}

fn claude_bin_env() -> Option<PathBuf> {
    std::env::var_os("ACC_CLAUDE_BIN").map(PathBuf::from).filter(|p| p.is_file())
}

/// Where Claude Code's installers put `claude`, in the order they are tried:
/// `~/.local/bin` (the native installer), `~/.claude/local` (the older local
/// install), `~/.npm-global/bin` (npm with a user prefix), Homebrew's
/// folders on Apple Silicon and on Intel (`/usr/local/bin` is also where npm
/// with a system Node puts it on Linux), then `/usr/bin` (a distro or
/// NodeSource Node on Linux).
#[cfg(unix)]
pub fn install_locations(home: Option<&Path>) -> Vec<PathBuf> {
    let in_home = home.into_iter().flat_map(|home| {
        [home.join(".local/bin/claude"), home.join(".claude/local/claude"), home.join(".npm-global/bin/claude")]
    });
    let system = ["/opt/homebrew/bin/claude", "/usr/local/bin/claude", "/usr/bin/claude"];
    in_home.chain(system.map(PathBuf::from)).collect()
}

/// `claude` in a folder of `search_path` (absolute folders only), else in
/// one of the [`install_locations`]: the first that `is_executable` accepts.
#[cfg(unix)]
pub fn find_claude(search_path: Option<&str>, home: Option<&Path>, is_executable: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let on_path = search_path.into_iter().flat_map(|path| path.split(':')).filter(|dir| dir.starts_with('/'));
    on_path.map(|dir| Path::new(dir).join("claude")).chain(install_locations(home)).find(|candidate| is_executable(candidate))
}

/// The names `claude` can have on Windows: `claude` with each extension of
/// `pathext` (the `PATHEXT` variable, `.COM;.EXE;.BAT;.CMD` when it is not
/// set) that Windows can start, in that order, as cmd looks for a program:
/// the native `claude.exe`, and the batch files npm and other package
/// managers make.
#[cfg(windows)]
pub fn claude_names(pathext: Option<&str>) -> Vec<String> {
    let pathext = pathext.filter(|pathext| !pathext.trim().is_empty()).unwrap_or(".COM;.EXE;.BAT;.CMD");
    let runnable = |ext: &&str| [".com", ".exe", ".bat", ".cmd"].iter().any(|known| ext.eq_ignore_ascii_case(known));
    let mut names: Vec<String> = Vec::new();
    for ext in pathext.split(';').map(str::trim).filter(runnable) {
        let name = format!("claude{}", ext.to_ascii_lowercase());
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// Where Claude Code's installers put it on Windows, in the order they are
/// tried: `%USERPROFILE%\.local\bin\claude.exe` (the native installer), then
/// `%APPDATA%\npm\claude.cmd` (npm).
#[cfg(windows)]
pub fn install_locations(home: Option<&Path>, app_data: Option<&Path>) -> Vec<PathBuf> {
    let native = home.map(|home| home.join(r".local\bin\claude.exe"));
    let npm = app_data.map(|app_data| app_data.join(r"npm\claude.cmd"));
    native.into_iter().chain(npm).collect()
}

/// One of the `names` (see [`claude_names`]) in a folder of `search_path`
/// (absolute folders only; a quoted one may hold a `;`), else one of the
/// [`install_locations`]: the first that `is_executable` accepts.
#[cfg(windows)]
pub fn find_claude(
    search_path: Option<&str>,
    names: &[String],
    home: Option<&Path>,
    app_data: Option<&Path>,
    is_executable: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let dirs = search_path.into_iter().flat_map(std::env::split_paths).filter(|dir| dir.is_absolute());
    let on_path = dirs.flat_map(|dir| names.iter().map(move |name| dir.join(name)));
    on_path.chain(install_locations(home, app_data)).find(|candidate| is_executable(candidate))
}

/// Whether `path` is a file with an execute bit.
#[cfg(unix)]
pub fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Whether `path` is a file Windows can start: a program (`.exe`, `.com`)
/// or a batch file (`.cmd`, `.bat`), which runs through cmd.exe.
#[cfg(windows)]
pub fn is_executable(path: &Path) -> bool {
    let runnable = |ext: &str| ["exe", "com", "cmd", "bat"].iter().any(|known| ext.eq_ignore_ascii_case(known));
    path.extension().and_then(|ext| ext.to_str()).is_some_and(runnable) && path.is_file()
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

#[cfg(unix)]
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

/// How Claude Code runs `hook_bin`. On macOS and Linux it runs the command
/// with `sh -c`, so the path is quoted for the shell.
#[cfg(unix)]
fn hook_command(hook_bin: &Path) -> Value {
    json!({"type": "command", "command": shell_quote(&hook_bin.to_string_lossy())})
}

/// How Claude Code runs `hook_bin`. On Windows a hook with `args` runs the
/// program directly, with no shell (Claude Code 2.1.139 or later), so a path
/// with spaces or backslashes needs no quoting, and it works whether Claude
/// Code has Git Bash or PowerShell.
#[cfg(windows)]
fn hook_command(hook_bin: &Path) -> Value {
    json!({"type": "command", "command": hook_bin.to_string_lossy(), "args": []})
}

/// Where [`write_settings`] writes a session's settings in `dir`.
pub fn settings_path(dir: &Path) -> PathBuf {
    dir.join("claude-settings.json")
}

/// Writes `dir/claude-settings.json` (creating `dir`) and returns its path.
pub fn write_settings(dir: &Path, hook_bin: &Path) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let mut hooks = serde_json::Map::new();
    for ev in HOOK_EVENTS {
        let mut entry = hook_command(hook_bin);
        // These two can wait for the user: a PermissionRequest for its
        // approval, a PreToolUse for the branch question.
        if ev == "PermissionRequest" || ev == "PreToolUse" {
            entry["timeout"] = json!(3600);
        }
        hooks.insert(ev.to_string(), json!([{ "hooks": [entry] }]));
    }
    // The status line reports the session's cost, context and the plan's
    // limits on each update. acc-hook passes them to the daemon and prints
    // the line it gets back.
    // The session's own tools (start a helper, list and message the sessions
    // in its folder). Listing and messaging need no approval; starting a
    // helper asks, as any other tool does.
    let permissions = json!({ "allow": ["mcp__acc__list_sessions", "mcp__acc__send_message"] });
    let mut status_line = hook_command(hook_bin);
    status_line["refreshInterval"] = json!(STATUS_REFRESH_SECS);
    let path = settings_path(dir);
    std::fs::write(&path, serde_json::to_vec_pretty(&json!({ "hooks": hooks, "statusLine": status_line, "permissions": permissions }))?)?;
    let mcp = json!({ "mcpServers": { "acc": { "type": "stdio", "command": hook_bin.to_string_lossy(), "args": ["mcp"] } } });
    std::fs::write(mcp_path(dir), serde_json::to_vec_pretty(&mcp)?)?;
    Ok(path)
}

/// The setting sources of a session in the project folder: the user's and
/// the project's settings, but not the checkout's untracked
/// `.claude/settings.local.json`, whose saved rules would let commands run
/// without a PermissionRequest, so the inbox would never see them. A new
/// worktree has no such file, so worktree sessions keep Claude Code's default.
const PROJECT_SETTING_SOURCES: &str = "user,project";

fn setting_sources(mode: SessionMode) -> Vec<String> {
    match mode {
        SessionMode::Project => vec!["--setting-sources".into(), PROJECT_SETTING_SOURCES.into()],
        SessionMode::Worktree => Vec::new(),
    }
}

/// How often Claude Code runs the status line while nothing else updates it.
const STATUS_REFRESH_SECS: u64 = 30;

/// Whether a payload is Claude Code's status line input rather than a hook
/// event: it names no hook event, or `Status`, and has the session's model
/// and workspace.
fn is_status_line(payload: &Value) -> bool {
    let event = payload.get("hook_event_name").and_then(Value::as_str);
    matches!(event, None | Some("Status")) && payload.get("model").is_some() && payload.get("workspace").is_some()
}

/// The session's use, and the plan's limits when the payload has them.
pub fn parse_status(payload: &Value, now: i64) -> (SessionUsage, Option<PlanUsage>) {
    let num = |v: &Value, path: &[&str]| path.iter().try_fold(v, |v, key| v.get(key)).and_then(Value::as_f64);
    let text = |path: &[&str]| path.iter().try_fold(payload, |v, key| v.get(key)).and_then(Value::as_str).map(str::to_string);
    let usage = SessionUsage {
        cost_usd: num(payload, &["cost", "total_cost_usd"]).unwrap_or(0.0),
        context_used_percentage: num(payload, &["context_window", "used_percentage"]),
        context_window_size: num(payload, &["context_window", "context_window_size"]).map(|n| n as u64),
        lines_added: num(payload, &["cost", "total_lines_added"]).unwrap_or(0.0) as u64,
        lines_removed: num(payload, &["cost", "total_lines_removed"]).unwrap_or(0.0) as u64,
        model: text(&["model", "display_name"]).or_else(|| text(&["model", "id"])),
        effort: text(&["effort", "level"]),
        updated_at: now,
    };
    let limit = |name: &str| {
        let window = payload.get("rate_limits")?.get(name)?;
        Some(PlanLimit {
            used_percentage: window.get("used_percentage")?.as_f64()?,
            resets_at: window.get("resets_at").and_then(Value::as_i64),
        })
    };
    let (five_hour, seven_day) = (limit("five_hour"), limit("seven_day"));
    let plan = (five_hour.is_some() || seven_day.is_some()).then_some(PlanUsage { five_hour, seven_day, updated_at: now });
    (usage, plan)
}

/// The SessionStart hook reply that adds `context` to the session's start.
pub fn session_start_reply(context: &str) -> String {
    json!({"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": context}}).to_string()
}

/// The line shown at the bottom of the session's terminal.
pub fn status_text(usage: &SessionUsage, plan: Option<&PlanUsage>) -> String {
    let mut parts = Vec::new();
    if let Some(model) = &usage.model {
        parts.push(model.clone());
    }
    if let Some(used) = usage.context_used_percentage {
        parts.push(format!("context {used:.0}%"));
    }
    parts.push(format!("${:.2}", usage.cost_usd));
    if let Some(window) = plan.and_then(|p| p.five_hour.as_ref()) {
        parts.push(format!("5h {:.0}%", window.used_percentage));
    }
    parts.join(" · ")
}

/// `prompt`, when given, is Claude's first message. Without it Claude starts
/// and waits for the user to type.
/// Where [`write_settings`] writes the session's tool server config, next to the settings.
pub fn mcp_path(dir: &Path) -> PathBuf {
    dir.join("mcp.json")
}

/// `--mcp-config` takes several values, so it goes before options that end
/// it, never right before the prompt.
fn mcp_args(settings: &Path) -> Vec<String> {
    let dir = settings.parent().unwrap_or(Path::new("."));
    vec!["--mcp-config".into(), mcp_path(dir).to_string_lossy().into_owned()]
}

pub fn launch_args(settings: &Path, session_id: &str, prompt: Option<&str>, mode: SessionMode) -> Vec<String> {
    let mut args = vec!["--settings".into(), settings.to_string_lossy().into_owned()];
    args.extend(mcp_args(settings));
    args.extend(["--session-id".to_string(), session_id.to_string()]);
    args.extend(setting_sources(mode));
    args.extend(prompt.map(str::to_string));
    args
}

pub fn resume_args(settings: &Path, session_id: &str, mode: SessionMode) -> Vec<String> {
    let mut args = vec!["--settings".into(), settings.to_string_lossy().into_owned()];
    args.extend(mcp_args(settings));
    args.extend(["--resume".to_string(), session_id.to_string()]);
    args.extend(setting_sources(mode));
    args
}

#[derive(Debug, Clone, PartialEq)]
pub enum HookEffect {
    Status(SessionStatus),
    /// Claude Code's status line input: the session's use and the plan's limits.
    StatusLine,
    /// The session started or resumed: it can be told who else works in its folder.
    SessionStart,
    /// A tool is about to run: the session is Working, and the daemon checks
    /// the call (the branch question, secret reads in the project folder).
    PreTool { tool: String, summary: String, input: Value },
    Approval { tool: String, summary: String, input: Value },
    Activity(String),
    Nothing,
}

pub fn interpret_hook(payload: &Value) -> HookEffect {
    if is_status_line(payload) {
        return HookEffect::StatusLine;
    }
    let Some(event) = payload.get("hook_event_name").and_then(Value::as_str) else {
        return HookEffect::Nothing;
    };
    let tool = payload.get("tool_name").and_then(Value::as_str);
    let input = payload.get("tool_input").cloned().unwrap_or(Value::Null);
    match event {
        "UserPromptSubmit" => HookEffect::Status(SessionStatus::Working),
        "SessionStart" => HookEffect::SessionStart,
        "PreToolUse" => {
            let tool = tool.unwrap_or("unknown");
            HookEffect::PreTool { tool: tool.to_string(), summary: summarize_tool(tool, &input), input }
        }
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
        // PowerShell is the shell tool Claude Code also has on Windows.
        "Bash" | "PowerShell" => "command",
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
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    fn status_payload() -> Value {
        json!({
            "session_id": "s1",
            "model": {"id": "claude-opus-5-5", "display_name": "Opus 5.5"},
            "workspace": {"current_dir": "/r"},
            "cost": {"total_cost_usd": 2.345, "total_lines_added": 120, "total_lines_removed": 30},
            "context_window": {"used_percentage": 64.4, "context_window_size": 200000},
            "effort": {"level": "high"},
            "rate_limits": {"five_hour": {"used_percentage": 68.0, "resets_at": 1795480000}, "seven_day": {"used_percentage": 31.5}}
        })
    }

    #[test]
    fn status_line_input_is_told_apart_from_hook_events() {
        assert_eq!(interpret_hook(&status_payload()), HookEffect::StatusLine);
        let mut named = status_payload();
        named["hook_event_name"] = json!("Status");
        assert_eq!(interpret_hook(&named), HookEffect::StatusLine);
        assert_eq!(interpret_hook(&json!({"hook_event_name": "Stop", "model": {}, "workspace": {}})), HookEffect::Status(SessionStatus::Idle));
    }

    #[test]
    fn status_reports_usage_and_plan_limits() {
        let (usage, plan) = parse_status(&status_payload(), 7);
        assert_eq!(usage.cost_usd, 2.345);
        assert_eq!((usage.context_used_percentage, usage.context_window_size), (Some(64.4), Some(200000)));
        assert_eq!((usage.lines_added, usage.lines_removed), (120, 30));
        assert_eq!((usage.model.as_deref(), usage.effort.as_deref(), usage.updated_at), (Some("Opus 5.5"), Some("high"), 7));
        let plan = plan.unwrap();
        assert_eq!(plan.five_hour, Some(PlanLimit { used_percentage: 68.0, resets_at: Some(1795480000) }));
        assert_eq!(plan.seven_day, Some(PlanLimit { used_percentage: 31.5, resets_at: None }));
        assert_eq!(status_text(&usage, Some(&plan)), "Opus 5.5 · context 64% · $2.35 · 5h 68%");

        // An API key or a third-party provider has no plan limits.
        let (usage, plan) = parse_status(&json!({"model": {"id": "x"}, "workspace": {}}), 1);
        assert_eq!(plan, None);
        assert_eq!(status_text(&usage, None), "x · $0.00");
    }

    #[test]
    fn settings_give_the_status_line_to_the_hook() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_settings(dir.path(), Path::new("/bin/acc-hook")).unwrap();
        let v: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(v["statusLine"]["type"], "command");
        assert_eq!(v["statusLine"]["refreshInterval"], 30);
        assert_eq!(v["statusLine"]["command"], v["hooks"]["Stop"][0]["hooks"][0]["command"]);
    }

    /// Where the tool server config of `/d/s.json` is.
    #[cfg(unix)]
    const MCP: &str = "/d/mcp.json";
    #[cfg(windows)]
    const MCP: &str = "/d\\mcp.json";

    #[test]
    fn launch_args_order() {
        assert_eq!(
            launch_args(Path::new("/d/s.json"), "abc", Some("Fix the bug"), SessionMode::Worktree),
            vec!["--settings", "/d/s.json", "--mcp-config", MCP, "--session-id", "abc", "Fix the bug"]
        );
        assert_eq!(
            launch_args(Path::new("/d/s.json"), "abc", Some("Fix the bug"), SessionMode::Project),
            vec!["--settings", "/d/s.json", "--mcp-config", MCP, "--session-id", "abc", "--setting-sources", "user,project", "Fix the bug"]
        );
        assert_eq!(
            launch_args(Path::new("/d/s.json"), "abc", None, SessionMode::Worktree),
            vec!["--settings", "/d/s.json", "--mcp-config", MCP, "--session-id", "abc"]
        );
    }

    #[test]
    fn resume_args_order() {
        assert_eq!(
            resume_args(Path::new("/d/s.json"), "abc", SessionMode::Worktree),
            vec!["--settings", "/d/s.json", "--mcp-config", MCP, "--resume", "abc"]
        );
        assert_eq!(
            resume_args(Path::new("/d/s.json"), "abc", SessionMode::Project),
            vec!["--settings", "/d/s.json", "--mcp-config", MCP, "--resume", "abc", "--setting-sources", "user,project"]
        );
    }

    #[cfg(unix)]
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
        assert_eq!(v["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"], 3600);
        assert!(v["hooks"]["Stop"][0]["hooks"][0].get("timeout").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn settings_quote_hook_path_with_spaces() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_settings(dir.path(), Path::new("/a b/acc-hook")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], "'/a b/acc-hook'");
    }

    #[cfg(unix)]
    #[test]
    fn shell_quote_escapes_single_quote() {
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[cfg(windows)]
    #[test]
    fn settings_run_the_hook_with_no_shell_on_windows() {
        let dir = tempfile::tempdir().unwrap();
        let hook = Path::new(r"C:\Program Files\Agent Control Center\acc-hook.exe");
        let p = write_settings(&dir.path().join("a"), hook).unwrap();
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        for ev in ["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "PermissionRequest", "Notification", "Stop", "SessionEnd"] {
            let entry = &v["hooks"][ev][0]["hooks"][0];
            assert_eq!(entry["type"], "command", "{ev}");
            // The path as it is, spaces and backslashes too, and `args`, so
            // that Claude Code starts it directly.
            assert_eq!(entry["command"], r"C:\Program Files\Agent Control Center\acc-hook.exe", "{ev}");
            assert_eq!(entry["args"], json!([]), "{ev}");
        }
        assert_eq!(v["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"], 3600);
        assert_eq!(v["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"], 3600);
        assert!(v["hooks"]["Stop"][0]["hooks"][0].get("timeout").is_none());
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
        assert_eq!(
            interpret_hook(&json!({"hook_event_name":"PreToolUse","tool_name":"Edit","tool_input":{"file_path":"/r/a.rs"}})),
            HookEffect::PreTool { tool: "Edit".into(), summary: "/r/a.rs".into(), input: json!({"file_path":"/r/a.rs"}) }
        );
    }

    #[test]
    fn interpret_post_tool_use_is_activity() {
        let p = json!({"hook_event_name":"PostToolUse","tool_name":"Read","tool_input":{"file_path":"/x.rs"}});
        assert_eq!(interpret_hook(&p), HookEffect::Activity("/x.rs".into()));
    }

    #[test]
    fn interpret_unknown_is_nothing() {
        assert_eq!(interpret_hook(&json!({"hook_event_name":"TeammateIdle"})), HookEffect::Nothing);
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

    #[cfg(unix)]
    #[test]
    fn install_locations_come_in_order() {
        let locations = [
            "/h/.local/bin/claude",
            "/h/.claude/local/claude",
            "/h/.npm-global/bin/claude",
            "/opt/homebrew/bin/claude",
            "/usr/local/bin/claude",
            "/usr/bin/claude",
        ];
        assert_eq!(install_locations(Some(Path::new("/h"))), locations.map(PathBuf::from));
        // Without a home folder, only the system ones.
        assert_eq!(install_locations(None), locations[3..].iter().map(PathBuf::from).collect::<Vec<_>>());
    }

    #[cfg(unix)]
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
        let expected = [
            "/a/claude",
            "/b/claude",
            "/h/.local/bin/claude",
            "/h/.claude/local/claude",
            "/h/.npm-global/bin/claude",
            "/opt/homebrew/bin/claude",
            "/usr/local/bin/claude",
            "/usr/bin/claude",
        ];
        assert_eq!(tried.into_inner(), expected.map(PathBuf::from));
        // The first executable one wins.
        let only = |wanted: &'static [&'static str]| move |path: &Path| wanted.iter().any(|w| path == Path::new(w));
        assert_eq!(find_claude(Some("/a:/b"), home, only(&["/b/claude", "/opt/homebrew/bin/claude"])), Some("/b/claude".into()));
        assert_eq!(find_claude(Some("/a"), home, only(&["/h/.claude/local/claude"])), Some("/h/.claude/local/claude".into()));
        assert_eq!(find_claude(None, None, only(&["/usr/local/bin/claude"])), Some("/usr/local/bin/claude".into()));
        // The Linux places: npm with a user prefix, and a system Node.
        let npm = only(&["/h/.npm-global/bin/claude", "/usr/bin/claude"]);
        assert_eq!(find_claude(None, home, npm), Some("/h/.npm-global/bin/claude".into()));
        assert_eq!(find_claude(Some("/a"), None, only(&["/usr/bin/claude"])), Some("/usr/bin/claude".into()));
    }

    #[cfg(unix)]
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

    #[cfg(unix)]
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

    #[test]
    fn summarize_powershell_uses_the_command() {
        assert_eq!(summarize_tool("PowerShell", &json!({"command":"Get-ChildItem src"})), "Get-ChildItem src");
        assert_eq!(summarize_tool("PowerShell", &json!({})), "PowerShell");
    }

    #[cfg(windows)]
    #[test]
    fn install_locations_on_windows_come_in_order() {
        let home = Path::new(r"C:\Users\x");
        let app_data = Path::new(r"C:\Users\x\AppData\Roaming");
        assert_eq!(
            install_locations(Some(home), Some(app_data)),
            [r"C:\Users\x\.local\bin\claude.exe", r"C:\Users\x\AppData\Roaming\npm\claude.cmd"].map(PathBuf::from)
        );
        assert_eq!(install_locations(None, None), Vec::<PathBuf>::new());
    }

    #[cfg(windows)]
    #[test]
    fn claude_names_follow_pathext() {
        assert_eq!(claude_names(None), ["claude.com", "claude.exe", "claude.bat", "claude.cmd"]);
        assert_eq!(claude_names(Some("")), claude_names(None));
        // Only what Windows can start, in the order given, each once.
        assert_eq!(claude_names(Some(".EXE;.JS;.CMD; .exe ;.PS1;.Bat")), ["claude.exe", "claude.cmd", "claude.bat"]);
    }

    #[cfg(windows)]
    #[test]
    fn find_claude_on_windows_tries_the_path_then_the_install_locations() {
        let (home, app_data) = (Some(Path::new(r"C:\Users\x")), Some(Path::new(r"C:\Users\x\AppData\Roaming")));
        let names = claude_names(Some(".EXE;.CMD"));
        // Nothing is executable: every place is tried once, in order. Relative
        // and empty PATH entries are skipped, and quotes around one are not
        // part of the folder.
        let tried = std::cell::RefCell::new(Vec::new());
        let found = find_claude(Some(r#"C:\a;relative;;"C:\b c""#), &names, home, app_data, |path| {
            tried.borrow_mut().push(path.to_path_buf());
            false
        });
        assert_eq!(found, None);
        let expected = [
            r"C:\a\claude.exe",
            r"C:\a\claude.cmd",
            r"C:\b c\claude.exe",
            r"C:\b c\claude.cmd",
            r"C:\Users\x\.local\bin\claude.exe",
            r"C:\Users\x\AppData\Roaming\npm\claude.cmd",
        ];
        assert_eq!(tried.into_inner(), expected.map(PathBuf::from));
        // The first executable one wins: claude.exe before claude.cmd, and the PATH first.
        let only = |wanted: &'static [&'static str]| move |path: &Path| wanted.iter().any(|w| path == Path::new(w));
        let npm_and_native = only(&[r"C:\a\claude.cmd", r"C:\a\claude.exe"]);
        assert_eq!(find_claude(Some(r"C:\a"), &names, home, app_data, npm_and_native), Some(r"C:\a\claude.exe".into()));
        let npm = only(&[r"C:\Users\x\AppData\Roaming\npm\claude.cmd"]);
        assert_eq!(find_claude(None, &names, home, app_data, npm), Some(r"C:\Users\x\AppData\Roaming\npm\claude.cmd".into()));
    }

    #[cfg(windows)]
    #[test]
    fn windows_runs_programs_and_batch_files_only() {
        let dir = tempfile::tempdir().unwrap();
        for (name, runs) in [("a.exe", true), ("b.CMD", true), ("c.bat", true), ("d.com", true), ("claude", false), ("e.ps1", false), ("f.js", false)] {
            let file = dir.path().join(name);
            std::fs::write(&file, "").unwrap();
            assert_eq!(is_executable(&file), runs, "{name}");
        }
        std::fs::create_dir(dir.path().join("folder.exe")).unwrap();
        assert!(!is_executable(&dir.path().join("folder.exe")));
        assert!(!is_executable(&dir.path().join("missing.exe")));
    }
}
