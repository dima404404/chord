import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { execFileSync, spawnSync } from 'node:child_process';

const ROOT = process.cwd();
const RS_PATH = path.join(ROOT, 'rust/chord-gpui/src/profile/spotify_widget.rs');
const MOD_PATH = path.join(ROOT, 'rust/chord-gpui/src/profile/mod.rs');
const MAIN_PATH = path.join(ROOT, 'rust/chord-gpui/src/main.rs');

test('TASK-1013B: spotify_widget.rs exists in chord-gpui/src/profile, is exported, forbids unsafe, and is < 800 lines', () => {
  assert.ok(fs.existsSync(RS_PATH), 'rust/chord-gpui/src/profile/spotify_widget.rs must exist');
  assert.ok(fs.existsSync(MOD_PATH), 'rust/chord-gpui/src/profile/mod.rs must exist');

  const modSrc = fs.readFileSync(MOD_PATH, 'utf8');
  assert.match(modSrc, /\bpub\s+mod\s+spotify_widget\s*;/, 'profile/mod.rs must export pub mod spotify_widget');

  const mainSrc = fs.readFileSync(MAIN_PATH, 'utf8');
  assert.match(mainSrc, /\bpub\s+mod\s+profile\s*;/, 'main.rs must export pub mod profile');

  const src = fs.readFileSync(RS_PATH, 'utf8');
  assert.match(src, /#!\[forbid\(unsafe_code\)\]/, 'must forbid unsafe_code');
  assert.ok(src.split('\n').length < 800, 'file must be under 800 lines');

  assert.match(src, /#1db954/i, 'must define Spotify brand green #1db954');
  assert.match(src, /SPOTIFY_DESYNC_THRESHOLD_MS\s*:\s*u64\s*=\s*2_?000/);
  assert.match(src, /ListenAlongCommand/);
  assert.match(src, /SpotifyProfileWidget/);
});

test('TASK-1013B: cargo test -p chord-gpui --bin chord-gpui profile::spotify_widget passes all unit tests', () => {
  const out = execFileSync(
    'cargo',
    ['test', '--manifest-path', 'rust/chord-gpui/Cargo.toml', '--bin', 'chord-gpui', 'profile::spotify_widget'],
    { cwd: ROOT, encoding: 'utf8' }
  );
  assert.match(out, /test result: ok\./);
  const match = out.match(/(\d+)\s+passed;\s+0\s+failed/);
  assert.ok(match && Number(match[1]) >= 6, `expected >= 6 passing Rust tests, got: ${out}`);
});

test('TASK-1013B: 5/5 mutation kill rate on spotify_widget.rs', () => {
  const original = fs.readFileSync(RS_PATH, 'utf8');
  const listenAlongResult = [
    'Some(ListenAlongCommand {',
    '            track_id,',
    '            position_ms: self.elapsed_ms,',
    '        })',
  ].join('\n');
  const mutants = [
    {
      name: 'wrong spotify green color',
      find: 'pub const SPOTIFY_GREEN_HEX: &str = "#1db954";',
      replace: 'pub const SPOTIFY_GREEN_HEX: &str = "#000000";',
      assertion: 'uses_spotify_green',
    },
    {
      name: 'progress bar continues when paused',
      find: 'if !self.is_playing {',
      replace: 'if false {',
      assertion: 'paused_playback_does_not_advance',
    },
    {
      name: 'desync threshold raised above 2s',
      find: 'pub const SPOTIFY_DESYNC_THRESHOLD_MS: u64 = 2_000;',
      replace: 'pub const SPOTIFY_DESYNC_THRESHOLD_MS: u64 = 10_000;',
      assertion: 'corrects_drift_over_two_seconds',
    },
    {
      name: 'progress ratio not clamped to 1.0',
      find: '(self.elapsed_ms as f64 / self.duration_ms as f64).clamp(0.0, 1.0)',
      replace: '(self.elapsed_ms as f64 / self.duration_ms as f64) + 0.5',
      assertion: 'progress_ratio_stays_in_range',
    },
    {
      name: 'listen along returns None when active',
      find: listenAlongResult,
      replace: `${listenAlongResult}.filter(|_| false)`,
      assertion: 'active_track_produces_a_positioned_listen_along_command',
    },
  ];

  // The timeline has no external dependencies. Compile each mutant independently so the
  // checkout and Cargo target stay untouched and compilation errors cannot count as kills.
  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'chord-spotify-mutations-'));
  const source = path.join(scratch, 'spotify_widget.rs');
  const binary = path.join(scratch, process.platform === 'win32' ? 'spotify-tests.exe' : 'spotify-tests');
  /** @type {import("node:child_process").SpawnSyncOptionsWithStringEncoding} */
  const commandOptions = { cwd: scratch, encoding: 'utf8' };

  try {
    for (const mutant of mutants) {
      assert.equal(original.split(mutant.find).length, 2, `Expected exactly one anchor: ${mutant.name}`);
      fs.writeFileSync(source, original.replace(mutant.find, mutant.replace));
      const compiled = spawnSync(
        process.env.RUSTC || 'rustc',
        ['--edition', '2024', '--test', source, '-o', binary],
        commandOptions,
      );
      assert.ifError(compiled.error);
      assert.equal(compiled.status, 0, `Mutant failed to compile: ${mutant.name}\n${compiled.stderr}`);

      const tested = spawnSync(binary, [`tests::${mutant.assertion}`, '--exact'], commandOptions);
      assert.ifError(tested.error);
      const output = `${tested.stdout}\n${tested.stderr}`;
      assert.equal(tested.status, 101, `Expected an assertion failure: ${mutant.name}\n${output}`);
      assert.match(output, /test result: FAILED\. 0 passed; 1 failed;/, `Mutant survived: ${mutant.name}`);
    }
  } finally {
    fs.rmSync(scratch, { recursive: true, force: true });
  }
});
