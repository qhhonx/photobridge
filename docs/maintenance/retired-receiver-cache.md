# Retiring duplicate caches from a previous receiver

This is an explicit maintenance operation, not a new default backup policy.
Delivery receipts remain scoped to the receiver that issued them. A new receiver
must not inherit another receiver's success status.

A developer launch can pass `--retire-delivered-cache PREVIOUS_RECEIVER_ID`.
The Apple application loads its existing pairing and uses that identity as the
current receiver; the argument cannot replace pairing trust. Normal launches do
not perform cross-receiver retirement.

The native operation archives only inactive previous-receiver jobs with an exact
match to a current-receiver received job: source identifier, revision, media kind
and all resource descriptors (including size and SHA-256). Running jobs, native
tasks and checkpoints are excluded. Full old job and receipt proof are stored in
`retired_cache_jobs` in the sender database before removal from the runnable queue.
The original receiver is never marked as having received these assets.

File cleanup is restricted to ordinary files in owned export directories, rejects
symlink paths, and preserves files referenced by any remaining unfinished job.
Receipt records, the source photo library and receiver files are unchanged.
Interrupted cleanup can be retried; archive records remain available for recovery
and audit. This does not provide a complete receiver-management interface.

The application writes `cache-retirement-result.json` to its private PhotoBridge
application-support directory, containing archived-job and reclaimed-byte counts.
Read this result and independently inspect the sender database after maintenance.
