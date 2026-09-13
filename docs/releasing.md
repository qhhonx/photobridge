# Release operations

## Version and publication

`release.json` is the release source of truth. Increase `version` and monotonically
increase `build`, update release notes, then push main. The publication workflow
validates and builds Mac and Android, signs artifacts, assembles checksums and
publishes a complete GitHub release. Existing published versions are immutable;
pushing unrelated changes does not replace them. Never reuse or lower a build.

The Vercel project uses the `site` directory and deploys every main push. Its feed
endpoint returns the exact signed appcast bytes from the newest complete release.
A draft or release missing either platform archive/manifests is not offered.

The production project is `photobridge-app`, connected to this public repository
with `main` as its production branch and `site` as its root directory. Website
deployment does not require application signing secrets. A documentation-only
push still deploys the site; application publication skips an unchanged version.

## Signing

Repository Actions secrets:

- `SPARKLE_PRIVATE_KEY`: base64 Ed25519 seed for signed Mac updates and feeds.
- `ANDROID_KEYSTORE_BASE64`: dedicated Android release keystore, base64 encoded.
- `ANDROID_KEYSTORE_PASSWORD`: password for that keystore, alias `photobridge`.

Never commit these values. Back them up offline: losing the Android key prevents
updates to existing installations; losing the Sparkle key also breaks updates for
ad hoc signed Mac installations. Public trust material is in app configuration.
The Mac package has an ad hoc integrity signature and is not Apple-notarized.

For local release builds, set `PHOTOBRIDGE_RELEASE=1`; local development signing
identities must not enter public packages. See scripts/package-release.sh and
scripts/verify-release.py for verification. iOS uses each developer's own team;
no personal iOS signing material is provided by CI.

## Acceptance

Inspect the ordinary production `/appcast.xml`, not only a cache-busted endpoint.
Verify its exact bytes/signature and release build, download both published files,
compare SHA256SUMS, and exercise native update checks. Keep release and website
failures visible; do not mark deployment complete from a queued action alone.
