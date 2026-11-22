// @ts-check
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { createHash } from 'node:crypto';

const root = fileURLToPath(new URL('../../', import.meta.url));
const linux = { skip: process.platform !== 'linux' && 'requires real Linux ELF tools' };

/** @param {import('node:test').TestContext} t */
function fixture(t) {
  const path = mkdtempSync(join(tmpdir(), 'arm64 candidate & '));
  t.after(() => rmSync(path, { recursive: true, force: true }));
  return path;
}

/** @param {NodeJS.ProcessEnv} env */
function elfContract(env) {
  const childEnv = { ...process.env, ...env };
  // A nested Node test runner must not inherit the parent's worker IPC protocol.
  delete childEnv.NODE_TEST_CONTEXT;
  return spawnSync(process.execPath, ['--test', '--test-name-pattern=SUB-990-2',
    join(root, 'tests/unit/arm64-packaging-990.test.mjs')], {
    cwd: root, encoding: 'utf8', timeout: 20_000,
    env: childEnv,
  });
}

test('candidate mode rejects the real historical syscall-only smoke harness', linux, t => {
  const dist = fixture(t);
  // This is an immutable negative specimen, never a successful GUI fixture.
  copyFileSync(join(root, 'rust/dist/discord-gpui-aarch64'), join(dist, 'discord-gpui-aarch64'));
  const result = elfContract({ CHORD_GPUI_TEST_DIST: dist, CHORD_GPUI_TEST_VERSION: '1.0.0-qa.1' });
  assert.notEqual(result.status, 0, result.stdout);
  assert.match(result.stdout + result.stderr, /versioned libc\.so\.6/);
});

test('candidate directory and version must be selected together', linux, t => {
  const dist = fixture(t);
  const result = elfContract({ CHORD_GPUI_TEST_DIST: dist, CHORD_GPUI_TEST_VERSION: '' });
  assert.notEqual(result.status, 0, result.stdout);
  assert.match(result.stdout + result.stderr, /CHORD_GPUI_TEST_VERSION/);
});

test('candidate version cannot silently select the legacy artifact directory', linux, () => {
  const result = elfContract({ CHORD_GPUI_TEST_DIST: '', CHORD_GPUI_TEST_VERSION: '1.0.0-qa.1' });
  assert.notEqual(result.status, 0, result.stdout);
  assert.match(result.stdout + result.stderr, /CHORD_GPUI_TEST_DIST/);
});

test('real tar packaging retains the supplied SBOM, guide and unchanged application bytes', linux, t => {
  const stage = fixture(t);
  copyFileSync('/bin/true', join(stage, 'discord-gpui-aarch64'));
  copyFileSync(join(root, 'packaging/arm64/cargo-config.toml'), join(stage, 'cargo-config-aarch64.toml'));
  // Test the container helper with real input files; this host ELF is not ARM qualification.
  const sbom = join(root, 'rust/dist/sbom.spdx.json');
  const source = [
    'import sys, tarfile; from pathlib import Path; from types import SimpleNamespace;',
    'sys.path.insert(0, sys.argv[1]); from build import package_tar;',
    'stage=Path(sys.argv[2]); sbom=Path(sys.argv[3]);',
    'package_tar(SimpleNamespace(version="1.0.0-qa.1", sbom_file=sbom), stage, 1234567890);',
    'archive=tarfile.open(stage / "discord-gpui-v1.0.0-qa.1-linux-aarch64.tar.gz");',
    'prefix="discord-gpui-v1.0.0-qa.1-linux-aarch64/";',
    'assert archive.extractfile(prefix+"sbom.spdx.json").read()==sbom.read_bytes();',
    'assert archive.extractfile(prefix+"BETA-TESTING-GUIDE.md").read()==Path(sys.argv[4]).read_bytes();',
    'assert archive.extractfile(prefix+"LICENSE").read()==Path(sys.argv[5]).read_bytes();',
    'assert archive.extractfile(prefix+"discord-gpui").read()==Path("/bin/true").read_bytes();',
    'assert all(x.uid==0 and x.gid==0 and x.mtime==1234567890 for x in archive.getmembers())',
  ].join(' ');
  const result = spawnSync('python3', ['-c', source, join(root, 'scripts/arm64'), stage, sbom,
    join(root, 'docs/BETA-TESTING-GUIDE.md'), join(root, 'LICENSE')], { encoding: 'utf8', timeout: 20_000 });
  assert.equal(result.status, 0, result.stderr);
});

test('AppDir staging records host dependencies without claiming libraries were bundled', linux, t => {
  const stage = fixture(t);
  copyFileSync('/bin/true', join(stage, 'discord-gpui-aarch64'));
  const source = [
    'import sys, json; from pathlib import Path; from types import SimpleNamespace;',
    'sys.path.insert(0, sys.argv[1]); from build import stage_appdir;',
    'stage=Path(sys.argv[2]); appdir=stage_appdir(SimpleNamespace(version="1.0.0-qa.1"), stage);',
    'manifest=json.loads((appdir/"usr/lib/bundled-deps.json").read_text());',
    'assert manifest["bundled"] is False;',
    'assert "libc.so.6" in manifest["elf_needed"];',
    'assert manifest["native_validation"]=="not_run";',
    'assert (appdir/"usr/bin/discord-gpui").read_bytes()==Path("/bin/true").read_bytes();',
    'assert (appdir/"usr/share/doc/discord-gpui/LICENSE").read_bytes()==Path(sys.argv[3]).read_bytes();',
    'assert (appdir/"AppRun").is_file() and (appdir/"discord-gpui.desktop").is_file()',
  ].join(' ');
  const result = spawnSync('python3', ['-c', source, join(root, 'scripts/arm64'), stage, join(root, 'LICENSE')], {
    encoding: 'utf8', timeout: 20_000,
  });
  assert.equal(result.status, 0, result.stderr);
});

test('real Debian packaging ships exact source license and no invented dual-license grant', linux, t => {
  const stage = fixture(t);
  // Test only Debian payload construction. Candidate application validation rejects this specimen.
  copyFileSync(join(root, 'rust/dist/discord-gpui-aarch64'), join(stage, 'discord-gpui-aarch64'));
  const source = [
    'import sys, subprocess, tarfile; from pathlib import Path; from types import SimpleNamespace;',
    'sys.path.insert(0, sys.argv[1]); from build import package_deb;',
    'stage=Path(sys.argv[2]); package_deb(SimpleNamespace(version="1.0.0-qa.1"), stage, {"node":sys.argv[3]});',
    'process=subprocess.Popen(["dpkg-deb","--fsys-tarfile",str(stage/"discord-gpui_1.0.0-qa.1_arm64.deb")], stdout=subprocess.PIPE);',
    'archive=tarfile.open(fileobj=process.stdout,mode="r|"); contents={m.name.lstrip("./"):archive.extractfile(m).read() for m in archive if m.isfile()};',
    'assert process.wait()==0;',
    'assert contents["usr/share/doc/discord-gpui/LICENSE"]==Path(sys.argv[4]).read_bytes();',
    'assert b"MIT or Apache" not in contents["usr/share/doc/discord-gpui/copyright"]',
  ].join(' ');
  const result = spawnSync('python3', ['-c', source, join(root, 'scripts/arm64'), stage,
    process.execPath, join(root, 'LICENSE')], { encoding: 'utf8', timeout: 20_000 });
  assert.equal(result.status, 0, result.stderr);
});

test('SBOM optional manifest reads the selected real Cargo workspace and matching lock', linux, t => {
  const dir = fixture(t);
  mkdirSync(join(dir, 'src'));
  writeFileSync(join(dir, 'src/lib.rs'), 'pub fn artifact_identity() -> u8 { 7 }\n');
  writeFileSync(join(dir, 'Cargo.toml'), '[package]\nname="arm-qa-sbom-oracle"\nversion="7.8.9"\nedition="2021"\nlicense="MIT"\n[workspace]\n');
  const env = { ...process.env };
  const lock = spawnSync('cargo', ['generate-lockfile', '--offline', '--manifest-path', join(dir, 'Cargo.toml')], {
    env, encoding: 'utf8', timeout: 20_000,
  });
  assert.equal(lock.status, 0, lock.stderr);
  const output = join(dir, 'selected.spdx.json');
  const result = spawnSync(process.execPath, [join(root, 'scripts/sbom.mjs'), '--manifest-path',
    join(dir, 'Cargo.toml'), '--output', output, '--check'], { env, encoding: 'utf8', timeout: 20_000 });
  assert.equal(result.status, 0, result.stderr);
  const document = JSON.parse(readFileSync(output, 'utf8'));
  assert.ok(document.packages.some((/** @type {{name: string, versionInfo: string}} */ pkg) =>
    pkg.name === 'arm-qa-sbom-oracle' && pkg.versionInfo === '7.8.9'));
  const digest = createHash('sha256').update(readFileSync(join(dir, 'Cargo.lock'))).digest('hex');
  assert.equal(document.documentNamespace, `https://discord-lab.local/spdx/${digest}`);
  assert.equal(document.spdxVersion, 'SPDX-2.3');
});

test('native ARM QA carrier builds all real profiles on a pinned Jammy ARM64 container', () => {
  const workflow = readFileSync(join(root, '.github/workflows/gpui-arm64-qa.yml'), 'utf8');
  assert.match(workflow, /runs-on: ubuntu-24\.04-arm/);
  assert.match(workflow, /image: ubuntu@sha256:2926e8e6ef0b816e19849e33ccc728c72b921a428286e6595d52fa92c5716626/);
  assert.match(workflow, /profile: \[generic, rpi5, asahi\]/);
  assert.match(workflow, /max-parallel: 1/);
  assert.match(workflow, /branches: \['qa\/task-990-gpui-arm64-\*'\]/);
  assert.match(workflow, /node scripts\/bootstrap-gpui\.mjs\n/);
  assert.match(workflow, /cargo fetch --locked --manifest-path rust\/chord-gpui\/Cargo\.toml/);
  assert.match(workflow, /node scripts\/sbom\.mjs --manifest-path rust\/chord-gpui\/Cargo\.toml/);
  assert.match(workflow, /bash scripts\/build-arm64\.sh --output "\$ARM_QA_OUTPUT"/);
  assert.match(workflow, /--profile "\$ARM_QA_PROFILE"/);
  assert.match(workflow, /if: always\(\) && steps\.packages\.outcome == 'success'/);
  assert.doesNotMatch(workflow, /continue-on-error: true|ubuntu-22\.04-arm|gh release|git push/);
});
