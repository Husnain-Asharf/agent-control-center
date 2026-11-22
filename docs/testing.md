# Testing

On a fresh clone run `npm ci` and then `npm run sidecars` once. `tauri-build` needs the bundled binaries in `src-tauri/binaries/`, so `cargo build` and `cargo test` fail without it. CI does the same before the cargo tests.

There are four layers. CI runs the first three on every push and pull request to `main`. The smoke script is run by hand.

## UI unit tests

```
npm test
```

Runs vitest on the React code: keyboard shortcuts, store logic, formatting and similar.

## Type check and build

```
npm run build
```

Runs `tsc --noEmit` (TypeScript strict) and a production Vite build.

## Rust tests

```
cargo test --workspace
```

Covers the daemon, the hook, the protocol and the bridge. This includes the rule matching for "Always allow", the protected path checks, session lifecycle, and a test that runs the bridge against the real `accd`.

## End-to-end smoke

`scripts/smoke.mjs` drives `accd` over its Unix socket the way the app does. It uses a temporary data directory and a temporary git repo, never your real data or repos, and removes both at the end.

Build first, then run:

```
npm run sidecars
cargo build --workspace
node scripts/smoke.mjs
```

Options:

- `--fake` runs the same steps against a stand-in for Claude. It checks the plumbing only and needs no Claude Code and no usage. A fake run does not prove the real integration.
- `--out <file>` also writes the results as a markdown table.

The real run needs Claude Code installed and signed in. It makes Claude run a few `touch` commands in the temporary worktree, so it uses your Claude Code usage. Each step waits up to 90 seconds for Claude, and a full run takes about two minutes. It answers Claude's first-run prompts and switches Claude out of auto or accept-edits mode, because those modes skip the approval hook.

### What the checklist covers

1. A new session starts in `.worktrees/<slug>` on branch `agent/<slug>`.
2. Typed input reaches Claude.
3. A permission request reaches Approvals, and Allow once runs it.
4. Always allow auto-allows the next identical request, and the event log shows the policy as the actor.
5. Deny is reported by Claude and the action does not happen.
6. Reconnecting keeps the session and its scrollback.
7. After a daemon restart the session shows as Stopped, and Restart resumes the same conversation.
8. A pending approval survives the app disconnecting, and pressing Esc in Claude cancels it.
9. The `PermissionRequest` payload has `tool_name` and `tool_input`, and the approval summary is the command.

When you change sessions, approvals or the hook, run the real smoke test and note the result in your pull request.
