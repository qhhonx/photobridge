# PhotoBridge protocol v1

Every route requires `Authorization: Bearer <token>`. Assets and responses use
JSON. Chunks use raw bytes. Maximum chunk size is advertised in capabilities and
is currently 4 MiB. Maximum manifest size is 64 KiB.

| Method | Path | Meaning |
| --- | --- | --- |
| GET | `/v1/capabilities` | Protocol version, chunk bound, supported asset/target capabilities |
| POST | `/v1/assets` | Validate/register a manifest and return persisted status |
| GET | `/v1/assets/{id}` | Query resource offsets, receipt and processing state |
| PUT | `/v1/assets/{id}/resources/{sha256}?offset=N&sha256=CHUNK_HASH` | Upload a verified chunk |
| POST | `/v1/assets/{id}/commit` | Verify all original resources and atomically acknowledge the asset |

Registration is idempotent. A full-file SHA-256 identifies each original resource.
The `sha256` query parameter is the digest of this request's chunk, not the entire
resource. Only resources belonging to the registered asset may be written.

Offsets are contiguous byte offsets. A replay wholly inside received bytes must
match those bytes. Gaps, conflicting bytes, and partial overlaps return conflict.
On a lost response or interruption, query status before scheduling another chunk.
After an HTTP error the caller must not infer that an operation did not happen.

Receipt statuses are `receiving` and `received`. Processing statuses are
`not_requested`, `pending`, `complete`, and `failed`. Processing completion has no
implied Google Photos cloud meaning. No deletion or cloud-verification endpoint
exists.

Typical errors: 400 invalid input, 401 missing/wrong authentication, 404 unknown
asset/resource, 409 conflict/incomplete asset, 413 oversized request, 422 integrity
failure, 429 busy, 507 capacity exhausted, 500 storage/internal failure. Original
paths and database errors are not returned over HTTP.


A received receipt records that all original resources were verified and accepted
at commit time. Receiver-local explicit retention policies can subsequently reclaim
originals after a verified archive or opt-in verified gallery relay. This does not
change asset identity, resume offsets or historical deduplication. Publication and
cloud backup are separate from transfer receipt; no cloud-proof claim is introduced.

## Optional complete-asset upload

A receiver advertising `bundle_upload: true` accepts `POST /v1/bundles` with
`Content-Type: application/x-photobridge-bundle`. Missing capability means false.
The file-backed envelope contains eight bytes `PBRG0001`, a four-byte big-endian
manifest length, the UTF-8 JSON manifest, then each resource's original bytes in
manifest order. Manifest and resource bounds are unchanged. A provided content
length must exactly match the envelope; trailing or missing bytes prevent commit.

The receiver streams bounded chunks into the same durable store. A replay verifies
existing bytes before appending; both original resources of a motion photo must
pass whole-file verification before a received receipt is returned. An existing
receipt remains authoritative even after explicitly authorized local reclamation.
A lost response is safe to replay, though the client currently resends the envelope
from its beginning. Normal receiver-side publication is independent of this request.

Apple senders cache the authenticated capability and prepare up to four immutable
file-backed OS requests. Already-started legacy transfers finish their existing
sequence. When the additional envelope would exceed the sender's staging budget
or free-space reserve, it uses the original bounded chunk path. No extra background
execution entitlement is implied; actual locked-device scheduling remains subject
to the operating system.
