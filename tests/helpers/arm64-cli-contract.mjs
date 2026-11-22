// @ts-check
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { verifyCandidateApplication } from "./arm64-candidate-oracle.mjs";

/**
 * Keep every positive/negative TASK-990 CLI assertion in one reusable contract.
 * Reuse is explicit, candidate-only, and never used by the native ARM build lane.
 * @param {{root:string, buildScript:string, candidate:boolean, dist:string, version:string,
 * parseToml:(path:string)=>Record<string,any>}} options
 */
export function runArmCliContract({root:ROOT, buildScript:BUILD_SCRIPT, candidate:CANDIDATE,
  dist:DIST_DIR, version:VERSION, parseToml:parseTomlWithPythonOracle}) {
    const reuseProfiles = CANDIDATE && process.arch !== "arm64" && process.env.CHORD_ARM64_QA_REUSE_PROFILES === "1";
    const tmp = mkdtempSync(join(tmpdir(), "arm64-cli-990-"));
    try {
      // 1. Справка --help -> код 0
      const helpRun = spawnSync("bash", [BUILD_SCRIPT, "--help"], {
        cwd: ROOT,
        encoding: "utf8",
      });
      assert.equal(helpRun.status, 0);
      assert.match(helpRun.stdout, /Использование:\s*scripts\/build-arm64\.sh/);

      // 2. Режим --dry-run не создаёт файлов на диске -> код 0
      const dryDir = join(tmp, "dry-dist");
      const dryRun = spawnSync(
        "bash",
        [BUILD_SCRIPT, "--dry-run", "--output", dryDir, "--profile", "asahi"],
        { cwd: ROOT, encoding: "utf8" },
      );
      assert.equal(dryRun.status, 0);
      assert.match(dryRun.stdout, /aarch64-unknown-linux-gnu/);
      assert.equal(
        existsSync(dryDir),
        false,
        "В режиме --dry-run выходной каталог и файлы не должны создаваться",
      );

      // 3. Изолированная сборка с профилем rpi5 (Raspberry Pi 5 Cortex-A76) и проверка маскирования токенов
      const rpiDist = CANDIDATE ? join(DIST_DIR, "qa-profiles/rpi5") : join(tmp, "rpi5-dist");
      const profileVersion = CANDIDATE ? VERSION : "1.0.0-rc.1";
      const rpiReportPath = join(tmp, "rpi5-report.json");
      const fakeDiscordToken = "mfa.SecretTokenValueForArm64Test123456789";
      const fakeGhToken = "ghp_1234567890abcdefghijklmnopqrstuvwxyzABCD";

      const rpiBuild = spawnSync(
        "bash",
        [
          BUILD_SCRIPT,
          ...(reuseProfiles ? ["--verify"] : []),
          "--output",
          rpiDist,
          "--version",
          profileVersion,
          "--profile",
          "rpi5",
          "--report-json",
          rpiReportPath,
        ],
        {
          cwd: ROOT,
          env: {
            ...process.env,
            DISCORD_TOKEN: fakeDiscordToken,
            GITHUB_TOKEN: fakeGhToken,
          },
          encoding: "utf8",
        },
      );
      assert.equal(
        rpiBuild.status,
        0,
        `Сборка профиля rpi5 упала:\nstdout: ${rpiBuild.stdout}\nstderr: ${rpiBuild.stderr}`,
      );
      if (CANDIDATE) verifyCandidateApplication(ROOT, rpiDist, "rpi5");

      // Проверка отсутствия утечки секретов в логах и отчёте
      const combinedOutput = `${rpiBuild.stdout}\n${rpiBuild.stderr}\n${readFileSync(rpiReportPath, "utf8")}`;
      assert.ok(
        !combinedOutput.includes(fakeDiscordToken) && !combinedOutput.includes(fakeGhToken),
        "Секретные токены из окружения должны быть замаскированы и не попадать в логи или JSON-отчёт",
      );

      const rpiCargoToml = parseTomlWithPythonOracle(join(rpiDist, "cargo-config-aarch64.toml"));
      const rpiFlags = rpiCargoToml.target["aarch64-unknown-linux-gnu"].rustflags.join(" ");
      assert.match(
        rpiFlags,
        /target-cpu=cortex-a76/,
        "Профиль rpi5 должен настраивать target-cpu=cortex-a76 для Raspberry Pi 5",
      );
      assert.match(
        rpiFlags,
        /max-page-size=16384/,
        "Профиль rpi5 должен сохранять выравнивание страниц max-page-size=16384",
      );

      // Проверка профиля asahi (Apple Silicon M1/M2)
      const asahiDist = CANDIDATE ? join(DIST_DIR, "qa-profiles/asahi") : rpiDist;
      const asahiCargoPath = reuseProfiles ? join(asahiDist, "cargo-config-aarch64.toml") : join(tmp, "asahi-cargo.toml");
      const asahiDry = spawnSync(
        "bash",
        [
          BUILD_SCRIPT,
          ...(reuseProfiles ? ["--verify"] : []),
          "--output",
          asahiDist,
          "--version",
          profileVersion,
          "--profile",
          "asahi",
          ...(reuseProfiles ? [] : ["--cargo-config", asahiCargoPath]),
        ],
        { cwd: ROOT, encoding: "utf8" },
      );
      assert.equal(asahiDry.status, 0);
      if (CANDIDATE) verifyCandidateApplication(ROOT, asahiDist, "asahi");
      const asahiCargoToml = parseTomlWithPythonOracle(asahiCargoPath);
      const asahiFlags = asahiCargoToml.target["aarch64-unknown-linux-gnu"].rustflags.join(" ");
      assert.match(
        asahiFlags,
        /target-cpu=apple-m1/,
        "Профиль asahi должен настраивать target-cpu=apple-m1 для Apple Silicon Asahi Linux",
      );

      // 4. Негативный сценарий: повреждение байтов в архиве роняет --verify и sha256sum -c с кодом 1
      // Corrupt a private copy so downloaded build evidence remains immutable.
      const mutationDist = CANDIDATE ? join(tmp, "profile-mutation-dist") : rpiDist;
      if (CANDIDATE) cpSync(rpiDist, mutationDist, { recursive: true });
      const rpiTarPath = join(mutationDist, `discord-gpui-v${profileVersion}-linux-aarch64.tar.gz`);
      const origTarBytes = readFileSync(rpiTarPath);
      const corruptedTarBytes = Buffer.from(origTarBytes);
      const corruptIdx = corruptedTarBytes.length - 10;
      corruptedTarBytes.writeUInt8((corruptedTarBytes.readUInt8(corruptIdx) ^ 0xff) & 0xff, corruptIdx);
      writeFileSync(rpiTarPath, corruptedTarBytes);

      const badVerifySha = spawnSync(
        "bash",
        [BUILD_SCRIPT, "--verify", "--output", mutationDist, "--version", profileVersion],
        { cwd: ROOT, encoding: "utf8" },
      );
      assert.equal(
        badVerifySha.status,
        1,
        "При несовпадении SHA256SUMS режим --verify должен завершаться с кодом 1",
      );
      writeFileSync(rpiTarPath, origTarBytes);

      // 5. Негативный сценарий: подмена бинарника на x86_64 ELF (e_machine = 62) должна отвергаться с кодом 1
      const rpiBinPath = join(mutationDist, "discord-gpui-aarch64");
      const origBinBytes = readFileSync(rpiBinPath);
      const x86FakeBytes = Buffer.from(origBinBytes);
      x86FakeBytes.writeUInt16LE(62, 18); // EM_X86_64 = 62 вместо EM_AARCH64 = 183
      writeFileSync(rpiBinPath, x86FakeBytes);
      const sumsPath = join(mutationDist, "SHA256SUMS");
      const originalSums = readFileSync(sumsPath, "utf8");
      if (CANDIDATE) {
        const changedHash = createHash("sha256").update(x86FakeBytes).digest("hex");
        writeFileSync(sumsPath, originalSums.replace(/^[0-9a-f]{64}  discord-gpui-aarch64$/m, `${changedHash}  discord-gpui-aarch64`));
      }

      const badArchVerify = spawnSync(
        "bash",
        [BUILD_SCRIPT, "--verify", "--output", mutationDist, "--version", profileVersion],
        { cwd: ROOT, encoding: "utf8" },
      );
      assert.equal(
        badArchVerify.status,
        1,
        "При подмене e_machine на x86_64 (62) режим --verify обязан завершаться с кодом 1",
      );
      if (CANDIDATE) assert.match(badArchVerify.stderr, /EM_AARCH64/);
      writeFileSync(rpiBinPath, origBinBytes);
      if (CANDIDATE) writeFileSync(sumsPath, originalSums);

      // 6. Негативные аргументы CLI -> код 1
      for (const badArgs of [
        ["--unknown-flag"],
        ["--version", "not_a_semver!"],
        ["--target", "x86_64-unknown-linux-gnu"],
        ["--profile", "unknown_cpu_profile"],
        ["--output"],
      ]) {
        const res = spawnSync("bash", [BUILD_SCRIPT, ...badArgs], {
          cwd: ROOT,
          encoding: "utf8",
        });
        assert.equal(
          res.status,
          1,
          `Ожидался код выхода 1 для аргументов ${JSON.stringify(badArgs)}, получено: ${res.status}`,
        );
      }

      // 7. Отсутствие артефактов при --verify -> код 2 (среда/файлы недоступны)
      const emptyVerifyDir = join(tmp, "empty-verify");
      spawnSync("mkdir", ["-p", emptyVerifyDir]);
      const missingRes = spawnSync(
        "bash",
        [BUILD_SCRIPT, "--verify", "--output", emptyVerifyDir],
        { cwd: ROOT, encoding: "utf8" },
      );
      assert.equal(
        missingRes.status,
        2,
        "При отсутствии артефактов в каталоге --verify должен возвращать код 2",
      );
    } finally {
      rmSync(tmp, { recursive: true, force: true });
    }
}
