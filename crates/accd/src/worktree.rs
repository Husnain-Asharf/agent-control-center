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
    if run_git(&repo.root, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_err() {
        return Err(WorktreeError::Git(
            "This repository has no commits yet, so it has nothing to make a worktree from. Make a first commit, or start the session in the project folder.".into(),
        ));
    }
    ensure_exclude(&repo.root)?;
    let path_str = path.to_string_lossy().into_owned();
    run_git(&repo.root, &["worktree", "add", "-b", &branch, &path_str, "HEAD"])
        .map_err(WorktreeError::Git)?;
    Ok((path, branch))
}

/// What a folder's `HEAD` points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    Branch(String),
    Detached,
}

/// Reads a folder's `HEAD` from git's files, without running git: from the
/// folder's `.git` folder, or from the one its `.git` file points to (a
/// linked worktree). `None` when there is neither.
pub fn read_head(folder: &Path) -> Option<Head> {
    let dot_git = folder.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        let text = std::fs::read_to_string(&dot_git).ok()?;
        let target = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
        if target.is_absolute() { target } else { folder.join(target) }
    };
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    Some(match head.trim().strip_prefix("ref: refs/heads/") {
        Some(branch) if !branch.is_empty() => Head::Branch(branch.to_string()),
        _ => Head::Detached,
    })
}

/// The branch `origin/HEAD` points to, read from git's files: the remote's
/// default branch, which can change after the project was added (a rename
/// from master to main). `None` when the repo has no such ref.
pub fn origin_head(folder: &Path) -> Option<String> {
    let dot_git = folder.join(".git");
    let common = if dot_git.is_dir() {
        dot_git
    } else {
        // A linked worktree: its gitdir has a `commondir` file pointing to the main one.
        let text = std::fs::read_to_string(&dot_git).ok()?;
        let gitdir = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
        let gitdir = if gitdir.is_absolute() { gitdir } else { folder.join(gitdir) };
        let common = std::fs::read_to_string(gitdir.join("commondir")).ok()?;
        let common = PathBuf::from(common.trim());
        if common.is_absolute() { common } else { gitdir.join(common) }
    };
    let text = std::fs::read_to_string(common.join("refs/remotes/origin/HEAD")).ok()?;
    text.trim().strip_prefix("ref: refs/remotes/origin/").filter(|b| !b.is_empty()).map(str::to_string)
}

/// Removes a session's worktree and then its branch, the way git allows
/// without losing work: `git worktree remove` refuses a worktree with
/// changes, and `git branch -d` keeps a branch that is not merged. Returns
/// what went, and git's reason for what stayed.
pub fn remove_worktree_and_branch(repo_root: &Path, worktree: &Path, branch: &str) -> (bool, bool, Option<String>) {
    let path = worktree.to_string_lossy().into_owned();
    if let Err(e) = run_git(repo_root, &["worktree", "remove", &path]) {
        return (false, false, Some(e));
    }
    match run_git(repo_root, &["branch", "-d", branch]) {
        Ok(_) => (true, true, None),
        Err(e) => (true, false, Some(e)),
    }
}

/// A `core.hooksPath` with no hooks in it, so git runs none.
#[cfg(unix)]
const NO_HOOKS: &str = "core.hooksPath=/dev/null";
#[cfg(windows)]
const NO_HOOKS: &str = "core.hooksPath=NUL";

/// The first of `agent/<slug>`, `agent/<slug>-2`, `-3` that is not a branch
/// of the folder's repo yet.
pub fn free_branch(folder: &Path, slug: &str) -> String {
    let mut n = 1;
    loop {
        let name = if n == 1 { format!("agent/{slug}") } else { format!("agent/{slug}-{n}") };
        if run_git(folder, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{name}")]).is_err() {
            return name;
        }
        n += 1;
    }
}

/// Makes the branch [`free_branch`] names at the folder's current commit
/// and points `HEAD` at it. Nothing is checked out, so the files and the
/// index stay as they are, and with hooks turned off no repo hook runs.
/// Returns the branch's name.
pub fn make_branch_in_place(folder: &Path, slug: &str) -> Result<String, WorktreeError> {
    let branch = free_branch(folder, slug);
    // A repo with no commit yet has nothing to branch from: pointing HEAD at
    // the new name is enough, and the first commit creates it.
    if run_git(folder, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_ok() {
        run_git(folder, &["-c", NO_HOOKS, "branch", "--no-track", &branch]).map_err(WorktreeError::Git)?;
    }
    run_git(folder, &["-c", NO_HOOKS, "symbolic-ref", "HEAD", &format!("refs/heads/{branch}")])
        .map_err(WorktreeError::Git)?;
    Ok(branch)
}

/// The folders whose files git runs as hooks when the repo sets
/// `core.hooksPath`, such as husky's. Husky 9 sets it to `.husky/_`, whose
/// scripts run the ones in `.husky/`, so a folder named `_` brings its
/// parent along. A relative setting is taken from `folder`. Empty when the
/// setting is not there (git's own `.git/hooks` is protected as part of
/// `.git`).
pub fn hook_dirs(folder: &Path) -> Vec<PathBuf> {
    let value = match run_git(folder, &["config", "--path", "--get", "core.hooksPath"]) {
        Ok(value) if !value.is_empty() => value,
        _ => return Vec::new(),
    };
    let path = PathBuf::from(value);
    let dir = if path.is_absolute() { path } else { folder.join(path) };
    let mut dirs = vec![dir.clone()];
    if dir.file_name().is_some_and(|name| name == "_") {
        dirs.extend(dir.parent().map(Path::to_path_buf));
    }
    dirs
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
    fn read_head_names_the_branch_or_says_detached() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("proj");
        init_repo(&dir);
        assert_eq!(read_head(&dir), Some(Head::Branch("main".into())));
        git(&dir, &["checkout", "-q", "--detach"]);
        assert_eq!(read_head(&dir), Some(Head::Detached));
        assert_eq!(read_head(t.path()), None);
    }

    #[test]
    fn read_head_follows_a_linked_worktree() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("proj");
        init_repo(&dir);
        let (path, _) = create_worktree(&inspect_repo(&dir).unwrap(), "fix").unwrap();
        assert_eq!(read_head(&path), Some(Head::Branch("agent/fix".into())));
    }

    #[test]
    fn make_branch_in_place_keeps_changes_and_runs_no_hook() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("my proj");
        init_repo(&dir);
        std::fs::write(dir.join("a.txt"), "changed").unwrap();
        std::fs::write(dir.join("new.txt"), "new").unwrap();
        git(&dir, &["add", "new.txt"]);
        let hooks = dir.join(".git").join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        for hook in ["post-checkout", "reference-transaction"] {
            let marker = dir.join(format!("{hook}-ran"));
            std::fs::write(hooks.join(hook), format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(hooks.join(hook), std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        assert_eq!(make_branch_in_place(&dir, "fix").unwrap(), "agent/fix");
        assert_eq!(read_head(&dir), Some(Head::Branch("agent/fix".into())));
        assert!(branch_exists(&dir, "agent/fix"));
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "changed");
        assert_eq!(run_git(&dir, &["diff", "--cached", "--name-only"]).unwrap(), "new.txt");
        assert!(!dir.join("post-checkout-ran").exists());
        assert!(!dir.join("reference-transaction-ran").exists());
    }

    #[test]
    fn make_branch_in_place_dedupes_and_works_before_the_first_commit() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("proj");
        init_repo(&dir);
        git(&dir, &["branch", "agent/fix"]);
        assert_eq!(make_branch_in_place(&dir, "fix").unwrap(), "agent/fix-2");
        let empty = t.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        git(&empty, &["init", "-q", "-b", "main"]);
        assert_eq!(make_branch_in_place(&empty, "first").unwrap(), "agent/first");
        assert_eq!(read_head(&empty), Some(Head::Branch("agent/first".into())));
    }

    #[test]
    fn origin_head_names_the_remote_default_branch() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("proj");
        init_repo(&dir);
        assert_eq!(origin_head(&dir), None);
        git(&dir, &["update-ref", "refs/remotes/origin/trunk", "HEAD"]);
        git(&dir, &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/trunk"]);
        assert_eq!(origin_head(&dir).as_deref(), Some("trunk"));
        let (path, _) = create_worktree(&inspect_repo(&dir).unwrap(), "fix").unwrap();
        assert_eq!(origin_head(&path).as_deref(), Some("trunk"));
    }

    #[test]
    fn a_repo_with_no_commit_gets_a_clear_worktree_error() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("empty");
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        let err = create_worktree(&inspect_repo(&dir).unwrap(), "fix").unwrap_err().to_string();
        assert!(err.contains("no commits yet"), "{err}");
    }

    #[test]
    fn hook_dirs_reads_core_hooks_path() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("proj");
        init_repo(&dir);
        assert!(hook_dirs(&dir).is_empty());
        git(&dir, &["config", "core.hooksPath", ".husky/_"]);
        assert_eq!(hook_dirs(&dir), vec![dir.join(".husky/_"), dir.join(".husky")]);
        git(&dir, &["config", "core.hooksPath", "tools/hooks"]);
        assert_eq!(hook_dirs(&dir), vec![dir.join("tools/hooks")]);
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
