use acc_protocol::{permission_reply, socket_path, HookReply, Request, RequestEnvelope, ServerMsg};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread::JoinHandle;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

const DENY_MSG: &str = "Agent Control Center is not running, so this was denied. Start the app and try again.";

fn run(stdin: &str, socket: &Path, session: Option<&str>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_acc-hook"));
    cmd.env("ACC_SOCKET", socket).env_remove("ACC_SESSION_ID");
    if let Some(s) = session {
        cmd.env("ACC_SESSION_ID", s);
    }
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

/// Where a daemon serving `dir` would listen: a socket in it, or a named
/// pipe on Windows.
fn endpoint(dir: &Path) -> PathBuf {
    socket_path(dir)
}

/// Listens at `socket` and accepts one connection. Reads one request line,
/// then writes `reply` and waits for the hook to close the connection, or
/// with an empty `reply` closes it at once. Returns the request line.
fn fake_server(socket: &Path, reply: Vec<u8>) -> JoinHandle<String> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let mut listener = runtime.block_on(async { acc_transport::Listener::bind(socket) }).unwrap();
    std::thread::spawn(move || {
        runtime.block_on(async move {
            let (reader, mut writer) = tokio::io::split(listener.accept().await.unwrap());
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            if !reply.is_empty() {
                writer.write_all(&reply).await.unwrap();
                let _ = reader.read_to_end(&mut Vec::new()).await;
            }
            line
        })
    })
}

fn reply_line(id: u64, reply: &HookReply) -> Vec<u8> {
    acc_protocol::encode_line(&ServerMsg::Response { id, result: serde_json::to_value(reply).unwrap() })
}

const PERM: &str = r#"{"hook_event_name":"PermissionRequest","tool_name":"Bash"}"#;
const STOP: &str = r#"{"hook_event_name":"Stop"}"#;

#[test]
fn forwards_and_prints_reply() {
    let dir = tempfile::tempdir().unwrap();
    let sock = endpoint(dir.path());
    let server = fake_server(&sock, reply_line(1, &HookReply { stdout: Some("X".into()), exit_code: 0 }));
    let out = run(PERM, &sock, Some("sess-1"));
    let req: RequestEnvelope = serde_json::from_str(&server.join().unwrap()).unwrap();
    assert_eq!(req.id, 1);
    assert_eq!(
        req.request,
        Request::Hook { session_id: "sess-1".into(), payload: serde_json::from_str(PERM).unwrap() }
    );
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "X");
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn exit_code_from_reply_is_used() {
    let dir = tempfile::tempdir().unwrap();
    let sock = endpoint(dir.path());
    let server = fake_server(&sock, reply_line(1, &HookReply { stdout: None, exit_code: 2 }));
    let out = run(STOP, &sock, Some("sess-1"));
    server.join().unwrap();
    assert!(out.stdout.is_empty());
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn no_session_env_is_silent() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(PERM, &endpoint(dir.path()), None);
    assert!(out.stdout.is_empty());
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn unreachable_daemon_denies_permission_requests() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(PERM, &endpoint(&dir.path().join("missing")), Some("sess-1"));
    assert_eq!(String::from_utf8(out.stdout).unwrap(), permission_reply(false, Some(DENY_MSG)));
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn unreachable_daemon_ignores_other_events() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(STOP, &endpoint(&dir.path().join("missing")), Some("sess-1"));
    assert!(out.stdout.is_empty());
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn daemon_closing_without_reply_denies() {
    let dir = tempfile::tempdir().unwrap();
    let sock = endpoint(dir.path());
    let server = fake_server(&sock, Vec::new());
    let out = run(PERM, &sock, Some("sess-1"));
    server.join().unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), permission_reply(false, Some(DENY_MSG)));
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn daemon_error_reply_denies() {
    let dir = tempfile::tempdir().unwrap();
    let sock = endpoint(dir.path());
    let msg = ServerMsg::Error { id: 1, code: "internal".into(), message: "boom".into() };
    let server = fake_server(&sock, acc_protocol::encode_line(&msg));
    let out = run(PERM, &sock, Some("sess-1"));
    server.join().unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), permission_reply(false, Some(DENY_MSG)));
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn unparseable_reply_denies() {
    let dir = tempfile::tempdir().unwrap();
    let sock = endpoint(dir.path());
    let server = fake_server(&sock, b"not json\n".to_vec());
    let out = run(PERM, &sock, Some("sess-1"));
    server.join().unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), permission_reply(false, Some(DENY_MSG)));
}

#[test]
fn invalid_stdin_is_silent() {
    let dir = tempfile::tempdir().unwrap();
    let out = run("this is not json", &endpoint(&dir.path().join("missing")), Some("sess-1"));
    assert!(out.stdout.is_empty());
    assert_eq!(out.status.code(), Some(0));
}

/// `acc-hook mcp` answers the MCP handshake and lists the session's tools,
/// and a tool call says plainly when the app is not running.
#[test]
fn mcp_lists_the_session_tools_and_fails_clearly_without_the_app() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_acc-hook"))
        .arg("mcp")
        .env("ACC_SESSION_ID", "s1")
        .env("ACC_SOCKET", dir.path().join("none.sock"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let lines = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_sessions","arguments":{}}}"#,
        r#"{"jsonrpc":"2.0","id":4,"method":"nope"}"#,
    ];
    {
        let stdin = child.stdin.as_mut().unwrap();
        for line in lines {
            writeln!(stdin, "{line}").unwrap();
        }
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    let replies: Vec<serde_json::Value> =
        String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(replies.len(), 4, "the notification gets no reply");
    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "acc");
    let names: Vec<&str> = replies[1]["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["start_helper", "list_sessions", "send_message"]);
    assert_eq!(replies[2]["result"]["isError"], true);
    assert!(replies[2]["result"]["content"][0]["text"].as_str().unwrap().contains("not running"));
    assert_eq!(replies[3]["error"]["code"], -32601);
}
