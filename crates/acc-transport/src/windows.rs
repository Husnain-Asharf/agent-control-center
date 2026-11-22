use std::ffi::{c_void, OsStr, OsString};
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, RawHandle};
use std::path::Path;
use std::ptr::null_mut;
use std::time::{Duration, Instant};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions};
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, ERROR_PIPE_BUSY, HANDLE};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    EqualSid, GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::SECURITY_IDENTIFICATION;
use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};

/// The daemon's end of a connection.
pub type ServerStream = NamedPipeServer;
/// The app's end of a connection.
pub type ClientStream = NamedPipeClient;
/// A connection for blocking reads and writes, as the hook makes.
pub type BlockingStream = std::fs::File;

/// How long a client keeps trying while every instance of the pipe is taken.
/// The daemon makes a new one as soon as a client takes one, so this only
/// matters for a burst of clients, such as hooks for parallel tool calls.
const BUSY_WAIT: Duration = Duration::from_secs(5);
const BUSY_RETRY: Duration = Duration::from_millis(10);

/// The daemon's named pipe. One instance waits for a client at a time.
pub struct Listener {
    name: OsString,
    security: PipeSecurity,
    next: NamedPipeServer,
}

impl Listener {
    /// Creates the pipe `endpoint`. Only this user can open it, remote
    /// clients are rejected, and it fails when the name is taken, so no other
    /// process serves the pipe at the same time.
    pub fn bind(endpoint: &Path) -> io::Result<Listener> {
        let name = endpoint.as_os_str().to_os_string();
        let security = PipeSecurity::current_user_only()?;
        let next = security.create(&name, true)?;
        Ok(Listener { name, security, next })
    }

    /// The next connection. The next client gets a new instance of the pipe,
    /// also when this one failed.
    pub async fn accept(&mut self) -> io::Result<ServerStream> {
        let connected = self.next.connect().await;
        let next = self.security.create(&self.name, false)?;
        let instance = std::mem::replace(&mut self.next, next);
        connected.map(|()| instance)
    }
}

/// Connects to the daemon at `endpoint`. Fails unless a process of this user
/// serves the pipe.
pub async fn connect(endpoint: &Path) -> io::Result<ClientStream> {
    let deadline = Instant::now() + BUSY_WAIT;
    let client = loop {
        // Identification only: the server can tell who connected but cannot
        // act as this user.
        match ClientOptions::new().open(endpoint) {
            Ok(client) => break client,
            Err(e) if is_busy(&e) && Instant::now() < deadline => tokio::time::sleep(BUSY_RETRY).await,
            Err(e) => return Err(e),
        }
    };
    check_server(client.as_raw_handle())?;
    Ok(client)
}

/// Connects to the daemon at `endpoint`, for blocking reads and writes.
/// Fails unless a process of this user serves the pipe.
pub fn connect_blocking(endpoint: &Path) -> io::Result<BlockingStream> {
    let deadline = Instant::now() + BUSY_WAIT;
    let pipe = loop {
        let opened =
            std::fs::OpenOptions::new().read(true).write(true).security_qos_flags(SECURITY_IDENTIFICATION).open(endpoint);
        match opened {
            Ok(pipe) => break pipe,
            Err(e) if is_busy(&e) && Instant::now() < deadline => std::thread::sleep(BUSY_RETRY),
            Err(e) => return Err(e),
        }
    };
    check_server(pipe.as_raw_handle())?;
    Ok(pipe)
}

/// The process id of the daemon at the other end.
pub fn server_pid(stream: &ClientStream) -> io::Result<u32> {
    pipe_server_pid(stream.as_raw_handle())
}

fn is_busy(e: &io::Error) -> bool {
    e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32)
}

fn pipe_server_pid(pipe: RawHandle) -> io::Result<u32> {
    let mut pid = 0;
    // SAFETY: `pipe` is an open handle and `pid` outlives the call.
    if unsafe { GetNamedPipeServerProcessId(pipe as HANDLE, &mut pid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(pid)
}

/// Fails unless the pipe behind `pipe` is served by a process of the current
/// user. Anyone can create a pipe with a free name, and a pipe made by another
/// user could pose as the daemon and answer permission requests.
fn check_server(pipe: RawHandle) -> io::Result<()> {
    let not_ours = |e: io::Error| {
        io::Error::new(io::ErrorKind::PermissionDenied, format!("the pipe's server is not this user ({e})"))
    };
    let pid = pipe_server_pid(pipe)?;
    // SAFETY: a plain call. A null handle is checked below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(not_ours(io::Error::last_os_error()));
    }
    let process = Handle(process);
    let server = TokenUserInfo::of(process.0).map_err(not_ours)?;
    // SAFETY: the pseudo handle of this process needs no closing.
    let me = TokenUserInfo::of(unsafe { GetCurrentProcess() })?;
    // SAFETY: both SIDs live in buffers that outlive the call.
    if unsafe { EqualSid(server.sid(), me.sid()) } == 0 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the pipe is served by another user"));
    }
    Ok(())
}

/// A handle closed on drop.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle is open and owned by this value.
        unsafe { CloseHandle(self.0) };
    }
}

/// The user of a process's token, as `GetTokenInformation(TokenUser)`
/// returns it. The buffer is `u64`s so the `TOKEN_USER` in it is aligned.
struct TokenUserInfo(Vec<u64>);

impl TokenUserInfo {
    fn of(process: HANDLE) -> io::Result<TokenUserInfo> {
        let mut token = null_mut();
        // SAFETY: `process` is a process handle and `token` outlives the call.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = Handle(token);
        let mut len = 0;
        // SAFETY: asks for the size only; this call fails and sets `len`.
        unsafe { GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut len) };
        if len == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0u64; (len as usize).div_ceil(8)];
        // SAFETY: `buffer` holds at least `len` bytes.
        if unsafe { GetTokenInformation(token.0, TokenUser, buffer.as_mut_ptr().cast(), len, &mut len) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(TokenUserInfo(buffer))
    }

    fn sid(&self) -> PSID {
        // SAFETY: the buffer holds a TOKEN_USER, whose SID points into the buffer.
        unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
}

/// A security descriptor that lets the current user, and no one else, use
/// the pipe. The user is also its owner.
struct PipeSecurity(PSECURITY_DESCRIPTOR);

// SAFETY: the descriptor is only read after it is made, and freed once.
unsafe impl Send for PipeSecurity {}
unsafe impl Sync for PipeSecurity {}

impl PipeSecurity {
    fn current_user_only() -> io::Result<PipeSecurity> {
        // SAFETY: the pseudo handle of this process needs no closing.
        let me = TokenUserInfo::of(unsafe { GetCurrentProcess() })?;
        let sid = sid_string(me.sid())?;
        // A protected DACL whose only entry gives the user full access.
        let sddl: Vec<u16> = format!("O:{sid}D:P(A;;GA;;;{sid})").encode_utf16().chain(Some(0)).collect();
        let mut descriptor = null_mut();
        // SAFETY: `sddl` ends with a nul, and `descriptor` outlives the call.
        let made = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut descriptor, null_mut())
        };
        if made == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(PipeSecurity(descriptor))
    }

    fn create(&self, name: &OsStr, first: bool) -> io::Result<NamedPipeServer> {
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        };
        let mut options = ServerOptions::new();
        options.first_pipe_instance(first).reject_remote_clients(true);
        // SAFETY: `attributes` is a valid SECURITY_ATTRIBUTES for the call.
        unsafe { options.create_with_security_attributes_raw(name, (&mut attributes as *mut SECURITY_ATTRIBUTES).cast::<c_void>()) }
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        // SAFETY: the descriptor was allocated by the conversion above.
        unsafe { LocalFree(self.0) };
    }
}

/// A SID in its `S-1-5-...` form.
fn sid_string(sid: PSID) -> io::Result<String> {
    let mut text = null_mut();
    // SAFETY: `sid` is a valid SID and `text` outlives the call.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `text` is a nul-terminated string the call allocated.
    let len = (0..).take_while(|&i| unsafe { *text.add(i) } != 0).count();
    let string = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len) });
    // SAFETY: allocated by the call above, freed once.
    unsafe { LocalFree(text.cast()) };
    Ok(string)
}
