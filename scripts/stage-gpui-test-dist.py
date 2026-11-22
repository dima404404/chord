#!/usr/bin/env python3
"""Stage a local GPUI package candidate for release-contract tests."""

import argparse
import gzip
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile

import yaml


ROOT = Path(__file__).resolve().parents[1]
BASE_VERSION = "1.0.0-beta.1"
LEGACY_DIST = ROOT / "rust" / "dist"
PROJECT_LICENSE = "GPL-3.0-only"


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def run(*command):
    return subprocess.run(command, cwd=ROOT, check=True, text=True, stdout=subprocess.PIPE).stdout


def replace_one(text, pattern, replacement):
    text, count = re.subn(pattern, lambda _: replacement, text, flags=re.MULTILINE)
    if count != 1:
        raise ValueError(f"Expected one packaging field matching {pattern!r}, got {count}")
    return text


def stage_specs(package_dir, version, tarball, sums):
    arch = package_dir / "arch"
    arch.mkdir(parents=True)
    sources = [tarball, "discord-gpui", "sbom.spdx.json"]
    arch_version = version.replace("-", "_")
    pkgbuild = arch / "PKGBUILD"
    text = (ROOT / "packaging" / "arch" / "PKGBUILD").read_text(encoding="utf-8")
    text = replace_one(text, r"^_upstream_ver=.*$", f"_upstream_ver={version}")
    text = replace_one(text, r"^pkgver=.*$", f"pkgver={arch_version}")
    arch_sums = "\n".join(f"  '{sums[name]}'" for name in sources)
    text = replace_one(text, r"^sha256sums=\(\n(?:[^\n]*\n)*?^\)", f"sha256sums=(\n{arch_sums}\n)")
    text = replace_one(text, r"^license=.*$", f"license=('{PROJECT_LICENSE}')")
    text = replace_one(
        text, r'^  # 2\. .*\n(?:  [^\n]*\n)+(?=\n  # 3\.)',
        '  install -Dm644 "${_extract_dir}/LICENSE" "${pkgdir}/usr/share/licenses/${pkgname}/LICENSE"\n',
    )
    pkgbuild.write_text(text, encoding="utf-8")

    srcinfo = arch / ".SRCINFO"
    text = (ROOT / "packaging" / "arch" / ".SRCINFO").read_text(encoding="utf-8")
    text = text.replace(BASE_VERSION, version).replace(BASE_VERSION.replace("-", "_"), arch_version)
    source_sums = "\n".join(f"\tsha256sums = {sums[name]}" for name in sources)
    text = replace_one(text, r"^\tsha256sums = [a-f0-9]{64}(?:\n\tsha256sums = [a-f0-9]{64})*$", source_sums)
    text = replace_one(text, r"^\tlicense = MIT\n\tlicense = Apache-2\.0$", f"\tlicense = {PROJECT_LICENSE}")
    srcinfo.write_text(text, encoding="utf-8")

    flatpak = Path("flatpak/com.discord_lab.DiscordGpui.yml")
    flatpak_manifest = yaml.safe_load((ROOT / "packaging" / flatpak).read_text(encoding="utf-8"))
    module = next(module for module in flatpak_manifest["modules"] if module["name"] == "discord-gpui")
    for source in module["sources"]:
        source["path"] = source["path"].replace(BASE_VERSION, version)
        source["sha256"] = sums[Path(source["path"]).name]
    module["build-commands"] = [
        command.replace(BASE_VERSION, version)
        .replace("LICENSE-MIT", "LICENSE")
        .replace("<project_license>MIT OR Apache-2.0", f"<project_license>{PROJECT_LICENSE}")
        for command in module["build-commands"]
        if not command.startswith("install -Dm644 LICENSE-APACHE ")
    ]

    snap = Path("snap/snapcraft.yaml")
    snap_manifest = yaml.safe_load((ROOT / "packaging" / snap).read_text(encoding="utf-8"))
    snap_manifest["version"] = version
    snap_manifest["license"] = PROJECT_LICENSE
    pull = snap_manifest["parts"]["discord-gpui"]["override-build"].splitlines(keepends=True)
    snap_manifest["parts"]["discord-gpui"]["override-build"] = "".join(
        line.replace(BASE_VERSION, version).replace("LICENSE-MIT", "LICENSE")
        for line in pull if "LICENSE-APACHE" not in line
    )
    for part in snap_manifest["parts"].values():
        part["source"] = part["source"].replace(BASE_VERSION, version)
        part["source-checksum"] = f"sha256/{sums[Path(part['source']).name]}"
        if "organize" in part:
            part["organize"] = {name.replace(BASE_VERSION, version): target for name, target in part["organize"].items()}

    for path, manifest in ((flatpak, flatpak_manifest), (snap, snap_manifest)):
        output = package_dir / path
        output.parent.mkdir()
        output.write_text(yaml.safe_dump(manifest, sort_keys=False, allow_unicode=True), encoding="utf-8")

    rpm = Path("rpm/discord-gpui.spec")
    text = (ROOT / "packaging" / rpm).read_text(encoding="utf-8")
    text = replace_one(text, r"^%global tarball_version .*$", f"%global tarball_version {version}")
    text = replace_one(text, r"^%global bin_sha256 .*$", f"%global bin_sha256 {sums['discord-gpui']}")
    text = replace_one(text, r"^%global tarball_sha256 .*$", f"%global tarball_sha256 {sums[tarball]}")
    rpm_version, release = version.split("-", 1)
    text = replace_one(text, r"^Version:.*$", f"Version:        {rpm_version}")
    text = replace_one(text, r"^Release:.*$", f"Release:        0.1.{release}%{{?dist}}")
    text = replace_one(text, r"^License:.*$", f"License:        {PROJECT_LICENSE}")
    text = replace_one(text, r"^%license .*$", "%license LICENSE")
    (package_dir / rpm).parent.mkdir()
    (package_dir / rpm).write_text(text, encoding="utf-8")


def stage_candidate(directory, binary, sbom, version):
    dist = directory / "rust" / "dist"
    dist.mkdir(parents=True)
    shutil.copyfile(binary, dist / "discord-gpui")
    (dist / "discord-gpui").chmod(0o755)
    shutil.copyfile(sbom, dist / "sbom.spdx.json")

    archive_root = f"discord-gpui-v{version}-linux-x86_64"
    tarball = archive_root + ".tar.gz"
    commit = run("git", "rev-parse", "HEAD").strip()
    timestamp = int(run("git", "show", "-s", "--format=%ct", commit))
    notes = directory / f"RELEASE-NOTES-v{version}.md"
    notes.write_text(
        f"# GPUI local test package {version}\n\n"
        f"Packaging source commit: `{commit}`.\n\n"
        "This package is for local verification of the package contents and startup. "
        "The beta-testing guide is included as a reference.\n",
        encoding="utf-8",
    )
    payload = [
        dist / "discord-gpui", dist / "sbom.spdx.json", ROOT / "LICENSE",
        ROOT / "docs" / "BETA-TESTING-GUIDE.md", notes,
    ]
    with (dist / tarball).open("xb") as target, gzip.GzipFile(fileobj=target, mode="wb", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w") as archive:
            for path in payload:
                info = archive.gettarinfo(str(path), f"{archive_root}/{path.name}")
                info.uid = info.gid = 0
                info.uname = info.gname = "root"
                info.mtime = timestamp
                with path.open("rb") as data:
                    archive.addfile(info, data)

    sums = {name: sha256(dist / name) for name in (tarball, "discord-gpui", "sbom.spdx.json")}
    stage_specs(directory / "packaging", version, tarball, sums)
    image = dist / f"discord-gpui-v{version}-x86_64.AppImage"
    subprocess.run([
        ROOT / "scripts" / "build-appimage.sh",
        "--binary", dist / "discord-gpui", "--sbom", dist / "sbom.spdx.json", "--output", image,
        "--version", version, "--report-json", directory / "appimage.json",
    ], cwd=ROOT, check=True)
    report_path = directory / "appimage.json"
    report = json.loads(report_path.read_text(encoding="utf-8"))
    report["appimage_path"] = str(image.relative_to(directory))
    report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    sums[image.name] = sha256(image)
    (dist / "SHA256SUMS").write_text(
        "".join(f"{digest}  {name}\n" for name, digest in sums.items()), encoding="utf-8",
    )
    (directory / "qa-artifacts.json").write_text(json.dumps({
        "version": version,
        "packaging_source": commit,
        "input_binary_sha256": sums["discord-gpui"],
        "input_sbom_sha256": sums["sbom.spdx.json"],
    }, indent=2) + "\n", encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True, help="Trusted local release-profile GPUI executable")
    parser.add_argument("--sbom", type=Path, required=True, help="SPDX SBOM for the supplied build")
    parser.add_argument("--version", required=True, help="Local version in x.y.z-qa.N form")
    parser.add_argument("--output", type=Path, required=True, help="New QA directory outside rust/dist")
    args = parser.parse_args()
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+-qa\.[1-9][0-9]*", args.version):
        parser.error("--version must have form x.y.z-qa.N")
    output = args.output.resolve()
    if output.is_relative_to(LEGACY_DIST) or output.exists():
        parser.error("--output must be a new directory outside rust/dist")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".gpui-qa-", dir=output.parent) as temporary:
        stage = Path(temporary) / "candidate"
        stage_candidate(stage, args.binary.resolve(), args.sbom.resolve(), args.version)
        stage.rename(output)
    print(output / "rust" / "dist")


if __name__ == "__main__":
    main()

