#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
platform="${1:-simulator}"
case "$platform" in
 simulator) target=aarch64-apple-ios-sim; sdk=iphonesimulator; swift_target=arm64-apple-ios17.0-simulator;;
 device) target=aarch64-apple-ios; sdk=iphoneos; swift_target=arm64-apple-ios17.0;;
 *) echo 'Usage: build-ios.sh [simulator|device]' >&2; exit 1;;
esac
export IPHONEOS_DEPLOYMENT_TARGET=17.0
cargo build --locked --release -p photobridge-native --target "$target"
app="build/ios/$platform/PhotoBridge.app"
mkdir -p "$app"
cp apps/ios/PhotoBridge/Info.plist "$app/Info.plist"
cp -R apps/apple/Resources/*.lproj "$app/"
set --
if [ "$platform" = simulator ]; then
    set -- -Xlinker -sectcreate -Xlinker __TEXT -Xlinker __entitlements -Xlinker apps/ios/Simulator.entitlements
fi
xcrun --sdk "$sdk" swiftc -swift-version 5 -O -module-cache-path "build/SwiftModuleCache-ios/$platform" \
 -sdk "$(xcrun --sdk "$sdk" --show-sdk-path)" -target "$swift_target" -parse-as-library \
 "$@" -import-objc-header crates/native/include/photobridge.h \
 apps/ios/PhotoBridge/*.swift apps/apple/Shared/*.swift "target/$target/release/libphotobridge_native.a" \
 -framework Security -framework SystemConfiguration -framework UIKit -framework SwiftUI -framework Photos -framework PhotosUI -framework AVFoundation -lsqlite3 -lz -liconv \
 -o "$app/PhotoBridge"
if [ "$platform" = simulator ]; then codesign --force --sign - "$app"; fi
printf 'Built %s\n' "$app"
