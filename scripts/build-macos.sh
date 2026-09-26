#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
export MACOSX_DEPLOYMENT_TARGET=14.0
./scripts/fetch-sparkle.sh
export CARGO_ENCODED_RUSTFLAGS=$(printf '%s\037%s' "--remap-path-prefix=$PWD=/photobridge" "--remap-path-prefix=$HOME=/builder")
cargo build --locked --release --target aarch64-apple-darwin -p photobridge-native --features folder-source
app=build/macos/PhotoBridge.app
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$app/Contents/Frameworks"
cp apps/macos/PhotoBridge/Info.plist "$app/Contents/Info.plist"
python3 scripts/stamp-version.py "$app/Contents/Info.plist"
ditto build/dependencies/sparkle/Sparkle.framework "$app/Contents/Frameworks/Sparkle.framework"
cp assets/PhotoBridge.icns "$app/Contents/Resources/"
cp LICENSE "$app/Contents/Resources/"
cp build/dependencies/sparkle/LICENSE "$app/Contents/Resources/Sparkle-LICENSE.txt"
cp -R apps/apple/Resources/*.lproj "$app/Contents/Resources/"
xcrun --sdk macosx swiftc -swift-version 5 -O -module-cache-path build/SwiftModuleCache-mac \
 -sdk "$(xcrun --sdk macosx --show-sdk-path)" -target arm64-apple-macos14.0 -parse-as-library \
 -debug-prefix-map "$PWD=/photobridge" -F build/dependencies/sparkle -framework Sparkle -Xlinker -rpath -Xlinker @executable_path/../Frameworks \
 -import-objc-header crates/native/include/photobridge.h \
 apps/apple/Shared/*.swift apps/macos/PhotoBridge/*.swift target/aarch64-apple-darwin/release/libphotobridge_native.a \
 -framework Security -framework SystemConfiguration -framework AVFoundation -framework ImageIO -framework CoreServices -framework AppKit -framework SwiftUI -framework Photos -framework Vision -lsqlite3 -lz -liconv \
 -o "$app/Contents/MacOS/PhotoBridge"
# Keep a stable local development identity across builds. This optional file is
# ignored by Git; release/CI builds may supply the environment variable instead.
signing_identity="${PHOTOBRIDGE_MAC_SIGN_IDENTITY:-}"
if [ "${PHOTOBRIDGE_RELEASE:-0}" != 1 ] && [ -z "$signing_identity" ] && [ -f build/macos-signing-identity ]; then
  IFS= read -r signing_identity < build/macos-signing-identity || true
fi
if [ "${PHOTOBRIDGE_RELEASE:-0}" = 1 ]; then signing_identity=-; fi
codesign --force --sign - "$app/Contents/Frameworks/Sparkle.framework"
xcrun strip -S "$app/Contents/MacOS/PhotoBridge"
codesign --force --sign "${signing_identity:--}" --identifier app.photobridge.mac "$app"
codesign --verify --deep --strict "$app"
printf 'Built %s\n' "$app"
