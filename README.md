# Agent Control Center

A free, open-source desktop app to run and supervise AI coding agents like Claude Code on macOS.

![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)
![Platform: macOS](https://img.shields.io/badge/platform-macOS-lightgrey.svg)
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
- A Cmd+K command palette and keyboard shortcuts.
- A soft chime when a session needs you.

## Download and install

1. Open the [latest release](../../releases/latest).
2. Download the `.dmg` for your Mac: `aarch64` for Apple Silicon, `x64` for Intel.
3. Open the `.dmg` and drag the app to Applications.

The builds are unsigned. On first open, right-click the app and choose Open, or remove the quarantine flag:

```
xattr -dr com.apple.quarantine "/Applications/Agent Control Center.app"
```

## Requirements

- macOS 13 or later (Apple Silicon or Intel).
- [Claude Code](https://docs.claude.com/en/docs/claude-code) installed and signed in. `claude` must work in a terminal.
- git.

## Quick start

1. Open the app and add a project: pick a git repository.
2. Start a session (press `n`), pick the project and describe the task.
3. When the agent asks for permission, open Approvals, read what it wants to run, then allow once, always allow or deny.

## Keyboard shortcuts

| Key | Action |
|-----|--------|
| `Cmd+K` | Open the command palette |
| `Cmd+J` | Jump to the next session that needs you |
| `n` | New session |
| `g` then `o`, `s`, `i`, `p`, `a` | Go to Overview, Sessions, Approvals, Projects, Activity |

## Platforms

macOS 13+ today. Linux and Windows are planned. Contributions are welcome.

## Build from source

You need Rust stable, Node 23 and git.

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
