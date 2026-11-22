import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { promisify } from 'node:util';

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

const profile = mkdtempSync(join(process.env.RUNNER_TEMP, 'chord-clean-webview-'));
const started = Date.now();
const { stdout, stderr } = await promisify(execFile)(executable, [], {
  cwd: directory,
  windowsHide: true,
  timeout: 65_000,
  maxBuffer: 4 * 1024 * 1024,
  env: {
    ...process.env,
    DL_FIXTURES: fixturePath,
    SPIKE_AUTH_DISABLED: '0',
    SPIKE_AUTOBENCH: '0',
    SPIKE_EXIT_AFTER_MS: '23000',
    WEBVIEW2_USER_DATA_FOLDER: profile,
  },
});
assert.ok(Date.now() - started >= 22_000, 'The app exited before the smoke interval elapsed');
assert.match(stdout, /window-ready/);
assert.doesNotMatch(stderr, /panicked at|segmentation fault/i);
const line = stdout.split(/\r?\n/).find((entry) => entry.startsWith('UI_SNAPSHOT '));
assert.ok(line, 'The packaged WebView2 did not send a DOM snapshot');
const dom = JSON.parse(line.slice('UI_SNAPSHOT '.length));
const snapshot = { ...dom.authentication, screens: dom.screenCount };
assert.deepEqual(snapshot, {
  state: 'no-session',
  gateVisible: true,
  formVisible: true,
  activeFixture: false,
  qrCode: false,
  qrNotice: 'Вход по QR-коду пока недоступен в Chord.',
  screens: 4,
});
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
