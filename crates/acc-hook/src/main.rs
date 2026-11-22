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

fn main() {
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
