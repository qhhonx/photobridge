# Device identity, backup scope and receiver capabilities

This specification records the product requirements from the September 12 review.
It describes intended behavior, not a claim that every feature is implemented.
The reference PixelBridge checkout was fast-forwarded to `b424f81`
(`0.1.0-beta.16`). Its burst implementation enumerates all burst members and
annotates delivery copies with GCamera BurstID/BurstPrimary. PhotoBridge retains
its own transport and original-preservation model instead of copying that pipeline.

## Device names

- Every installation receives a durable random device ID and a memorable default
  name from a built-in adjective/nature/animal catalog, such as Amber Otter or
  Moonlit Cedar. Generation and persistence belong in shared Rust code.
- Generate once, never on each launch, IP change or reconnection. Do not read a
  person's account name, computer name or phone name to create the default.
- Offer an editable name in Settings and on the pairing screen. Validate length
  and control characters. If names collide, show a short device-ID suffix.
- A name is display metadata, not authentication. Certificates, credentials,
  receiver identity and receipt matching must not depend on the name. Renaming
  must never duplicate jobs, invalidate pairing or change asset content IDs.
- Use the same persisted name on both ends, including when their UI languages
  differ. Localize generated names at creation, not each time a peer is displayed.
- The sender shows "To Amber Otter"; the receiver shows "From Moonlit Cedar".
  Device type and recent activity are secondary information. Support multiple
  sender records instead of overwriting one global last-sender label.
- Exchange device profiles through the authenticated pairing/registration path.
  A nickname added only to QR data is insufficient: iOS direct pairing, desktop
  reverse pairing, reconnects and offline history must all retain peer identity.
  Keep peer metadata separate from photo manifests and backup deduplication.
- Add bounded optional profile fields with an explicit compatibility contract.
  Current Rust pairing envelopes reject unknown fields, so extension behavior
  must be tested rather than assumed. Old stored credentials must remain readable.

Acceptance: pair in both directions, restart both apps, rename, reconnect, use
identical nicknames, change UI language, and receive from two senders. Verify the
same peer attribution and unchanged receipts/queue in every case.

### Current implementation

Rust owns `devices.sqlite3`, separate from the asset store. Local names use a
256-combination English/Chinese catalog with a short random suffix, generated
once using the initial locale. Rename preserves the random installation ID.
Settings on Apple and Android expose the same persisted name. Android shows
known senders with last-contact time and an ID suffix in the full device list;
this is not a live-connection indicator or per-photo sender attribution.

After pairing, `/v1/device-profile` exchanges bounded optional profiles through
the existing authenticated, certificate-pinned channel. Pairing JSON remains
unchanged; a 404 from older receivers disables only the optional profile result.
Apple refreshes at most once a minute while active and attempts refresh after
pairing/rename. Offline failures retain the previous label and do not change
transfer state. Peer display history is bounded to 256 entries; eviction never
removes credentials, tasks or receipts. Names are not independently authenticated
sender identities and must not be used to grant access.

Validation: Rust tests cover durable names, locale changes, concurrent reopening,
invalid input, same-name distinct senders, renamed peers, rejected credentials,
legacy 404 compatibility and unchanged media identity/receipt. The isolated
Android validation package passed JNI/TLS profile exchange, rename propagation,
invalid-input preservation and restart alongside synthetic photo/video/motion
transfer and verified original-archive tests. Apple sources typecheck; the iOS
device build and Mac fixture build pass. Mac English/Chinese fixture rendering
also checks rename persistence and invalid-save preservation. Full physical
direct/reverse pairing presentation remains an acceptance item.

## Default media support

Photos, videos, Live Photos and bursts belong to the normal backup flow; no
experimental toggle is needed to enable these media types.

- Live Photos remain one logical asset with original photo and paired-video
  resources. The sender transfers originals; receiver capabilities decide the
  appropriate presentation format after durable receipt.
- Burst support means all member frames plus stable group membership and primary
  frame metadata, not merely the representative photo. Keep every original byte.
- Apple browse/export/thumbnail/anchor fetches now explicitly include all burst
  assets. This fixes membership only; it does not establish receiver grouping.
- Expand a selected burst group consistently in manual, incremental and historical
  import paths. A grouped gallery cell should display a burst icon and frame count,
  with an optional member view rather than repeated long labels.
- Persist generic burst group and primary-frame data. GCamera XMP belongs only in
  the Pixel publication adapter and only in a derived delivery copy. Existing
  queued/retry manifests must retain their original content identity.
- If presentation conversion fails, keep a visible receiver processing failure
  with preserved originals. Never label a static-only output as a working motion
  asset or silently discard burst siblings.

Acceptance: mixed photo/video/motion/burst batches, partial burst selection,
changed representative frame, retry/restart, original digest checks, grouped
receiver presentation and actual motion playback. Existing motion playback
verification does not establish burst grouping.

### Current burst implementation

The shared preparation path expands a selected burst into all accessible siblings,
using batches of at most 200 metadata records. Siblings with an existing task are
not recursively re-added; failed existing tasks keep their normal retry workflow.
The same path handles manual selection, pending incremental discovery and historic
preparation. Native grids currently retain individual frames and add a burst icon;
grouped cells need aggregate receipt state before they can safely replace these.

Rust generates a hashed group reference and validates the optional generic
`burst_group_ref`, `burst_primary` and `burst_metadata_version` fields. Raw native
group identifiers are not written to public gallery copies. Each frame remains
an independently verified original asset. Existing queued/received revisions are
reused unchanged; this deliberately does not retrofit old gallery copies or
change prior manifest identities when a primary frame changes.

The Pixel adapter writes GCamera BurstID/BurstPrimary into a derived JPEG. Ordinary
JPEG scan data, EXIF, ICC and unrelated standard XMP are preserved without pixel
re-encoding. The XML parser resolves namespaces before inserting metadata; an
existing matching packet is idempotent. Conflicting burst tags, extended XMP,
malformed metadata and multi-picture JPEGs currently stop processing with the
original retained, rather than damaging offsets or claiming successful grouping.
HEIC and other native-decodable stills produce an oriented JPEG delivery copy with
selected EXIF fields, using the same codec path as motion photos. Such delivery
copies are not byte-identical original archives. Motion packaging can also carry
burst fields in its single XMP packet.

Rust tests verify original bytes, JPEG scan/EXIF/ICC preservation, unrelated XMP,
alternate namespace prefixes, empty RDF, idempotence, rejected malformed or
conflicting metadata, destination protection and unchanged motion-video tails.
Pixel validation publishes five synthetic assets including two burst frames,
reads their XMP back through Android ExifInterface, verifies a shared group and
one primary, replays publication without duplicates, and completes verified
original archive/reclamation. Google Photos UI grouping, native Apple burst
selection and HEIC burst color/metadata fidelity remain unverified.

Format references: [ExifTool Google camera tags](https://exiftool.org/TagNames/Google.html)
and [quick-xml namespace-aware reader](https://docs.rs/quick-xml/0.38.4/quick_xml/reader/struct.NsReader.html).

## Historical import and ongoing backup

Provide two independent controls in Backup settings:

1. **Automatically back up new photos** continues using native library changes.
2. **Import existing library** is a one-time, resumable import run for the current
   receiver. Enabling it schedules the currently accessible library. Show its
   state (scanning, preparing, transferring, paused, complete) instead of leaving
   an unexplained switch enabled forever. Offer "Scan again" after completion.

- New-photo backup must continue while historical import is pending. Turning
  either control off must not silently disable the other.
- Create a durable scan/run identity before enumeration. Persist bounded batches
  into Rust's source queue before advancing progress. Restarting a scan must not
  redownload or retransmit assets already represented by the same source revision.
- Do not materialize the full library or original files in memory. Do not use a
  capture-date cutoff as a substitute for library membership; imported older
  images and equal timestamps must not be skipped.
- Limited PhotoKit access means "available photos", not "entire library". Show
  the permission scope and provide the native permission-management entry point.
  Hidden assets require an explicit policy; default media support does not imply
  exposing hidden or inaccessible photos.
- Pausing historical import stops adding work and preserves its checkpoint.
  Already queued work and in-flight requests need explicit, explained controls;
  do not imply that disabling the scan removes existing backup tasks.
- Changing receiver creates a separate import scope; it must not inherit another
  receiver's completion flag. A completed scan is not a completed transfer.
- Counts come from the durable queue, not the first UI page. Expose unavailable
  and failed items with retry controls rather than declaring full completion.

Acceptance: thousands of mixed assets, process restart during enumeration,
low-space pause/recovery, repeated enabling, newly inserted older photos, limited
permission changes, receiver replacement and simultaneous incremental discovery.

### Current historical-import implementation

Preparation and eligible transfer jobs use capture time, newest first, on both
iOS and macOS. Pending queues from earlier versions are backfilled in batches of
200 using PhotoKit metadata only; original files are not downloaded for sorting.
Incremental discovery also orders its pending identifiers by capture time.
Equal timestamps use source identifiers as a stable tie-breaker; unavailable dates
sort last. Retry deadlines and active uploads are preserved, so concurrent
completion order can differ from scheduling order. Already submitted OS uploads
are not cancelled to make room for a newly discovered photo.

Apple backup preferences now offer Import existing photos, Pause/Resume library
scan and Scan library again. The backup overview shows scan state and pending
preparation separately from transfer receipts. Global backup pause also pauses
scanning; a contextual Resume backup action is available in the scan settings.
Starting or pausing a historical scan does not enable or disable new-photo backup.
The existing iOS processing-task callback can advance a batch when the OS grants
execution; this is not a promise of uninterrupted background enumeration.

Rust stores a receiver-scoped run and its source membership in the maintenance
SQLite database. Batches are limited to 200 source/revision pairs. Membership and
pending preparation insertions are one transaction; paused/replaced runs reject
late batches. Existing queued/received revisions are omitted from preparation.
Apple also checks the durable source-revision index before any original download,
covering manual selection and replay. Failed jobs remain eligible for preparation
so missing staged originals can be repaired by the existing queue behavior.

A PhotoKit fetch result is an in-process metadata snapshot. Its numerical offset
is never reused against a different snapshot: after app restart, metadata starts
at zero and Rust deduplicates already represented sources. This trades some
metadata enumeration time for avoiding skipped photos after library reordering
or deletions. Source membership survives restart, as does pending preparation;
large original files are not materialized to construct the scan. Apple serializes
scan batches with original preparation; selections arriving during a scan are
queued durably for the normal preparation worker. Start/pause/resume/finish
transitions also appear in localized activity logs. A scan may
finish while preparation or transfer failures remain, so the UI says Scan
finished and retains pending counts rather than saying Backup complete.

Validation: the real Rust JSON boundary scanned 10,000 synthetic source IDs
across two processes, retained pause state and concurrent incremental work,
replayed earlier metadata without duplicate jobs, and detected a changed source
revision. Store tests also cover old-run rejection and receiver isolation.
Mac production-view fixtures render paused/finished states in English/Chinese
and assert that finished scans retain pending preparation counts. iOS compiles
against the device SDK. Physical PhotoKit changes, full historical transfers and
prolonged iOS background execution remain acceptance work.

## Sender/receiver boundary

The sender knows device identity, negotiated generic media capabilities, transfer
progress, integrity-confirmed receipt and generic capacity/backpressure. It does
not implement or advertise Google Photos, Pixel cleanup or NAS-specific steps.

Apple sender receipt/pairing copy has been made receiver-neutral. The receiver
owns conversion, system-gallery publication, external cloud integrations and
local retention policy. A receiver processing status may be exposed generically,
but a saved receipt must never be relabeled as confirmed third-party cloud backup.

## Google Photos space release on Android

This is a receiver integration, optional and disabled until explicitly enabled.
Android AccessibilityService can inspect accessible UI and perform actions after
user enablement. That makes a device-local adapter feasible without a Mac/ADB
connection, but does not establish that every Google Photos version is supported.
Ordinary apps do not inherit ADB shell privileges. PixelBridge's force-stop
recovery cannot be copied as a standard Android API operation.

Start with a capability probe on the actual device: supported Google Photos
package, unlocked screen, recognizable account/menu, official safe-backup
confirmation, progress/completion and return navigation. Use fresh semantic node
state, not fixed screen coordinates or only a version whitelist. Unknown pages
stop automation; never repeat an uncertain final cleanup tap. Do not wait for
all uploads to finish if Google Photos itself offers eligible backed-up items.
Serialize publication/UI-cleanup transitions while normal capacity accounting
provides backpressure to senders.

The first Android implementation is a read-only compatibility check under
Settings → Experiments, with explicit disclosure, manual system permission and a
user-started 120-second observation window. Observations are fixed categories;
there is no automatic action execution, account binding or cloud-backup proof.
Unknown/incomplete screens remain unrecognized. Final automation must separately
validate account continuity, fresh actionable nodes, publication serialization,
uncertain-action recovery and return navigation before offering an opt-in toggle.

Implementation references: [Android accessibility services](https://developer.android.com/guide/topics/ui/accessibility/service),
[Google Play accessibility API policy](https://support.google.com/googleplay/android-developer/answer/10964491?hl=en),
and [Google Photos device space release](https://support.google.com/photos/answer/6128843?co=GENIE.Platform%3DAndroid&hl=en).
The service is not declared an accessibility tool for disability support. Any
future Play distribution needs its own policy declaration/review; local device
validation does not establish store approval.

The receiver storage page must separately report:

- **Original archive:** Rust's private, verified original resources.
- **Gallery copies:** derived/public MediaStore items, potentially backed up by
  Google Photos. Its Free up space action can remove eligible copies here.

Google Photos cleanup does not free PhotoBridge's private originals. The default
retention policy requires a separately verified original archive before reclaiming
those files. The user can instead explicitly enable receiver relay mode, described
below. A converted Motion Photo is not a byte-for-byte archive of its HEIC/MOV pair.
Neither policy infers Google cloud backup from local publication.

### Current storage presentation

Android has a dedicated storage page with device free space, original files,
gallery copies and the original-retention rule. Rust reports complete-file and
partial-file sizes separately from catalog capacity reservations. Shared blobs
are counted once, including completion renames observed during enumeration.
The snapshot is read-only, does not hash resources or acquire the receiver writer,
and does not include database, cache or filesystem overhead.

The Android adapter queries indexed sizes of this installation's PhotoBridge
images/videos, restricts by album path, owning package and delivery filename,
and separates pending publication from visible copies. Unknown indexed sizes and
query failures are visible instead of being presented as a zero-byte success.
No photo-library read permission is added. Media-index updates may lag actual
writes/removal; moved files, trash, old installations and other apps are outside
this view. Device available space remains the disk-wide capacity measurement.

Detailed scans run off the UI thread only while the storage page is requested,
at most once per ten seconds in its visible refresh loop, with a manual refresh
and immediate refresh on entry. A refresh already in progress is not duplicated.
Original budget controls preserve exact stored byte values, including custom
limits that are not whole GiB.

### Optional receiver relay mode

The storage page offers “Reclaim originals after saving to gallery”, off by
default. Enabling requires a native explanation/confirmation: this is local
gallery storage, not cloud backup, and converted motion/burst originals will
no longer be recoverable from this receiver. Turning the switch off prevents
new reclamation and does not restore earlier files. Sender UI stays neutral.

Rust persists `receiver_relay` with a false migration default. Before writing a
delivery item, the Android host persists its expected locator, size and SHA-256.
It verifies bytes while writing, marks the owned MediaStore item ready, then
reopens and hashes the saved file. Indexed size or a successful insertion alone
is insufficient. Existing complete files are never overwritten on mismatch.
Pre-write evidence lets a crash/retry verify an already-produced motion file
without running the codec again or changing the delivery identity.

`gallery_publication` atomically stores immutable verified evidence and completes
local publication. Relay policy is checked again under the receiver/settings
locks before reclaiming. The release marker is durable before file deletion;
shared original blobs remain until all original-retaining references are gone.
Restart recovery and historical transport receipts remain idempotent. History
distinguishes `gallery` reclamation from a verified-original `archive`.

While reception runs, a bounded four-item maintenance sweep also checks existing
published assets. It reopens recorded copies; older unconverted photo/video
copies can be compared with original hashes. Legacy transformed copies without
durable evidence retain their originals rather than repeatedly re-encoding them.
Missing, changed, pending, moved or trashed copies retain originals. The sweep
never recreates a missing gallery item to justify reclamation. It runs under the
same media-operation mutex as publication/archive work and records fixed-vocabulary
reclamation/waiting events without file paths, locators or account data.

This opt-in policy preserves a verified local presentation copy, not necessarily
the original format. Users should keep their source library or another original
backup. A retained receipt records a completed historical transfer; it is not
a guarantee of indefinite original retention or third-party cloud availability.


References:
- https://developer.android.com/reference/android/provider/MediaStore.MediaColumns
- https://developer.android.com/training/data-storage/shared/media
- https://developer.android.com/guide/topics/ui/accessibility/service
- https://developer.android.com/about/versions/14/behavior-changes-all
- https://support.google.com/photos/answer/6128843?co=GENIE.Platform%3DAndroid&hl=en

## Implementation order

1. Receiver-neutral sender copy and consistent burst fetch membership.
2. Shared persistent device profiles and authenticated bidirectional peer exchange.
3. Durable historical scan/import controls, independent of incremental backup.
4. Burst grouping, metadata and receiver presentation, with preserved originals.
5. Receiver storage breakdown, explicit retention policy and Google Photos
   capability probe before implementing optional cleanup execution.


## Grouped Apple library presentation

The library still fetches all accessible burst assets. A separate incremental
metadata index collapses shared burst identifiers into display rows; it does not
change source IDs, revisions, receipts or the sender's burst expansion. Native
UIKit/AppKit cells remain reusable and keep the existing bounded thumbnail cache.
The index fills pages of 240 display rows on a worker task, reading additional
metadata when a large burst occupies the source page. It does not decode images
or enumerate the entire library before showing the initial page. Filter changes
and permission revocation cancel obsolete index work.

A burst has a stable presentation token for selection and scroll restoration.
Backing up a selection resolves tokens to actual accessible PhotoKit asset IDs
before entering the existing scheduler, which expands accessible siblings. A
representative frame encountered in later metadata can replace the cover while
preserving the group's position and identity. The library header still counts
underlying photos/videos; a burst cell separately shows its accessible frame count.

Visible groups resolve member revisions off the UI thread; metadata caching is
limited to 128 entries / 4096 member descriptors. Failed/empty resolution is not
cached as a successful group. Source-state lookups use batches of at most 400.
A received cover alone yields a partial state; a green received mark requires a
receipt state for every resolved member. This remains a receiver receipt, not
Google Photos cloud confirmation.

PhotoKit's default burst fetch can include both a representative and user-picked
members, so simply disabling includeAllBurstAssets would not provide the desired
one-row grouping or preserve the complete backup scope. See Apple's
[burst fetch options](https://developer.apple.com/documentation/photos/phfetchoptions/includeallburstassets)
and [representative burst assets](https://developer.apple.com/documentation/photos/phasset/representsburst).


## Android help and recovery

Settings contains a separate Help and recovery destination with five topic pages:
connection, background reception, backup results, storage and recovery. English
and Chinese guidance distinguishes durable receipt, system-gallery publication
and cloud backup. It explains preservation defaults, optional relay behavior,
background scheduling and preview installation constraints.

The receiver status keeps a specific recovery hint while retrying. Lost Wi-Fi
opens network settings; insufficient space opens Storage; a system-blocked start
offers a foreground restart; unknown failures link to recovery and diagnostics.
An address change restarts reception on the new address while retaining its
original cryptographic identity. Help shows the current address while the
receiver is running. Original certificate authority names remain part of trust,
not routing; credentials and certificate material are not returned to that view.
Apple senders discover an identity-only Bonjour advertisement, validate the new
local route with saved trust and persist only an authenticated update. Old-route
OS tasks are cancelled as interruptions, and the recovered peer's network or
authentication failures are requeued without overriding user pauses. Guest
networks may require a fresh QR scan. Discovery does not force iOS wakeups.

## Sender management, task attribution and library feedback

Receiver device rows show the last observed connection IP and optional native
platform category (iPhone, iPad or Mac for current Apple clients). IP comes from
connection metadata, not a client-supplied address. It is historical information,
not a claim that the device is online. Device names remain display identities.

The device menu offers reversible **Disable reception / Enable reception**.
A persisted receiver policy rejects identified sender requests and rechecks at
bundle chunk boundaries. This is a trusted-client reception control, **not bearer
credential revocation**: existing pairing remains, and legacy requests without a
sender header remain unidentified. Do not label this action unpair or remove.

Current Apple requests include the installation's sender ID. Accepted registration
and bundle requests retain an asset-to-sender relationship independent of filenames
and blob hashes. Receiver history filters by sender before pagination and displays
sender names on each row. Multiple identified senders can reference one asset;
pre-update tasks without recorded provenance appear as Unknown sender (older tasks).
No retrospective attribution is guessed from filenames or timing.

Transport idempotency and checksum-addressed resource reuse remain in Rust. They
prevent retries and retained-receipt replay from creating repeated transfers or
recreating reclaimed originals. This is separate from Google Photos presentation
or similarity grouping; no new visual-similarity deletion policy is introduced.

Apple library changes now use PhotoKit change details. Availability/metadata-only
changes update visible content and badges without recycling the entire collection;
structural membership/order changes retain the existing scroll-anchor restoration.
Failed status lookups preserve displayed badges instead of reporting not queued.
Preparation queue membership is included only in UI status requests, leaving the
export scheduler's existing-job deduplication query unchanged. Visible status
requests also reject results for an outdated queue/preparation generation.

Both Apple library filters include Bursts, reusing existing grouped selection and
all-member export. Visible thumbnails first request local images, then allow at
most three iCloud fallback requests. Preheating stays local, offscreen requests are
cancelled, and transient nil callbacks do not erase an already displayed image.
PhotoKit determines what data it needs to download; this is not a promise that the
network transfer is only the returned thumbnail's byte size.

The iOS grid extends beneath the native translucent tab bar, with bottom content
padding for the final row. Backup controls use explicit image/text content and
consistent outline status symbols. Transfer rows add media/burst type, capture date
when present, and the next retry time while waiting, alongside byte progress.

### Stable backup overview and source drilldowns

Both Apple apps keep a fixed set of stage/count entries in the backup overview.
Changing preparation or upload activity does not insert photo rows on this page.
The transfer destination includes preparation and scanned-library filters as well
as the existing job-state filters. Original preparation progress lives inside its
source row on the detail page.

Source browsing reads receiver-scoped persisted records in pages of 100, including
preparation retries whose scheduled time is still in the future. It never starts
an import. Scan rows show discovered source membership and known transfer state;
scan completion is not a receipt. A new scan run resets the displayed pages so
membership from separate runs cannot be mixed. Visible rows resolve PhotoKit
metadata and use the same bounded thumbnail pipeline as transfer history.
