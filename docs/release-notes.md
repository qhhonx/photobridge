# Beta 10

- Newly published Pixel gallery files use a capture-date name such as `PB_20260815_024141Z_a1b2.jpg`. The time is UTC; the four-character suffix grows only if another item already has that name. Original source filenames remain in PhotoBridge's transfer records.
- Existing completed gallery copies keep their names. An unfinished copy created by an older build resumes in place instead of creating a duplicate.
- The receiver keeps Wi-Fi active while listening with its screen off. Apple senders wait for the paired receiver to be reachable before submitting more background requests, and can rediscover its local address during a background wake.

Existing pairings, pending sources and receipts are preserved. Google Photos cloud backup remains separate from receipt on Pixel; previously uploaded cloud filenames are not changed by this update.

Mac requires Apple Silicon and macOS 14+. Android requires arm64 and Android 10+.
iOS 17+ is available from source. The Mac beta is not Apple-notarized.
Receiver delivery and Google Photos cloud backup remain separate states.
