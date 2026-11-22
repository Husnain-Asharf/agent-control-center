# Contributing

Thanks for helping with Agent Control Center. This guide covers setup, layout, tests and how to send a change.

By taking part you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Development setup

You need macOS 13 or later, Rust stable 1.100 or newer, Node 23, git, and Claude Code installed and signed in (needed only for the real smoke test).

```
npm ci
npm run sidecars
npm run tauri dev
```

`tauri-build` needs the bundled binaries, so `npm run sidecars` must run once on a fresh clone before `cargo build` or `cargo test`. `npm run tauri dev` runs it itself and builds `accd` and `acc-hook` first. A running `accd` keeps serving, even one from an older build. After you change the daemon or the hook, run `pkill accd` and start the app again.

To make a release bundle, run `npm run tauri build`. Its `beforeBuildCommand` runs `npm run sidecars && npm run build`. `npm run sidecars` builds `accd` and `acc-hook` in release mode and copies them to `src-tauri/binaries/` as `accd-<target-triple>` and `acc-hook-<target-triple>`. They are bundled through `bundle.externalBin` and end up next to the app binary in `Contents/MacOS/`.

## Project layout

- `crates/accd`: the daemon. Sessions, PTYs, worktrees, approvals, rules, SQLite, the socket server.
- `crates/acc-hook`: the hook binary Claude Code calls. Forwards hooks to `accd`.
- `crates/acc-protocol`: message types shared by the daemon, the hook and the bridge.
- `src-tauri`: the Tauri app and the bridge to the daemon.
- `src`: the React UI (screens, components, dialogs, store, terminal).
- `scripts`: the end-to-end smoke script.
- `docs`: extra documentation.

## Running tests

```
npm run sidecars          # once on a fresh clone
npm test                  # UI unit tests (vitest)
npm run build             # type check and production build of the UI
cargo test --workspace    # Rust tests
node scripts/smoke.mjs --fake   # end-to-end plumbing, no Claude
node scripts/smoke.mjs          # end-to-end with real Claude Code
```

The real smoke run starts Claude Code and makes it run a few commands, so it uses your Claude Code usage. Run it when you change session handling, approvals or the hook, and say so in the PR. CI runs the first three commands, not the smoke script. See [docs/testing.md](docs/testing.md).

## Code style

- Rust: `cargo fmt` before you commit. Keep `cargo test --workspace` green.
- TypeScript: strict mode is on. `npm run build` must pass with no type errors.
- Keep pull requests small and focused on one change. Add or update tests with the change.
- Anything that widens what "Always allow" covers needs a test and a clear reason. See [docs/safety.md](docs/safety.md).

## Commit messages

Use a conventional prefix and a short subject, for example:

- `feat: show the rule preview in the approval`
- `fix: deny a pending approval when the run ends`
- `docs: explain the data directory`
- `test: run the bridge against the real accd`
- `chore: bump the Tauri version`

Add a body only when the reason is not clear from the diff.

## Pull request checklist

- [ ] The change is focused and the title uses a conventional prefix.
- [ ] `npm test`, `npm run build` and `cargo test --workspace` pass.
- [ ] New behavior has tests.
- [ ] `cargo fmt` has been run.
- [ ] Docs and `CHANGELOG.md` are updated if users will notice the change.
- [ ] No secrets, personal paths or local data in the diff.
- [ ] If you changed sessions, approvals or the hook, you ran the real smoke test, or say why not.

## Maintainers

To release, bump the version in `package.json`, `src-tauri/tauri.conf.json` and the Cargo manifests, add a [CHANGELOG.md](CHANGELOG.md) entry, commit, tag `vX.Y.Z` and push the tag. The release workflow builds both `.dmg` files on `macos-latest` (the Intel build is cross-compiled) and creates a draft GitHub release. Edit the notes and publish it.

Before the first release, enable Actions and allow the workflow token write access (Settings, Actions, General, Workflow permissions). The first tagged release is the first real run of the workflow.

Upload `docs/images/social-preview.png` as the repository social preview (Settings, General, Social preview), and enable private vulnerability reporting (Settings, Code security).

## Reporting bugs and proposing features

Use the [issue forms](../../issues/new/choose). For a bug, include your macOS version, chip, app version, `claude --version`, and the relevant lines from `accd.log` in the data directory (remove secrets first). For a feature, describe the problem before the solution. For security issues do not open a public issue. See [SECURITY.md](SECURITY.md).
