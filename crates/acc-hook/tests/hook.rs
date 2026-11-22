use acc_protocol::{permission_reply, HookReply, Request, RequestEnvelope, ServerMsg};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread::JoinHandle;

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

/// Accepts one connection, reads one request line, then lets `respond` write the reply.
fn fake_server(socket: &Path, respond: impl FnOnce(&mut std::os::unix::net::UnixStream) + Send + 'static) -> JoinHandle<String> {
    let listener = UnixListener::bind(socket).unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).unwrap();
        respond(&mut stream);
        line
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
    let sock = dir.path().join("accd.sock");
    let server = fake_server(&sock, |s| {
        s.write_all(&reply_line(1, &HookReply { stdout: Some("X".into()), exit_code: 0 })).unwrap();
    });
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
    let sock = dir.path().join("accd.sock");
    let server = fake_server(&sock, |s| {
        s.write_all(&reply_line(1, &HookReply { stdout: None, exit_code: 2 })).unwrap();
    });
    let out = run(STOP, &sock, Some("sess-1"));
    server.join().unwrap();
    assert!(out.stdout.is_empty());
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn no_session_env_is_silent() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(PERM, &dir.path().join("accd.sock"), None);
    assert!(out.stdout.is_empty());
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn unreachable_daemon_denies_permission_requests() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(PERM, &dir.path().join("missing.sock"), Some("sess-1"));
    assert_eq!(String::from_utf8(out.stdout).unwrap(), permission_reply(false, Some(DENY_MSG)));
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn unreachable_daemon_ignores_other_events() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(STOP, &dir.path().join("missing.sock"), Some("sess-1"));
    assert!(out.stdout.is_empty());
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn daemon_closing_without_reply_denies() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("accd.sock");
    let server = fake_server(&sock, |_| {});
    let out = run(PERM, &sock, Some("sess-1"));
    server.join().unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), permission_reply(false, Some(DENY_MSG)));
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn daemon_error_reply_denies() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("accd.sock");
    let server = fake_server(&sock, |s| {
        let msg = ServerMsg::Error { id: 1, code: "internal".into(), message: "boom".into() };
        s.write_all(&acc_protocol::encode_line(&msg)).unwrap();
    });
    let out = run(PERM, &sock, Some("sess-1"));
    server.join().unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), permission_reply(false, Some(DENY_MSG)));
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn unparseable_reply_denies() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("accd.sock");
    let server = fake_server(&sock, |s| {
        s.write_all(b"not json\n").unwrap();
    });
    let out = run(PERM, &sock, Some("sess-1"));
    server.join().unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), permission_reply(false, Some(DENY_MSG)));
}

#[test]
fn invalid_stdin_is_silent() {
    let dir = tempfile::tempdir().unwrap();
    let out = run("this is not json", &dir.path().join("missing.sock"), Some("sess-1"));
    assert!(out.stdout.is_empty());
    assert_eq!(out.status.code(), Some(0));
}
