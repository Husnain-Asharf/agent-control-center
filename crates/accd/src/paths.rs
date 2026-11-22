//! Where the daemon keeps its socket, database and per-session files.

use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Paths {
    pub data: PathBuf,
    pub socket: PathBuf,
    pub db: PathBuf,
    pub sessions: PathBuf,
    /// Held locked by the daemon that serves this dir.
    pub lock: PathBuf,
}

impl Paths {
    /// The data dir from `ACC_DATA_DIR` or the platform default, created if missing.
    pub fn resolve() -> anyhow::Result<Paths> {
        let paths = Paths::in_dir(&acc_protocol::data_dir());
        paths.create_dirs()?;
        Ok(paths)
    }

    pub fn in_dir(dir: &Path) -> Paths {
        Paths {
            data: dir.to_path_buf(),
            socket: acc_protocol::socket_path(dir),
            db: dir.join("acc.db"),
            sessions: dir.join("sessions"),
            lock: dir.join("accd.lock"),
        }
    }

    /// Holds the session's `claude-settings.json`.
    pub fn session_dir(&self, session_id: &str) -> PathBuf {
        self.sessions.join(session_id)
    }

    /// Creates the data dir and `sessions/` with mode 0700 where missing.
    /// The data dir can fall back to a shared temp dir, so it must stay private.
    pub fn create_dirs(&self) -> std::io::Result<()> {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(&self.data)?;
        builder.create(&self.sessions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn in_dir_lays_out_files() {
        let p = Paths::in_dir(Path::new("/d d"));
        assert_eq!(p.socket, PathBuf::from("/d d/accd.sock"));
        assert_eq!(p.db, PathBuf::from("/d d/acc.db"));
        assert_eq!(p.sessions, PathBuf::from("/d d/sessions"));
        assert_eq!(p.lock, PathBuf::from("/d d/accd.lock"));
        assert_eq!(p.session_dir("s1"), PathBuf::from("/d d/sessions/s1"));
    }

    #[test]
    fn create_dirs_makes_private_dirs() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::in_dir(&t.path().join("a/data"));
        p.create_dirs().unwrap();
        p.create_dirs().unwrap();
        for dir in [&p.data, &p.sessions] {
            assert_eq!(std::fs::metadata(dir).unwrap().permissions().mode() & 0o777, 0o700, "{}", dir.display());
        }
    }
}
