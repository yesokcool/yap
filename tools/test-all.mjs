#!/usr/bin/env node
/*
 * Every test, against a test server of its own (CLAUDE.md › Commands):
 *
 *   npm run test:all                       typecheck, the Node suites, then a fresh test
 *                                          server for protocol + e2e + backup → restore
 *   npm run test:all -- --fast             typecheck + the Node suites only (no server)
 *   npm run test:all -- --only unit,protocol   just these steps (names below)
 *   npm run test:all -- --ports 8807-8809  the ports it may use (default: any free ones)
 *   npm run test:all -- --keep             keep the temp folder (databases, backups,
 *                                          screenshots, server logs) even when all passed
 *
 * The server is `npm run dev:test` with everything the full suites need: the
 * fake ntfy and Web Push services (run by the protocol test, on free ports),
 * TEST_HANGS, TEST_PINS (old-PIN accounts for the tests), TEST_OBS_GRACE (an
 * OBS link's 5 minutes after its account left, in seconds), a fresh VAPID key
 * and a brand-new database. For the backup →
 * restore round trip it's backed up, stopped, and started again on an empty
 * database to restore into. Temp files go to $TMPDIR (TMPDIR=… moves them).
 */
import { execFileSync, spawn } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { ROOT } from './lib.mjs';

const SCRIPTS = JSON.parse(fs.readFileSync(path.join(ROOT, 'package.json'), 'utf8')).scripts;
const KEY = 'testkey'; // the dev:test script's YAP_KEY and ADMIN_KEY
const ADMIN = 'testadmin';

// Steps, in order. `server` steps share one test server.
const STEPS = [
  { name: 'check', script: 'check' },
  { name: 'unit', script: 'test:unit' },
  { name: 'worker', script: 'test:worker' },
  { name: 'extras', script: 'test:extras' },
  { name: 'webpush', script: 'test:webpush' },
  { name: 'gif', script: 'test:gif' },
  { name: 'vault', script: 'test:vault' },
  { name: 'migrations', script: 'test:migrations' },
  { name: 'tools', script: 'test:tools' },
  { name: 'protocol', script: 'test', server: true },
  { name: 'e2e', script: 'test:e2e', server: true, args: (t) => [t.shots] },
  { name: 'roundtrip', server: true }, // backup → restore into an empty server (test/roundtrip.test.mjs)
];

const args = process.argv.slice(2);
const option = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};
const only = (option('--only') ?? '').split(',').map((s) => s.trim()).filter(Boolean);
const unknown = only.filter((n) => !STEPS.some((s) => s.name === n));
if (unknown.length) {
  console.error(`test-all: no step called ${unknown.join(', ')} (steps: ${STEPS.map((s) => s.name).join(', ')})`);
  process.exit(2);
}
const steps = STEPS.filter((s) => (only.length ? only.includes(s.name) : !(args.includes('--fast') && s.server)));
const keep = args.includes('--keep');
const portRange = option('--ports') ?? process.env.YAP_TEST_PORTS ?? '';

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'yap-test-'));
const t = {
  tmp,
  shots: path.join(tmp, 'shots'),
  backup: path.join(tmp, 'backup'),
  log: path.join(tmp, 'server.log'),
};
let server = null; // { child, url, port }

// ── Ports ─────────────────────────────────────────────────────────────────

/** Can we listen on this port (any address, like the servers will)? */
const portFree = (port) => new Promise((resolve) => {
  const s = net.createServer();
  s.once('error', () => resolve(false));
  s.listen(port, () => s.close(() => resolve(true)));
});

/** n free ports: from --ports a-b if given, else whatever the OS hands out. */
async function freePorts(n) {
  const out = [];
  if (portRange) {
    const [from, to] = portRange.split('-').map(Number);
    if (!(from > 0 && to >= from)) throw new Error(`--ports wants a range like 8807-8809, not "${portRange}"`);
    for (let p = from; p <= to && out.length < n; p++) if (await portFree(p)) out.push(p);
    if (out.length < n) throw new Error(`need ${n} free ports in ${portRange}, found ${out.length} (${out.join(', ') || 'none'})`);
    return out;
  }
  const held = [];
  for (let i = 0; i < n; i++) {
    const s = net.createServer();
    await new Promise((resolve) => s.listen(0, resolve));
    held.push(s);
    out.push(s.address().port);
  }
  await Promise.all(held.map((s) => new Promise((resolve) => s.close(resolve))));
  return out;
}

// ── Running things ────────────────────────────────────────────────────────

const quote = (s) => `'${String(s).replace(/'/g, `'\\''`)}'`;
const PATH = `${path.join(ROOT, 'node_modules', '.bin')}${path.delimiter}${process.env.PATH}`;

/** Run a package.json script (like `npm run`, without npm), streaming its output. */
function run(command, env = {}) {
  return new Promise((resolve) => {
    const child = spawn(command, { cwd: ROOT, shell: true, env: { ...process.env, PATH, ...env }, stdio: ['ignore', 'pipe', 'pipe'] });
    let output = '';
    const tee = (stream) => (chunk) => {
      output += chunk;
      stream.write(chunk);
    };
    child.stdout.on('data', tee(process.stdout));
    child.stderr.on('data', tee(process.stderr));
    child.on('close', (code) => resolve({ ok: code === 0, output }));
  });
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Start the test server on `port` with a fresh database in `db`. */
async function startServer(port, db, vars) {
  fs.mkdirSync(db, { recursive: true });
  const flags = ['--port', port, '--persist-to', db, ...Object.entries(vars).flatMap(([k, v]) => ['--var', `${k}:${v}`])];
  const log = fs.openSync(t.log, 'a');
  fs.writeSync(log, `\n── test server on :${port}, database ${db} ──\n`);
  // Its own process group, so stopping it stops wrangler and workerd too.
  const child = spawn(`${SCRIPTS['dev:test']} ${flags.map(quote).join(' ')}`, {
    cwd: ROOT, shell: true, detached: true, env: { ...process.env, PATH }, stdio: ['ignore', log, log],
  });
  let exited = false;
  child.on('exit', () => (exited = true));
  const url = `http://localhost:${port}`;
  server = { child, url, port };
  const until = Date.now() + 90_000;
  while (Date.now() < until) {
    if (exited) throw new Error(`the test server quit while starting (log: ${t.log})\n${tail(t.log)}`);
    try {
      if ((await fetch(`${url}/api/check`)).ok) return url;
    } catch {}
    await sleep(300);
  }
  throw new Error(`the test server didn't come up on :${port} in 90 s (log: ${t.log})\n${tail(t.log)}`);
}

async function stopServer() {
  if (!server) return;
  const { child, port } = server;
  server = null;
  const kill = (signal) => {
    try {
      process.kill(-child.pid, signal);
    } catch {}
  };
  kill('SIGTERM');
  for (let i = 0; i < 50 && !(await portFree(port)); i++) await sleep(100);
  if (!(await portFree(port))) {
    kill('SIGKILL');
    for (let i = 0; i < 30 && !(await portFree(port)); i++) await sleep(100);
  }
}

const tail = (file, lines = 40) => {
  try {
    return fs.readFileSync(file, 'utf8').trimEnd().split('\n').slice(-lines).join('\n');
  } catch {
    return '';
  }
};

// Never leave a server running behind us.
for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, async () => {
    await stopServer();
    process.exit(130);
  });
}

/** "12/12 passed" (and "3 skipped") from a suite's output. */
function counts(output) {
  const m = [...output.matchAll(/(\d+)\/(\d+) passed/g)].at(-1);
  const skipped = output.match(/(\d+) skipped/)?.[1];
  return m ? `${m[1]}/${m[2]}${skipped ? `, ${skipped} skipped` : ''}` : '';
}

// ── Main ──────────────────────────────────────────────────────────────────

const summary = [];
async function step(name, fn) {
  console.log(`\n━━ ${name} ━━`);
  const started = Date.now();
  let result;
  try {
    result = await fn();
  } catch (err) {
    console.error(`${name}: ${err.message}`);
    result = { ok: false, output: '' };
  }
  summary.push({ name, ok: result.ok, counts: counts(result.output), secs: ((Date.now() - started) / 1000).toFixed(1) });
  return result.ok;
}

async function main() {
  for (const s of steps.filter((x) => !x.server)) await step(s.name, () => run(SCRIPTS[s.script]));

  const serverSteps = steps.filter((x) => x.server);
  if (serverSteps.length) {
    const [port, ntfyPort, pushPort] = await freePorts(3);
    // A fresh Web Push key for this run (never printed).
    const vapid = execFileSync(process.execPath, [path.join(ROOT, 'tools/make-vapid.mjs')], { encoding: 'utf8' }).trim();
    // TEST_OBS_GRACE: an OBS link keeps listening 4 s (not 5 minutes) after its account left the call.
    const vars = { NTFY_SERVER: `http://127.0.0.1:${ntfyPort}`, TEST_HANGS: '1', TEST_PINS: '1', TEST_OBS_GRACE: '4', VAPID_PRIVATE_JWK: vapid };
    const env = (url) => ({
      YAP_URL: url, YAP_KEY: KEY, YAP_ADMIN_KEY: ADMIN,
      YAP_TEST_NTFY_PORT: String(ntfyPort), YAP_TEST_PUSH_PORT: String(pushPort),
      // Everything the suites need is set up here, so a skipped test is a failure.
      YAP_TEST_STRICT: '1',
    });
    console.log(`\n━━ test server ━━\nports: server ${port}, fake ntfy ${ntfyPort}, fake push ${pushPort}; temp: ${tmp}`);
    let url;
    try {
      url = await startServer(port, path.join(tmp, 'db'), vars);
      console.log(`up at ${url}`);
    } catch (err) {
      console.error(err.message);
      summary.push({ name: 'test server', ok: false, counts: '', secs: '' });
    }
    if (url) {
      for (const s of serverSteps.filter((x) => x.script)) {
        await step(s.name, () => run([SCRIPTS[s.script], ...(s.args?.(t) ?? []).map(quote)].join(' '), env(url)));
      }
      if (serverSteps.some((x) => x.name === 'roundtrip')) {
        // Back up the server the suites just filled, then restore it into an empty one.
        await step('roundtrip', async () => {
          const seeded = await run(`node test/roundtrip.test.mjs seed ${quote(t.backup)}`, env(url));
          await stopServer();
          if (!seeded.ok) return seeded;
          const fresh = await startServer(port, path.join(tmp, 'db-restore'), vars);
          const verified = await run(`node test/roundtrip.test.mjs verify ${quote(t.backup)}`, env(fresh));
          return { ok: verified.ok, output: verified.output };
        });
      }
    }
    await stopServer();
  }

  const failed = summary.filter((s) => !s.ok);
  console.log('\n━━ summary ━━');
  for (const s of summary) console.log(`${s.ok ? '✓' : '✗'} ${s.name.padEnd(10)} ${s.counts.padEnd(18)} ${s.secs ? `${s.secs} s` : ''}`);
  if (failed.length || keep) console.log(`\ntemp folder (databases, backups, screenshots, server log): ${tmp}`);
  else fs.rmSync(tmp, { recursive: true, force: true });
  if (failed.length && fs.existsSync(t.log)) console.log(`last lines of the server log:\n${tail(t.log, 15)}`);
  console.log(failed.length ? `\n${failed.length} step(s) failed: ${failed.map((s) => s.name).join(', ')}` : '\nall passed');
  process.exit(failed.length ? 1 : 0);
}

main().catch(async (err) => {
  console.error(err);
  await stopServer();
  process.exit(1);
});
