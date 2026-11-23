# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
follows [Semantic Versioning](https://semver.org/).

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

[0.3.1]: ../../releases/tag/v0.3.1
[0.3.0]: ../../releases/tag/v0.3.0
[0.2.0]: ../../releases/tag/v0.2.0
[0.1.0]: ../../releases/tag/v0.1.0
