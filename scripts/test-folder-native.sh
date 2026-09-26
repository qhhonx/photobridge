#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mkdir -p build/folder-tests
xcrun --sdk macosx swiftc -swift-version 5 -target "$(uname -m)-apple-macos14.0" -module-cache-path build/SwiftModuleCache-folder -parse-as-library \
  apps/macos/PhotoBridge/FolderMedia.swift apps/macos/PhotoBridge/FolderWatch.swift tests/folder-native.swift \
  -framework AVFoundation -framework ImageIO -framework CoreServices -framework AppKit \
  -o build/folder-tests/native
build/folder-tests/native
