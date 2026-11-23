# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
follows [Semantic Versioning](https://semver.org/).

## [0.6.0] - 2026-11-24

Helper sessions, and more control over the Glass style.

### Added

- A session can start helper sessions in its project folder for parts of a
  bigger task, list the sessions there, and send them messages. Starting a
  helper asks you in the inbox. A helper shows which session it helps, and a
  lead lists its helpers.
- Glass settings: how much it blurs and how see-through it is.

## [0.5.0] - 2026-11-24

Usage numbers, more Settings, and fixes from testing 0.4.0.

### Added

- Usage on the Overview: the Claude plan's 5-hour and weekly use with when
  each resets, each project's cost, app data and database size, and the
  memory sessions use. Each session shows its cost, how full its context is,
  lines changed, model, memory and disk. The numbers come from Claude Code's
  status line, so the line at the bottom of an app session's terminal is the
  app's.
- A Coordination view on the project page: the sessions sharing the project
  folder, the files each changed in the last 2 hours, and the files two
  sessions both changed. A session in a shared folder is told about the
  others when it starts, and to split the work through Claude Code's
  messages between sessions.
- Remove a stopped session from the app, and with its own worktree, also
  its worktree and branch when git can do it without losing work. Remove a
  project from the app. Folders and history stay.
- Settings: a color editor with presets (Default, High contrast, Warm, Blue)
  and custom colors for each theme, and a Glass style (window vibrancy on
  macOS, translucent panels on Windows and Linux).
- An Analytics screen: plan use, cost and memory over time (sampled once a
  minute, kept 7 days), and a table of every session's cost, context,
  memory, disk, lines, time working and waiting, with recommendations such
  as compacting a session whose context is 70% full.
- Compact a session (Claude Code's `/compact`) from its page or from
  Analytics.
- Settings, Restart: reload the window, restart the app, or restart the
  daemon after a confirmation.
- The base branch of a project can be changed on its page, and "Allow on the
  base branch" can be undone with "Ask again".

### Changed

- The New session description names the session and is no longer sent to
  Claude: Claude starts and waits for you. "Also send it to Claude as the
  first message" sends it, as before.

### Fixed

- A yes to the branch question no longer lets a whole command through:
  `git commit && git push` still goes through the approvals. Edits outside
  the project folder ask no branch question.
- "Allow once" in a toast or on the Overview no longer answers a branch
  question.
- The branch question says "on no branch" when the folder is not on one.
- The base branch also counts the remote's current default branch.
- A branch switched outside a session shows in its header within 2 seconds.
- Restarting a session whose folder was deleted says so instead of hanging.
- A repo with no commits gets a clear message for a separate worktree.
- At large text sizes on a narrow window, the zoom stops where the layout
  still fits.
- "Needs you" jumps to a failed session when no approval waits, a banner
  shows when the daemon is not connected, and text that still said every
  session gets its own worktree is updated.

## [0.4.0] - 2026-11-23

Sessions in the project folder, a Settings screen, and fixes from the first
run on a real Mac.

### Changed

- New sessions work in the project folder, on its current branch, so sessions
  in one project build on each other's changes. No worktree and no branch are
  made when a session starts. Choose "Separate folder and branch" in the New
  session form to give a session its own worktree and `agent/<slug>` branch,
  as before.

### Added

- The first file edit or commit of a session in the project folder, while the
  folder is on the base branch, asks in the inbox: make a branch first, allow
  it on the base branch, or deny. Make branch keeps uncommitted changes, runs
  no repo hook, and moves every session in the folder to the new branch. It
  asks in Claude Code's auto and accept-edits modes too.
- A Settings screen: theme, text size for the whole app, terminal text size,
  bolder text and the approval chime. Cmd or Ctrl with `=`, `-` and `0` change
  the text size.
- A New session button on the project page.

### Fixed

- On macOS the window can be dragged by its title bar.
- Sessions get the PATH from an interactive login shell, so folders added in
  `~/.zshrc` or `~/.bashrc`, such as nvm's Node, are found. The login shell
  alone is the fallback.

### Security

- Sessions in the project folder do not load the checkout's
  `.claude/settings.local.json`, whose saved rules would run commands without
  reaching the inbox. A request of theirs on a protected path waits for an
  approval in every permission mode.
- `.git`, the folder that `core.hooksPath` names, keystores (`.jks`, `.p12`,
  `.pfx`, `.keystore`, `.p8`), `key.properties` and `credentials*.json` and
  `service-account*.json` files are protected paths.
- Rules never cover a path in `.worktrees`, where other sessions' worktrees
  are.

## [0.3.1] - 2026-11-23

Windows fixes and updated dependencies.

### Fixed

- On Windows, git revisions such as `HEAD~1` no longer show as protected
  paths. A short (8.3) name counts as protected only when it can stand for a
  protected name, such as `CLAUDE~1`; other ones still ask each time.
- The Windows `-setup.exe` ends the daemon before it upgrades or uninstalls
  the app, so it no longer needs `taskkill` first. Running sessions end with
  the daemon.

### Changed

- React and react-dom 19.3.1, Vitest 5.1.1, and the CI actions
  actions/checkout 7, actions/setup-node 7 and tauri-action 1.

### Known limitations

- The `.msi` does not end the daemon: run `taskkill /IM accd.exe /F` before you
  upgrade or uninstall with it, and before you uninstall 0.3.0.

## [0.3.0] - 2026-11-23

Windows support.

### Added

- Windows (x64) builds: an `.msi` and an NSIS `-setup.exe` in each release,
  next to the macOS and Linux files. They need Windows 10 or 11, Git for
  Windows, and Claude Code 2.1.139 or later. The data directory on Windows is
  `%LOCALAPPDATA%\AgentControlCenter`.
- On Windows the daemon listens on a named pipe that only your user can open,
  sessions run on ConPTY, and Stop ends every process a session started.
- `claude` is found on Windows on the PATH with the extensions of `PATHEXT`
  (`claude.exe`, `claude.cmd`, `claude.bat`), then in `%USERPROFILE%\.local\bin`
  and `%APPDATA%\npm`. A
  `claude.cmd` runs through `cmd.exe`, which passes the task on unchanged,
  except that a line break becomes a space.
- On Windows the Always allow checks also know `cmd`, `powershell`, `pwsh`,
  `wsl`, `start` and `runas` as wrappers, `py` as python and `git.exe` as git,
  and protected paths match with backslashes. A protected name also matches
  with dots or spaces at its end (`.claude.`), and a short (8.3) name such as
  `CLAUDE~1` counts as a protected path.
- CI runs the tests on Windows as well as macOS and Ubuntu 22.04.

### Changed

- A request from PowerShell, the shell tool Claude Code also has on Windows,
  shows its command. Always allow allows it once and adds no rule.
- The tests use a small stand-in program for Claude Code instead of shell
  scripts, so they run the same on every platform.
- The shortcut hints show `Ctrl+K` on Windows, as on Linux.

### Known limitations

- Windows builds are new and less tested than the others. Please report what
  does not work.
- Windows builds are x64 only, and unsigned: SmartScreen warns before the
  first run.
- On Windows, Always allow never covers a shell command with an absolute path
  or a `:` in a word, or a path that Windows can read as another file, such as
  the short name `CLAUDE~1`. Such requests ask each time.
- On Windows, end the daemon (`taskkill /IM accd.exe /F`) before you upgrade
  or uninstall. A task for a `claude.cmd` can be about 7,000 characters at most.
- On Windows, Stop ends a session at once, without a hangup signal first.
- Windows has no key for "next that needs you", as on Linux.

## [0.2.0] - 2026-11-22

Linux support.

### Added

- Linux (x86_64) builds: an AppImage, a `.deb` and an `.rpm` in each release,
  next to the macOS `.dmg` files. They are built on Ubuntu 22.04 and need
  glibc 2.35 or later. The data directory on Linux is
  `~/.local/share/AgentControlCenter`.
- CI runs the tests on Ubuntu 22.04 as well as macOS, on every branch.

### Changed

- `claude` is also looked for in `~/.npm-global/bin` and `/usr/bin`, where npm
  puts it on Linux.
- With no `$SHELL`, the session PATH is read from zsh, else bash, else sh.
  It assumed zsh.
- The title bar leaves room for the traffic lights on macOS only. Linux has
  its own window title bar.
- The shortcut hints show `Ctrl+K` on Linux.

### Fixed

- The Overview says "Asked to edit" for a file edit, not "Asked to run".

### Known limitations

- Linux builds are new and less tested than the macOS ones. Please report what
  does not work.
- Linux builds are x86_64 only, and unsigned like the macOS ones.
- Linux has no key for "next that needs you" (`Cmd+J` on macOS), because
  `Ctrl+J` is a newline in the session terminal.
- No Windows build yet.

## [0.1.0] - 2026-11-22

First public release.

### Added

- `accd` daemon that keeps each Claude Code session running in its own git
  worktree (`agent/<slug>`), also after the window closes.
- `acc-hook` binary that forwards Claude Code permission requests to the daemon
  and returns the decision. Requests are denied if the daemon is unreachable.
- Approvals inbox: allow once, allow always for the project, or deny.
- "Always allow" rules that never cover protected paths (credentials, the
  agent's own config) or risky commands (inline code, push, package runners with
  flags, URLs). The approval shows what a rule covers before you choose it.
- Activity log of sessions, approvals and decisions.
- Tauri 2 and React UI: Overview with fleet timelines, Sessions, live terminal,
  Approvals, Projects and Activity.
- Command palette (Cmd+K) and keyboard shortcuts.
- Short chime when a new approval is waiting. Turn it off from the command
  palette ("Turn sound off").
- Restart of a stopped session, resuming the same Claude conversation.
- Unsigned macOS builds for Apple Silicon and Intel.

### Known limitations

- Claude Code is the only agent.
- The daemon socket has no authentication. It is mode 0600, but any process of
  the same user can use it.
- Terminal scrollback is kept in memory and lost when the daemon restarts.
- Rules cannot be edited in the app.
- Worktrees and `agent/*` branches are not removed automatically.
- Local only and macOS only.

[0.6.0]: ../../releases/tag/v0.6.0
[0.5.0]: ../../releases/tag/v0.5.0
[0.4.0]: ../../releases/tag/v0.4.0
[0.3.1]: ../../releases/tag/v0.3.1
[0.3.0]: ../../releases/tag/v0.3.0
[0.2.0]: ../../releases/tag/v0.2.0
[0.1.0]: ../../releases/tag/v0.1.0
