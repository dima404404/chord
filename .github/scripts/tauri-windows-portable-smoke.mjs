import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

const target = process.env.TAURI_TARGET;
const directory = resolve('artifacts', `tauri-${target}`);
const executable = join(directory, 'chord-client.exe');
const provenancePath = join(directory, 'provenance.json');
const provenance = JSON.parse(readFileSync(provenancePath, 'utf8'));
const digest = (path) => createHash('sha256').update(readFileSync(path)).digest('hex');
const fixturePath = join(directory, 'fixtures');

assert.equal(process.platform, 'win32');
assert.equal(provenance.target, target);
assert.equal(provenance.source_commit, process.env.GITHUB_SHA);
assert.equal(existsSync(fixturePath), false);
const arm = target === 'aarch64-pc-windows-msvc';
assert.equal(process.arch, arm ? 'arm64' : 'x64');
for (const [path, hash] of Object.entries(provenance.files)) {
  assert.equal(digest(join(directory, path)), hash, path);
  if (/\.(dll|exe)$/i.test(path)) {
    const pe = readFileSync(join(directory, path));
    assert.equal(pe.toString('ascii', 0, 2), 'MZ', path);
    const header = pe.readUInt32LE(0x3c);
    assert.equal(pe.toString('ascii', header, header + 4), 'PE\0\0', path);
    assert.equal(pe.readUInt16LE(header + 4), arm ? 0xaa64 : 0x8664, path);
  }
}

const port = 48994;
const profile = mkdtempSync(join(process.env.RUNNER_TEMP, 'chord-clean-webview-'));
const child = spawn(executable, [], {
  cwd: directory,
  windowsHide: true,
  env: {
    ...process.env,
    DL_FIXTURES: fixturePath,
    SPIKE_AUTH_DISABLED: '0',
    SPIKE_AUTOBENCH: '0',
    SPIKE_EXIT_AFTER_MS: '35000',
    WEBVIEW2_USER_DATA_FOLDER: profile,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port} --remote-debugging-address=127.0.0.1`,
  },
});
let stdout = '';
let stderr = '';
child.stdout.on('data', (data) => { stdout += data; });
child.stderr.on('data', (data) => { stderr += data; });
const closed = new Promise((done) => {
  child.once('error', (error) => done({ error }));
  child.once('close', (code) => done({ code }));
});

const inspectAuth = `(() => {
  const gate = document.querySelector('#auth-gate');
  const form = document.querySelector('#auth-form');
  const notice = document.querySelector('#auth-qr-unavailable');
  return {
    state: gate?.getAttribute('data-state'),
    gateVisible: !!gate && getComputedStyle(gate).display !== 'none',
    formVisible: !!form && form.getClientRects().length > 0,
    activeFixture: window.__SPIKE_AUTH_DISABLED__ === true,
    qrCode: !!document.querySelector('#auth-qr-code'),
    qrNotice: notice?.textContent.trim(),
    screens: document.querySelectorAll('.screen').length
  };
})()`;

function evaluate(socketUrl) {
  const url = new URL(socketUrl);
  assert.equal(url.protocol, 'ws:');
  assert.match(url.hostname, /^(127\.0\.0\.1|localhost)$/);
  return new Promise((done, reject) => {
    const socket = new WebSocket(url);
    const timeout = setTimeout(() => {
      socket.close();
      reject(new Error('WebView2 did not answer the DOM probe'));
    }, 2500);
    socket.addEventListener('open', () => {
      socket.send(JSON.stringify({
        id: 1,
        method: 'Runtime.evaluate',
        params: { expression: inspectAuth, returnByValue: true },
      }));
    });
    socket.addEventListener('message', (event) => {
      const response = JSON.parse(event.data);
      if (response.id !== 1) return;
      clearTimeout(timeout);
      socket.close();
      if (response.error || response.result?.exceptionDetails) reject(new Error(JSON.stringify(response)));
      else done(response.result?.result?.value);
    });
    socket.addEventListener('error', () => {
      clearTimeout(timeout);
      reject(new Error('Could not connect to the local WebView2 inspector'));
    });
  });
}

try {
  let snapshot;
  let lastError;
  const deadline = Date.now() + 28_000;
  while (Date.now() < deadline && child.exitCode === null) {
    try {
      const response = await fetch(`http://127.0.0.1:${port}/json/list`, {
        signal: AbortSignal.timeout(1500),
      });
      if (response.ok) {
        const pages = await response.json();
        const page = pages.find((item) => item.type === 'page' && item.webSocketDebuggerUrl);
        if (page) snapshot = await evaluate(page.webSocketDebuggerUrl);
        if (snapshot?.state === 'no-session') break;
      }
    } catch (error) {
      lastError = error;
    }
    await delay(350);
  }
  assert.equal(snapshot?.state, 'no-session', JSON.stringify({ snapshot, lastError: lastError?.message }));
  assert.equal(snapshot.activeFixture, false);
  assert.equal(snapshot.gateVisible, true);
  assert.equal(snapshot.formVisible, true);
  assert.equal(snapshot.qrCode, false);
  assert.equal(snapshot.qrNotice, 'Вход по QR-коду пока недоступен в Chord.');
  assert.equal(snapshot.screens, 4);

  const outcome = await Promise.race([closed, delay(40_000, { timeout: true }, { ref: false })]);
  assert.equal(outcome.code, 0, JSON.stringify(outcome));
  assert.match(stdout, /window-ready/);
  assert.doesNotMatch(stderr, /panicked at|segmentation fault/i);
  const receipt = {
    source_commit: provenance.source_commit,
    target,
    executable_sha256: digest(executable),
    fixtures_present: false,
    snapshot,
  };
  writeFileSync(join(directory, 'windows-startup-smoke.json'), JSON.stringify(receipt, null, 2) + '\n');
  writeFileSync(provenancePath, JSON.stringify({ ...provenance, native_runtime_qualified: true }, null, 2) + '\n');
  console.log(JSON.stringify(receipt));
} catch (error) {
  console.error(stdout.slice(-4000), stderr.slice(-4000));
  throw error;
} finally {
  if (child.exitCode === null) child.kill();
}
