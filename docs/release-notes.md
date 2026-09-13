# Beta 6

- Prepare and schedule Apple photo backups by capture time, newest first.
- Automatically reorder existing pending photos using metadata, without downloading originals again.
- Correct incremental discovery and batch export ordering on both iPhone and Mac.
- Preserve active transfers, completed receipts, pairings and retry deadlines. Concurrent transfers may still finish in a different order.

This release fixes backup ordering. iOS background scheduling is unchanged.

Mac requires Apple Silicon and macOS 14+. Android requires arm64 and Android 10+.
iOS 17+ is available from source. The Mac beta is not Apple-notarized.
Receiver delivery and Google Photos cloud backup remain separate states.
