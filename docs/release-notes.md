# Beta 11

- Pixel now packages supported Apple Live Photos using their original HEIC or JPEG still and MOV or MP4 video, without re-encoding either media stream. JPEG + MOV deliveries include a motion-video index for Google Photos compatibility.
- If a still image cannot be packaged directly, Pixel converts only that still; a compatible MOV or MP4 stays unchanged. Older unfinished JPEG deliveries continue with their original format so they can resume safely.
- New HEIC motion deliveries use a `.heic` gallery file. Existing completed gallery copies are not reprocessed or renamed.

Existing pairings, pending sources and receipts are preserved. Google Photos cloud backup remains separate from receipt on Pixel.

Mac requires Apple Silicon and macOS 14+. Android requires arm64 and Android 10+.
iOS 17+ is available from source. The Mac beta is not Apple-notarized.
Receiver delivery and Google Photos cloud backup remain separate states.
