//! The Windows parts of running a session on ConPTY: the job object that
//! holds its processes, the cursor request ConPTY makes when it starts, and
//! batch files, which only `cmd.exe` can run.

use std::ffi::c_void;
use std::io;
use std::os::windows::io::RawHandle;
use std::path::{Path, PathBuf};
use std::ptr::null;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

/// A job object that holds a session's process and every process it starts.
/// Dropping it ends the ones still running, as the end of the PTY does for a
/// session on Unix.
pub(crate) struct Job(HANDLE);

// SAFETY: a job handle can be used and closed from any thread.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Job {
    /// A new job holding `process`.
    pub(crate) fn holding(process: RawHandle) -> io::Result<Job> {
        // SAFETY: no attributes and no name.
        let job = unsafe { CreateJobObjectW(null(), null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Job(job);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
        // SAFETY: `limits` is the structure this class takes, of `size` bytes.
        let set = unsafe {
            SetInformationJobObject(job.0, JobObjectExtendedLimitInformation, (&raw const limits).cast::<c_void>(), size)
        };
        // SAFETY: `process` is an open process handle.
        if set == 0 || unsafe { AssignProcessToJobObject(job.0, process as HANDLE) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    /// Ends every process in the job.
    pub(crate) fn terminate(&self) -> io::Result<()> {
        // SAFETY: the handle is open.
        match unsafe { TerminateJobObject(self.0, 1) } {
            0 => Err(io::Error::last_os_error()),
            _ => Ok(()),
        }
    }

    /// The ids of the processes in the job.
    #[cfg(test)]
    pub(crate) fn process_ids(&self) -> Vec<usize> {
        use windows_sys::Win32::System::JobObjects::{
            JobObjectBasicProcessIdList, QueryInformationJobObject, JOBOBJECT_BASIC_PROCESS_ID_LIST,
        };
        // The two counts take the first word, and 64 ids fit after them.
        let mut buffer = [0usize; 66];
        let size = std::mem::size_of_val(&buffer) as u32;
        // SAFETY: the buffer is large and aligned enough for the list.
        let ok = unsafe {
            QueryInformationJobObject(self.0, JobObjectBasicProcessIdList, buffer.as_mut_ptr().cast(), size, std::ptr::null_mut())
        };
        assert_ne!(ok, 0, "{}", io::Error::last_os_error());
        // SAFETY: the call filled in a JOBOBJECT_BASIC_PROCESS_ID_LIST.
        let list = unsafe { &*buffer.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>() };
        buffer[1..1 + list.NumberOfProcessIdsInList as usize].to_vec()
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: the handle is open and owned by this value. Closing the last
        // handle ends the job's processes (JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE).
        unsafe { CloseHandle(self.0) };
    }
}

/// What ConPTY writes first when it starts: a request for the terminal's
/// cursor position. It shows nothing until that is answered.
const CURSOR_REQUEST: &[u8] = b"\x1b[6n";
/// The answer: a session starts on an empty screen, with the cursor top left.
pub(crate) const CURSOR_AT_TOP_LEFT: &[u8] = b"\x1b[1;1R";
/// After this many bytes without it, no request is coming.
const REQUEST_WITHIN: usize = 256;

/// Finds ConPTY's cursor request in the first output. The daemon answers it,
/// and takes it out of the output, so a client that attaches later does not
/// answer it again from the scrollback.
pub(crate) struct CursorRequest {
    seen: Option<usize>,
}

impl CursorRequest {
    pub(crate) fn new() -> CursorRequest {
        CursorRequest { seen: Some(0) }
    }

    /// `chunk` without the request, when it holds it. `None` passes the
    /// chunk on as it is.
    pub(crate) fn remove_from(&mut self, chunk: &[u8]) -> Option<Vec<u8>> {
        let seen = self.seen?;
        match chunk.windows(CURSOR_REQUEST.len()).position(|window| window == CURSOR_REQUEST) {
            Some(at) => {
                self.seen = None;
                Some([&chunk[..at], &chunk[at + CURSOR_REQUEST.len()..]].concat())
            }
            None => {
                self.seen = Some(seen + chunk.len()).filter(|seen| *seen < REQUEST_WITHIN);
                None
            }
        }
    }
}

/// Holds the command line for a batch file while `cmd.exe` reads it. The
/// command line clears it first, so the agent does not inherit it.
pub(crate) const BATCH_COMMAND_VAR: &str = "ACC_BATCH_COMMAND";

/// Whether `program` is a batch file, which needs `cmd.exe` to run.
pub(crate) fn is_batch_file(program: &Path) -> bool {
    program.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"))
}

/// The program, arguments and extra variable that run the batch file
/// `program` with `args`: `cmd.exe /d /e:on /v:off /c %ACC_BATCH_COMMAND%`,
/// with the command line in that variable. cmd reads the variable once and
/// does not expand it again, so `%` in an argument stays as it is, and
/// arguments in Unicode reach it intact. `/d` skips AutoRun commands, and
/// `/v:off` keeps `!` as it is.
pub(crate) fn batch_launch(program: &Path, args: &[String]) -> (PathBuf, Vec<String>, (String, String)) {
    let cmd = std::env::var_os("ComSpec")
        .map(PathBuf::from)
        .filter(|cmd| cmd.is_absolute())
        .or_else(|| std::env::var_os("SystemRoot").map(|root| Path::new(&root).join(r"System32\cmd.exe")))
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32\cmd.exe"));
    let cmd_args = ["/d", "/e:on", "/v:off", "/c", &format!("%{BATCH_COMMAND_VAR}%")].map(String::from).to_vec();
    let line = format!("set \"{BATCH_COMMAND_VAR}=\" & {}", batch_command_line(program, args));
    (cmd, cmd_args, (BATCH_COMMAND_VAR.to_string(), line))
}

/// `program` and `args`, each in double quotes, as cmd passes them on to a
/// batch file and the batch file (with `%*`) to a program. Inside quotes cmd
/// treats `&`, `|`, `<`, `>`, `^` and `(` as text. A quote in an argument is
/// doubled, which keeps cmd inside the quotes and which programs read as one
/// quote. Backslashes before a quote are doubled, as programs expect. Line
/// breaks would end the command, so they become spaces.
pub(crate) fn batch_command_line(program: &Path, args: &[String]) -> String {
    let program = program.to_string_lossy();
    std::iter::once(program.as_ref()).chain(args.iter().map(String::as_str)).map(quote).collect::<Vec<_>>().join(" ")
}

fn quote(arg: &str) -> String {
    let mut quoted = String::from('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
                quoted.push_str("\"\"");
                backslashes = 0;
            }
            _ => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(if c == '\r' || c == '\n' { ' ' } else { c });
                backslashes = 0;
            }
        }
    }
    // Before the closing quote, backslashes are doubled too.
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_request_is_taken_out_once() {
        let mut request = CursorRequest::new();
        // Mode changes may come first, in their own chunk.
        assert_eq!(request.remove_from(b"\x1b[?9001h\x1b[?1004h"), None);
        assert_eq!(request.remove_from(b"\x1b[6n\x1b[?25l"), Some(b"\x1b[?25l".to_vec()));
        // Later requests are the agent's own, for its terminal to answer.
        assert_eq!(request.remove_from(b"\x1b[6n"), None);

        let mut request = CursorRequest::new();
        assert_eq!(request.remove_from(b"\x1b[6n"), Some(Vec::new()));

        // Plain output first means no request is coming.
        let mut request = CursorRequest::new();
        assert_eq!(request.remove_from(&[b'x'; REQUEST_WITHIN]), None);
        assert_eq!(request.remove_from(b"\x1b[6n"), None);
    }

    #[test]
    fn batch_arguments_are_quoted_for_cmd() {
        let program = Path::new(r"C:\Users\x y\AppData\Roaming\npm\claude.cmd");
        let line = |args: &[&str]| batch_command_line(program, &args.iter().map(|a| a.to_string()).collect::<Vec<_>>());
        assert_eq!(
            line(&["--settings", r"C:\Users\x y\data\claude-settings.json", "Fix it"]),
            r#""C:\Users\x y\AppData\Roaming\npm\claude.cmd" "--settings" "C:\Users\x y\data\claude-settings.json" "Fix it""#
        );
        // One argument as it is quoted, after the program.
        let quoted = |arg: &str| line(&[arg]).strip_prefix(r#""C:\Users\x y\AppData\Roaming\npm\claude.cmd" "#).unwrap().to_string();
        // cmd's own characters stay inside the quotes; quotes are doubled.
        assert_eq!(quoted("a & b | c > d ^ e %PATH% !x! (y)"), r#""a & b | c > d ^ e %PATH% !x! (y)""#);
        assert_eq!(quoted(r#"say "hi""#), r#""say ""hi""""#);
        // Backslashes before a quote, or at the end, are doubled; others stay.
        assert_eq!(quoted(r#"a\"b"#), r#""a\\""b""#);
        assert_eq!(quoted(r"C:\dir\"), r#""C:\dir\\""#);
        assert_eq!(quoted(""), r#""""#);
        // Line breaks become spaces.
        assert_eq!(quoted("one\r\ntwo\nthree"), r#""one  two three""#);
    }

    #[test]
    fn batch_files_run_through_cmd() {
        assert!(is_batch_file(Path::new(r"C:\npm\claude.cmd")));
        assert!(is_batch_file(Path::new(r"C:\npm\CLAUDE.BAT")));
        assert!(!is_batch_file(Path::new(r"C:\bin\claude.exe")));
        let (cmd, args, (name, value)) = batch_launch(Path::new(r"C:\npm\claude.cmd"), &["--resume".into(), "abc".into()]);
        assert!(cmd.ends_with("cmd.exe"), "{}", cmd.display());
        assert_eq!(args, ["/d", "/e:on", "/v:off", "/c", "%ACC_BATCH_COMMAND%"]);
        assert_eq!(name, "ACC_BATCH_COMMAND");
        assert_eq!(value, r#"set "ACC_BATCH_COMMAND=" & "C:\npm\claude.cmd" "--resume" "abc""#);
    }
}
