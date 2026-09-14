#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mkdir -p build/dependencies
archive=build/dependencies/Sparkle-2.9.6.tar.xz
checksum=52bf9e88cdd972fc0c81501377a880e90d47031bd8ca5462488f843e2609e192
if [ ! -f "$archive" ]; then
  # Proxies can report a gateway timeout as curl error 56 rather than HTTP 22.
  # Retry this read-only download too; only a verified archive is unpacked below.
  if ! curl --fail --location --retry 3 --retry-all-errors --retry-max-time 180 --connect-timeout 20 --max-time 180 https://github.com/sparkle-project/Sparkle/releases/download/2.9.6/Sparkle-2.9.6.tar.xz -o "$archive.tmp"; then
    # The official asset API is independent of the release-page redirect route.
    curl --fail --location --retry 3 --retry-all-errors --retry-max-time 180 --connect-timeout 20 --max-time 180 \
      --header 'Accept: application/octet-stream' \
      https://api.github.com/repos/sparkle-project/Sparkle/releases/assets/517328209 -o "$archive.tmp"
  fi
  printf '%s  %s\n' "$checksum" "$archive.tmp" | shasum -a 256 -c -
  mv "$archive.tmp" "$archive"
fi
printf '%s  %s\n' "$checksum" "$archive" | shasum -a 256 -c -
# Always unpack from the verified archive, including CI signing tools.
rm -rf build/dependencies/sparkle
mkdir -p build/dependencies/sparkle
tar -xJf "$archive" -C build/dependencies/sparkle
