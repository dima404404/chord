# Genuine ARM64 QA candidates

TASK-990 accepts a new, explicitly selected QA artifact family without rewriting
the historical `rust/dist` files or their checksum/signature records. The tracked
25,728-byte ARM executable is a syscall-only historical smoke specimen. Candidate
validation rejects it because it does not link versioned libc; a printed success
line alone cannot establish that the GUI was built or tested.

## Artifact layout

Use `CHORD_GPUI_TEST_VERSION=1.0.0-qa.1` and a fresh external directory:

```text
candidate/
  discord-gpui                         # real x64 executable, staged separately
  discord-gpui-v1.0.0-qa.1-linux-x86_64.tar.gz
  discord-gpui-aarch64                 # actual Cargo chord-gpui executable
  discord-gpui-v1.0.0-qa.1-linux-aarch64.tar.gz
  discord-gpui_1.0.0-qa.1_arm64.deb
  discord-gpui-v1.0.0-qa.1-aarch64.AppImage
  cargo-config-aarch64.toml
  arm64-build-provenance.json
  sbom.spdx.json
  SHA256SUMS
  runtime-aarch64                      # official, independently pinned runtime
  qa-profiles/
    rpi5/                             # complete, separately built ARM family
    asahi/                            # complete, separately built ARM family
```

Each ARM family has its own binary, three packages, config, provenance, SBOM and
checksum catalog. Merge catalog entries only after verifying the actual bytes;
never substitute a historical digest for a new artifact. Keep the three profile
directories distinct. Preserve executable modes when transferring artifacts,
for example with an enclosing tar archive.

The tar, Debian payload and AppDir ship the exact root `LICENSE` (GPLv3 text).
The current source does not provide `LICENSE-MIT` or `LICENSE-APACHE`.
The official AppImage uses the pinned runtime followed by SquashFS; historical
custom-container assertions remain separate. Its dependency manifest explicitly
states that system ABI libraries are required and were not bundled.

## Build prerequisites

The manual/push-only `gpui-arm64-qa.yml` carrier builds native ARM64 on an
`ubuntu-24.04-arm` host inside the pinned official Jammy ARM64 container. The
deprecated `ubuntu-22.04-arm` runner label is unnecessary. Its three isolated
profile jobs run real locked builds; separate runner filesystems bound target
disk usage. Runner scheduling is controlled by the coordinator.

Use an exact Rust toolchain with AArch64 Linux std, Node 24, Python >=3.11 that
runs on Jammy, clang/clang++/lld/llvm-ar, pkg-config, binutils, dpkg-deb,
unsquashfs, tar/gzip, flock and git. Install the development libraries declared
by `scripts/arm64/build.py` from official Jammy repositories, including universe:
ALSA, fontconfig, Wayland, XCB/xkbcommon, GTK3/WebKit4.1/libsoup3, GLib/GIO,
libsecret, PipeWire, enchant/hunspell, OpenSSL and zstd. Record effective package
and compiler versions; the pinned base image alone does not pin apt updates.
Never mix newer-distribution headers/libraries into the target ABI.

Initialize and verify the committed authoritative GPUI submodule with
`node scripts/bootstrap-gpui.mjs`. Supply official AppImage assets matching every
SHA/size in `toolchain-pins.json`; the builder never downloads replacements.
Generate a current standalone GPUI SPDX document before packaging:

```sh
cargo fetch --locked --manifest-path rust/chord-gpui/Cargo.toml
node scripts/sbom.mjs --manifest-path rust/chord-gpui/Cargo.toml \
  --output /absolute/qa/gpui.spdx.json
export CHORD_ARM64_SBOM_FILE=/absolute/qa/gpui.spdx.json
export CHORD_ARM64_RUNTIME_FILE=/absolute/tools/runtime-aarch64
export CHORD_ARM64_APPIMAGETOOL=/absolute/tools/appimagetool-aarch64.AppImage
bash scripts/build-arm64.sh --output /absolute/qa/candidate \
  --version 1.0.0-qa.1 --profile generic
```

SPDX generation preserves unavailable license metadata as `NOASSERTION`; it does
not imply that the separate license allowlist audit passed. The existing default
workspace audit and its assertions remain intact.

## Verify downloaded candidates on x64

Verify in a checkout containing the compiled commit in its ancestry, identical
Rust source/lock content, and the clean pinned GPUI submodule:

```sh
export CHORD_GPUI_TEST_DIST=/absolute/qa/candidate
export CHORD_GPUI_TEST_VERSION=1.0.0-qa.1
export CHORD_ARM64_RUNTIME_FILE=/absolute/qa/candidate/runtime-aarch64
export CHORD_ARM64_QA_REUSE_PROFILES=1
node --test tests/unit/arm64-packaging-990.test.mjs
```

Both candidate selectors are mandatory together. Without them, legacy paths,
versions, historical container checks and checksum expectations remain the
defaults. Reuse is explicit and applies only to non-ARM verification hosts; a
native ARM test run still builds both hardware profiles. Missing/corrupt profile
families fail instead of skipping or starting an unintended x64 cross build.
All positive CLI exit/config assertions remain, and private copies undergo the
original archive corruption and wrong-architecture checks. The candidate
wrong-architecture check also updates the changed file's checksum, requiring
the ELF machine validation itself to reject it.

These are artifact, provenance, ABI and headless contracts. A hosted 4KiB kernel
does not qualify Raspberry Pi or Asahi hardware, 16KiB kernel execution, GPU UI,
audio routing or the optional instruction floor. The current generic source
profile still enables CRC/AES/SHA2; the name alone is not portable ARMv8-A proof.
The legacy report's `page_size_bytes` field describes ELF LOAD alignment,
not a measurement of the running kernel's page size. Native device acceptance
remains separate and must report actual CPU/kernel/display/audio results.

References: [GitHub runner deprecation](https://github.com/actions/runner-images/issues/14254),
[native runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners),
[official Ubuntu image](https://hub.docker.com/_/ubuntu),
[Jammy WebKit4.1](https://packages.ubuntu.com/jammy-updates/libwebkit2gtk-4.1-dev).
