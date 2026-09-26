# Beta 14

- Mac navigation now separates photo and folder sources from the backup overview and transfer tasks. Folder details keep the sidebar and a clear return button.
- Folder cards show localized system-folder names and their actual paths. Common actions are visible, More contains source management, and the automatic-backup switch has its own row.
- Manual checks and backup starts show immediate feedback and report errors. Pausing a source explains that existing queued transfers continue.
- Photo and video thumbnails are available in folder-file and transfer lists. Settings lets you disable list thumbnails to reduce disk reads and decoding. File previews load only for visible rows, with bounded requests and memory cache.
- Folder-file lists can switch between Flat and Folders. The choice is remembered; folders load on expansion, with separate pages per directory. Only indexed photos and videos are shown.
- Folder transfer previews support existing tasks and indexed file moves, and safely fall back to icons when originals are missing, changed, or inaccessible.

Existing pairings, source settings and transfer receipts are preserved. Large external-drive libraries still need real-world performance testing.

Mac requires Apple Silicon and macOS 14+. Android requires arm64 and Android 10+.
iOS 17+ is available from source. The Mac beta is not Apple-notarized.
Receiver delivery and Google Photos cloud backup remain separate states.
