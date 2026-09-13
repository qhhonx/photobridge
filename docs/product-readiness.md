# Beta scope and configuration

## Sender

- Editable device name; QR pairing with receiver identity and pinned TLS trust.
- Native photo library, photo/video/motion/burst filters and transfer status.
- Explicit selection, new-photo discovery and separate existing-library import.
- Persistent pause, automatic transient retry and background task reconciliation.
- Configurable staging-cache budget, minimum free space and log retention.
- Wi-Fi policy on iOS; background scheduling remains controlled by the system.
- macOS update checks can be automatic or manual. Installation is user-approved.

## Receiver

- Editable device name and paired-sender list with device type and last observed IP.
- Per-sender reception enable/disable and history filters. This is cooperative
  reception control, not cryptographic revocation of shared pairing credentials.
- Persistent reception controls, storage budgets, low-space reporting and logs.
- Optional private-original reclamation after verified system-gallery publication.
- Experimental Google Photos space management, requiring accessibility permission
  and the app's own safe-backup confirmation. It never blindly deletes files.
- System/light/dark appearance and automatic/manual update checks.
- Signed APK updates require Android's install confirmation.

## Distribution

Mac: Apple Silicon, macOS 14+, ad hoc signing and signed Sparkle updates; not
Apple-notarized. Android: arm64, Android 10+, dedicated release signing identity.
iOS: iOS 17+, source builds using the user's own development team. No public
TestFlight/App Store release or direct in-app updater is offered.

Windows senders, NAS adapters and online receivers are future work. The protocol
and Rust core are portable; this does not imply those products are already shipped.
