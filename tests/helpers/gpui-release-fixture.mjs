// @ts-check
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const candidateDist = process.env.CHORD_GPUI_TEST_DIST;
export const gpuiTestDist = candidateDist
  ? resolve(candidateDist)
  : fileURLToPath(new URL("../../rust/dist/", import.meta.url));
export const gpuiTestVersion = candidateDist
  ? process.env.CHORD_GPUI_TEST_VERSION ?? "1.0.0-beta.1"
  : "1.0.0-beta.1";
export const gpuiCandidate = Boolean(candidateDist);
if (gpuiCandidate) assert.match(gpuiTestVersion, /^\d+\.\d+\.\d+-qa\.[1-9]\d*$/);
export const gpuiTestLicense = gpuiCandidate ? "GPL-3.0-only" : "MIT OR Apache-2.0";
export const gpuiTestTarball = `discord-gpui-v${gpuiTestVersion}-linux-x86_64.tar.gz`;
export const gpuiTestPackaging = candidateDist
  ? resolve(gpuiTestDist, "../../packaging")
  : fileURLToPath(new URL("../../packaging/", import.meta.url));

/** @param {string} filename */
export function gpuiTestArtifact(filename) {
  const path = join(gpuiTestDist, filename);
  if (candidateDist) {
    const sums = readFileSync(join(gpuiTestDist, "SHA256SUMS"), "utf8");
    const matches = sums.split(/\r?\n/).flatMap((line) => {
      const match = /^([a-f0-9]{64})\s+\*?(.+)$/.exec(line);
      return match?.[2] === filename ? [match[1]] : [];
    });
    assert.equal(matches.length, 1, "Expected one SHA256SUMS entry for " + filename);
    assert.equal(createHash("sha256").update(readFileSync(path)).digest("hex"), matches[0], path);
  }
  return path;
}
