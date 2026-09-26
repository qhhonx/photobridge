# Folder backup on Mac

Folder sources are a desktop capability. The iOS app continues to use PhotoKit;
there is no folder picker, watcher or folder-source Rust dependency in its build.

## Use

1. Open **Folders**, then **Add folder**. Select a folder on the Mac,
   an external drive or an SD card. Subfolders are included.
2. Choose whether to include existing files and whether to automatically back up
   new and changed files. With existing files excluded, the first completed scan
   establishes a baseline; subsequent additions and changes are eligible.
3. Pair a receiver if necessary. **Automatic Backup** continuously handles new
   and changed photos/videos. Turning it off stops adding new tasks; queued files
   continue transferring. **Back Up Existing Files** includes unbacked current
   files. With automatic backup off, this is one run and its button changes to
   **Stop This Backup** until preparation finishes. Global pause still applies;
   neither operation implicitly resumes all sources.
4. Open the folder source to browse indexed files and receiver receipts. The
   transfer list is shared with the photo library, with source labels and a source
   filter. **Received** means the paired receiver verified the resources, not
   that Google Photos completed its cloud backup.

**Stop This Backup** stops further preparation from a manual run, but does not
cancel already prepared transfers. Use global pause to suspend all transfers. Removing
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
reviewed in the source page and retried with **Back Up Existing Files**.

On Unix, file identity uses the inode within the registered source/volume;
renames within the source do not alone require retransmission. Content changes
produce new revisions. Repeating Back Up retains actual receiver submission
records and skips unchanged files already queued or received. It removes only the
initial exclusion baseline and retries skipped media; it does not force re-upload.
Changing receivers permits a separate backup to the new target. Hard links to the same file share this identity. Across
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

## Source controls and failed files

Add Folder and shared folder guidance live in the window toolbar. Cards show
original names, paths, compact state/counts, View Files, one manual-run action,
and the Automatic Backup switch. More contains Rescan, Show in Finder, source
statistics and Remove Source. Guidance is shared once rather than repeated in
an information popup on every card. The manual button changes to Stop This
Backup for an active manual run; automatic backup is stopped with its switch.
Sources paused by older versions migrate to an off switch without resuming work.

A global pause banner offers explicit Resume All Backups. Per-folder actions
respect this pause and the current paired receiver. Settings separates System
Library discovery/import, folder switches and shared pause, transfer and cache
controls. Mac help distinguishes Photos permissions and iCloud originals from
folder bookmarks, drive availability and exported media grouping. iOS settings
remain photo-library only.

Files Not Backed Up lists preparation failures, including the original path,
recorded reason, Show in Finder and Retry. Unreadable media, access denial and
missing files have distinct explanations. Legacy failures honestly state that no
reason was recorded; retry can obtain a current error. A targeted retry clears
only that file's skip and can prepare that file while automatic/manual backup is
off. It does not include unrelated baseline-excluded files or change global
pause. Requests and failure details survive restart. Receipts, stability checks,
capacity deferral and receiver isolation remain in effect; completed jobs are
not duplicated. Pausing/stopping clears pending retries, and asynchronous media
inspection rechecks current pause, source enablement and receiver before enqueue.
Failed files have a separate page, entered from the count on the folder card or file-list header; they are not embedded above the normal file list. Ignore This Version removes the failure record and persists a revision-scoped user dismissal independently of automatic failure skips. Bulk retries and relaunch do not revive it; a changed revision is eligible again. Originals and queued tasks are preserved. Directory and failed-file pages retain the sidebar and return actions. Sorting is
an intrinsic-width menu rather than a control that fills the file-list header.

The menu-bar status item already exists (`MenuBarExtra` / `MacStatusMenu`). It
shows transfer status and offers open-window, pause/resume, settings, update and
quit actions. Closing the window leaves the app running; quitting stops it.

## List display preferences

Mac Settings → Backup → List appearance contains **Show photo and video
thumbnails**, enabled by default. This applies to transfer/file lists and compact
transfer previews; it does not disable the photo library grid. Turning it off
cancels visible-row thumbnail requests and uses media icons. File previews use
Quick Look Thumbnailing, with at most four active requests, at most 128 pending
requests and a 32 MiB / 256-item in-memory cache. Requests are cancelled when rows
leave the screen. The cache is keyed by source, current path, indexed revision and
size; source files are never cropped or rewritten. System thumbnail generation
still performs disk reads and decoding, especially for videos and cold caches.

Folder-transfer previews look up the task's recorded identity through the folder
inventory, including tasks created before this update and renames discovered by
later scans. The primary asset is selected explicitly for Live Photos. Missing,
changed, removed or inaccessible originals fall back to icons; no cached staging
file is retained merely for a preview. Symbolic links are not followed.

The file-list header offers **Flat / Folders** and remembers the choice. Flat
defaults to newest-modified ordering and offers modification time, file size or
relative-path sorting in both directions. Folders offers ascending or descending
name order, with directories first, and queries a directory only when expanded.
Each layout remembers its sorting choice. Sorting happens in the inventory query
before pagination, so it covers all indexed files rather than only loaded rows.
Each directory pages independently in groups of 100; root pagination includes
folders beyond the first 100 rows. The view uses indexed relative paths rather
than enumerating the drive again. Only folders containing supported, indexed
photos/videos appear, so this is not a general-purpose Finder replacement.

Directory queries scan the relevant indexed subtree to group immediate child
names; their response and UI allocations are bounded. Expanded directories refresh
after scans, preserving expansion state and loaded page depth. Whole-library
indexing and cold thumbnail generation on multi-terabyte mechanical drives still
need real hardware measurements; configurable views do not eliminate those costs.

## Folder rules

Add Folder has an optional Backup Rules disclosure; each card's More menu and
file detail also offer Backup Rules. Each folder has its own include/exclude
lists, one glob per line, matched against relative paths with case sensitivity.
An empty include list accepts all supported media; matching any exclude wins.
`*` and `?` do not cross `/`; `**` crosses directories (including zero levels),
with character classes and brace alternatives supported. Examples:
`**/*.{jpg,JPG,png}` includes those formats at all levels; `Trips/**` excludes
all descendants of Trips. Leading absolute paths and `..` components are rejected.
Patterns are compiled by the Rust globset library, not a custom matcher.

Rules are applied before media inventory insertion. Excluded directories may
still be traversed to discover eligible descendants; rules do not promise to
eliminate enumeration costs. The file list reflects the accepted inventory,
while failed-file history remains available separately until retried or ignored.
Saving valid rules hides the previous inventory until a requested full scan
rebuilds it. Invalid rules preserve the current inventory and configuration.
Receiver receipts, source identities and initial baselines are preserved.
Newly included files without a baseline/receipt are eligible if backup is enabled.
Rules cannot enable unsupported formats or alter already queued transfers.

Native preparation rechecks both rules and user dismissals for every resource.
An exported Live Photo is grouped only if both photo and video are eligible;
otherwise its allowed component is processed individually. Filtering cannot
silently include an excluded companion. Configuration and user dismissals
survive restart and stay isolated to their source.
