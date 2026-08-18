#!/usr/bin/env bash
set -euo pipefail

ROOT="${PUNCTUAL_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
VERSION="${PUNCTUAL_VERSION:-0.1.0-alpha.5}"
DIST="${PUNCTUAL_DIST_DIR:-${ROOT}/dist}"
HOST_ARCH="$(uname -m)"

case "${HOST_ARCH}" in
  arm64)
    PACKAGE_ARCH="arm64"
    MACHO_ARCH="arm64"
    CHROME_PLATFORM="mac-arm64"
    GECKO_PLATFORM="macos-aarch64"
    ;;
  x86_64)
    PACKAGE_ARCH="x64"
    MACHO_ARCH="x86_64"
    CHROME_PLATFORM="mac-x64"
    GECKO_PLATFORM="macos"
    ;;
  *)
    echo "Unsupported macOS architecture: ${HOST_ARCH}" >&2
    exit 1
    ;;
esac

RUNTIME="${DIST}/runtime-macos-${PACKAGE_ARCH}"
APP="${DIST}/Punctual.app"
DMG_ROOT="${DIST}/dmg-root-${PACKAGE_ARCH}"
DMG="${DIST}/Punctual-${VERSION}-macos-${PACKAGE_ARCH}.dmg"
SIGN_IDENTITY="${PUNCTUAL_CODESIGN_IDENTITY:--}"

require() {
  command -v "$1" >/dev/null 2>&1 || { echo "Missing command: $1" >&2; exit 1; }
}

require cargo
require curl
require python3
require ditto
require hdiutil
require codesign
require lipo

[[ "$(uname -s)" == "Darwin" ]] || { echo "This script must run on macOS" >&2; exit 1; }

rm -rf "${RUNTIME}" "${APP}" "${DMG_ROOT}" "${DMG}" "${DMG}.sha256"
mkdir -p "${RUNTIME}/managed-browser" "${RUNTIME}/bin" "${DIST}"

cd "${ROOT}"
cargo build --release --locked -p punctual-app
[[ -x target/release/punctual-app ]]
[[ "$(lipo -archs target/release/punctual-app)" == "${MACHO_ARCH}" ]]

curl --fail --location --retry 3 \
  https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json \
  -o "${RUNTIME}/chrome-for-testing.json"

python3 - "${RUNTIME}" "${CHROME_PLATFORM}" <<'PY'
import json
import sys
from pathlib import Path
runtime = Path(sys.argv[1])
platform = sys.argv[2]
data = json.loads((runtime / "chrome-for-testing.json").read_text())
stable = data["channels"]["Stable"]
asset = next(item for item in stable["downloads"]["chrome"] if item["platform"] == platform)
(runtime / "chrome-url.txt").write_text(asset["url"])
(runtime / "chrome-version.txt").write_text(stable["version"])
PY

curl --fail --location --retry 3 "$(cat "${RUNTIME}/chrome-url.txt")" -o "${RUNTIME}/chrome.zip"
ditto -x -k "${RUNTIME}/chrome.zip" "${RUNTIME}/chrome-extracted"
CHROME_APP="$(find "${RUNTIME}/chrome-extracted" -type d -name 'Google Chrome for Testing.app' -print -quit)"
[[ -n "${CHROME_APP}" ]]
ditto "${CHROME_APP}" "${RUNTIME}/managed-browser/Google Chrome for Testing.app"

GECKO_VERSION="${PUNCTUAL_GECKODRIVER_VERSION:-0.37.1}"
GECKO_ARCHIVE="geckodriver-v${GECKO_VERSION}-${GECKO_PLATFORM}.tar.gz"
curl --fail --location --retry 3 \
  "https://github.com/mozilla/geckodriver/releases/download/v${GECKO_VERSION}/${GECKO_ARCHIVE}" \
  -o "${RUNTIME}/${GECKO_ARCHIVE}"
curl --fail --location --retry 3 \
  "https://api.github.com/repos/mozilla/geckodriver/releases/tags/v${GECKO_VERSION}" \
  -o "${RUNTIME}/geckodriver-release.json"
python3 - "${RUNTIME}" "${GECKO_ARCHIVE}" <<'PY'
import hashlib
import json
import sys
from pathlib import Path
runtime = Path(sys.argv[1])
name = sys.argv[2]
release = json.loads((runtime / "geckodriver-release.json").read_text())
asset = next(item for item in release["assets"] if item["name"] == name)
digest = asset.get("digest") or ""
if digest.startswith("sha256:"):
    expected = digest.removeprefix("sha256:")
    actual = hashlib.sha256((runtime / name).read_bytes()).hexdigest()
    if actual != expected:
        raise SystemExit("geckodriver SHA-256 mismatch")
PY

tar -xzf "${RUNTIME}/${GECKO_ARCHIVE}" -C "${RUNTIME}/bin"
chmod 755 "${RUNTIME}/bin/geckodriver"

CHROME_BIN="${RUNTIME}/managed-browser/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"
[[ -x "${CHROME_BIN}" ]]
[[ "$(lipo -archs "${CHROME_BIN}")" == *"${MACHO_ARCH}"* ]]
[[ "$(lipo -archs "${RUNTIME}/bin/geckodriver")" == *"${MACHO_ARCH}"* ]]

CONTENTS="${APP}/Contents"
MACOS="${CONTENTS}/MacOS"
RESOURCES="${CONTENTS}/Resources"
mkdir -p "${MACOS}" "${RESOURCES}/managed-browser" "${RESOURCES}/bin"
cp target/release/punctual-app "${MACOS}/Punctual"
chmod 755 "${MACOS}/Punctual"
ditto "${RUNTIME}/managed-browser/Google Chrome for Testing.app" \
  "${RESOURCES}/managed-browser/Google Chrome for Testing.app"
cp "${RUNTIME}/bin/geckodriver" "${RESOURCES}/bin/geckodriver"
chmod 755 "${RESOURCES}/bin/geckodriver"
cp "${RUNTIME}/chrome-version.txt" "${RESOURCES}/managed-browser/version.txt"

cat > "${CONTENTS}/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleDevelopmentRegion</key><string>zh_CN</string>
<key>CFBundleDisplayName</key><string>Punctual</string>
<key>CFBundleExecutable</key><string>Punctual</string>
<key>CFBundleIdentifier</key><string>com.punctual.desktop</string>
<key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
<key>CFBundleName</key><string>Punctual</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>${VERSION}</string>
<key>CFBundleVersion</key><string>5</string>
<key>LSArchitecturePriority</key><array><string>${MACHO_ARCH}</string></array>
<key>LSMinimumSystemVersion</key><string>12.0</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSPrincipalClass</key><string>NSApplication</string>
</dict></plist>
PLIST

plutil -lint "${CONTENTS}/Info.plist"
codesign --force --deep --sign "${SIGN_IDENTITY}" --timestamp=none \
  "${RESOURCES}/managed-browser/Google Chrome for Testing.app"
codesign --force --sign "${SIGN_IDENTITY}" --timestamp=none "${RESOURCES}/bin/geckodriver"
codesign --force --deep --sign "${SIGN_IDENTITY}" --timestamp=none "${APP}"
codesign --verify --deep --strict --verbose=2 "${APP}"

mkdir -p "${DMG_ROOT}"
ditto "${APP}" "${DMG_ROOT}/Punctual.app"
ln -s /Applications "${DMG_ROOT}/Applications"
hdiutil create -volname "Punctual ${VERSION}" -srcfolder "${DMG_ROOT}" -ov -format UDZO "${DMG}"
hdiutil verify "${DMG}"
(cd "${DIST}" && shasum -a 256 "$(basename "${DMG}")" | tee "$(basename "${DMG}").sha256")
printf 'Created %s\n' "${DMG}"
