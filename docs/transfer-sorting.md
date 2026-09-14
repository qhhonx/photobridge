# Transfer list ordering

Each Apple transfer sublist saves its own sort field and direction. The default
is based on the question the list answers:

| List | Default | Other useful choice |
| --- | --- | --- |
| Preparing / Queued | Newest capture first | Queue entry order |
| Running | Earliest start first | Capture time |
| Waiting to retry | Soonest retry first | Capture time |
| Failed | Latest failure first | Capture time |
| Received | Newest capture first | Latest receipt first |
| Paused | Newest capture first | Queue entry order |
| Scanned library | Newest capture first | Scan discovery order |
| All tasks | Latest entry first | Capture time |

Display ordering does not alter transfer scheduling. Direction is explicit and
can be reversed for the selected field. `Use list default` resets only that list.

Sorting happens in SQLite before pagination. A cursor includes both the sort
value and row ID, keeping ties stable across pages. Unknown dates are last in
both directions. Capture date comes from PhotoKit or the transmitted manifest,
never from the revision string. Older scan records are enriched with PhotoKit
metadata in bounded batches without requesting image files.

New state transitions record their time. Old completion/failure times were not
persisted and remain unknown; opening or upgrading the app must not manufacture
completion dates for those tasks. Rows show the corresponding event time when
that field is selected.
