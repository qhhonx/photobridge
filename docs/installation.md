# Install PhotoBridge

## macOS sender

Download the Apple Silicon ZIP from the official website or GitHub Releases.
Extract it and move PhotoBridge to Applications before opening it.
The free beta uses an ad hoc integrity signature and signed Sparkle updates, but
is not notarized by Apple. If macOS blocks opening it, review the source and release,
then use System Settings → Privacy & Security → Open Anyway.
Do not disable Gatekeeper globally. Grant Photos and local network access.

Use PhotoBridge → Check for Updates, or enable automatic checks in Settings.
Updates pause and persist transfer work before replacing the application.

## Android receiver

Download the arm64 APK on Android 10 or later. Allow your browser to install it
when Android asks. Start reception and pair your sender. If you use Google Photos,
enable backup for the PhotoBridge device folder separately.
Settings → App updates checks new releases. The app verifies the downloaded file,
package ID, increasing build number and signing certificate before opening the
system installer. Installation requires your confirmation. Automatic checks run
at most once a day while opening the receiver, and can be disabled.

Development APKs use a different signing key from public releases. Android cannot
update across these identities. Keep your existing development installation until
you have safely exported or preserved its originals and are ready to switch; the
public installer does not erase or migrate that data automatically.

## iOS sender

No App Store or TestFlight distribution is available yet. Build from source using
Xcode, XcodeGen, Rust and the `aarch64-apple-ios` target:

```sh
xcodegen generate --spec apps/ios/project.yml
open apps/ios/PhotoBridge.xcodeproj
```

Select your own signing team in Xcode and run on your iPhone or iPad (iOS 17+).
Do not commit team IDs, provisioning profiles or signing credentials. Personal
Team installations require periodic renewal. There is no in-app updater for iOS.

Scan the receiver code on iPhone. For Mac, scan the computer's code using the
Android receiver. Photos/local network permissions are needed. New-photo backup
and existing-photo import are separate choices. iOS schedules background work;
continuous execution while locked is not guaranteed. Avoid force-quitting the app.
