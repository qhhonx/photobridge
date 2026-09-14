# Auditing existing cloud dates

The capture-date publication fix does not change existing Google Photos items.
For a historical audit, first narrow the cloud date range to the days when
transfers occurred. Include normal recent photos: comparison should skip correct
items, rather than require people to identify them beforehand.

`scripts/audit-capture-dates.py` reads a consistent receiver SQLite snapshot and
optionally a MediaStore text export and cloud observations. It emits a JSON audit
and a CSV review table without changing any media or contacting a service.

```sh
python3 scripts/audit-capture-dates.py \
  --receiver-db /path/to/receiver.sqlite3 \
  --media-index /path/to/media-index.txt \
  --cloud-json /path/to/cloud-observations.json \
  --since 2026-09-12 --until 2026-09-15 \
  --timezone Asia/Shanghai --output /path/to/private-report
```

The end date is exclusive. Keep database snapshots, observations and reports out
of Git. A snapshot must include the SQLite WAL, or be made through SQLite backup;
copying a live database file alone can omit recent records.

Cloud observations are a JSON array of `url`, `label`, `filename`, and
`detail_date`, read from the visible photo and information panel. For example,
`label` includes an absolute date with seconds and `detail_date` includes the
photo's `GMT+08:00` or other explicit offset. A collector may instead supply
`observed_iso`, an ISO 8601 timestamp retaining that explicit offset. Without the offset the result is
unknown; a cloud photo may use a different timezone from the auditing computer.

`PB_<asset-id>` filenames allow an exact match to a receiver manifest. Unknown
filenames or missing source dates are not guessed. The report distinguishes
correct dates, mismatches, unmatched photos and unreadable metadata. MediaStore
suspects are only leads, not proof of cloud-date errors. Only supplied cloud
observations are covered; the script does not claim complete cloud enumeration.

A future write mode must recheck the cloud identity and current date against the
reviewed plan, skip already-correct items, record the original value for recovery,
and stop on ambiguous or changed pages. The public Google Photos Library API
cannot edit capture dates. An automated visible-page adapter can perform the
repetitive operations without asking a language model to decide each photo;
this audit script deliberately has no write mode.
