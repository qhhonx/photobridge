package app.photobridge

import android.content.ContentUris
import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import org.json.JSONObject
import java.io.File
import java.io.InputStream
import java.security.MessageDigest

internal data class GalleryCopy(val locator: String, val sha256: String, val size: Long) {
    fun json() = JSONObject().put("locator", locator).put("sha256", sha256).put("size", size)
    companion object { fun parse(value: JSONObject) = GalleryCopy(value.getString("locator"), value.getString("sha256"), value.getLong("size")) }
}
internal object MediaPublisher {
    suspend fun publish(context: Context, item: JSONObject, existingOnly: Boolean = false): GalleryCopy {
        val evidence = NativeBridge.request(JSONObject().put("op", "gallery_evidence").put("id", item.getString("id"))) as JSONObject
        evidence.optJSONObject("copy")?.let { stored ->
            val copy = GalleryCopy.parse(stored)
            try { return verify(context, copy) }
            catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
            catch (error: Exception) {
                // An incomplete write can resume. A complete but changed/missing
                // copy must never be overwritten merely to reclaim originals.
                if (evidence.getBoolean("confirmed") || !ownedPending(context, Uri.parse(copy.locator))) throw error
            }
        }
        val asset = item.getJSONObject("asset")
        if (asset.getString("kind") == "motion") return MotionProcessor.publish(context, item, existingOnly)
        if (asset.optJSONObject("metadata")?.has("burst_group_ref") == true) return BurstProcessor.publish(context, item, existingOnly)
        val resource = asset.getJSONArray("resources").getJSONObject(0)
        val filename = resource.getString("filename")
        val extension = filename.substringAfterLast('.', "").let { if (it.isEmpty()) "" else ".$it" }
        val source = File(item.getJSONObject("resources").getString(resource.getString("sha256")))
        val dated = MediaDates.prepare(context, source, resource.getString("media_type"), asset.optJSONObject("metadata"))
        try {
            return publishFile(context, dated, "PB_${item.getString("id")}$extension", resource.getString("media_type"),
                asset.optJSONObject("metadata"), existingOnly, if (dated == source) resource.getString("sha256") else null)
        } finally { if (dated != source) dated.delete() }
    }
    private fun ownedPending(context: Context, uri: Uri): Boolean {
        if (uri.scheme != "content" || uri.authority != "media") return false
        return context.contentResolver.query(uri, arrayOf(MediaStore.MediaColumns.IS_PENDING, MediaStore.MediaColumns.OWNER_PACKAGE_NAME), null, null, null)?.use {
            it.moveToFirst() && it.getInt(0) == 1 && it.getString(1) == context.packageName
        } ?: false
    }
    private suspend fun hash(input: InputStream, limit: Long, write: ((ByteArray, Int) -> Unit)? = null): Pair<String, Long> {
        val digest = MessageDigest.getInstance("SHA-256"); val buffer = ByteArray(65_536); var bytes = 0L
        while (true) {
            currentCoroutineContext().ensureActive()
            val count = input.read(buffer); if (count < 0) break
            check(count.toLong() <= limit - bytes) { "gallery_copy_changed" }
            digest.update(buffer, 0, count); bytes += count; write?.invoke(buffer, count)
        }
        return digest.digest().joinToString("") { "%02x".format(it) } to bytes
    }
    /** Reopen the owned, ready MediaStore item; indexed size alone is not proof. */
    suspend fun verify(context: Context, copy: GalleryCopy): GalleryCopy {
        val uri = Uri.parse(copy.locator)
        check(uri.scheme == "content" && uri.authority == "media" && copy.size > 0 && copy.sha256.matches(Regex("[0-9a-f]{64}"))) { "gallery_copy_changed" }
        checkReady(context, uri)
        val actual = checkNotNull(context.contentResolver.openInputStream(uri)).use { hash(it, copy.size) }
        check(actual.first == copy.sha256 && actual.second == copy.size) { "gallery_copy_changed" }
        checkReady(context, uri)
        return copy
    }
    private fun checkReady(context: Context, uri: Uri) {
        val columns = mutableListOf(MediaStore.MediaColumns.IS_PENDING, MediaStore.MediaColumns.OWNER_PACKAGE_NAME, MediaStore.MediaColumns.RELATIVE_PATH)
        if (Build.VERSION.SDK_INT >= 30) columns += MediaStore.MediaColumns.IS_TRASHED
        checkNotNull(context.contentResolver.query(uri, columns.toTypedArray(), null, null, null)).use { cursor ->
            check(cursor.moveToFirst() && cursor.getInt(0) == 0 && cursor.getString(1) == context.packageName && cursor.getString(2) == "DCIM/PhotoBridge/" && (Build.VERSION.SDK_INT < 30 || cursor.getInt(3) == 0)) { "gallery_copy_missing" }
        }
    }
    suspend fun publishFile(context: Context, source: File, name: String, mime: String, metadata: JSONObject?, existingOnly: Boolean = false, originalHash: String? = null): GalleryCopy {
        val resolver = context.contentResolver
        val collection = if (mime.startsWith("video/")) MediaStore.Video.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
            else MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        val relative = "DCIM/PhotoBridge/"
        val captured = MediaDates.captured(metadata)
        val columns = arrayOf(MediaStore.MediaColumns._ID, MediaStore.MediaColumns.IS_PENDING, MediaStore.MediaColumns.OWNER_PACKAGE_NAME)
        val selection = "${MediaStore.MediaColumns.DISPLAY_NAME}=? AND ${MediaStore.MediaColumns.RELATIVE_PATH}=?"
        var destination: Uri? = null; var ready = false
        checkNotNull(resolver.query(collection, columns, selection, arrayOf(name, relative), null)).use { cursor ->
            check(cursor.count <= 1) { "gallery_copy_ambiguous" }
            if (cursor.moveToFirst()) {
                check(cursor.getString(2) == context.packageName) { "gallery_copy_changed" }
                destination = ContentUris.withAppendedId(collection, cursor.getLong(0)); ready = cursor.getInt(1) == 0
            }
        }
        if (existingOnly) check(destination != null && ready) { "gallery_copy_missing" }
        val size = source.length(); check(size > 0) { "gallery_copy_missing" }
        val expected = originalHash ?: source.inputStream().use { hash(it, size).first }
        if (ready) return verify(context, GalleryCopy(requireNotNull(destination).toString(), expected, size))
        if (destination == null) {
            val values = ContentValues().apply {
                put(MediaStore.MediaColumns.DISPLAY_NAME, name); put(MediaStore.MediaColumns.MIME_TYPE, mime)
                put(MediaStore.MediaColumns.RELATIVE_PATH, relative); put(MediaStore.MediaColumns.IS_PENDING, 1)
                putAll(MediaDates.values(captured))
            }
            destination = checkNotNull(resolver.insert(collection, values)) { "publication_failed" }
        }
        val uri = requireNotNull(destination)
        val copy = GalleryCopy(uri.toString(), expected, size)
        val assetID = name.removePrefix("PB_").take(64)
        NativeBridge.request(JSONObject().put("op", "prepare_gallery").put("id", assetID).put("copy", copy.json()))
        val copied = source.inputStream().use { input ->
            checkNotNull(resolver.openOutputStream(uri, "wt")).use { output -> hash(input, size) { buffer, count -> output.write(buffer, 0, count) } }
        }
        check(copied.first == expected && copied.second == size) { "gallery_copy_changed" }
        MediaDates.stampPending(context, uri, captured)
        check(resolver.update(uri, MediaDates.values(captured).apply { put(MediaStore.MediaColumns.IS_PENDING, 0) }, null, null) == 1) { "publication_failed" }
        return verify(context, copy)
    }
}
