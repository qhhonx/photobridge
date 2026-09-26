#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
: "${ANDROID_HOME:=$HOME/Library/Android/sdk}"
: "${ANDROID_NDK_HOME:=$ANDROID_HOME/ndk/27.1.12297006}"
case "$(uname -s)" in Darwin) host=darwin-x86_64;; Linux) host=linux-x86_64;; *) echo 'Use macOS or Linux to build Android.' >&2; exit 1;; esac
compiler="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$host/bin/aarch64-linux-android29-clang"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$compiler"
export CC_aarch64_linux_android="$compiler"
export AR_aarch64_linux_android="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$host/bin/llvm-ar"
export CARGO_ENCODED_RUSTFLAGS=$(printf '%s\037%s' "--remap-path-prefix=$PWD=/photobridge" "--remap-path-prefix=$HOME=/builder")
cargo build --locked --release -p photobridge-native --no-default-features --target aarch64-linux-android
mkdir -p apps/android/app/src/main/jniLibs/arm64-v8a
cp target/aarch64-linux-android/release/libphotobridge_native.so apps/android/app/src/main/jniLibs/arm64-v8a/
cd apps/android
export ANDROID_HOME
./gradlew "${ANDROID_BUILD_TASK:-assembleDebug}"
