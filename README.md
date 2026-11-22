# Agent Control Center

A free, open-source desktop app to run and supervise AI coding agents like Claude Code on macOS and Linux.

![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)
![Platform: macOS and Linux](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-lightgrey.svg)
![Tauri 2](https://img.shields.io/badge/Tauri-2-24C8DB.svg)
![Rust](https://img.shields.io/badge/Rust-daemon-orange.svg)
![React](https://img.shields.io/badge/React-UI-61DAFB.svg)

## What it does

Agent Control Center runs several Claude Code sessions in one window. Each session is a real coding agent in its own terminal and its own git worktree, owned by a small local daemon, so it keeps running when you close the window. Permission requests from all sessions land in one approvals inbox.

## Features

- Each session runs in its own git worktree and `agent/<slug>` branch, so your checkout is untouched.
- Sessions keep running when the window closes. Reopen the app and reattach.
- A live terminal for each session, in the real Claude Code UI.
- One approvals inbox with Allow once, Always allow and Deny.
- "Always allow" never covers secrets files or risky commands.
- An overview of all sessions and an activity log of every approval and event.
- A command palette (Cmd+K, or Ctrl+K on Linux) and keyboard shortcuts.
- A soft chime when a session needs you.

## Download and install

Open the [latest release](../../releases/latest) and download the file for your system. The builds are unsigned.

### macOS

1. Download the `.dmg` for your Mac: `aarch64` for Apple Silicon, `x64` for Intel.
2. Open the `.dmg` and drag the app to Applications.

On first open, right-click the app and choose Open, or remove the quarantine flag:

```
xattr -dr com.apple.quarantine "/Applications/Agent Control Center.app"
```

### Linux

The Linux builds are for x86_64.

- AppImage, for any distribution: make it executable, then run it.

  ```
  chmod +x <file>.AppImage
  ./<file>.AppImage
  ```

  If it says it needs FUSE, install FUSE 2: `sudo apt install libfuse2` on Ubuntu 22.04, `libfuse2t64` on 24.04 and later.
- `.deb`, for Debian and Ubuntu. It installs the app and what it needs:

  ```
  sudo apt install ./<file>.deb
  ```

- `.rpm`, for Fedora: `sudo dnf install ./<file>.rpm`.

## Requirements

- macOS 13 or later (Apple Silicon or Intel), or Linux on x86_64 with glibc 2.35 or later, such as Ubuntu 22.04, Debian 12, Fedora 36 or newer.
- [Claude Code](https://docs.claude.com/en/docs/claude-code) installed and signed in. `claude` must work in a terminal.
- git.

## Quick start

1. Open the app and add a project: pick a git repository.
2. Start a session (press `n`), pick the project and describe the task.
3. When the agent asks for permission, open Approvals, read what it wants to run, then allow once, always allow or deny.

## Keyboard shortcuts

| Key | Action |
|-----|--------|
| `Cmd+K` (`Ctrl+K` on Linux) | Open the command palette |
| `Cmd+J` (macOS only) | Jump to the next session that needs you |
| `n` | New session |
| `g` then `o`, `s`, `i`, `p`, `a` | Go to Overview, Sessions, Approvals, Projects, Activity |

## Platforms

macOS 13+ and Linux (x86_64). Windows is planned. Contributions are welcome.

## Build from source

You need Rust stable, Node 23 and git. On Linux you also need the Tauri system packages listed in [CONTRIBUTING.md](CONTRIBUTING.md).

```
npm ci
npm run tauri dev
```

On a fresh clone, run `npm run sidecars` before `cargo build` or `cargo test`. See [CONTRIBUTING.md](CONTRIBUTING.md) for tests and the full workflow.

## Safety and privacy

- Permission requests from Claude Code wait for your decision in the inbox. If the daemon cannot be reached, the request is denied.
- "Always allow" is conservative and shows what a rule covers before you choose it.
- Everything stays on your machine: no telemetry, no account, no network calls of its own. Claude Code itself talks to Anthropic as usual.
- The daemon socket has no authentication yet. It is limited to your user (mode 0600).

The full rules, protected paths, data locations, keyboard shortcuts and known limitations are in [docs/safety.md](docs/safety.md).

## Links

- [How it works](docs/architecture.md)
- [Roadmap](docs/roadmap.md)
- [Contributing](CONTRIBUTING.md)
- [Security](SECURITY.md)
- [Changelog](CHANGELOG.md)
- [License](LICENSE) (MIT)

Agent Control Center is an independent open-source project and is not affiliated with or endorsed by Anthropic. Claude and Claude Code are trademarks of Anthropic.
