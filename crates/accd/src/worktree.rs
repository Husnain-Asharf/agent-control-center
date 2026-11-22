use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct RepoInfo {
    pub name: String,
    pub root: PathBuf,
    pub host: Option<String>,
    pub base_branch: String,
}

#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error("not a git repository: {0}")]
    NotARepo(String),
    #[error("git failed: {0}")]
    Git(String),
}

/// `git`. On Windows it gets a console of its own with no window: the
/// daemon may have no console, and then each git would open a window.
fn git_command() -> Command {
    #[allow(unused_mut)]
    let mut git = Command::new("git");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        git.creation_flags(CREATE_NO_WINDOW);
    }
    git
}

fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = git_command()
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

pub fn inspect_repo(path: &Path) -> Result<RepoInfo, WorktreeError> {
    let root = run_git(path, &["rev-parse", "--show-toplevel"])
        .map_err(|_| WorktreeError::NotARepo(path.display().to_string()))?;
    let root = PathBuf::from(root);
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let host = run_git(&root, &["remote", "get-url", "origin"]).ok().and_then(|url| {
        if url.contains("github.com") {
            Some("GitHub".to_string())
        } else if url.contains("gitlab") {
            Some("GitLab".to_string())
        } else {
            None
        }
    });
    let base_branch = run_git(&root, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .ok()
        .map(|b| b.strip_prefix("origin/").unwrap_or(&b).to_string())
        .filter(|b| !b.is_empty())
        .or_else(|| {
            run_git(&root, &["symbolic-ref", "--short", "HEAD"])
                .ok()
                .filter(|b| !b.is_empty())
        })
        .unwrap_or_else(|| "main".to_string());
    Ok(RepoInfo { name, root, host, base_branch })
}

pub fn slugify(task: &str) -> String {
    let lowered: String = task
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() { c } else { '-' })
        .collect();
    let joined = lowered
        .split('-')
        .filter(|w| !w.is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join("-");
    let cut: String = joined.chars().take(40).collect();
    let cut = cut.trim_matches('-');
    if cut.is_empty() {
        "session".to_string()
    } else {
        cut.to_string()
    }
}

fn ensure_exclude(root: &Path) -> Result<(), WorktreeError> {
    let rel = run_git(root, &["rev-parse", "--git-path", "info/exclude"]).map_err(WorktreeError::Git)?;
    let file = root.join(rel);
    let existing = std::fs::read_to_string(&file).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == ".worktrees/") {
        return Ok(());
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| WorktreeError::Git(e.to_string()))?;
    }
    let mut out = existing;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(".worktrees/\n");
    std::fs::write(&file, out).map_err(|e| WorktreeError::Git(e.to_string()))
}

pub fn create_worktree(repo: &RepoInfo, slug: &str) -> Result<(PathBuf, String), WorktreeError> {
    let base = repo.root.join(".worktrees");
    let mut n = 1;
    let (path, branch) = loop {
        let name = if n == 1 { slug.to_string() } else { format!("{slug}-{n}") };
        let path = base.join(&name);
        let branch = format!("agent/{name}");
        let branch_ref = format!("refs/heads/{branch}");
        let branch_taken = run_git(&repo.root, &["rev-parse", "--verify", "--quiet", &branch_ref]).is_ok();
        if !path.exists() && !branch_taken {
            break (path, branch);
        }
        n += 1;
    };
    ensure_exclude(&repo.root)?;
    let path_str = path.to_string_lossy().into_owned();
    run_git(&repo.root, &["worktree", "add", "-b", &branch, &path_str, "HEAD"])
        .map_err(WorktreeError::Git)?;
    Ok((path, branch))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }

    fn init_repo(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        git(dir, &["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        git(dir, &["add", "."]);
        git(dir, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "init"]);
    }

    fn branch_exists(dir: &Path, b: &str) -> bool {
        Command::new("git")
            .arg("-C").arg(dir)
            .args(["rev-parse", "--verify", "--quiet", &format!("refs/heads/{b}")])
            .stdout(std::process::Stdio::null()).status().unwrap().success()
    }

    #[test]
    fn slugify_examples() {
        assert_eq!(slugify("Rate limit the public API!"), "rate-limit-the-public");
        assert_eq!(slugify("   "), "session");
        assert_eq!(slugify("ÄÖ build 2"), "build-2");
    }

    #[test]
    fn inspect_repo_reads_name_and_branch() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("proj");
        init_repo(&dir);
        let info = inspect_repo(&dir).unwrap();
        assert_eq!(info.name, "proj");
        assert_eq!(info.base_branch, "main");
        assert_eq!(info.host, None);
        assert_eq!(info.root.file_name().unwrap(), "proj");
    }

    #[test]
    fn inspect_repo_detects_host_from_origin() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("proj");
        init_repo(&dir);
        git(&dir, &["remote", "add", "origin", "git@github.com:x/y.git"]);
        assert_eq!(inspect_repo(&dir).unwrap().host.as_deref(), Some("GitHub"));
        git(&dir, &["remote", "set-url", "origin", "https://gitlab.example.com/x/y.git"]);
        assert_eq!(inspect_repo(&dir).unwrap().host.as_deref(), Some("GitLab"));
    }

    #[test]
    fn inspect_repo_rejects_plain_folder() {
        let t = tempfile::tempdir().unwrap();
        assert!(matches!(inspect_repo(t.path()), Err(WorktreeError::NotARepo(_))));
    }

    #[test]
    fn create_worktree_makes_branch_and_dir() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("proj");
        init_repo(&dir);
        let info = inspect_repo(&dir).unwrap();
        let (path, branch) = create_worktree(&info, "fix").unwrap();
        assert_eq!(branch, "agent/fix");
        assert_eq!(path, info.root.join(".worktrees").join("fix"));
        assert!(path.join("a.txt").exists());
        assert!(branch_exists(&dir, "agent/fix"));
    }

    #[test]
    fn create_worktree_dedupes_slug() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("proj");
        init_repo(&dir);
        let info = inspect_repo(&dir).unwrap();
        create_worktree(&info, "fix").unwrap();
        let (path, branch) = create_worktree(&info, "fix").unwrap();
        assert_eq!(branch, "agent/fix-2");
        assert_eq!(path, info.root.join(".worktrees").join("fix-2"));
        assert!(path.exists());
        assert!(branch_exists(&dir, "agent/fix-2"));
    }

    #[test]
    fn exclude_line_added_once() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("proj");
        init_repo(&dir);
        let info = inspect_repo(&dir).unwrap();
        create_worktree(&info, "one").unwrap();
        create_worktree(&info, "two").unwrap();
        let ex = std::fs::read_to_string(dir.join(".git/info/exclude")).unwrap();
        assert_eq!(ex.lines().filter(|l| *l == ".worktrees/").count(), 1);
    }

    #[test]
    fn create_worktree_in_repo_with_spaces() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("my repo");
        init_repo(&dir);
        let info = inspect_repo(&dir).unwrap();
        assert_eq!(info.name, "my repo");
        let (path, branch) = create_worktree(&info, "fix").unwrap();
        assert!(path.join("a.txt").exists());
        assert!(branch_exists(&dir, &branch));
    }
}
