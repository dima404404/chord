// @ts-check
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';

/** @param {string} path */
const digest = path => createHash('sha256').update(readFileSync(path)).digest('hex');

/** @param {NodeJS.ProcessEnv} env @param {string} root */
export function selectArmCandidate(env, root) {
  const dist = env.CHORD_GPUI_TEST_DIST?.trim();
  const version = env.CHORD_GPUI_TEST_VERSION?.trim();
  if (Boolean(dist) !== Boolean(version)) {
    throw new Error('Set both CHORD_GPUI_TEST_DIST and CHORD_GPUI_TEST_VERSION for candidate artifacts');
  }
  assert.match(version || '1.0.0-beta.1', /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/, 'Candidate version must be SemVer');
  return { candidate: Boolean(dist), dist: dist ? resolve(dist) : join(root, 'rust/dist'), version: version || '1.0.0-beta.1' };
}

/** @param {Record<string, any>} parsed @param {boolean} generated @param {string} key */
export function cargoEnvironmentValue(parsed, generated, key) {
  const entry = parsed.env?.[key];
  if (!generated) return entry;
  assert.equal(entry?.force, true, `Generated ${key} must be forced for the actual build`);
  assert.equal(typeof entry.value, 'string');
  return entry.value;
}

/** @param {string} root @param {string} directory @param {'generic'|'rpi5'|'asahi'} profile */
export function verifyCandidateApplication(root, directory, profile) {
  const binary = join(directory, 'discord-gpui-aarch64');
  const elf = spawnSync('python3', ['-c',
    'import sys; sys.path.insert(0, sys.argv[1]); from verify import validate_gui_application; validate_gui_application(sys.argv[2])',
    join(root, 'scripts/arm64'), binary], { encoding: 'utf8' });
  assert.equal(elf.status, 0, elf.stderr);
  const provenance = JSON.parse(readFileSync(join(directory, 'arm64-build-provenance.json'), 'utf8'));
  for (const token of ['aarch64-unknown-linux-gnu', '--locked', 'chord-gpui']) assert.ok(provenance.cargo_command.includes(token));
  assert.equal(provenance.binary_sha256, digest(binary));
  assert.match(provenance.source.commit, /^[0-9a-f]{40}$/);
  assert.match(provenance.cargo_reported_artifact.executable, /aarch64-unknown-linux-gnu\/release\/chord-gpui$/);
  const source = spawnSync('python3', ['-c',
    'import sys,json; sys.path.insert(0,sys.argv[1]); from build import source_identity; print(json.dumps(source_identity()))',
    join(root, 'scripts/arm64')], { encoding: 'utf8' });
  assert.equal(source.status, 0, source.stderr);
  const current = JSON.parse(source.stdout);
  for (const key of ['cargo_lock_sha256', 'rust_sources_sha256', 'gpui_gitlink']) {
    assert.equal(provenance.source[key], current[key], `Candidate ${key} must match the current production source`);
  }
  assert.equal(provenance.profile, profile);
  assert.equal(provenance.target_cpu, { generic: 'generic', rpi5: 'cortex-a76', asahi: 'apple-m1' }[profile]);
  return provenance;
}

/** @param {string} root @param {string} path */
export function assertSourceLicense(root, path) {
  assert.deepEqual(readFileSync(path), readFileSync(join(root, 'LICENSE')));
}

/** @param {string} root @param {string} image @param {string} extraction @param {string} expectedSha */
export function inspectOfficialArmAppImage(root, image, extraction, expectedSha) {
  const pins = JSON.parse(readFileSync(join(root, 'packaging/arm64/toolchain-pins.json'), 'utf8'));
  const runtime = process.env.CHORD_ARM64_RUNTIME_FILE;
  assert.ok(runtime, 'CHORD_ARM64_RUNTIME_FILE must identify the pinned official runtime');
  assert.equal(digest(runtime), pins.runtime.sha256);
  assert.equal(statSync(runtime).size, pins.runtime.size);
  const prefix = spawnSync('python3', ['-c',
    'import sys; sys.path.insert(0, sys.argv[1]); from verify import appimage_binary_hash; print(appimage_binary_hash(sys.argv[2], sys.argv[3]))',
    join(root, 'scripts/arm64'), image, runtime], { encoding: 'utf8' });
  assert.equal(prefix.status, 0, prefix.stderr);
  assert.equal(prefix.stdout.trim(), expectedSha);
  const extract = spawnSync('unsquashfs', ['-no-progress', '-o', String(pins.runtime.size),
    '-d', extraction, image], { encoding: 'utf8' });
  assert.equal(extract.status, 0, extract.stderr);
  assert.equal(digest(join(extraction, 'usr/bin/discord-gpui')), expectedSha);
  assert.ok(existsSync(join(extraction, 'AppRun')));
  assert.ok(existsSync(join(extraction, 'discord-gpui.desktop')));
  assertSourceLicense(root, join(extraction, 'usr/share/doc/discord-gpui/LICENSE'));
  const dependencies = JSON.parse(readFileSync(join(extraction, 'usr/lib/bundled-deps.json'), 'utf8'));
  assert.equal(dependencies.bundled, false);
  assert.ok(dependencies.elf_needed.includes('libc.so.6'));
}
