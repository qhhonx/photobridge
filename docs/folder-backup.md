# Folder backup on Mac

Folder sources are a desktop capability. The iOS app continues to use PhotoKit;
there is no folder picker, watcher or folder-source Rust dependency in its build.

## Use

1. Open **Backup sources**, then **Add folder**. Select a folder on the Mac,
   an external drive or an SD card. Subfolders are included.
2. Choose whether to include existing files and whether to automatically back up
   new and changed files. With existing files excluded, the first completed scan
   establishes a baseline; subsequent additions and changes are eligible.
3. The source is scanned in batches. Pair a receiver if necessary. Global pause
   still applies; **Back up existing files** starts/resumes preparation and also
   allows an explicit backup to a newly paired receiver.
4. Open the folder source to browse indexed files and receiver receipts. The
   transfer list is shared with the photo library, with source labels and a source
   filter. **Received** means the paired receiver verified the resources, not
   that Google Photos completed its cloud backup.

**Pause new tasks** stops further preparation from a source, but does not cancel
already prepared transfers. Use global pause to suspend all transfers. Removing
an origin stops scanning and keeps prepared tasks, receiver data and originals.

## Automatic checks and external drives

FSEvents provides debounced file-change hints on its own dispatch queue. Affected
parent directories are rescanned recursively; overlapping hints are coalesced.
Dropped events and root changes cause a full check. App startup, drive mount and
wake also cause a full check. A periodic full scan repairs missed notifications;
if the native stream cannot start, periodic checking runs more frequently.

This works while the application is running, including with its window closed.
Quitting the application stops work; relaunch catches up. Offline or inaccessible
sources retain their inventory and receipts. They are not interpreted as deleted
remote files. Read-only security-scoped bookmarks restore access across launches.

## Media and dates

The initial format allowlist is JPEG, HEIC/HEIF, PNG, GIF, WebP, TIFF, MOV, MP4 and
M4V. The receiver can impose further format limits. Non-media files, hidden items,
symbolic links and nested Unix mount points are not traversed. RAW files and
arbitrary document backup are not supported.

An exported Apple Live Photo can be grouped when its photo and MOV have the same
basename and matching Apple pairing identifiers. Same-name files alone are never
proof of a motion pair. Motion resources use the existing atomic asset protocol
and receiver packaging. Folder exports without recoverable burst relationships
are transferred as individual files; their original bytes remain unchanged.

ImageIO and AVFoundation read embedded capture dates on Mac. EXIF UTC offsets are
applied when available; timezone-free EXIF dates use the Mac's local timezone.
Missing capture dates fall back to the file modification time, with `date_source`
recorded in task metadata. Original metadata is never edited by the source adapter.
Folder hierarchy is not mapped to Google Photos albums.

## Storage and identity

`photobridge-folder-source` owns a SQLite inventory, bounded directory work queue,
stability window, source identity and per-receiver submission records. Scans do
not hash or copy the full library. Only current eligible assets are copied into
owned staging, checked against their indexed revision and identity, then hashed
and passed to the existing Rust queue. Large assets wait when staging allowance is
insufficient; temporary deferral lets other assets proceed. Increase the shared
cache budget if a single asset is larger than it. Failed media inspection can be
reviewed in the source page and retried with **Back up existing files**.

On Unix, file identity uses the inode within the registered source/volume;
renames within the source do not alone require retransmission. Content changes
produce new revisions. Hard links to the same file share this identity. Across
sources, semantic/photo similarity deduplication is not promised.

Sources never become cache-owned paths. Existing cache reclamation removes only
app-owned verified staging. Folder deletion never propagates to the receiver.
Existing PhotoKit source IDs, revisions and receipts are unchanged.

## Build boundary and validation

Mac enables `photobridge-native/folder-source`; it is disabled by default. iOS and
Android builds explicitly disable default features, and enabling this feature for
mobile targets is a compile error. `scripts/check-folder-isolation.py` verifies
mobile dependency trees. Platform-independent transfer and queue crates do not
depend on the folder scanner.

Validation includes bounded/incremental/subtree scans, restart, rename, offline
sources, baseline-only import, permanent/temporary skips, safe path containment,
real TLS motion-resource transfer, repeat submission, staging reclamation, native
EXIF timezone handling, real FSEvents and mobile build isolation. Physical external
hardware and a 12 TB dataset still require user testing; synthetic tests do not
establish their performance or Google Photos cloud completion.
