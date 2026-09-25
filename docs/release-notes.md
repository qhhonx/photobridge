# Beta 12

- The Apple photo library now distinguishes a receipt for the current photo version from a receipt for an earlier version of the same photo. A later PhotoKit metadata change no longer makes a previously received photo look as if it was never transferred.
- Active, failed, and pending work for the current version still takes precedence. Preparing and deduplication continue to match the exact source revision; this display fix does not silently skip edited photos or transfer them again.
- Burst status still requires a receipt for every member before showing a completed mark. The hint is scoped to the paired receiver and does not imply Google Photos cloud backup.

Existing pairings, pending sources and receipts are preserved.

Mac requires Apple Silicon and macOS 14+. Android requires arm64 and Android 10+.
iOS 17+ is available from source. The Mac beta is not Apple-notarized.
Receiver delivery and Google Photos cloud backup remain separate states.
