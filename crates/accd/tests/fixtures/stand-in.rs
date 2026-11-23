//! A stand-in for Claude Code, acc-hook and accd in the tests, on every
//! platform. What it does depends on its file name, so a test copies it
//! under the name it needs. As Claude Code it first writes what it was given
//! to `stand-in-run.json` in its working directory (the worktree).
//!
//! - `fake-claude`, and any other name: prints `ARGS: <args>` and
//!   `ENV:<session id>`, echoes its input and runs for 300 s.
//! - `stubborn-claude`: the same, but ignores SIGHUP.
//! - `exiting-claude`: prints `bye <task>` and exits with the task as its code.
//! - `flood-claude`: prints `line 1` to `line 2000`, then runs for 300 s.
//! - `stream-claude`: prints `line 1` to `line 20000` and `END`, then runs for 300 s.
//! - `acc-hook`: exits at once.
//! - `accd-sleeps`: prints its arguments, its pid and whether it has a
//!   console window, then runs for 30 s.
//! - `accd-prints`: prints a line to stdout and one to stderr, then exits.
//! - `accd-exits`: prints `started` and exits with 3 after half a second.
//!
//! With `WARM_UP` set it exits at once, whatever its name.

use std::io::{BufRead, Write};
use std::time::Duration;

fn main() {
    if std::env::var_os("WARM_UP").is_some() {
        return;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let exe = std::env::current_exe().expect("the stand-in's own path");
    let name = exe.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default().to_string();
    match name.as_str() {
        "acc-hook" => {}
        "accd-sleeps" => {
            println!("args: {} {}", args.len(), args.join(" "));
            println!("pid: {}", std::process::id());
            println!("console window: {}", if has_console_window() { "yes" } else { "none" });
            std::thread::sleep(Duration::from_secs(30));
        }
        "accd-prints" => {
            println!("to stdout");
            eprintln!("to stderr");
        }
        "accd-exits" => {
            println!("started");
            std::thread::sleep(Duration::from_millis(500));
            std::process::exit(3);
        }
        "exiting-claude" => {
            record(&args);
            std::thread::sleep(Duration::from_millis(5));
            let task = args.last().cloned().unwrap_or_default();
            println!("bye {task}");
            std::process::exit(task.parse().unwrap_or(1));
        }
        "flood-claude" => {
            record(&args);
            print_lines(2000, false);
            std::thread::sleep(Duration::from_secs(300));
        }
        "stream-claude" => {
            record(&args);
            print_lines(20000, true);
            println!("END");
            std::thread::sleep(Duration::from_secs(300));
        }
        other => {
            if other == "stubborn-claude" {
                ignore_hangup();
            }
            record(&args);
            println!("ARGS: {}", args.join(" "));
            println!("ENV:{}", std::env::var("ACC_SESSION_ID").unwrap_or_default());
            // Echo input, as a terminal program would show it.
            std::thread::spawn(|| {
                for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                    println!("IN:{line}");
                }
            });
            std::thread::sleep(Duration::from_secs(300));
        }
    }
}

/// Writes down what the run was given.
fn record(args: &[String]) {
    let env = |name: &str| std::env::var(name).ok();
    let record = serde_json::json!({
        "args": args,
        "cwd": std::env::current_dir().map(|dir| dir.to_string_lossy().into_owned()).ok(),
        "session_id": env("ACC_SESSION_ID"),
        "socket": env("ACC_SOCKET"),
        "term": env("TERM"),
        "path": env("PATH"),
        "batch_command": env("ACC_BATCH_COMMAND"),
    });
    // Written whole and then renamed, so a reader never sees half of it. The
    // temporary name is this process's own: sessions that share a folder run
    // several stand-ins in it.
    let tmp = format!("stand-in-run.json.{}.tmp", std::process::id());
    std::fs::write(&tmp, record.to_string()).expect("write the run record");
    // On Windows a rename fails while another stand-in has the file open; the
    // record is then the other one's, which tests in a shared folder do not read.
    if std::fs::rename(&tmp, "stand-in-run.json").is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Prints `line 1` to `line <n>`. A `steady` stream pauses a moment every
/// 100 lines, about as fast as a shell loop prints them, so a test can
/// attach while it runs.
fn print_lines(n: usize, steady: bool) {
    let mut out = std::io::stdout().lock();
    for i in 1..=n {
        writeln!(out, "line {i}").unwrap();
        if steady && i % 100 == 0 {
            out.flush().unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    out.flush().unwrap();
}

#[cfg(unix)]
fn ignore_hangup() {
    // SAFETY: setting a signal to ignored is always sound.
    unsafe { libc::signal(libc::SIGHUP, libc::SIG_IGN) };
}

#[cfg(windows)]
fn ignore_hangup() {}

#[cfg(unix)]
fn has_console_window() -> bool {
    false
}

#[cfg(windows)]
fn has_console_window() -> bool {
    // SAFETY: a plain call.
    !unsafe { windows_sys::Win32::System::Console::GetConsoleWindow() }.is_null()
}
