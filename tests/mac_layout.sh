#!/bin/bash
# macOS-only visual checks; renders fixture state, not the user's photo library.
set -euo pipefail
cd "$(dirname "$0")/.."
work=build/mac-layout-check
app="$work/LayoutCheck.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
python3 - <<'PY'
from pathlib import Path
import plistlib
source = Path('apps/macos/PhotoBridge/PhotoBridgeMacApp.swift').read_text()
assert source.count('@main struct PhotoBridgeMacApp') == 1
# Replace only the process entry point in a generated copy. Production views
# remain unchanged, including native controls and their asynchronous queries.
Path('build/mac-layout-check/PhotoBridgeMacApp.swift').write_text(
    source.replace('@main struct PhotoBridgeMacApp', 'struct PhotoBridgeMacApp', 1))
Path('build/mac-layout-check/LayoutCheck.app/Contents/Info.plist').write_bytes(plistlib.dumps({
    'CFBundleIdentifier': 'app.photobridge.layout-tests',
    'CFBundleExecutable': 'LayoutCheck', 'CFBundleName': 'PhotoBridge Layout Check',
    'CFBundlePackageType': 'APPL', 'CFBundleDevelopmentRegion': 'en',
    'CFBundleLocalizations': ['en', 'zh-Hans'], 'CFBundleShortVersionString': '0.1.0',
    'LSUIElement': True,
}))
PY
MACOSX_DEPLOYMENT_TARGET=14.0 cargo build --locked --release --target aarch64-apple-darwin -p photobridge-native -j 2
cp -R apps/apple/Resources/*.lproj "$app/Contents/Resources/"
sources=(apps/apple/Shared/*.swift)
for file in apps/macos/PhotoBridge/*.swift; do
  if [[ "$file" != */PhotoBridgeMacApp.swift ]]; then sources+=("$file"); fi
done
xcrun --sdk macosx swiftc -swift-version 5 -O -module-cache-path build/SwiftModuleCache-mac \
  -sdk "$(xcrun --sdk macosx --show-sdk-path)" -target arm64-apple-macos14.0 -parse-as-library \
  -import-objc-header crates/native/include/photobridge.h \
  "${sources[@]}" "$work/PhotoBridgeMacApp.swift" tests/render_macos.swift \
  target/aarch64-apple-darwin/release/libphotobridge_native.a \
  -framework Security -framework SystemConfiguration -framework AppKit -framework SwiftUI \
  -framework Photos -framework Vision -lsqlite3 -lz -liconv -o "$app/Contents/MacOS/LayoutCheck"
codesign --force --sign - "$app"
for language in en zh-Hans; do
  "$app/Contents/MacOS/LayoutCheck" "$work/screenshots/$language" -AppleLanguages "($language)" "$@"
done
