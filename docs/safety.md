# Safety and privacy

This page has the full safety rules for Agent Control Center: how approvals work, what "Always allow" covers, session PATH, where data lives, keyboard shortcuts and known limitations. The [README](../README.md) has the short version.

## Session PATH and finding claude

Sessions use the PATH from your login shell (`$SHELL`, or zsh, else bash, else sh when `$SHELL` is not set), read once when the first session starts. The shell is asked as an interactive login shell first (`-l -i`), so it also reads `~/.zshrc`, where nvm is often set up, and `~/.bashrc` when `~/.bash_profile` or `~/.profile` reads it, as most do. That try gets 5 seconds. If it fails, takes longer, or prints no absolute folder, `accd.log` says why, and the shell is asked again as a login shell only (`-l`, up to 3 seconds), which reads only the login files such as `~/.zprofile` and `~/.profile`. After the PATH come the folders of the app's own PATH that it lacks, then the folder of the `claude` that was found. If the login shell fails too, `accd.log` says why, and sessions get the app's PATH with `/opt/homebrew/bin`, `/usr/local/bin`, `~/.local/bin` and the folder of `claude` added. The app finds `claude` on that PATH, or in `~/.local/bin`, `~/.claude/local`, `~/.npm-global/bin`, `/opt/homebrew/bin`, `/usr/local/bin` and `/usr/bin`. Set `$ACC_CLAUDE_BIN` to point at a specific binary.

Windows has no login shell. Sessions get the app's PATH, which an app started from the Start menu takes from the system settings, followed by the folder of the `claude` that was found. The app finds `claude` on that PATH with the extensions of `PATHEXT` that Windows can start (`claude.exe`, `claude.cmd`, `claude.bat`), or in `%USERPROFILE%\.local\bin` (the native installer) and `%APPDATA%\npm` (npm). A `claude.cmd` runs through `cmd.exe`, and a line break in its task becomes a space. `$ACC_CLAUDE_BIN` works there too.

## Safety model

**Approvals.** A permission request from Claude Code is held by `acc-hook` until you decide. The request waits for your decision in the inbox. Claude Code runs some actions without asking, depending on its own permission mode (see Known limitations). If the daemon is unreachable the request is denied, and a pending approval expires after 55 minutes.

**Sessions in the project folder.** A new session works in the project's own folder, on the branch the folder is on. No worktree and no branch are made when it starts.

- The first time it wants to edit a file, or to run a git command that commits on the current branch (`commit`, `merge`, `cherry-pick`, `revert`, `pull`, `am` or `rebase`, anywhere in the command), while the folder is on the project's base branch or on no branch, the inbox asks first.
- **Make branch** creates `agent/<slug>` at the current commit without a checkout: the files, the index and uncommitted changes stay as they are, and no repo hook runs. Every session in the folder goes on with the new branch, and their own branch questions are settled by it.
- **Allow on `<base>`** lets this session change the base branch, and it is not asked again. **Deny** blocks the call.
- The question comes in every Claude Code permission mode, auto and accept-edits included.
- A request on a protected path (below) waits for an approval in every mode too, because Claude Code reads files in its own folder without asking, and the project folder holds the real `.env` and key files that a worktree never has.
- These sessions start with `--setting-sources user,project`, so the checkout's untracked `.claude/settings.local.json` is not loaded. Its saved rules would run commands without asking, and the inbox would never see them. Its local hooks and plugin choices are not loaded either.
- Choose **Separate folder and branch** in the New session form to give a session a worktree of its own instead (see [Data and privacy](#data-and-privacy)).

**Always allow.** `Always allow in <project>` saves a rule for that project. The approval shows what the rule will cover before you choose it.

- Bash: commands that start with the same first two words, such as `npm install`. For a runner the rule names what it runs: `npx vitest`, `uv run pytest`, `python3 -m pytest`. A rule covers a command only when it has no shell operators (`;`, `&`, `|`, `` ` ``, `$(`, `>`, `<`, newline), no expansions (`*`, `?`, `[`, `{`, `\`, `~`, `$`), no `..`, no URL, no absolute path outside the session's folder or in its `.worktrees` (also when the path is attached to a flag, `-C/path`), and at most 256 words.
- Edit, Write, MultiEdit, Read, NotebookEdit, Glob and Grep: files inside the session's folder (its worktree, or the project folder), except other sessions' worktrees in `.worktrees`.
- WebFetch: URLs with the same origin (scheme, host and port).

**What Always allow never covers.** For these, Always allow allows the request once and saves nothing, and no saved rule ever allows them. Program names match in any case, because macOS file systems ignore case (`GIT` runs git).

- a request on a protected path (below);
- a command with shell operators or expansion characters, an absolute path outside the session's folder, a path in `.worktrees`, `..` or a URL, or one of more than 256 words or with more than 8 runners nested;
- inline code: an interpreter with code on its command line. The interpreters are `python` (any python or pypy build, such as `python3.12` or `python3.13t`), `node`, `deno`, `bun`, `tsx`, `ts-node`, `ipython`, `bash`, `sh`, `zsh`, `fish`, `dash`, `ksh`, `tcsh`, `csh`, `perl`, `ruby`, `php` and `osascript`. The flags are:
  - `-c`, `-e`, `-E`, `-p`, `-r`, `eval`, `--eval`, `--print` and `--command`;
  - `--require`, `--import`, `--loader` and `--preload`, which load code before the script;
  - any of these in a cluster of short flags (`-lc`), with the code attached (`-c'...'`), or as an abbreviated long option (`fish --comm`);
- Python modules that run code from their arguments: `timeit`, `pdb`, `code`, `runpy`, `trace`, `cProfile` and `profile`, run with `-m` anywhere in the command. That includes through another module runner (`coverage run -m timeit`), and python or `coverage` with `-m` inside a cluster of flags (`python3 -Im`, `coverage run -Lm`);
- wrapper commands: `env`, `xargs`, `sudo`, `doas`, `nohup`, `time`, `nice`, `timeout`, `watch`, `exec`, `command`, `builtin`, `eval`, `source`, `.`, `arch`, `caffeinate` and `xcrun`, also when a runner runs them (`uv run env`);
- a command that starts with a variable assignment (`FOO=1 make`);
- `git push`, `gh pr`, `gh api`, `glab mr` and `glab api`;
- git options that set config: `git -c`, `--config-env`, and for `clone` also `--config` and `-c` stuck to its value or in a cluster (`-qc`);
- git options and subcommands that run another program:
  - `-x` and `--exec`, `--extcmd`, `--upload-pack`, `--receive-pack`, `-u` with `clone`, `fetch` or `ls-remote`, `-O` and `--open-files-in-pager`, `--ext-diff`, and `--exec-path`;
  - `submodule foreach`, `bisect run` and `filter-branch`;
  - long options count abbreviated too (`--ex`, `--upl`), and the short letters count in any git command, so `git clean -x` asks every time;
- `git config` that changes a setting. Reads keep their rule: `--get`, `--get-all`, `--get-regexp`, `--list` or `-l` (also with `--show-origin` or a scope such as `--global`), `git config <key>`, `git config get <key>` and `git config list`;
- a runner followed by a flag or by nothing, such as `npx -y`, because a rule would cover anything it runs. The same goes for a runner with a flag before its subcommand (`npm --prefix web exec`), and for `mise exec` with `--`, `-c` or `--command`. The runners are:
  - `npx`, `bunx`, `pnpx`, `uvx`, `pnpm dlx`, `yarn dlx`, `npm exec`, `npm x`, `bun x`, `pipx run` and `uv tool run`;
  - `uv run`, `poetry run`, `pnpm exec`, `yarn exec`, `bundle exec`, `pipenv run`, `pdm run`, `hatch run`, `rye run`, `pixi run`, `conda run`, `deno run` and `mise exec`;
  - `go run`, `python -m` and `coverage run -m`;
- a command whose first two words no rule could match, such as `cat /etc/hosts`;
- tools with no known input shape, such as MCP tools and WebSearch;
- PowerShell commands, from the shell tool Claude Code also has on Windows.

On Windows, also `cmd`, `powershell`, `pwsh`, `wsl`, `start` and `runas` count as wrappers, `py` counts as python, and a program name counts without `.exe`, `.com`, `.cmd` or `.bat` (`git.exe push` is a push). A path or a shell word that Windows can read as another file, such as the short name `CLAUDE~1`, gets no rule either (see Known limitations).

Rules for runners name what they run: `npx vitest`, `uv run pytest`, `python3 -m pytest`. A WebFetch rule covers one origin.

**Protected paths**, matched in any case in the request's summary and in its path fields. On Windows a backslash separates folders too:

- a file whose name starts with `.env`, such as `.env.local` or `.envrc`;
- a file whose name ends with `.pem`, `.key`, `.jks`, `.p12`, `.pfx`, `.keystore` or `.p8`;
- a JSON file whose name starts with `credentials` or `service-account`;
- anything in or through `.ssh`, `.aws`, `.gnupg`, `.kube`, `.docker`, `.claude` or `.git` (its hooks and config run code later), and the files `.netrc`, `.git-credentials`, `.npmrc`, `.pypirc`, `.mcp.json` and `key.properties`;
- anything in the folder the repo's `core.hooksPath` names, read when the session starts, such as husky's `.husky/_` and `.husky`;
- anything in `.config/gh` or `.config/gcloud`.

On Windows a name also matches without the dots and spaces at its end, which Windows drops (`.claude.` is `.claude`). A short (8.3) name, a `~` followed by a digit, counts as a protected path when it can stand for one of these names: its first two characters are those of a protected name's short form, as in `CLAUDE~1` and `CL3F2A~1` (`.claude`), `ENV~1`, `SSH~1`, `MCP~1.JSO` or `CONFIG~1`, or its extension is that of a protected file, such as `JSO`, `PEM`, `KEY` or `JKS`, as in `SETTIN~1.JSO`. Other short names, such as the git revision `HEAD~1`, are not protected paths, but they still ask each time. Short names in the session's own worktree path do not count.

**Editing rules by hand.** Rules cannot be edited in the app yet. They live in `acc.db` in the [data directory](#data-and-privacy), table `rules`, and `accd` reads them on every request, so a change applies at once. To list them with their project names (on Linux the file is `~/.local/share/AgentControlCenter/acc.db`, on Windows `%LOCALAPPDATA%\AgentControlCenter\acc.db`):

```
sqlite3 ~/Library/Application\ Support/AgentControlCenter/acc.db <<'SQL'
SELECT r.rowid, json_extract(p.data, '$.name') AS project, r.tool, r.prefix
FROM rules r LEFT JOIN projects p ON p.id = r.project_id;
SQL
```

To delete one, by the `rowid` from that list:

```
sqlite3 ~/Library/Application\ Support/AgentControlCenter/acc.db "DELETE FROM rules WHERE rowid = 3;"
```

## Keyboard shortcuts

| Key | Action |
|-----|--------|
| `Cmd+K` (`Ctrl+K` on Linux and Windows) | Open the command palette |
| `Cmd+J` (macOS only) | Jump to the next session that needs you |
| `Cmd+=` and `Cmd+-` (`Ctrl+=` and `Ctrl+-` on Linux and Windows) | Make the text bigger or smaller |
| `Cmd+0` (`Ctrl+0` on Linux and Windows) | Set the text size back to 100% |
| `n` | New session |
| `g` then `o` | Go to Overview |
| `g` then `s` | Go to Sessions |
| `g` then `i` | Go to Approvals |
| `g` then `p` | Go to Projects |
| `g` then `a` | Go to Activity |

Plain keys do not fire while you type in an input, the terminal or a dialog. Linux and Windows have no key for the jump, because `Ctrl+J` is a newline in the session's terminal: click the "needs you" button in the title bar instead. The text size keys work everywhere, also in the terminal, which does not use them. The Settings screen has the same text size, and a separate one for the terminal.

## Data and privacy

Everything stays on your machine. The app has no telemetry, no accounts and makes no network calls of its own. Claude Code itself talks to Anthropic as usual.

Data lives in `$ACC_DATA_DIR` if set, else:

- macOS: `~/Library/Application Support/AgentControlCenter`;
- Linux: `~/.local/share/AgentControlCenter` (`$XDG_DATA_HOME/AgentControlCenter` if `XDG_DATA_HOME` is set);
- Windows: `%LOCALAPPDATA%\AgentControlCenter`.

It holds:

- `accd.sock`: the Unix socket, mode 0600. The only way in. On Windows there is a named pipe instead, `\\.\pipe\agent-control-center-<id>` with an id made from the data directory. Only your user can open it, clients on other machines cannot reach it, and the app and `acc-hook` only talk to a pipe that a process of your user serves.
- `accd.lock`: keeps one daemon per data dir.
- `accd.log`: output of `accd`.
- `acc.db`: SQLite database with projects, sessions, approvals, rules and the event log.
- `sessions/<session id>/claude-settings.json`: the hook settings passed to Claude Code.

A session with a separate folder works in a worktree in the project repo, at `.worktrees/<slug>` on branch `agent/<slug>`. The daemon adds `.worktrees/` to `.git/info/exclude`. A session in the project folder makes no worktree, and Make branch creates `agent/<slug>` in the folder itself.

To stop the daemon, and the sessions with it:

```
pkill accd
```

On Windows: `taskkill /IM accd.exe /F`.

`accd` removes the variables that mark a nested Claude Code session (`CLAUDECODE`, `CLAUDE_CODE_ENTRYPOINT`, `CLAUDE_CODE_SSE_PORT` and `CLAUDE_CODE_CHILD_SESSION`) from each session's environment, so the app also works when started from a terminal inside Claude Code. It also removes a parent agent session's own channels (`CLAUDE_CODE_MESSAGING_SOCKET`, `CLAUDE_CODE_MESSAGING_TOKEN` and `CLAUDE_CODE_CHILD_NOTES_DIR`), so that token never reaches another agent. Every other variable reaches the session.

## Known limitations

- Unsigned builds. See the README for how to open them.
- On macOS and Linux, the PATH for sessions comes from your login shell, read once at the first session, followed by the app's own PATH. If the shell fails or takes more than 5 seconds with `~/.zshrc` or `~/.bashrc`, a PATH set only there is not seen: move it to `~/.zprofile` or `~/.profile`. Changes need a daemon restart (`pkill accd`).
- Claude Code is the only agent.
- Terminal scrollback is kept in memory, 1 MiB per session. It is lost when the daemon restarts.
- Sessions die with the daemon. After a restart they show as Stopped, and `Restart` resumes the same Claude conversation (`--resume`).
- On Windows, Stop ends the session's processes at once. On macOS and Linux they get a hangup signal and a second to exit first.
- Approvals fail closed. If `acc-hook` cannot reach the daemon, the request is denied. A pending approval expires after 55 minutes.
- Claude Code's auto and accept-edits permission modes do not ask. Claude runs what the mode accepts and the request never reaches the inbox. Keep sessions in the default mode (Shift+Tab in the session's terminal changes it). In the project folder, the branch question and protected paths still ask.
- Sessions in the project folder edit your own checkout. If you edit the same files at the same time, the changes mix. Sessions in one folder also share git: one session's `git stash`, `git checkout -- .` or `git reset --hard` can drop the others' uncommitted work.
- Two sessions writing one file at the same moment: Claude Code's Edit refuses a file that changed since it was read, but Write and shell commands do not check.
- A shell command that changes files on the base branch without committing, such as `npm install` or `sed -i`, does not ask the branch question. It goes through the normal approvals.
- The socket has no authentication. It is mode 0600 (on Windows only your user can open the pipe), so other users cannot connect, but any process of yours can, code the agent runs included, and it can do what the app does, such as approve its own requests. See [SECURITY.md](../SECURITY.md). Authentication is planned.
- On Windows the hooks need Claude Code 2.1.139 or later, which starts a hook program directly, with no shell.
- On Windows the session terminal is ConPTY, which draws the screen rather than passing every byte through. Output that scrolls by very fast can be missing from the scrollback.
- Rules cannot be edited in the app yet.
- Always allow for shell commands is best-effort pattern matching. Check the rule preview before you choose it, and prefer Allow once for anything that runs other programs. It does not detect:
  - git aliases;
  - textconv and diff drivers named in `.gitattributes`;
  - `GIT_*` variables already exported in the shell;
  - external `git-<name>` programs;
  - interpreters and code flags not on the list above, such as `fish -C`;
  - code run from files in the session's folder: `npm install` lifecycle scripts, `git clone --template`, and `npm run` or `make` targets. Edits in `.git` and in the `core.hooksPath` folder are protected, but a `core.hooksPath` set after the session started is not known until it restarts.
- On Windows, Always allow never covers a shell command with an absolute path or a `:` in a word, such as `C:\x`, `/c/x`, `D:notes` or `host:3000`. It asks each time.
- On Windows, Always allow never covers a short (8.3) name such as `CLAUDE~1`, a name that ends in a dot or a space, a device name such as `NUL` or `COM1`, a data stream such as `file::$DATA`, or a `\\?\` or `\\.\` prefix. Windows can read these as another file, so they ask each time.
- On Windows, a git revision such as `HEAD~1` looks like a short name, so a command with one asks each time. It shows as a protected path only when its first two characters match a protected name's short form, as in `cleanup~1` (`.claude`) or `docs~1` (`.docker`).
- On Windows, credentials that tools keep under `AppData`, such as those of gh and gcloud, are not protected paths.
- On Windows the `-setup.exe` ends the daemon before it upgrades or uninstalls the app, and running sessions end with it. The `.msi` does not: end the daemon first (`taskkill /IM accd.exe /F`), as you must before uninstalling 0.3.0, because Windows cannot replace or remove the files of a running program.
- If you set `ACC_DATA_DIR` on Windows, point it at a folder only you can open. The default, under `%LOCALAPPDATA%`, is private.
- On Windows, a task for a `claude.cmd` (npm) can be about 7,000 characters at most, because that is what cmd.exe can run. The native `claude.exe` takes longer ones.
- Worktrees and `agent/*` branches are never removed. Remove them yourself (`git worktree remove`, `git branch -D`) when you no longer need them.
- A worktree starts from the repo's `HEAD`, so uncommitted changes in your checkout are not in it.
- Local only: one machine, no remote hosts. Windows builds are x64 only, and Linux builds x86_64 only.
