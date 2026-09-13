# Validation

The Rust workspace has automated protocol, durable-state, retry, cancellation,
capacity and receiver-processing tests. Apple native tests cover PhotoKit export,
video transport, thumbnail state, source browsing and background scheduling.
Android instrumentation covers reception, publication, motion conversion, burst
metadata, retention and accessibility-based cleanup with fixtures.

## Device observations

Mixed photos, a short video and supported Live Photos have been transferred to a
Pixel and played in Google Photos. A later overnight iOS run recorded 80 distinct
background uploads and matching complete receiver assets: 61 photos and 19 motion
assets. That sample contained no standalone videos. It does not prove background
video throughput, uninterrupted lock-screen execution, or every media variant.

The background preparation loop now continues in bounded batches until work is
drained, paused, waiting for resources, or expired by iOS. Simulator tests cover
multi-batch preparation, pause and expiration; real-world throughput depends on
iOS scheduling and device conditions.

## Release verification

Release builds must pass source privacy checks, package validation, native builds,
Rust tests and website release-selection tests. Signed Mac archives and signed
feeds are verified before publication. Android APK signatures, package/version
metadata and checksums are checked before publication and again by the updater.
A published artifact is not a substitute for observing an actual in-app update.

## Remaining limits

- Do not treat receipt on the receiver as confirmation of cloud backup.
- Large histories, uncommon codecs and burst display differ between platform versions.
- Google Photos space management is experimental and stops on unrecognized UI.
- Continuous iOS background execution is not guaranteed.
- Keep originals and an independent backup while evaluating the beta.

## Update checks

An isolated macOS fixture completed a real Sparkle update from build 1 to build 2;
a modified signed feed was rejected. The normal production feed was also checked
byte-for-byte against its signed release asset.

On Android 10, an isolated APK and real PackageManager accepted a higher build
with the same signing certificate and rejected wrong digest, size, version and
foreign-signature inputs. This exposed and fixed legacy certificate lookup when
`signingInfo` is absent. No production receiver data was modified by those tests.
The system installer still requires the user's installation confirmation.
