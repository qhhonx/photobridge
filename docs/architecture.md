# Architecture and boundaries

## Asset model

An asset is a logical photo, video, or motion photo. Its manifest contains source
identity, revision, metadata, and ordered original resources. A motion asset has
one photo and one paired video. Resource digests refer to exact original bytes;
existing EXIF and container metadata therefore survive transport unchanged.
Gallery-only metadata is carried separately by a native source adapter.

The asset ID is SHA-256 of the validated compact JSON serialization. Metadata keys
are sorted. Bindings should call Rust's ID function rather than invent their own
JSON canonicalization. Resource order is part of the contract. Content digests
allow resource deduplication even when two clients have different source IDs.

## Shared Rust responsibilities

The common engine owns identity, transfer actions, acknowledgement semantics,
validation, and target-independent processing states. The HTTP implementation is
a reference executor, not a mandatory network runtime for every native app.
Native background upload engines execute persisted actions and reconcile with
receiver status after interruptions. Source file handles must not be assumed to
be universal filesystem paths. The current CLI materializes regular files; native
hosts must supply an appropriate resource export/access implementation.

The sender queue persists manifests, opaque source access references, receiver
identity, attempts, retry timestamps, pause state, confirmed progress and OS task
IDs. Each claim receives a generation. Acknowledgements from superseded attempts
are rejected. Foreground attempts left running on process exit are requeued;
OS-managed attempts stay attached until the native host reconciles its task list.
Unknown OS tasks are returned for cancellation. Missing tasks query receiver state
before uploading again. Network, capacity and busy errors retry with capped
backoff; authentication, integrity and source-access failures require attention.

The iOS host executes immutable file-backed requests through background URLSession.
Rust prepares register/chunk/commit operations and atomically records each reply
with its next-operation checkpoint. Swift binds the OS task before resume and
reconciles live tasks on process startup. No Rust network future needs to survive
suspension. Native PhotoKit change tokens and pending identifiers are persisted
atomically before export; the shared queue deduplicates the exported asset.
BGProcessingTask provides opportunistic discovery and retry wakes, not a timer or
an assurance of immediate work while the app is closed.

## Receiver durability

The receiver has a single writer lock for its root. SQLite stores assets and
unique blob reservations. Registration reserves all new resource sizes in one
transaction, so a rejected motion asset cannot leave half a reservation behind.
Uploaded bytes are written into hash-named partial files. Each accepted chunk is
synced before acknowledgement. Its actual durable length is the resume offset.
Exact chunk replays are allowed; contradictory or overlapping writes are rejected.

After the full resource hash matches, the file is renamed into the blob store and
its ready flag is recorded. A restart reconciles a complete partial file or a
renamed blob whose ready flag was not committed. The final asset commit verifies
all resources and records receipt. Missing committed files are integrity failures,
not evidence that a source should automatically upload again.

Capacity is a reservation budget for original resource bytes, including unfinished
uploads. It is not a physical disk-free-space guarantee. Filesystem exhaustion
returns an error without committing the asset. Native receivers must also expose
actual space/temperature constraints. Reservation cancellation and orphan garbage
collection are intentionally absent from this first milestone.

## Receipt and target processing

Receipt: `receiving -> received`.

Optional target processing:
`not_requested -> pending -> complete`, or `pending -> failed -> pending`.

A received asset remains received when processing fails. Ordinary Pixel assets
request original publication. Motion assets request Motion Photo generation only
if the Android host has a real converter. The Android host invokes native codecs, Rust container packaging and MediaStore
publication. The generic protocol still advertises no target-specific processing
capability; automatic capability negotiation remains future work. Receiver
receipt never claims Google Photos cloud backup.

Conversion location can be optimized later through capability negotiation without
changing asset identity or the original-resource receipt contract. Original
retention and derived-output cleanup require an explicit target policy.

## Native hosts

- iOS/macOS: photo access, permission UI, native transfers/scheduling and media APIs.
- Android: source gallery access and/or receiver lifecycle, MediaStore publication,
  hardware media APIs, device conditions.
- Windows: selected file sources, native desktop UI and lifecycle integration.

Hosts may be senders, receivers, or both. Only Pixel has a defined target extension.
There are no invented NAS, cloud-account, billing, or multi-tenant abstractions.

## Security scope

The executable binds loopback and uses a private randomly generated bearer token.
The client permits HTTPS origins or loopback HTTP, rejects credentials in URLs,
and does not follow redirects. Routes authenticate before body extraction. Chunk
size, manifest size, concurrent requests and request duration are bounded. Client
filenames never become receiver storage paths. The receiver root is trusted and
must not be modified by another application while it is serving.

The native receiver hosts HTTPS using a private generated identity. Its pairing
code contains a certificate and bearer token; clients verify the certificate
identity and TLS hostname. iOS keeps the pairing in its device-only Keychain.
The preview uses one receiver-wide token and a fixed network address. One-time
pairing, per-client revocation, discovery and multi-user isolation remain future
work. It is not an Internet-facing service.


## Local receiver presentation

`ReceiverHistory` is a host-only FFI query, not a network route. Its catalog opens
SQLite read-only and uses stable descending row cursors, filters before pagination,
and returns at most 100 items per page. Reading progress only inspects file lengths;
it does not invoke transfer verification, rename partial files, or remove blobs.
A retained receipt records completed receipt even after verified originals were
archived and reclaimed, or after the user opted into receiver relay retention.
Relay reclamation requires a fresh host-side hash of a ready delivery copy,
matching durable expected/publication evidence; it is distinct from an original
archive. It does not assert that a gallery copy or cloud backup still exists.
See [receiver retention](device-and-backup-experience.md#optional-receiver-relay-mode).

Android uses RecyclerView/ListAdapter for reusable rows and asynchronous diffs.
Visible-page refreshes do not reload every previously browsed page. Thumbnail
work is limited to two concurrent requests and a 12 MiB cache. MediaStore supplies
160-pixel previews; unsupported previews use a media icon. Theme and font changes
use native configuration handling. Main navigation views stay alive across tabs.

`ReceiverSettings` and `ReceiverLogs` are local commands that also work while the
network service is stopped. Saved settings before the first start are retained;
they are not overwritten by the host's initial capacity argument. No credentials
or original-resource paths are returned by these presentation queries.

Platform references: [Material navigation](https://github.com/material-components/material-components-android/blob/master/docs/components/BottomNavigation.md),
[RecyclerView ListAdapter](https://developer.android.com/reference/androidx/recyclerview/widget/ListAdapter).


## Background diagnostics

Lifecycle snapshots query whole-queue Rust counts and enumerate the platform's
system requests. The journal records pending exports, queued/running/waiting/failed
counts, pause state, request stage, system-reported bytes and the next retry time.
Completion callbacks include HTTP status and numeric platform error codes; only
a separately validated receiver receipt proves receipt. System-reported sent
bytes are diagnostic, not durable confirmed progress.

Dispatch decisions are recorded when their reason or queue/import state changes,
not on every unchanged foreground poll. The UI keeps these fields under an
expandable Details row and includes them in the existing native log export.
Context uses a Rust allowlist of typed counters and fixed enum values; paths,
source identifiers, endpoints and credentials cannot be added as free-form
context. The maintenance database adds a nullable context column, preserving old
events and the user's retention settings. Missing context in an old event means
unknown, not zero pending work. A short background completion or a manually
recorded test snapshot does not prove sustained lock-screen execution.

## Desktop folder-source capability

The optional `photobridge-folder-source` crate produces generic assets from a
read-only, persistent folder index. Mac owns folder authorization, filesystem
notifications and media metadata APIs; the existing Rust sender owns snapshots,
checksums, durable tasks and transfer. Mobile native builds never link the folder
crate. Source identity is namespaced without changing existing PhotoKit IDs.
See [folder backup](folder-backup.md) for behavior and validation boundaries.
