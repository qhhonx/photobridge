package app.photobridge

import android.content.Context
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.net.Uri
import androidx.exifinterface.media.ExifInterface
import androidx.media3.common.MediaItem
import androidx.media3.common.MimeTypes
import androidx.media3.transformer.*
import kotlinx.coroutines.*
import org.json.JSONObject
import java.io.File
import java.io.IOException
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/** Android codec adapter; Rust owns the container and transfer state. */
internal object MotionProcessor {
    suspend fun publish(context: Context, item: JSONObject, existingOnly: Boolean = false): GalleryCopy {
        val asset = item.getJSONObject("asset")
        val resources = asset.getJSONArray("resources")
        val paths = item.getJSONObject("resources")
        val still = File(paths.getString(resources.getJSONObject(0).getString("sha256")))
        val video = File(paths.getString(resources.getJSONObject(1).getString("sha256")))
        val work = File(context.cacheDir, "motion-${item.getString("id")}")
        check(work.exists() || work.mkdirs()) { "storage" }
        val jpeg = File(work, "still.jpg")
        val mp4 = File(work, "motion.mp4")
        val motion = File(work, "output.jpg")
        try {
            prepareStill(still, jpeg)
            if (mp4.exists()) check(mp4.delete()) { "storage" }
            try { transcode(context, video, mp4) }
            catch (_: TimeoutCancellationException) { throw IOException("video_conversion_timeout") }
            val partial = File(work, "output.motion.partial")
            if (partial.exists()) check(partial.delete()) { "storage" }
            val dated = MediaDates.prepare(context, jpeg, "image/jpeg", asset.optJSONObject("metadata"))
            try {
                NativeBridge.request(JSONObject().put("op", "package_motion").put("jpeg", dated.path).put("mp4", mp4.path).put("output", motion.path).put("metadata", asset.optJSONObject("metadata") ?: JSONObject()))
            } finally { if (dated != jpeg) dated.delete() }
            return MediaPublisher.publishFile(context, motion, "PB_${item.getString("id")}.jpg", "image/jpeg", asset.optJSONObject("metadata"), existingOnly)
        } finally {
            listOf(jpeg, mp4, motion).forEach { it.delete() }
            work.delete()
        }
    }
    fun prepareStill(still: File, jpeg: File) {
        val bitmap = ImageDecoder.decodeBitmap(ImageDecoder.createSource(still)) { decoder, info, _ ->
            require(info.size.width.toLong() * info.size.height <= 50_000_000) { "unsupported" }
            decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
        }
        try { jpeg.outputStream().use { check(bitmap.compress(Bitmap.CompressFormat.JPEG, 95, it)) { "image_conversion" } } }
        finally { bitmap.recycle() }
        copyExif(still, jpeg)
    }
    private fun copyExif(source: File, target: File) {
        val original = ExifInterface(source)
        val derived = ExifInterface(target)
        val tags = listOf(ExifInterface.TAG_DATETIME, ExifInterface.TAG_DATETIME_ORIGINAL, ExifInterface.TAG_DATETIME_DIGITIZED,
            ExifInterface.TAG_OFFSET_TIME_ORIGINAL, ExifInterface.TAG_MAKE, ExifInterface.TAG_MODEL, ExifInterface.TAG_EXPOSURE_TIME,
            ExifInterface.TAG_F_NUMBER, ExifInterface.TAG_PHOTOGRAPHIC_SENSITIVITY, ExifInterface.TAG_FOCAL_LENGTH,
            ExifInterface.TAG_GPS_LATITUDE, ExifInterface.TAG_GPS_LATITUDE_REF, ExifInterface.TAG_GPS_LONGITUDE,
            ExifInterface.TAG_GPS_LONGITUDE_REF, ExifInterface.TAG_GPS_ALTITUDE, ExifInterface.TAG_GPS_ALTITUDE_REF)
        tags.forEach { tag -> original.getAttribute(tag)?.let { derived.setAttribute(tag, it) } }
        // ImageDecoder applies orientation to pixels.
        derived.setAttribute(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL.toString())
        derived.saveAttributes()
    }
    private suspend fun transcode(context: Context, source: File, output: File) = withContext(Dispatchers.Main) {
        var transformer: Transformer? = null
        try {
            withTimeout(180_000) {
                suspendCancellableCoroutine<Unit> { continuation ->
                    transformer = Transformer.Builder(context).setVideoMimeType(MimeTypes.VIDEO_H264).setAudioMimeType(MimeTypes.AUDIO_AAC)
                        .addListener(object : Transformer.Listener {
                            override fun onCompleted(composition: Composition, result: ExportResult) {
                                if (continuation.isActive) continuation.resume(Unit)
                            }
                            override fun onError(composition: Composition, result: ExportResult, error: ExportException) {
                                if (continuation.isActive) continuation.resumeWithException(IOException("video_conversion"))
                            }
                        }).build()
                    transformer!!.start(EditedMediaItem.Builder(MediaItem.fromUri(Uri.fromFile(source))).build(), output.path)
                }
            }
        } finally {
            // Runs on the application's main looper even after coroutine cancellation.
            transformer?.cancel()
        }
    }
}
