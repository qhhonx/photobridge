# Preserving capture dates during gallery publication

PhotoKit's `creationDate` is separate from the date embedded in an exported
original. A file can have no EXIF capture date even when its Apple Photos asset
has a valid date. On Android 10, a media rescan can replace an insert-time
`MediaStore.DATE_TAKEN` with an unknown date. A newly created file then exposes
its import-time modification date as a fallback.

The sender's `metadata.created_at_ms` carries the capture instant. The Android
receiver enriches a delivery copy when an image lacks an EXIF capture date,
without modifying the verified original or re-encoding its pixels:

- `MediaDates.kt` handles JPEG/PNG metadata and coordinates HEIC/HEIF processing.
- `crates/pixel/src/photo_date.rs` adds date metadata to HEIC/HEIF copies through
  the `write_photo_date` native command.
- DateTimeOriginal, digitized time, UTC offsets and fractional seconds describe
  the same instant. Existing valid EXIF capture dates remain unchanged.
- The receiver stamps file modification time after writing the last byte and
  before clearing `IS_PENDING`. MediaStore milliseconds and seconds are handled
  separately.
- Motion and burst JPEG dates are set before packaging, so later EXIF rewrites
  cannot invalidate the motion video's XMP offsets.
- Publication verification uses the delivered copy's size and hash. Original
  transfer integrity and gallery-copy verification remain separate.

## Validation and limits

Rust tests cover round trips, absent EXIF, malformed input, subsecond consistency
and preservation of originals. Android instrumentation mode `media_dates`
compares publication before and after a full rescan, hashes and decoded pixels.
Its fixtures are synthetic. The regular integration test checks dates on motion
and burst publications alongside receipt and cache-retention behavior.

Pixel XL / Android 10 testing confirmed JPEG and HEIC capture dates survive a
rescan. Android 10's scanner did not use the PNG EXIF capture date; the PNG's
embedded metadata and modification-time fallback were nevertheless corrected.
This does not establish Google Photos cloud behavior for every media format.
Video embedded dates are preserved, not rewritten; video metadata conflicts need
separate investigation. Existing valid EXIF dates are not overridden by a later
Apple Photos date edit.

This fix affects new publications. It does not update dates on existing Google
Photos cloud items, republish verified gallery copies, or delete cloud media.
Historical repair must match each cloud item to its correct source date and
skip already-correct items. Neither a date range nor a filename alone is enough
to resolve ambiguous matches safely.
