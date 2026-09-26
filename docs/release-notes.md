# Beta 13

- Mac can now back up photos and videos directly from folders and external drives, without importing them into Apple Photos.
- Add folders in Backup Sources, choose whether to include existing files, and enable automatic checks for new or changed files. Checks resume after startup, wake, and drive reconnection.
- Exported Live Photos are grouped when their photo and video share a matching Apple content identifier. Original source files are preserved.
- Transfer lists can be filtered by source. Folder scanning and preparation reuse the existing encrypted transfer queue, receipts, retry handling, and managed cache cleanup.
- Folder support is isolated to Mac. iOS and Android retain their existing source and receiver behavior.

Existing pairings, pending sources and receipts are preserved. Folder scans are incremental; performance with multi-terabyte libraries has not yet been verified on a real drive.

Mac requires Apple Silicon and macOS 14+. Android requires arm64 and Android 10+.
iOS 17+ is available from source. The Mac beta is not Apple-notarized.
Receiver delivery and Google Photos cloud backup remain separate states.
