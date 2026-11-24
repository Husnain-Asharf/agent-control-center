use acc_protocol::{
    data_dir, encode_line, permission_reply, socket_path, HookReply, Request, RequestEnvelope, ServerMsg,
};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;

const FAIL_CLOSED_MESSAGE: &str =
    "Agent Control Center is not running, so this was denied. Start the app and try again.";

/// Sends one `hook` request and returns the daemon's reply.
/// Any failure (connect, write, no reply line, bad JSON, error reply) is an `Err`.
fn ask_daemon(session_id: String, payload: serde_json::Value) -> Result<HookReply, ()> {
    let socket = std::env::var_os("ACC_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| socket_path(&data_dir()));
    // On Windows this also fails unless the pipe is served by this user.
    let mut stream = acc_transport::connect_blocking(&socket).map_err(|_| ())?;
    let request = RequestEnvelope { id: 1, request: Request::Hook { session_id, payload } };
    stream.write_all(&encode_line(&request)).map_err(|_| ())?;
    let mut line = String::new();
    let read = BufReader::new(stream).read_line(&mut line).map_err(|_| ())?;
    if read == 0 {
        return Err(());
    }
    match serde_json::from_str::<ServerMsg>(&line).map_err(|_| ())? {
        ServerMsg::Response { result, .. } => serde_json::from_value(result).map_err(|_| ()),
        _ => Err(()),
    }
}

/// Sends one request to the daemon and returns its result, or its error message.
fn call_daemon(request: Request) -> Result<serde_json::Value, String> {
    let socket = std::env::var_os("ACC_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| socket_path(&data_dir()));
    let mut stream = acc_transport::connect_blocking(&socket).map_err(|_| "Agent Control Center is not running.".to_string())?;
    let envelope = RequestEnvelope { id: 1, request };
    stream.write_all(&encode_line(&envelope)).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).map_err(|e| e.to_string())?;
    match serde_json::from_str::<ServerMsg>(&line).map_err(|e| e.to_string())? {
        ServerMsg::Response { result, .. } => Ok(result),
        ServerMsg::Error { message, .. } => Err(message),
        _ => Err("unexpected reply".into()),
    }
}

/// The session's own tools, which Claude Code lists from `tools/list`.
fn tools() -> serde_json::Value {
    serde_json::json!([
        {
            "name": "start_helper",
            "description": "Start a helper Claude Code session in this project folder to take one part of a bigger task. The helper gets the task as its first message and works next to you on the same files, so give each helper its own files. You get its session id to message it. The user approves each helper.",
            "inputSchema": {"type": "object", "properties": {"task": {"type": "string", "description": "What the helper should do, with the files it owns and how to report back."}}, "required": ["task"]}
        },
        {
            "name": "list_sessions",
            "description": "List the running Claude Code sessions in this folder, with their ids, tasks, status and which ones are helpers of which lead.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "send_message",
            "description": "Send a message to another running session in this folder. It arrives as if typed into that session, marked as from you, and waits if that session is busy. Use it to hand out work, report results, or ask before touching a file another session is changing.",
            "inputSchema": {"type": "object", "properties": {"session_id": {"type": "string"}, "text": {"type": "string"}}, "required": ["session_id", "text"]}
        }
    ])
}

fn tool_call(session_id: &str, name: &str, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let text = |key: &str| args.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let from_session = session_id.to_string();
    match name {
        "start_helper" => call_daemon(Request::HelperStart { from_session, task: text("task") }),
        "list_sessions" => call_daemon(Request::FolderSessions { from_session }),
        "send_message" => call_daemon(Request::SessionSend { from_session, to_session: text("session_id"), text: text("text") }),
        _ => Err(format!("Unknown tool {name}.")),
    }
}

/// Answers one MCP message, or `None` for a notification.
fn mcp_reply(session_id: &str, msg: &serde_json::Value) -> Option<serde_json::Value> {
    let id = msg.get("id")?.clone();
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or_default();
    let result = match method {
        "initialize" => {
            if session_id.is_empty() {
                return Some(serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": "These tools only work in a session started by Agent Control Center (ACC_SESSION_ID is not set)."} }));
            }
            // Answer with the client's version when it is one this server speaks, else the newest one it does.
            const VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
            let asked = msg.pointer("/params/protocolVersion").and_then(|v| v.as_str()).unwrap_or_default();
            let version = VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(VERSIONS[0]);
            serde_json::json!({
                "protocolVersion": version,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "acc", "version": env!("CARGO_PKG_VERSION")}
            })
        }
        "ping" => serde_json::json!({}),
        "tools/list" => serde_json::json!({ "tools": tools() }),
        "tools/call" => {
            let name = msg.pointer("/params/name").and_then(|v| v.as_str()).unwrap_or_default();
            let args = msg.pointer("/params/arguments").cloned().unwrap_or(serde_json::json!({}));
            match tool_call(session_id, name, &args) {
                Ok(value) => serde_json::json!({ "content": [{"type": "text", "text": value.to_string()}] }),
                Err(e) => serde_json::json!({ "content": [{"type": "text", "text": e}], "isError": true }),
            }
        }
        _ => {
            return Some(serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Method not found"} }))
        }
    };
    Some(serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// `acc-hook mcp`: the session's own tools, over MCP on stdin and stdout,
/// one JSON message per line.
fn run_mcp() {
    let session_id = std::env::var("ACC_SESSION_ID").unwrap_or_default();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else {
            let _ = writeln!(stdout, "{}", serde_json::json!({ "jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"} }));
            let _ = stdout.flush();
            continue;
        };
        if let Some(reply) = mcp_reply(&session_id, &msg) {
            let _ = writeln!(stdout, "{reply}");
            let _ = stdout.flush();
        }
    }
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        return run_mcp();
    }
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(&input) else { return };
    let Ok(session_id) = std::env::var("ACC_SESSION_ID") else { return };

    let is_permission_request = payload.get("hook_event_name").and_then(|v| v.as_str()) == Some("PermissionRequest");
    match ask_daemon(session_id, payload) {
        Ok(reply) => {
            if let Some(out) = reply.stdout {
                let mut stdout = std::io::stdout();
                let _ = stdout.write_all(out.as_bytes());
                let _ = stdout.flush();
            }
            std::process::exit(reply.exit_code);
        }
        Err(()) => {
            if is_permission_request {
                print!("{}", permission_reply(false, Some(FAIL_CLOSED_MESSAGE)));
            }
        }
    }
}
