#!/usr/bin/env node
// End-to-end smoke test. Drives target/debug/accd over its Unix socket
// the way the app does (newline-delimited JSON), with real Claude Code.
//
//   cargo build --workspace --offline
//   node scripts/smoke.mjs [--fake] [--out <file>] [--keep]
//
// --fake  uses target/debug/stand-in (crates/accd/tests/fixtures/stand-in.rs)
//         instead of Claude and simulates the permission hooks over a second
//         connection. It checks the plumbing only; items that need a real
//         Claude are SKIP (fake).
// --out   also writes the results as a markdown table to <file>.
// --keep  leaves the temp dirs in place (for debugging).
//
// Everything lives in a temp dir (a temp data dir and a temp git repo named
// acc-smoke). The real data dir and real repos are never touched.

import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
import { fileURLToPath } from 'node:url';

const args = process.argv.slice(2);
const FAKE = args.includes('--fake');
const KEEP = args.includes('--keep');
const outIdx = args.indexOf('--out');
const OUT_FILE = outIdx >= 0 ? args[outIdx + 1] : null;

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const BIN = path.join(ROOT, 'target/debug');
const ACCD = path.join(BIN, 'accd');
const ACC_HOOK = path.join(BIN, 'acc-hook');
const FIXTURE = path.join(BIN, 'stand-in');

const CLAUDE_WAIT = 90_000; // Claude to reach a tool call, or to finish a turn
const STEP_WAIT = 30_000; // everything else
const SOCKET_WAIT = 15_000;
const PROTOCOL_VERSION = 1;
const DENIED_MESSAGE = 'Denied in Agent Control Center.';
const ALLOW_REPLY = '{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}';
const PRE_TOOL_ALLOW = '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow"}}';
const DENY_REPLY = `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"${DENIED_MESSAGE}"}}}`;
// Claude Code's first-run folder trust prompt.
// Matched with whitespace removed, because the TUI positions words with cursor moves.
const TRUST_RE = /isthisaprojectyoucreated|doyoutrustthefiles|trustthefilesinthisfolder|doyouwanttoworkinthisfolder|yes,itrustthisfolder/i;
const DENY_RE = /agent control center|denied|declined|rejected/i;
const ANSI_RE = /\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()][A-Za-z0-9]|\x1b[=>78]/g;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const b64 = (s) => Buffer.from(s).toString('base64');
const clean = (buf) => buf.toString('utf8').replace(/\x1b\[\d*C/g, ' ').replace(ANSI_RE, '').replace(/\r/g, '');
const log = (...a) => console.log(...a);

// ---------------------------------------------------------------- client

class Timeout extends Error {}

/** One connection to accd. Keeps every event it receives. */
class Client {
  constructor() {
    this.sock = null;
    this.buf = '';
    this.nextId = 0;
    this.calls = new Map();
    this.events = [];
    this.waiters = [];
    this.closed = false;
    this.tracker = null;
  }

  static async connect(socketPath) {
    const c = new Client();
    c.sock = await new Promise((resolve, reject) => {
      const s = net.createConnection(socketPath);
      s.once('connect', () => resolve(s));
      s.once('error', reject);
    });
    c.sock.on('data', (d) => c.onData(d));
    c.sock.on('close', () => {
      c.closed = true;
      for (const { reject } of c.calls.values()) reject(new Error('connection closed'));
      c.calls.clear();
    });
    c.sock.on('error', () => {});
    return c;
  }

  static async app(socketPath) {
    const c = await Client.connect(socketPath);
    const hello = await c.call('hello', { client: 'app', version: PROTOCOL_VERSION });
    if (hello.version !== PROTOCOL_VERSION) throw new Error(`unexpected hello reply ${JSON.stringify(hello)}`);
    return c;
  }

  onData(d) {
    this.buf += d.toString('utf8');
    let i;
    while ((i = this.buf.indexOf('\n')) >= 0) {
      const line = this.buf.slice(0, i);
      this.buf = this.buf.slice(i + 1);
      if (line.trim()) this.onMsg(JSON.parse(line));
    }
  }

  onMsg(msg) {
    if (msg.type === 'event') {
      this.events.push(msg.event);
      if (this.tracker) this.tracker.feed(msg.event);
      this.waiters = this.waiters.filter((w) => !w.test(this.events.length - 1));
    } else {
      const call = this.calls.get(msg.id);
      if (!call) return;
      this.calls.delete(msg.id);
      if (msg.type === 'response') call.resolve(msg.result);
      else call.reject(Object.assign(new Error(`${msg.code}: ${msg.message}`), { code: msg.code }));
    }
  }

  call(method, params) {
    const id = ++this.nextId;
    const req = params === undefined ? { id, method } : { id, method, params };
    return new Promise((resolve, reject) => {
      this.calls.set(id, { resolve, reject });
      this.sock.write(JSON.stringify(req) + '\n');
    });
  }

  /** Sends a hook request (no hello). Resolves with the HookReply, or null if the daemon closed the connection. */
  hookCall(sessionId, payload) {
    const id = ++this.nextId;
    return new Promise((resolve) => {
      this.calls.set(id, { resolve, reject: () => resolve(null) });
      this.sock.write(JSON.stringify({ id, method: 'hook', params: { session_id: sessionId, payload } }) + '\n');
    });
  }

  mark() {
    return this.events.length;
  }

  /** Resolves with the first event at or after index `from` that matches `pred`. */
  waitEvent(desc, pred, ms = STEP_WAIT, from = 0) {
    return new Promise((resolve, reject) => {
      const w = { timer: null };
      w.test = (i) => {
        const ev = this.events[i];
        if (i < from || !pred(ev)) return false;
        clearTimeout(w.timer);
        resolve(ev);
        return true;
      };
      for (let i = from; i < this.events.length; i++) if (w.test(i)) return;
      w.timer = setTimeout(() => {
        this.waiters = this.waiters.filter((x) => x !== w);
        reject(new Timeout(`timed out after ${ms / 1000}s waiting for ${desc}`));
      }, ms);
      this.waiters.push(w);
    });
  }

  close() {
    this.sock?.destroy();
  }
}

/** Collects one session's output stream (offsets are per run) so each byte counts once. */
class Tracker {
  constructor(client, sessionId, onChunk) {
    this.sessionId = sessionId;
    this.buf = Buffer.alloc(0);
    this.end = 0;
    this.onChunk = onChunk;
    client.tracker = this;
  }

  seed(bytes, end) {
    this.buf = Buffer.from(bytes);
    this.end = end;
  }

  feed(ev) {
    if (ev.kind !== 'session_output' || ev.data.session_id !== this.sessionId) return;
    const bytes = Buffer.from(ev.data.data_b64, 'base64');
    const { offset } = ev.data;
    let chunk = bytes;
    if (offset + bytes.length <= this.end) return;
    if (offset < this.end) chunk = bytes.subarray(this.end - offset);
    this.buf = Buffer.concat([this.buf, chunk]);
    this.end = Math.max(this.end, offset + bytes.length);
    this.onChunk?.(this);
  }

  /** Output after byte position `mark`, cleaned of terminal escapes. */
  since(mark = 0) {
    return clean(this.buf.subarray(mark));
  }

  tail(n = 2048) {
    return clean(this.buf).slice(-n);
  }
}

// ---------------------------------------------------------------- environment

const results = new Map();
let tmpRoot = null;
let accd = null;
let accdLog = null;
let stragglerToken = null;
let main = null;
let hookConns = [];
let cleaned = false;

function sh(cmd, argv, opts = {}) {
  const r = spawnSync(cmd, argv, { encoding: 'utf8', ...opts });
  if (r.status !== 0) throw new Error(`${cmd} ${argv.join(' ')} failed: ${r.stderr || r.stdout}`);
  return r.stdout.trim();
}

function git(dir, ...argv) {
  return sh('git', ['-C', dir, ...argv]);
}

function makeRepo(parent) {
  const repo = path.join(parent, 'acc-smoke');
  fs.mkdirSync(repo);
  git(repo, 'init', '-q', '-b', 'main');
  fs.writeFileSync(path.join(repo, 'README.md'), '# acc-smoke\n');
  git(repo, 'add', '.');
  git(repo, '-c', 'user.name=t', '-c', 'user.email=t@t', '-c', 'commit.gpgsign=false', 'commit', '-q', '-m', 'init');
  return repo;
}

async function startAccd(dataDir) {
  // The environment is passed on as is. accd itself removes the variables that would make
  // Claude a session nested in one that started this script.
  const env = { ...process.env, ACC_DATA_DIR: dataDir };
  if (FAKE) env.ACC_CLAUDE_BIN = FIXTURE;
  else delete env.ACC_CLAUDE_BIN;
  accdLog = fs.openSync(path.join(path.dirname(dataDir), 'accd-out.log'), 'a');
  accd = spawn(ACCD, ['--data-dir', dataDir], { env, stdio: ['ignore', accdLog, accdLog] });
  const exited = new Promise((r) => accd.once('exit', r));
  accd.exited = exited;
  const socketPath = path.join(dataDir, 'accd.sock');
  const t0 = Date.now();
  for (;;) {
    if (accd.exitCode !== null) throw new Error(`accd exited early (code ${accd.exitCode}): ${readLog()}`);
    try {
      const probe = await new Promise((resolve, reject) => {
        const s = net.createConnection(socketPath);
        s.once('connect', () => resolve(s));
        s.once('error', reject);
      });
      probe.destroy();
      return socketPath;
    } catch {
      if (Date.now() - t0 > SOCKET_WAIT) throw new Error(`accd socket did not come up: ${readLog()}`);
      await sleep(100);
    }
  }
}

function readLog() {
  try {
    return fs.readFileSync(path.join(tmpRoot, 'accd-out.log'), 'utf8').slice(-1000);
  } catch {
    return '';
  }
}

async function killAccd() {
  if (!accd || accd.exitCode !== null) return;
  accd.kill('SIGTERM'); // what `pkill accd` sends
  await Promise.race([accd.exited, sleep(5000)]);
  if (accd.exitCode === null) {
    accd.kill('SIGKILL');
    await accd.exited;
  }
}

async function cleanup() {
  if (cleaned) return;
  cleaned = true;
  try {
    if (accd && accd.exitCode === null) {
      // A normal shutdown: the daemon closes its clients and kills its sessions.
      accd.kill('SIGINT');
      await Promise.race([accd.exited, sleep(4000)]);
    }
  } catch {}
  for (const h of hookConns) h.close();
  main?.close();
  await killAccd();
  if (stragglerToken) {
    // Anything still running from this smoke (its args carry the temp dir name).
    spawnSync('pkill', ['-9', '-f', stragglerToken]);
  }
  if (accdLog !== null) {
    try {
      fs.closeSync(accdLog);
    } catch {}
  }
  if (tmpRoot && !KEEP) {
    // Hooks of a dying Claude (plugins writing logs) can recreate files, so remove twice.
    fs.rmSync(tmpRoot, { recursive: true, force: true });
    await sleep(1000);
    fs.rmSync(tmpRoot, { recursive: true, force: true });
  }
  else if (tmpRoot) log(`kept ${tmpRoot}`);
}

for (const sig of ['SIGINT', 'SIGTERM']) {
  process.on(sig, () => {
    cleanup().then(() => process.exit(130));
  });
}

// ---------------------------------------------------------------- results

const NAMES = {
  1: 'Worktree session starts in .worktrees/<slug> on agent/<slug>',
  2: 'Typed input reaches Claude',
  3: 'Permission request reaches Approvals, Allow once runs it',
  4: 'Always allow auto-allows the next identical request (actor policy)',
  5: 'Deny is reported by Claude',
  6: 'Reconnect keeps the session and its scrollback',
  7: 'Daemon restart: session Stopped, Restart resumes it',
  8: 'Pending approval survives the app; Esc in Claude cancels it',
  9: 'PermissionRequest fields: tool_name and tool_input',
  10: 'Project folder session makes no worktree and no branch',
  11: 'First edit on the base branch asks; Make branch keeps the changes',
};

function record(n, status, note) {
  results.set(n, { status, note });
  log(`  ${n}. ${status.padEnd(11)} ${NAMES[n]}${note ? `\n     ${note}` : ''}`);
}

/** Runs one checklist item. The body returns a note, or { status, note }. */
async function item(n, body, tracker) {
  try {
    const r = await body();
    if (r && typeof r === 'object') record(n, r.status, r.note);
    else record(n, 'PASS', r || '');
    return true;
  } catch (e) {
    let note = e.message;
    if (e instanceof Timeout && tracker?.()) note += `\n     last 2 KB of output:\n${tracker().tail().replace(/^/gm, '       ')}`;
    record(n, 'FAIL', note);
    return false;
  }
}

function skip(n, why) {
  record(n, 'SKIP (fake)', why);
}

function markdown() {
  const rows = [...results.entries()]
    .sort((a, b) => a[0] - b[0])
    .map(([n, r]) => `| ${n} | ${NAMES[n]} | ${r.status} | ${r.note.replace(/\n/g, ' ').replace(/\|/g, '\\|')} |`);
  return [
    `Run: ${new Date().toISOString()}, mode: ${FAKE ? 'fake' : 'real Claude Code'}`,
    '',
    '| # | Check | Result | Notes |',
    '|---|-------|--------|-------|',
    ...rows,
    '',
  ].join('\n');
}

// ---------------------------------------------------------------- the run

const bash = (command) => ({ hook_event_name: 'PermissionRequest', tool_name: 'Bash', tool_input: { command } });
const realpath = (p) => fs.realpathSync(p);

async function main_() {
  for (const bin of [ACCD, ACC_HOOK]) {
    if (!fs.existsSync(bin)) throw new Error(`${bin} is missing. Run: cargo build --workspace --offline`);
  }
  if (FAKE && !fs.existsSync(FIXTURE)) throw new Error(`${FIXTURE} is missing. Run: cargo build --workspace --offline`);

  tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'acc smoke '));
  stragglerToken = path.basename(tmpRoot);
  const dataDir = path.join(tmpRoot, 'data');
  if (path.join(dataDir, 'accd.sock').length > 100) throw new Error('temp path is too long for a Unix socket');
  const repo = makeRepo(tmpRoot);
  log(`mode: ${FAKE ? 'fake (the stand-in, simulated hooks)' : 'real Claude Code'}`);
  log(`temp: ${tmpRoot}`);

  let socketPath = await startAccd(dataDir);
  main = await Client.app(socketPath);

  const project = await main.call('project_add', { path: repo });
  // First-run prompts that block Claude Code, answered by polling the output:
  // the folder trust prompt (default is "No, exit", so move down, then Enter on "Yes") and
  // the "Try the new fullscreen renderer?" prompt (answer 2, "Not now").
  let trustMark = 0;
  let trustAnswers = 0;
  let lastTrustAt = 0;
  let session = null;
  const onChunk = () => {};
  const answerPrompts = () => {
    const t = main?.tracker;
    if (FAKE || !t || !session || trustAnswers >= 12 || Date.now() - lastTrustAt < 1500) return;
    const compact = clean(t.buf.subarray(Math.min(trustMark, t.buf.length))).replace(/\s+/g, '');
    let key = null;
    if (/tryThenewfullscreenrenderer|tryThenewfullscreenrenderer/i.test(compact) || /trythenewfullscreenrenderer/i.test(compact)) {
      key = '2';
    } else if (TRUST_RE.test(compact)) {
      const onYes = compact.lastIndexOf('❯Yes') > compact.lastIndexOf('❯No');
      const onNo = compact.lastIndexOf('❯No') > compact.lastIndexOf('❯Yes');
      key = onYes ? '\r' : onNo ? '\x1b[B' : null;
    }
    if (key === null) return;
    trustAnswers++;
    lastTrustAt = Date.now();
    trustMark = t.buf.length;
    main.call('session_input', { session_id: session.id, data_b64: b64(key) }).catch(() => {});
  };
  const promptTimer = setInterval(answerPrompts, 500);
  promptTimer.unref();
  const tracker = () => main?.tracker;
  const send = (text) => main.call('session_input', { session_id: session.id, data_b64: b64(text) });
  const typeLine = async (text) => {
    await send(text);
    await sleep(400);
    await send('\r');
  };
  const statusIs = (status) => (e) => e.kind === 'session_updated' && e.data.id === session.id && e.data.status === status;
  const approvalCreated = (e) => e.kind === 'approval_created' && e.data.session_id === session.id;
  // Waits for the turn to finish (Stop hook gives Idle). In fake mode a Stop hook is simulated.
  const waitIdle = async (soft = false) => {
    const from = main.mark();
    if (FAKE) hookQuiet({ hook_event_name: 'Stop' });
    try {
      await main.waitEvent('the session to go Idle', statusIs('idle'), FAKE && soft ? 1500 : CLAUDE_WAIT, from);
      return true;
    } catch (e) {
      if (!soft) throw e;
      return false;
    }
  };
  const hookConn = async () => {
    const h = await Client.connect(socketPath);
    hookConns.push(h);
    return h;
  };
  const hookQuiet = (payload) => hookConn().then((h) => h.hookCall(session.id, payload).then(() => h.close()));
  const fileExists = async (name) => {
    for (let i = 0; i < STEP_WAIT / 500; i++) {
      if (fs.existsSync(path.join(session.worktree, name))) return true;
      await sleep(500);
    }
    return false;
  };
  const listEvents = (limit = 500) => main.call('events_list', { session_id: session.id, limit });

  log('\nchecklist');
  const TASK = 'Reply with the single word READY. Do not use any tools.';
  let approval3 = null;

  // 1. worktree and branch
  const ok1 = await item(1, async () => {
    session = await main.call('session_start', { project_id: project.id, agent: 'claude', task: TASK, mode: 'worktree', send_task: true });
    new Tracker(main, session.id, onChunk);
    const slug = session.branch.replace(/^agent\//, '');
    if (!session.branch.startsWith('agent/') || !slug) throw new Error(`branch is ${session.branch}`);
    const expected = path.join(realpath(repo), '.worktrees', slug);
    if (realpath(session.worktree) !== expected) throw new Error(`worktree is ${session.worktree}, expected ${expected}`);
    const head = git(session.worktree, 'rev-parse', '--abbrev-ref', 'HEAD');
    if (head !== session.branch) throw new Error(`worktree HEAD is ${head}, expected ${session.branch}`);
    const listed = git(repo, 'worktree', 'list', '--porcelain');
    if (!listed.includes(`branch refs/heads/${session.branch}`)) throw new Error('git worktree list does not show the branch');
    return `${path.relative(realpath(repo), realpath(session.worktree))} on ${session.branch}`;
  });
  if (!ok1) {
    for (const n of [2, 3, 4, 5, 6, 7, 8, 9]) record(n, 'NOT RUN', 'item 1 failed');
  }

  // 10 and 11. A session in the project folder, and its branch question. The tool call is
  // simulated over a hook connection in both modes: it checks the daemon and git, not Claude.
  let folderSession = null;
  const ok10 = await item(10, async () => {
    const branchesBefore = git(repo, 'branch', '--list', 'agent/*');
    folderSession = await main.call('session_start', { project_id: project.id, agent: 'claude', task: TASK, send_task: true });
    if (folderSession.mode !== 'project') throw new Error(`mode is ${folderSession.mode}`);
    if (realpath(folderSession.worktree) !== realpath(repo)) throw new Error(`folder is ${folderSession.worktree}`);
    const head = git(repo, 'rev-parse', '--abbrev-ref', 'HEAD');
    if (folderSession.branch !== head) throw new Error(`branch is ${folderSession.branch}, the folder is on ${head}`);
    const branchesAfter = git(repo, 'branch', '--list', 'agent/*');
    if (branchesAfter !== branchesBefore) throw new Error(`branches changed: ${branchesAfter}`);
    return `in ${path.basename(realpath(repo))} on ${folderSession.branch}`;
  });
  if (ok10) {
    await item(11, async () => {
      const file = path.join(realpath(repo), 'README.md');
      fs.appendFileSync(file, 'changed by the smoke test\n');
      const from = main.mark();
      const hk = await hookConn();
      const reply = hk.hookCall(folderSession.id, {
        hook_event_name: 'PreToolUse',
        tool_name: 'Edit',
        tool_input: { file_path: file, old_string: 'a', new_string: 'b' },
      });
      const isBranch = (e) => e.kind === 'approval_created' && e.data.session_id === folderSession.id && e.data.kind === 'branch';
      const ev = await main.waitEvent('the branch question', isBranch, STEP_WAIT, from);
      await main.call('approval_decide', { approval_id: ev.data.id, decision: 'make_branch' });
      const answer = await reply;
      if (!answer || answer.stdout !== PRE_TOOL_ALLOW) throw new Error(`hook reply was ${JSON.stringify(answer)}`);
      const head = git(repo, 'rev-parse', '--abbrev-ref', 'HEAD');
      if (head !== ev.data.branch) throw new Error(`the folder is on ${head}, expected ${ev.data.branch}`);
      if (!fs.readFileSync(file, 'utf8').includes('changed by the smoke test')) throw new Error('the uncommitted change is gone');
      return `made ${head}, the change is kept`;
    });
    await main.call('session_stop', { session_id: folderSession.id }).catch(() => {});
  } else {
    record(11, 'NOT RUN', 'item 10 failed');
  }
  if (!ok1) return;

  // Claude reads the task, answers, and goes Idle. First run may show the trust prompt.
  let ready = true;
  try {
    await waitIdle();
  } catch (e) {
    ready = false;
    record(2, 'FAIL', `${e.message}\n     last 2 KB of output:\n${tracker().tail().replace(/^/gm, '       ')}`);
  }

  // Claude Code may start in a permission mode that never asks (auto, accept edits, plan, bypass),
  // for example from the user's own settings. Press Shift+Tab until the footer shows none of them.
  let modeNote = '';
  if (ready && !FAKE) {
    const MODES = /automodeon|accepteditson|planmodeon|bypasspermissionson/;
    const footer = () => clean(tracker().buf.subarray(Math.max(0, tracker().buf.length - 3000))).replace(/\s+/g, '').toLowerCase();
    const first = footer().match(MODES);
    if (first) {
      const seen = [first[0]];
      for (let i = 0; i < 6; i++) {
        const mark = tracker().buf.length;
        await send('\x1b[Z');
        await sleep(1200);
        const after = clean(tracker().buf.subarray(mark)).replace(/\s+/g, '').toLowerCase();
        const m = after.match(MODES);
        if (!m) break;
        seen.push(m[0]);
      }
      modeNote = `Claude started in a mode that does not ask (${seen[0]}); pressed Shift+Tab ${seen.length} time(s) to reach the default mode. `;
      log(`  note: ${modeNote}`);
    }
  }

  // 2. typing reaches Claude
  const line = 'Run the command: touch approval-test.txt';
  if (ready) {
    await item(2, async () => {
      const mark = tracker().buf.length;
      await send(line);
      const t0 = Date.now();
      while (!tracker().since(mark).includes('approval-test.txt')) {
        if (Date.now() - t0 > STEP_WAIT) throw new Timeout('typed text did not show in the output');
        await sleep(100);
      }
      return FAKE ? 'plumbing only: the fake has no UI, so this is the tty echo' : `text shown after ${Date.now() - t0} ms`;
    }, tracker);
  }

  // 3 and 9. permission, Allow once
  let ok3 = false;
  if (ready) {
    ok3 = await item(3, async () => {
      const from = main.mark();
      let hk = null;
      let t0;
      if (FAKE) {
        hk = await hookConn();
        hk.reply = hk.hookCall(session.id, bash('touch approval-test.txt'));
        await send('\r');
        t0 = Date.now();
      } else {
        await send('\r');
        t0 = Date.now();
      }
      const ev = await main.waitEvent('approval_created', approvalCreated, FAKE ? STEP_WAIT : CLAUDE_WAIT, from);
      const latency = Date.now() - t0;
      approval3 = ev.data;
      await main.call('approval_decide', { approval_id: approval3.id, decision: 'allow_once' });
      let note;
      if (FAKE) {
        const reply = await Promise.race([hk.reply, sleep(STEP_WAIT).then(() => 'timeout')]);
        if (!reply || reply === 'timeout' || reply.stdout !== ALLOW_REPLY) throw new Error(`hook reply was ${JSON.stringify(reply)}`);
        if (latency > 2000) throw new Error(`approval_created took ${latency} ms (limit 2000)`);
        note = `hook request to approval_created ${latency} ms; hook got the allow reply; file creation not checked in fake mode`;
      } else {
        if (!(await fileExists('approval-test.txt'))) throw new Error('approval-test.txt was not created after Allow once');
        note = `Enter to approval_created ${(latency / 1000).toFixed(1)} s (includes model time, not the hook-to-inbox latency); file created`;
      }
      await main.waitEvent('approval_resolved', (e) => e.kind === 'approval_resolved' && e.data.id === approval3.id && e.data.state === 'allowed', STEP_WAIT, from);
      return note;
    }, tracker);
  } else {
    record(3, 'NOT RUN', 'session never went Idle');
  }

  if (approval3) {
    await item(9, async () => {
      const cmd = approval3.input && approval3.input.command;
      if (approval3.tool === 'unknown' || approval3.tool !== 'Bash') throw new Error(`tool is ${JSON.stringify(approval3.tool)}, expected Bash`);
      if (typeof cmd !== 'string' || !cmd.includes('approval-test.txt')) throw new Error(`tool_input is ${JSON.stringify(approval3.input)}`);
      if (!approval3.summary.includes('approval-test.txt')) throw new Error(`summary is ${JSON.stringify(approval3.summary)}`);
      const requested = (await listEvents()).find((e) => e.kind === 'approval.requested');
      return `tool=${approval3.tool}, summary=${JSON.stringify(approval3.summary)}${FAKE ? ' (from the simulated payload, which only checks the mapping)' : ''}${requested ? '' : '; no approval.requested event'}`;
    });
  } else {
    record(9, 'NOT RUN', 'item 3 produced no approval');
  }
  if (!ok3) {
    for (const n of [4, 5, 6, 7, 8]) record(n, 'NOT RUN', 'item 3 failed');
    return;
  }
  await waitIdle(true);

  // 4. Always allow, then the same command again
  await item(4, async () => {
    const cmd = 'touch approval-always.txt';
    const ask = async (decision) => {
      const from = main.mark();
      let hk = null;
      if (FAKE) {
        hk = await hookConn();
        hk.reply = hk.hookCall(session.id, bash(cmd));
      } else {
        await typeLine(`Run the command: ${cmd}`);
      }
      return { from, hk };
    };
    // first time: asks, user answers Always allow
    const first = await ask();
    const ev = await main.waitEvent('approval_created for the first request', approvalCreated, FAKE ? STEP_WAIT : CLAUDE_WAIT, first.from);
    const preview = ev.data.rule_preview;
    if (typeof preview !== 'string' || ev.data.protected !== false) {
      throw new Error(`the approval has rule_preview ${JSON.stringify(preview)} and protected ${JSON.stringify(ev.data.protected)}`);
    }
    await main.call('approval_decide', { approval_id: ev.data.id, decision: 'allow_always' });
    if (FAKE) {
      const reply = await first.hk.reply;
      if (!reply || reply.stdout !== ALLOW_REPLY) throw new Error(`first hook reply was ${JSON.stringify(reply)}`);
    } else if (!(await fileExists('approval-always.txt'))) {
      throw new Error('approval-always.txt was not created after Always allow');
    }
    await waitIdle(true);
    // second time: the rule allows it, no approval_created
    const second = await ask();
    if (FAKE) {
      const reply = await Promise.race([second.hk.reply, sleep(STEP_WAIT).then(() => 'timeout')]);
      if (!reply || reply === 'timeout' || reply.stdout !== ALLOW_REPLY) throw new Error(`second hook reply was ${JSON.stringify(reply)}`);
    }
    const isPolicy = (e) => e.kind === 'event_appended' && e.data.kind === 'approval.decided' && e.data.actor === 'policy' && e.data.session_id === session.id;
    const next = await main.waitEvent('a policy decision or a new approval', (e) => approvalCreated(e) || isPolicy(e), FAKE ? STEP_WAIT : CLAUDE_WAIT, second.from);
    if (next.kind === 'approval_created') {
      await main.call('approval_decide', { approval_id: next.data.id, decision: 'deny' }).catch(() => {});
      throw new Error(`the same command asked again: ${JSON.stringify(next.data.summary)}`);
    }
    const policy = (await listEvents()).filter((e) => e.kind === 'approval.decided' && e.actor === 'policy');
    if (!policy.length) throw new Error('events_list has no approval.decided event with actor policy');
    await waitIdle(true);
    return `rule ${JSON.stringify(policy[0].payload.rule)} (preview "${preview}") auto-allowed the repeat; events_list shows actor policy`;
  }, tracker);

  // 5. Deny
  await item(5, async () => {
    const cmd = 'touch approval-no.txt';
    const from = main.mark();
    let hk = null;
    if (FAKE) {
      hk = await hookConn();
      hk.reply = hk.hookCall(session.id, bash(cmd));
    } else {
      await typeLine(`Run the command: ${cmd}`);
    }
    const ev = await main.waitEvent('approval_created', approvalCreated, FAKE ? STEP_WAIT : CLAUDE_WAIT, from);
    const mark = tracker().buf.length;
    await main.call('approval_decide', { approval_id: ev.data.id, decision: 'deny' });
    if (FAKE) {
      const reply = await Promise.race([hk.reply, sleep(STEP_WAIT).then(() => 'timeout')]);
      if (!reply || reply === 'timeout' || reply.stdout !== DENY_REPLY) throw new Error(`hook reply was ${JSON.stringify(reply)}`);
      return 'plumbing only: the hook got the deny reply with the denial message; Claude reporting it is not checked in fake mode';
    }
    const t0 = Date.now();
    while (!DENY_RE.test(tracker().since(mark))) {
      if (Date.now() - t0 > CLAUDE_WAIT) throw new Timeout('no denial text in the output');
      await sleep(250);
    }
    if (fs.existsSync(path.join(session.worktree, 'approval-no.txt'))) throw new Error('the denied command ran anyway');
    return 'Claude reported the denial; the file was not created';
  }, tracker);
  await waitIdle(true);

  // 6. reconnect keeps the scrollback
  await item(6, async () => {
    const seen = tracker().end;
    main.close();
    await sleep(500);
    main = await Client.app(socketPath);
    const snap = await main.call('snapshot');
    const s = snap.sessions.find((x) => x.id === session.id);
    if (!s) throw new Error('session missing from the snapshot after reconnect');
    new Tracker(main, session.id, onChunk);
    const att = await main.call('session_attach', { session_id: session.id });
    const scrollback = Buffer.from(att.scrollback_b64, 'base64');
    tracker().seed(scrollback, att.end);
    if (att.end < seen) throw new Error(`end ${att.end} is below the ${seen} bytes seen before`);
    if (scrollback.length === 0) throw new Error('scrollback is empty');
    const capped = att.end > 1024 * 1024;
    if (!capped && !clean(scrollback).includes('approval-test.txt')) throw new Error('scrollback lost the earlier output');
    return `session ${s.status}, ${scrollback.length} bytes of scrollback, end ${att.end} (seen before: ${seen})${capped ? ', scrollback was capped so contents were not compared' : ''}`;
  }, tracker);

  // 8. pending approval outlives the app; Esc cancels it
  await item(8, async () => {
    const cmd = 'touch approval-esc.txt';
    const from = main.mark();
    let hk = null;
    if (FAKE) {
      hk = await hookConn();
      hk.reply = hk.hookCall(session.id, bash(cmd));
    } else {
      await typeLine(`Run the command: ${cmd}`);
    }
    const ev = await main.waitEvent('approval_created', approvalCreated, FAKE ? STEP_WAIT : CLAUDE_WAIT, from);
    const id = ev.data.id;
    main.close();
    await sleep(1500);
    main = await Client.app(socketPath);
    new Tracker(main, session.id, onChunk);
    const att = await main.call('session_attach', { session_id: session.id });
    tracker().seed(Buffer.from(att.scrollback_b64, 'base64'), att.end);
    const snap = await main.call('snapshot');
    const pending = snap.approvals.find((a) => a.id === id);
    if (!pending || pending.state !== 'pending') throw new Error('the approval did not stay pending after the app disconnected');
    const mark = main.mark();
    await main.call('session_input', { session_id: session.id, data_b64: b64('\x1b') });
    if (FAKE) {
      // Esc in real Claude kills the hook process; the fake closes its hook connection.
      await sleep(300);
      hk.close();
    }
    const resolved = await main.waitEvent('the approval to become cancelled', (e) => e.kind === 'approval_resolved' && e.data.id === id, STEP_WAIT, mark);
    if (resolved.data.state !== 'cancelled') throw new Error(`approval ended as ${resolved.data.state}, expected cancelled`);
    return FAKE ? 'plumbing only: still pending after disconnect; cancelled when the hook connection closed' : 'still pending after disconnect; cancelled after Esc';
  }, tracker);

  // 7. daemon restart
  await item(7, async () => {
    const id = session.id;
    main.close();
    await killAccd();
    socketPath = await startAccd(dataDir);
    main = await Client.app(socketPath);
    const snap = await main.call('snapshot');
    const s = snap.sessions.find((x) => x.id === id);
    if (!s) throw new Error('session missing after the daemon restarted');
    if (s.status !== 'stopped') throw new Error(`session is ${s.status} after the restart, expected stopped`);
    if (snap.approvals.some((a) => a.state === 'pending')) throw new Error('a pending approval survived the restart');
    session = s;
    const t = new Tracker(main, id, onChunk);
    trustMark = 0;
    const restarted = await main.call('session_restart', { session_id: id });
    if (restarted.id !== id) throw new Error('restart returned a different session id');
    // Real Claude replays the earlier conversation; the fake prints its arguments. The session id
    // alone proves nothing: "No conversation found with session ID: <id>" holds it too.
    const t0 = Date.now();
    const limit = FAKE ? STEP_WAIT : CLAUDE_WAIT;
    const expected = FAKE ? `--resume ${id}` : 'approval-test.txt';
    for (;;) {
      const compact = t.since(0).replace(/\s+/g, '');
      if (/noconversationfound/i.test(compact)) throw new Error(`Claude found no conversation to resume (its transcript was not saved): ${t.tail(300).trim()}`);
      if (compact.includes(expected.replace(/\s+/g, ''))) break;
      if (Date.now() - t0 > limit) throw new Timeout(`output after the restart does not show ${FAKE ? 'the --resume argument' : 'the earlier conversation'}`);
      await sleep(250);
    }
    const started = (await listEvents()).filter((e) => e.kind === 'session.started');
    const resumed = started.some((e) => e.payload && e.payload.resume === true);
    if (!resumed) throw new Error('no session.started event with resume true');
    return `Stopped after the restart, no pending approval, Restart resumed it (${FAKE ? 'the fake got --resume with the session id' : 'output shows the earlier conversation'})`;
  }, () => main?.tracker);

  try {
    await main.call('session_stop', { session_id: session.id });
  } catch {}
}

(async () => {
  let failed = false;
  try {
    await main_();
  } catch (e) {
    failed = true;
    console.error(`\nsmoke aborted: ${e.message}`);
  }
  await cleanup();
  log('\nresults');
  const rows = [...results.entries()].sort((a, b) => a[0] - b[0]);
  for (const [n, r] of rows) log(`  ${n}. ${r.status.padEnd(11)} ${NAMES[n]}`);
  if (OUT_FILE) fs.writeFileSync(OUT_FILE, markdown());
  const bad = rows.some(([, r]) => r.status === 'FAIL' || r.status === 'NOT RUN');
  const missing = rows.length < 9;
  log(`\n${failed || bad || missing ? 'SMOKE FAILED' : FAKE ? 'SMOKE OK (fake: plumbing only)' : 'SMOKE OK'}`);
  process.exit(failed || bad || missing ? 1 : 0);
})();
