# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
follows [Semantic Versioning](https://semver.org/).

## [0.2.0] - 2026-11-22

Linux support.

### Added

- Linux (x86_64) builds: an AppImage, a `.deb` and an `.rpm` in each release,
  next to the macOS `.dmg` files. They are built on Ubuntu 22.04 and need
  glibc 2.35 or later. The data directory on Linux is
  `~/.local/share/AgentControlCenter`.
- CI runs the tests on Ubuntu 22.04 as well as macOS, on every branch. A failed
  check shows the last lines of its output as an annotation.

### Changed

- `claude` is also looked for in `~/.npm-global/bin` and `/usr/bin`, where npm
  puts it on Linux.
- With no `$SHELL`, the session PATH is read from zsh, else bash, else sh.
  It assumed zsh.
- The title bar leaves room for the traffic lights on macOS only. Linux has
  its own window title bar.
- The shortcut hints show `Ctrl+K` on Linux.

### Known limitations

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

[0.2.0]: ../../releases/tag/v0.2.0
[0.1.0]: ../../releases/tag/v0.1.0
