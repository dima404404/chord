#!/usr/bin/env bash
# Скрипт сборки автономного пакета AppImage (Type 2 x86_64) для нативного клиента discord-gpui (TASK-961).
#
# Выполняет:
# 1. Валидацию входного ELF64 бинарника discord-gpui и его динамических зависимостей (ldd).
# 2. Формирование структуры AppDir (AppRun, discord-gpui.desktop, иконки PNG/SVG, .DirIcon, лицензии, SBOM).
# 3. Бандлинг пользовательских разделяемых библиотек (libxkbcommon, libxkbcommon-x11, libxcb-xkb, libXau, libXdmcp)
#    в AppDir/usr/lib/ с генерацией манифеста bundled-deps.json.
# 4. Сборку исполняемого Type 2 x86_64 AppImage (ELF64 + магическая сигнатура AI\x02 на смещении 8..11)
#    с поддержкой --appimage-extract, --appimage-extract-and-run, --appimage-offset, --appimage-updateinformation.
# 5. Верификацию холодного запуска --headless-smoke непосредственно внутри собранного AppImage
#    с контролем бюджета памяти VmRSS < 60 MB.
#
# Коды выхода:
#   0 — успешная сборка и верификация
#   1 — ошибка аргументов командной строки, невалидный ELF/зависимости или превышение лимита VmRSS
#   2 — исходный бинарник или обязательные системные утилиты среды недоступны
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="1.0.0-beta.1"
BINARY="$ROOT/rust/dist/discord-gpui"
SBOM="$ROOT/rust/dist/sbom.spdx.json"
OUTPUT=""
REPORT_JSON=""
APPDIR_OUT=""
APPIMAGETOOL="${CHORD_APPIMAGE_TOOL:-}"
RUNTIME_FILE="${CHORD_APPIMAGE_RUNTIME:-}"
MAX_RSS_MB="60"
DRY_RUN=0
SKIP_SMOKE=0

usage() {
  cat <<'EOF'
Использование: scripts/build-appimage.sh [ОПЦИИ]

Опции:
  --sbom PATH          Путь к SBOM входной сборки (по умолчанию rust/dist/sbom.spdx.json)
  --binary PATH        Путь к входному ELF-бинарнику (по умолчанию rust/dist/discord-gpui)
  --output PATH        Путь к выходному файлу .AppImage или директории (по умолчанию rust/dist/discord-gpui-v<VERSION>-x86_64.AppImage)
  --version SEMVER     Версия пакета в формате SemVer (по умолчанию 1.0.0-beta.1)
  --report-json PATH   Записать JSON-отчёт о сборке, зависимостях и замере VmRSS
  --appimagetool PATH  Путь к официальному appimagetool из packaging/arm64/toolchain-pins.json
  --runtime-file PATH Путь к официальному Type 2 x86_64 runtime из того же манифеста
  --appdir-out DIR     Сохранить сформированное дерево AppDir по указанному пути
  --max-rss-mb NUM     Максимально допустимый объём памяти VmRSS в МБ при smoke-тесте (по умолчанию 60)
  --skip-smoke         Пропустить финальную проверку --headless-smoke внутри AppImage
  --dry-run            Вывести план сборки без записи файлов на диск
  -h, --help           Показать эту справку
EOF
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --binary)
      [[ "$#" -ge 2 ]] || { echo "build-appimage: опция --binary требует путь к файлу" >&2; exit 1; }
      BINARY="$2"
      shift 2
      ;;
    --sbom)
      [[ "$#" -ge 2 ]] && [[ "$2" != --* ]] || { echo "build-appimage: опция --sbom требует путь" >&2; exit 1; }
      SBOM="$2"
      shift 2
      ;;
    --output|--out-dir)
      [[ "$#" -ge 2 ]] || { echo "build-appimage: опция $1 требует путь" >&2; exit 1; }
      OUTPUT="$2"
      shift 2
      ;;
    --version)
      [[ "$#" -ge 2 ]] || { echo "build-appimage: опция --version требует строку SemVer" >&2; exit 1; }
      VERSION="$2"
      shift 2
      ;;
    --report-json)
      [[ "$#" -ge 2 ]] || { echo "build-appimage: опция --report-json требует путь к файлу" >&2; exit 1; }
      REPORT_JSON="$2"
      shift 2
      ;;
    --appimagetool|--runtime-file)
      [[ "$#" -ge 2 ]] || { echo "build-appimage: опция $1 требует путь" >&2; exit 1; }
      if [[ "$1" == "--appimagetool" ]]; then
        APPIMAGETOOL="$2"
      else
        RUNTIME_FILE="$2"
      fi
      shift 2
      ;;
    --appdir-out)
      [[ "$#" -ge 2 ]] || { echo "build-appimage: опция --appdir-out требует путь к директории" >&2; exit 1; }
      APPDIR_OUT="$2"
      shift 2
      ;;
    --max-rss-mb)
      [[ "$#" -ge 2 ]] || { echo "build-appimage: опция --max-rss-mb требует числовое значение" >&2; exit 1; }
      MAX_RSS_MB="$2"
      shift 2
      ;;
    --skip-smoke)
      SKIP_SMOKE=1
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "build-appimage: неизвестный аргумент: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

# Валидация SemVer версии
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]]; then
  echo "build-appimage: некорректный формат версии SemVer: $VERSION" >&2
  exit 1
fi

# Валидация числового порога памяти MAX_RSS_MB
if [[ ! "$MAX_RSS_MB" =~ ^[0-9]+(\.[0-9]+)?$ ]]; then
  echo "build-appimage: некорректное значение --max-rss-mb: $MAX_RSS_MB" >&2
  exit 1
fi

DEFAULT_FILENAME="discord-gpui-v${VERSION}-x86_64.AppImage"
if [[ -z "$OUTPUT" ]]; then
  OUTPUT_FILE="$ROOT/rust/dist/$DEFAULT_FILENAME"
elif [[ -d "$OUTPUT" ]]; then
  OUTPUT_FILE="${OUTPUT%/}/$DEFAULT_FILENAME"
else
  OUTPUT_FILE="$OUTPUT"
fi

APPRUN_SRC="$ROOT/packaging/appimage/AppRun"
DESKTOP_SRC="$ROOT/packaging/appimage/discord-gpui.desktop"
ICON_SVG_SRC="$ROOT/packaging/appimage/discord-gpui.svg"
ICON_PNG_SRC="$ROOT/rust/gui-spike/icons/icon.png"

if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "build-appimage: dry-run (запись файлов и сборка не выполняются)"
  echo "+ входной бинарник: $BINARY"
  echo "+ версия пакета: $VERSION"
  echo "+ спецификация AppRun: $APPRUN_SRC"
  echo "+ спецификация Desktop: $DESKTOP_SRC"
  echo "+ выходной пакет AppImage: $OUTPUT_FILE"
  echo "+ проверка --headless-smoke (лимит VmRSS < ${MAX_RSS_MB} МБ)"
  exit 0
fi

# Проверка доступности входного бинарника и обязательных файлов спецификации
if [[ ! -f "$BINARY" ]]; then
  echo "build-appimage: входной бинарник не найден: $BINARY" >&2
  exit 2
fi
if [[ ! -f "$APPRUN_SRC" || ! -f "$DESKTOP_SRC" || ! -f "$ICON_SVG_SRC" ]]; then
  echo "build-appimage: отсутствуют файлы спецификации в packaging/appimage/" >&2
  exit 2
fi

for tool in python3 ldd readelf file sha256sum unsquashfs; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "build-appimage: обязательная системная утилита не найдена: $tool" >&2
    exit 2
  fi
done

# Проверка заголовка входного ELF64 x86_64 бинарника
if ! python3 - "$BINARY" <<'PY'
import struct, sys
path = sys.argv[1]
with open(path, "rb") as f:
    hdr = f.read(20)
if len(hdr) < 20 or hdr[:7] != b"\x7fELF\x02\x01\x01":
    sys.stderr.write(f"build-appimage: файл не является 64-битным LSB ELF: {path}\n")
    sys.exit(1)
e_type, e_machine = struct.unpack("<HH", hdr[16:20])
if e_type not in (2, 3) or e_machine != 62:
    sys.stderr.write(f"build-appimage: неподдерживаемый тип ELF (e_type={e_type}, e_machine={e_machine})\n")
    sys.exit(1)
PY
then
  exit 1
fi

# Verify the official tool and runtime before executing either of them.
if [[ ! -x "$APPIMAGETOOL" || ! -f "$RUNTIME_FILE" ]]; then
  echo "build-appimage: supply --appimagetool and --runtime-file from packaging/arm64/toolchain-pins.json" >&2
  exit 2
fi
python3 - "$ROOT/packaging/arm64/toolchain-pins.json" "$APPIMAGETOOL" "$RUNTIME_FILE" <<'PY'
import hashlib
import json
from pathlib import Path
import sys

pins = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
for name, pin in zip(sys.argv[2:], (pins["appimagetool"]["assets"]["x86_64"], pins["runtime_x86_64"])):
    path = Path(name)
    if path.stat().st_size != pin["size"] or hashlib.sha256(path.read_bytes()).hexdigest() != pin["sha256"]:
        sys.exit(f"build-appimage: official file SHA-256/size mismatch: {path}")
PY
APPIMAGETOOL="$(realpath -- "$APPIMAGETOOL")"
RUNTIME_FILE="$(realpath -- "$RUNTIME_FILE")"

# Валидация desktop-файла через desktop-file-validate (если установлен)
if command -v desktop-file-validate >/dev/null 2>&1; then
  desktop-file-validate "$DESKTOP_SRC"
fi

WORK_TMP="$(mktemp -d "${TMPDIR:-/tmp}/discord-gpui-appimage.XXXXXX")"
cleanup() {
  rm -rf "$WORK_TMP"
}
trap cleanup EXIT

APPDIR="$WORK_TMP/DiscordGpui.AppDir"
mkdir -p \
  "$APPDIR/usr/bin" \
  "$APPDIR/usr/lib" \
  "$APPDIR/usr/share/applications" \
  "$APPDIR/usr/share/icons/hicolor/128x128/apps" \
  "$APPDIR/usr/share/icons/hicolor/scalable/apps" \
  "$APPDIR/usr/share/doc/discord-gpui" \
  "$APPDIR/usr/share/metainfo"

# 1. Установка AppRun, бинарника, .desktop и иконок
install -m 0755 "$APPRUN_SRC" "$APPDIR/AppRun"
install -m 0755 "$BINARY" "$APPDIR/usr/bin/discord-gpui"
install -m 0644 "$DESKTOP_SRC" "$APPDIR/discord-gpui.desktop"
install -m 0644 "$DESKTOP_SRC" "$APPDIR/usr/share/applications/discord-gpui.desktop"
install -m 0644 "$ICON_SVG_SRC" "$APPDIR/usr/share/icons/hicolor/scalable/apps/discord-gpui.svg"

if [[ -f "$ICON_PNG_SRC" ]]; then
  install -m 0644 "$ICON_PNG_SRC" "$APPDIR/discord-gpui.png"
  install -m 0644 "$ICON_PNG_SRC" "$APPDIR/.DirIcon"
  install -m 0644 "$ICON_PNG_SRC" "$APPDIR/usr/share/icons/hicolor/128x128/apps/discord-gpui.png"
fi

# 2. Копирование лицензионных файлов и SBOM
for doc_file in LICENSE LICENSE-MIT LICENSE-APACHE README.md; do
  if [[ -f "$ROOT/$doc_file" ]]; then
    install -m 0644 "$ROOT/$doc_file" "$APPDIR/usr/share/doc/discord-gpui/$doc_file"
  fi
done
if [[ -f "$SBOM" ]]; then
  install -m 0644 "$SBOM" "$APPDIR/usr/share/doc/discord-gpui/sbom.spdx.json"
fi

# 3. Инспекция ldd и бандлинг разделяемых библиотек в AppDir/usr/lib/
python3 - "$BINARY" "$APPDIR/usr/lib" <<'PY'
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys

binary_path = sys.argv[1]
usr_lib_dir = sys.argv[2]

env = dict(os.environ)
env["LC_ALL"] = "C"
baseline_libraries = os.environ.get("CHORD_APPIMAGE_LIBRARY_DIR")
if baseline_libraries:
    if not os.path.isdir(baseline_libraries):
        sys.exit(f"build-appimage: library directory does not exist: {baseline_libraries}")
    env["LD_LIBRARY_PATH"] = os.path.abspath(baseline_libraries)

def require_glibc_baseline(path):
    versions = subprocess.check_output(["readelf", "--version-info", path], env=env, text=True)
    needs = versions.partition("Version needs section")[2]
    for major, minor in re.findall(r"Name: GLIBC_(\d+)\.(\d+)(?:\.\d+)?\s+Flags: none", needs):
        if (int(major), int(minor)) > (2, 35):
            sys.exit(f"build-appimage: {path} requires GLIBC_{major}.{minor}; use Ubuntu 22.04 libraries")

require_glibc_baseline(binary_path)
ldd_out = subprocess.check_output(["ldd", binary_path], env=env, text=True)

# Базовые библиотеки ABI хоста (glibc, загрузчик, базовый звуковой/X11 сокетный транспорт),
# которые по спецификации AppImage остаются системными:
HOST_BASE_LIBS = {
    "linux-vdso.so.1",
    "ld-linux-x86-64.so.2",
    "libc.so.6",
    "libm.so.6",
    "libdl.so.2",
    "librt.so.1",
    "libpthread.so.0",
    "libresolv.so.2",
    "libgcc_s.so.1",
    "libstdc++.so.6",
    "libasound.so.2",
    "libxcb.so.1",
}

PROPRIETARY_PATTERNS = [
    re.compile(r"^libcuda\.so", re.I),
    re.compile(r"^libnvidia-", re.I),
    re.compile(r"^libamdhip64\.so", re.I),
]

bundled = []
host_excluded = []

for raw_line in ldd_out.splitlines():
    line = raw_line.strip()
    if not line:
        continue
    if "=>" in line:
        raw_name, rhs = [part.strip() for part in line.split("=>", 1)]
        name = os.path.basename(raw_name)
        if rhs.lower().startswith("not found"):
            sys.stderr.write(f"build-appimage: не найдена разделяемая библиотека: {name}\n")
            sys.exit(1)
        resolved = re.sub(r"\s*\(0x[0-9a-fA-F]+\)\s*$", "", rhs).strip()
    else:
        token = re.sub(r"\s*\(0x[0-9a-fA-F]+\)\s*$", "", line).strip()
        name = os.path.basename(token)
        resolved = token if token.startswith("/") else ""

    for pat in PROPRIETARY_PATTERNS:
        if pat.search(name):
            sys.stderr.write(f"build-appimage: обнаружена недопустимая привязка к проприетарному драйверу: {name}\n")
            sys.exit(1)

    if name in HOST_BASE_LIBS or not resolved or not os.path.isfile(resolved):
        host_excluded.append({"name": name, "resolved_path": resolved or None})
        continue

    require_glibc_baseline(resolved)
    dest_path = os.path.join(usr_lib_dir, name)
    shutil.copy2(resolved, dest_path)
    os.chmod(dest_path, 0o755)
    with open(dest_path, "rb") as f:
        sha256_hex = hashlib.sha256(f.read()).hexdigest()
    bundled.append({
        "name": name,
        "source_path": os.path.realpath(resolved),
        "bundled_path": f"usr/lib/{name}",
        "size_bytes": os.path.getsize(dest_path),
        "sha256": sha256_hex,
    })

manifest = {
    "schema_version": "1.0",
    "binary": "usr/bin/discord-gpui",
    "bundled_libraries": bundled,
    "host_abi_libraries": host_excluded,
}
with open(os.path.join(usr_lib_dir, "bundled-deps.json"), "w", encoding="utf-8") as f:
    json.dump(manifest, f, indent=2, ensure_ascii=False)
PY

if [[ -n "$APPDIR_OUT" ]]; then
  rm -rf "$APPDIR_OUT"
  mkdir -p "$(dirname "$APPDIR_OUT")"
  cp -a "$APPDIR" "$APPDIR_OUT"
fi

# Build with the upstream Type 2 runtime and a real SquashFS filesystem.
CANDIDATE_APPIMAGE="$WORK_TMP/$(basename -- "$OUTPUT_FILE")"
UPDATE_INFO="gh-releases-zsync|discord-lab|discord-gpui|latest-pre|discord-gpui-*-x86_64.AppImage.zsync"
(
  cd "$WORK_TMP"
  APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 "$APPIMAGETOOL" \
    --runtime-file "$RUNTIME_FILE" --updateinformation "$UPDATE_INFO" "$APPDIR" "$CANDIDATE_APPIMAGE"
)
OFFSET="$(env -u APPIMAGE_EXTRACT_AND_RUN "$CANDIDATE_APPIMAGE" --appimage-offset)"
unsquashfs -s -o "$OFFSET" "$CANDIDATE_APPIMAGE" >/dev/null

# Run the candidate without FUSE. Publish the local file after validation.
SMOKE_STDOUT=""
if [[ "$SKIP_SMOKE" -eq 0 ]]; then
  SMOKE_STDOUT="$(APPIMAGE_EXTRACT_AND_RUN=1 "$CANDIDATE_APPIMAGE" --headless-smoke)"
  if ! grep -q "^GPUI_SMOKE_OK " <<<"$SMOKE_STDOUT"; then
    echo "build-appimage: в выводе --headless-smoke отсутствует маркер GPUI_SMOKE_OK" >&2
    exit 1
  fi
fi

# 8. Проверка лимита памяти VmRSS и запись опционального JSON-отчёта
python3 - "$CANDIDATE_APPIMAGE" "$APPDIR/usr/lib/bundled-deps.json" "$MAX_RSS_MB" "$SMOKE_STDOUT" "$REPORT_JSON" "$VERSION" "$OUTPUT_FILE" "$OFFSET" <<'PY'
import hashlib
import json
import os
import sys

appimage_path, deps_path, max_rss_str, smoke_stdout, report_json_path, version, output_path, offset = sys.argv[1:]
max_rss_mb = float(max_rss_str)

smoke_payload = None
if smoke_stdout.strip():
    for line in smoke_stdout.splitlines():
        if line.startswith("GPUI_SMOKE_OK "):
            smoke_payload = json.loads(line[len("GPUI_SMOKE_OK "):])
            break
    if smoke_payload is None:
        sys.stderr.write("build-appimage: не удалось разобрать JSON из GPUI_SMOKE_OK\n")
        sys.exit(1)
    vmrss_mb = float(smoke_payload.get("vmrss_mb", 0.0))
    if smoke_payload.get("status") != "ok" or not smoke_payload.get("sqlite_wal_ok") or not smoke_payload.get("fps_budget_ok"):
        sys.stderr.write(f"build-appimage: не пройдена внутренняя диагностика smoke: {smoke_payload}\n")
        sys.exit(1)
    if vmrss_mb <= 0.0 or vmrss_mb >= max_rss_mb:
        sys.stderr.write(
            f"build-appimage: превышен бюджет памяти VmRSS внутри AppImage: {vmrss_mb:.2f} МБ (лимит < {max_rss_mb:.2f} МБ)\n"
        )
        sys.exit(1)

with open(appimage_path, "rb") as f:
    data = f.read()
    sha256_hex = hashlib.sha256(data).hexdigest()

with open(deps_path, "r", encoding="utf-8") as f:
    deps_manifest = json.load(f)

report = {
    "task_id": "TASK-961",
    "status": "passed",
    "version": version,
    "appimage_path": os.path.abspath(output_path),
    "squashfs_offset": int(offset),
    "appimage_size_bytes": len(data),
    "appimage_sha256": sha256_hex,
    "elf_type2_magic_ok": data[:7] == b"\x7fELF\x02\x01\x01" and data[8:11] == b"AI\x02",
    "bundled_libraries": deps_manifest.get("bundled_libraries", []),
    "host_abi_libraries": deps_manifest.get("host_abi_libraries", []),
    "headless_smoke": smoke_payload,
    "vmrss_budget_mb": max_rss_mb,
}

if report_json_path:
    os.makedirs(os.path.dirname(os.path.abspath(report_json_path)), exist_ok=True)
    with open(report_json_path, "w", encoding="utf-8") as f:
        json.dump(report, f, indent=2, ensure_ascii=False)
PY

mkdir -p "$(dirname "$OUTPUT_FILE")"
install -m 0755 "$CANDIDATE_APPIMAGE" "$OUTPUT_FILE"
if [[ -f "$CANDIDATE_APPIMAGE.zsync" ]]; then
  install -m 0644 "$CANDIDATE_APPIMAGE.zsync" "$OUTPUT_FILE.zsync"
fi
(
  cd "$(dirname "$OUTPUT_FILE")"
  sha256sum "$(basename "$OUTPUT_FILE")" > "$(basename "$OUTPUT_FILE").sha256"
)

echo "build-appimage: успешно собран и верифицирован пакет $OUTPUT_FILE"
