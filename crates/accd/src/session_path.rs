//! The PATH agent sessions get. An app opened from Finder gives the daemon
//! launchd's PATH, `/usr/bin:/bin:/usr/sbin:/sbin`, and one opened from a
//! Linux desktop gets the desktop session's, which lacks what only the
//! shell's startup files add. So Claude Code and the commands it runs would
//! not find Homebrew, nvm and the like. The daemon asks the user's login
//! shell for its PATH, once, and adds the folders of its own PATH that the
//! login PATH lacks.
//!
//! Windows has no login shell: an app started from the Start menu gets the
//! PATH the user set in the system settings. There sessions get the daemon's
//! PATH, with the folder of `claude` added.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::{ask_login_shell, login_path, login_shell, session_path, LOGIN_SHELL_TIMEOUT};

/// The PATH for agent sessions on Windows: the inherited PATH, then
/// `claude_dir`, the folder of the `claude` that was found, unless it is in
/// it already (in any case, with either slash, quotes or a trailing
/// backslash aside).
#[cfg(windows)]
pub fn session_path(inherited: Option<&str>, claude_dir: Option<&std::path::Path>) -> String {
    let inherited = inherited.unwrap_or_default();
    let Some(dir) = claude_dir.map(|dir| dir.to_string_lossy().into_owned()) else { return inherited.to_string() };
    let folder = |entry: &str| entry.replace('/', "\\").trim_end_matches('\\').to_lowercase();
    // split_paths reads a quoted entry, which may hold a `;`, as one folder.
    if std::env::split_paths(inherited).any(|entry| folder(&entry.to_string_lossy()) == folder(&dir)) {
        return inherited.to_string();
    }
    match inherited.trim_end_matches(';') {
        "" => dir,
        path => format!("{path};{dir}"),
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn sessions_get_the_inherited_path_and_the_folder_of_claude() {
        let inherited = r"C:\Windows\system32;C:\Program Files\Git\cmd";
        assert_eq!(
            session_path(Some(inherited), Some(Path::new(r"C:\Users\x\.local\bin"))),
            r"C:\Windows\system32;C:\Program Files\Git\cmd;C:\Users\x\.local\bin"
        );
        // A folder already there, written another way, is not added again.
        for same in [r"c:\program files\git\cmd", r"C:\Program Files\Git\cmd\", "C:/Program Files/Git/cmd"] {
            assert_eq!(session_path(Some(inherited), Some(Path::new(same))), inherited, "{same}");
        }
        let quoted = r#"C:\Windows;"C:\Tools;x\bin""#;
        assert_eq!(session_path(Some(quoted), Some(Path::new(r"C:\Tools;x\bin"))), quoted);
        // A trailing separator, no inherited PATH, and no claude folder.
        assert_eq!(session_path(Some(r"C:\Windows;"), Some(Path::new(r"C:\bin"))), r"C:\Windows;C:\bin");
        assert_eq!(session_path(None, Some(Path::new(r"C:\bin"))), r"C:\bin");
        assert_eq!(session_path(Some(inherited), None), inherited);
        assert_eq!(session_path(None, None), "");
    }
}
