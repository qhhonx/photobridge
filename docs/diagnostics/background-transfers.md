# Diagnosing background transfer stalls

Use a development build on both peers. Overwrite-install to preserve pairings,
checkpoints and receipts. Do not clear application storage or restart the receiver
midway through a measurement window.

## Correlating an attempt

Apple senders persist a random, non-identifying `request_id` in the URLSession task
descriptor and send it as `X-PhotoBridge-Request`. Each HTTP attempt gets its own
ID; job ID, queue generation and URLSession task ID provide local correlation.
Legacy task descriptors remain readable but lack this correlation information.

Submission events record `submitted_at_ms`, `submission_execution`, transfer mode,
power and network flags. Completion events capture application state and time at
delegate entry, before asynchronous receipt processing. Network metrics are
attached if URLSession provides them. Missing metrics or missing phase dates are
unknown, not proof that the OS did not schedule a task. A reused connection can
legitimately have no connect/TLS timestamps.

Receiver observations, emitted only for authenticated requests with a valid
numeric correlation header, include request admission, first body data and handler
response readiness. The body is streamed without changing content. Byte counts
are wire-body bytes, not photo counts. `receiver_response_ready` does not prove the
sender received the response. `body_complete=false` is normal for a commit handler
that does not consume its request body. The ordinary 60-second handler timeout
and streaming-bundle idle timeout remain unchanged.

Compare elapsed durations on each peer; do not subtract absolute timestamps
across devices without estimating clock offset. A missing receiver observation
is not by itself evidence of an iOS scheduling delay: authentication/admission,
TLS, network errors, process lifetime and bounded log retention can also explain it.

## Transfer modes

- `bundle`: one asset envelope submitted to the OS.
- `cache_limited_chunks`: insufficient temporary staging allowance for an envelope.
- `legacy_chunks`: peer bundle mode is not enabled.
- `resume_chunks`: continue an existing checkpoint instead of discarding progress.

The existing `bytes` field on a submission is the immutable request-body size.
URLSession byte counters can be zero or unavailable at particular callbacks.
Receiver body counters and successful response status provide independent evidence.

## Preparation backlog and storage

An empty prepared transfer queue does not mean library backup is complete.
`pending_imports` is reloaded from the durable source queue, including after a
background process relaunch. `discovery_pending` describes incremental discovery
and is not the historical-library total.

Snapshots include `cache_used_bytes`, `free_bytes`, `min_free_bytes`, and
`export_allowance` when storage has been measured. A zero export allowance blocks
preparing the next originals even when existing requests can finish.
`preparation_storage_blocked` records that condition during background preparation;
`processing_finished` means the current execution window ended, not that every
library item was backed up. Resolve the storage constraint before attributing
this situation to OS scheduling or comparing charging conditions.

## Controlled device run

1. Confirm at least one correlated successful request on both peers.
2. Keep the receiver on the same Wi-Fi, powered and receiving. Do not interact
   with cleanup or change transfer concurrency during the window.
3. Put the iPhone on a charger, keep Low Power Mode off, leave the application
   normally and lock the screen for 30 minutes. Do not attach a debugger or
   swipe the application away. Record the window start and end.
4. Copy event databases and WAL files without foregrounding the sender. Work on
   copies, run SQLite `quick_check`, and retain original snapshots privately.
5. Repeat without charging, keeping other conditions comparable. Record queue
   size, bytes and modes because different assets can otherwise confound results.

Separate completed network requests, completed assets and gallery publication.
None of these establishes Google Photos cloud backup. If ordinary metrics remain
inconclusive, use Apple's Background Networking diagnostic profile and sysdiagnose
with the device owner's participation; these artifacts can contain private data.

Diagnostics contain typed counters, fixed vocabularies and random request IDs.
They exclude URLs, SSIDs, headers, tokens, photo names and certificate material.
