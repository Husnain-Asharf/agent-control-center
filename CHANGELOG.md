# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
follows [Semantic Versioning](https://semver.org/).

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

[0.1.0]: ../../releases/tag/v0.1.0
