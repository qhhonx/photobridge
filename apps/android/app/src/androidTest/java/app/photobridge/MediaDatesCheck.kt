package app.photobridge

import android.app.Instrumentation
import android.content.ContentValues
import android.graphics.Bitmap
import android.graphics.Color
import android.media.MediaScannerConnection
import android.net.Uri
import android.provider.MediaStore
import org.json.JSONObject
import java.io.File
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** Only synthetic media in a separate validation album; no user rows are edited. */
internal fun Instrumentation.checkMediaDates(): String {
    val context = targetContext
    val resolver = context.contentResolver
    val captured = 1786761701000L
    check(MediaDates.captured(JSONObject().put("created_at_ms", captured.toString())) == captured)
    for (bad in listOf("", "bad", "0", "-1", "9223372036854775807")) {
        check(MediaDates.captured(JSONObject().put("created_at_ms", bad)) == null)
    }
    check(MediaDates.captured(null) == null)
    val root = File(context.cacheDir, "dates-${UUID.randomUUID()}").apply { mkdirs() }
    val rows = mutableListOf<Uri>()
    val report = mutableListOf<String>()
    fun digest(bytes: ByteArray) = MessageDigest.getInstance("SHA-256").digest(bytes).toList()
    try {
        val bitmap = Bitmap.createBitmap(128, 96, Bitmap.Config.ARGB_8888).apply { eraseColor(Color.rgb(60, 130, 180)) }
        val jpeg = File(root, "plain.jpg").also { f -> f.outputStream().use { bitmap.compress(Bitmap.CompressFormat.JPEG, 95, it) } }
        val png = File(root, "plain.png").also { f -> f.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) } }
        bitmap.recycle()
        val heic = File(root, "plain.heic").also { f -> this.context.assets.open("dates.heic").use { i -> f.outputStream().use { i.copyTo(it) } } }
        val video = File(root, "plain.mp4").also { f -> this.context.assets.open("motion.mp4").use { i -> f.outputStream().use { i.copyTo(it) } } }
        val samples = mutableListOf(jpeg to "image/jpeg", png to "image/png", heic to "image/heic", video to "video/mp4")
        val realSample = File(context.filesDir, "date-sample.heic")
        if (realSample.exists()) samples += realSample to "image/heic"
        for ((source, mime) in samples) {
            for (fixed in listOf(false, true)) {
                val originalDigest = digest(source.readBytes())
                val delivery = if (fixed) MediaDates.prepare(context, source, mime, JSONObject().put("created_at_ms", captured.toString())) else source
                if (fixed && mime.startsWith("image/")) {
                    check(MediaDates.prepare(context, delivery, mime, JSONObject().put("created_at_ms", captured.toString())) == delivery) { "date_rewritten_on_replay" }
                    val beforePixels = android.graphics.ImageDecoder.decodeBitmap(android.graphics.ImageDecoder.createSource(source)) { decoder, _, _ -> decoder.allocator = android.graphics.ImageDecoder.ALLOCATOR_SOFTWARE }
                    val afterPixels = android.graphics.ImageDecoder.decodeBitmap(android.graphics.ImageDecoder.createSource(delivery)) { decoder, _, _ -> decoder.allocator = android.graphics.ImageDecoder.ALLOCATOR_SOFTWARE }
                    check(beforePixels.sameAs(afterPixels)) { "pixels_changed" }
                    beforePixels.recycle(); afterPixels.recycle()
                }
                val collection = if (mime.startsWith("video/")) MediaStore.Video.Media.EXTERNAL_CONTENT_URI else MediaStore.Images.Media.EXTERNAL_CONTENT_URI
                val uri = checkNotNull(resolver.insert(collection, ContentValues().apply {
                    put(MediaStore.MediaColumns.DISPLAY_NAME, "DateCheck-${UUID.randomUUID()}.${source.extension}")
                    put(MediaStore.MediaColumns.MIME_TYPE, mime)
                    put(MediaStore.MediaColumns.RELATIVE_PATH, if (mime.startsWith("video/")) "Movies/PhotoBridgeValidation/" else "Pictures/PhotoBridgeValidation/")
                    put(MediaStore.MediaColumns.IS_PENDING, 1)
                    put(MediaStore.MediaColumns.DATE_TAKEN, captured)
                }))
                rows += uri
                resolver.openOutputStream(uri, "wt")!!.use { o -> delivery.inputStream().use { it.copyTo(o) } }
                if (fixed) MediaDates.stampPending(context, uri, captured)
                val publish = if (fixed) MediaDates.values(captured) else ContentValues()
                publish.put(MediaStore.MediaColumns.IS_PENDING, 0)
                check(resolver.update(uri, publish, null, null) == 1)
                val path = resolver.query(uri, arrayOf(MediaStore.MediaColumns.DATA), null, null, null)!!.use { it.moveToFirst(); it.getString(0) }
                val before = resolver.query(uri, arrayOf(MediaStore.MediaColumns.DATE_TAKEN, MediaStore.MediaColumns.DATE_MODIFIED), null, null, null)!!.use { it.moveToFirst(); it.getLong(0) to it.getLong(1) }
                val done = CountDownLatch(1)
                MediaScannerConnection.scanFile(context, arrayOf(path), arrayOf(mime)) { _, _ -> done.countDown() }
                check(done.await(15, TimeUnit.SECONDS)) { "scan_timeout" }
                val after = resolver.query(uri, arrayOf(MediaStore.MediaColumns.DATE_TAKEN, MediaStore.MediaColumns.DATE_MODIFIED), null, null, null)!!.use { it.moveToFirst(); it.getLong(0) to it.getLong(1) }
                val actual = resolver.openInputStream(uri)!!.use { digest(it.readBytes()) }
                check(actual == digest(delivery.readBytes())) { "delivery_bytes_changed" }
                check(digest(source.readBytes()) == originalDigest) { "original_bytes_changed" }
                if (fixed) {
                    check(File(path).lastModified() / 1000 == captured / 1000) { "file_date_changed:$mime" }
                    check(after.second == captured / 1000) { "indexed_modified_wrong:$mime:$after" }
                    if (mime.startsWith("image/")) {
                        val exifDate = androidx.exifinterface.media.ExifInterface(delivery).dateTimeOriginal
                        check(exifDate == captured) { "embedded_capture_wrong:$mime:$exifDate" }
                        // Android 10's platform scanner ignores PNG eXIf; Google
                        // Photos readers still receive EXIF plus a matching file-date fallback.
                        check(after.first == captured || (mime == "image/png" && after.first == 0L)) { "indexed_capture_wrong:$mime:$after" }
                    }
                }
                report += "$mime real=${source == realSample} fixed=$fixed before=$before after=$after original=unchanged pixels=unchanged"
                if (delivery != source) delivery.delete()
                resolver.delete(uri, null, null); rows.remove(uri)
            }
        }
        return "PASS: " + report.joinToString("; ")
    } finally {
        rows.forEach { resolver.delete(it, null, null) }
        root.deleteRecursively()
    }
}
