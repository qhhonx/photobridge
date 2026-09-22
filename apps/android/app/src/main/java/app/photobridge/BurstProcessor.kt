package app.photobridge

import android.content.Context
import org.json.JSONObject
import java.io.File

/** The receiver's delivery copy may change format; verified originals never do. */
internal object BurstProcessor {
    suspend fun publish(context: Context, item: JSONObject, existingOnly: Boolean = false, resumeLocator: String? = null): GalleryCopy {
        val asset = item.getJSONObject("asset")
        val resource = asset.getJSONArray("resources").getJSONObject(0)
        val original = File(item.getJSONObject("resources").getString(resource.getString("sha256")))
        val work = File(context.cacheDir, "burst-${item.getString("id")}")
        check(work.exists() || work.mkdirs()) { "storage" }
        val decoded = File(work, "still.jpg")
        val output = File(work, "output.jpg")
        val partial = File(work, "output.burst.partial")
        try {
            // JPEG pixels, EXIF, ICC and existing standard XMP remain untouched.
            // HEIC/other supported still formats use the native image decoder.
            val jpeg = if (resource.getString("media_type") == "image/jpeg") original
                else decoded.also { MotionProcessor.prepareStill(original, it) }
            if (partial.exists()) check(partial.delete()) { "storage" }
            val dated = MediaDates.prepare(context, jpeg, "image/jpeg", asset.getJSONObject("metadata"))
            try {
                NativeBridge.request(JSONObject().put("op", "package_burst").put("jpeg", dated.path)
                    .put("output", output.path).put("metadata", asset.getJSONObject("metadata")))
            } finally { if (dated != jpeg) dated.delete() }
            return MediaPublisher.publishFile(context, output, item, "image/jpeg", asset.getJSONObject("metadata"), existingOnly, resumeLocator = resumeLocator)
        } finally {
            listOf(decoded, output, partial).forEach { it.delete() }
            work.delete()
        }
    }
}
