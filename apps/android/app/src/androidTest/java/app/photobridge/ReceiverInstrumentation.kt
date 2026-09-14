package app.photobridge

import android.app.Activity
import android.app.Instrumentation
import android.graphics.Bitmap
import android.graphics.Color
import android.os.Bundle
import android.provider.MediaStore
import kotlinx.coroutines.runBlocking
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.UUID

/** Real Android JNI, TLS, codec and MediaStore test with synthetic media only. */
class ReceiverInstrumentation : Instrumentation() {
    private var arguments = Bundle()
    override fun onCreate(arguments: Bundle?) { this.arguments = arguments ?: Bundle(); super.onCreate(arguments); start() }
    override fun onStart() {
        if (arguments.getString("mode") == "media_dates") {
            val result = runCatching { checkMediaDates() }
            finish(if (result.isSuccess) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
                putString("result", result.getOrElse { "FAIL: ${it.stackTraceToString()}" })
            })
            return
        }
        if (arguments.getString("mode") == "cleanup_unlock") {
            val result = runCatching { checkCleanupUnlock() }
            finish(if (result.isSuccess) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
                putString("result", result.getOrElse { "FAIL: ${it.stackTraceToString()}" })
            })
            return
        }
        if (arguments.getString("mode") == "app_updates") {
            val result = runCatching { checkAppUpdates(arguments) }
            finish(if (result.isSuccess) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
                putString("result", result.getOrElse { "FAIL: ${it.stackTraceToString()}" })
            })
            return
        }
        if (arguments.getString("mode") == "receiver_discovery") {
            val result = runCatching { checkReceiverDiscovery() }
            finish(if (result.isSuccess) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
                putString("result", result.getOrElse { "FAIL: ${it.javaClass.simpleName}: ${it.message}" })
            })
            return
        }
        if (arguments.getString("mode") == "receiver_help") {
            val result = runCatching { checkReceiverHelp(arguments) }
            finish(if (result.isSuccess) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
                putString("result", result.getOrElse { "FAIL: ${it.javaClass.simpleName}: ${it.message}" })
            })
            return
        }
        if (arguments.getString("mode") == "photos_probe") {
            val result = runCatching { checkPhotosProbe() }
            finish(if (result.isSuccess) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
                putString("result", result.getOrElse { "FAIL: ${it.javaClass.simpleName}: ${it.message}" })
            })
            return
        }
        if (arguments.getString("mode") == "storage_ui") {
            val result = runCatching { checkStorageUI(arguments) }
            finish(if (result.isSuccess) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
                putString("result", result.getOrElse { "FAIL: ${it.javaClass.simpleName}: ${it.message}" })
            })
            return
        }
        val results = Bundle()
        var resultCode = Activity.RESULT_CANCELED
        val root = File(targetContext.filesDir, "integration-${UUID.randomUUID()}").apply { mkdirs() }
        val publishedIds = mutableListOf<String>()
        try {
            runBlocking {
                val galleryBefore = GalleryInventory.read(targetContext)
                val photo = File(root, "fixture.jpg")
                val bitmap = Bitmap.createBitmap(128, 96, Bitmap.Config.ARGB_8888).apply { eraseColor(Color.rgb(24, 120, 180)) }
                photo.outputStream().use { check(bitmap.compress(Bitmap.CompressFormat.JPEG, 95, it)) }; bitmap.recycle()
                val movie = File(root, "fixture.mp4")
                context.assets.open("motion.mp4").use { input -> movie.outputStream().use { input.copyTo(it) } }
                val pairing = NativeBridge.request(JSONObject().put("op", "start_receiver").put("root", File(root, "receiver").path)
                    .put("listen", "127.0.0.1:38484").put("capacity", 32 * 1024 * 1024)) as JSONObject
                NativeBridge.request(JSONObject().put("op", "open_sender").put("root", File(root, "sender").path))
                fun profile(directory: File, name: String? = null): JSONObject {
                    val request = JSONObject().put("op", "device_status").put("root", directory.path).put("language", "en")
                    if (name != null) request.put("name", name)
                    return NativeBridge.request(request) as JSONObject
                }
                val receiverDirectory = File(root, "receiver/store")
                val senderDirectory = File(root, "sender")
                val receiverProfile = profile(receiverDirectory, "Amber Otter").getJSONObject("device")
                val senderProfile = profile(senderDirectory, "Moonlit Cedar").getJSONObject("device")
                val exchanged = NativeBridge.request(JSONObject().put("op", "exchange_device").put("pairing", pairing)) as JSONObject
                check(exchanged.getString("name") == "Amber Otter") { "receiver_name_missing" }
                val knownSenders = profile(receiverDirectory).getJSONArray("peers")
                check(knownSenders.length() == 1 && knownSenders.getJSONObject(0).getJSONObject("profile").getString("name") == "Moonlit Cedar") { "sender_name_missing" }
                check(runCatching { profile(senderDirectory, "\n") }.isFailure) { "invalid_name_accepted" }
                check(profile(senderDirectory).getJSONObject("device").getString("name") == "Moonlit Cedar") { "invalid_rename_changed_name" }
                check(profile(senderDirectory, "Kitchen Mac").getJSONObject("device").getString("id") == senderProfile.getString("id")) { "rename_changed_identity" }
                NativeBridge.request(JSONObject().put("op", "exchange_device").put("pairing", pairing))
                check(profile(receiverDirectory).getJSONArray("peers").getJSONObject(0).getJSONObject("profile").getString("name") == "Kitchen Mac") { "rename_not_propagated" }
                for (kind in listOf("photo", "video", "motion", "burst-primary", "burst-secondary")) {
                    val resources = JSONArray()
                    fun resource(file: File, role: String, mime: String) = JSONObject().put("role", role).put("filename", file.name).put("media_type", mime).put("path", file.path)
                    if (kind == "video") resources.put(resource(movie, "video", "video/mp4"))
                    else resources.put(resource(photo, "photo", "image/jpeg"))
                    if (kind == "motion") resources.put(resource(movie, "paired_video", "video/mp4"))
                    val metadata = if (kind.startsWith("burst-")) NativeBridge.request(JSONObject().put("op", "burst_metadata").put("identifier", root.name).put("primary", kind == "burst-primary")) as JSONObject else JSONObject()
                    metadata.put("created_at_ms", "1786761701000")
                    NativeBridge.request(JSONObject().put("op", "enqueue").put("receiver_id", pairing.getString("receiver_id"))
                        .put("source_id", "fixture-$kind-${root.name}").put("revision", "1").put("kind", if (kind.startsWith("burst-")) "photo" else kind).put("metadata", metadata).put("resources", resources))
                    val job = NativeBridge.request(JSONObject().put("op", "run_sender").put("pairing", pairing)) as JSONObject
                    check(job.getString("state") == "received") { "receipt_missing" }
                }
                val items = NativeBridge.request(JSONObject().put("op", "publications")) as JSONArray
                check(items.length() == 5) { "asset_count" }
                for (index in 0 until items.length()) {
                    val item = items.getJSONObject(index)
                    val id = item.getString("id")
                    publishedIds += id
                    MediaPublisher.publish(targetContext, item)
                    // Publication replay must find the same MediaStore row.
                    MediaPublisher.publish(targetContext, item)
                    NativeBridge.request(JSONObject().put("op", "processed").put("id", id).put("success", true))
                }
                check((NativeBridge.request(JSONObject().put("op", "publications")) as JSONArray).length() == 0) { "processing_incomplete" }
                var total = 0
                for (collection in listOf(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, MediaStore.Video.Media.EXTERNAL_CONTENT_URI)) {
                    targetContext.contentResolver.query(collection, arrayOf(MediaStore.MediaColumns._ID, MediaStore.MediaColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
                        while (cursor.moveToNext()) if (publishedIds.any { cursor.getString(1).startsWith("PB_$it") }) total++
                    }
                }
                check(total == 5) { "publication_duplicate_or_missing" }
                var burstFrames = 0
                var primaryFrames = 0
                val expectedBurst = NativeBridge.request(JSONObject().put("op", "burst_metadata").put("identifier", root.name).put("primary", true)) as JSONObject
                val images = MediaStore.Images.Media.EXTERNAL_CONTENT_URI
                targetContext.contentResolver.query(images, arrayOf(MediaStore.MediaColumns._ID, MediaStore.MediaColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
                    while (cursor.moveToNext()) if (publishedIds.any { cursor.getString(1).startsWith("PB_$it") }) {
                        val uri = android.content.ContentUris.withAppendedId(images, cursor.getLong(0))
                        targetContext.contentResolver.openInputStream(uri)?.use { input ->
                            val exif = androidx.exifinterface.media.ExifInterface(input)
                            check(exif.getAttribute(androidx.exifinterface.media.ExifInterface.TAG_DATETIME_ORIGINAL) == "2026:08:15 02:41:41") { "publication_date_missing" }
                            val xmp = exif.getAttribute(androidx.exifinterface.media.ExifInterface.TAG_XMP) ?: ""
                            if (xmp.contains(expectedBurst.getString("burst_group_ref"))) {
                                burstFrames++
                                if (xmp.contains("GCamera:BurstPrimary=\"1\"")) primaryFrames++
                            }
                        }
                    }
                }
                check(burstFrames == 2 && primaryFrames == 1) { "burst_metadata_missing_or_ambiguous" }

                val gallery = GalleryInventory.read(targetContext)
                check(gallery.readyCount == galleryBefore.readyCount + 5 && gallery.readyBytes > galleryBefore.readyBytes) { "gallery_usage_missing_or_duplicate" }
                val pendingID = UUID.randomUUID().toString().replace("-", "") + UUID.randomUUID().toString().replace("-", "")
                publishedIds += pendingID
                val pendingURI = checkNotNull(targetContext.contentResolver.insert(images, android.content.ContentValues().apply {
                    put(MediaStore.MediaColumns.DISPLAY_NAME, "PB_${pendingID}.jpg")
                    put(MediaStore.MediaColumns.MIME_TYPE, "image/jpeg")
                    put(MediaStore.MediaColumns.RELATIVE_PATH, "DCIM/PhotoBridge/")
                    put(MediaStore.MediaColumns.IS_PENDING, 1)
                }))
                val pendingUsage = GalleryInventory.read(targetContext)
                check(pendingUsage.readyCount == gallery.readyCount && pendingUsage.pendingCount == gallery.pendingCount + 1) { "pending_gallery_usage_incorrect" }
                targetContext.contentResolver.delete(pendingURI, null, null)
                check(GalleryInventory.read(targetContext) == gallery) { "gallery_removal_not_reflected" }
                val receiverRoot = File(root, "receiver").path
                val originalUsage = NativeBridge.request(JSONObject().put("op", "receiver_storage_usage").put("root", receiverRoot)) as JSONObject
                check(originalUsage.getLong("ready_bytes") == photo.length() + movie.length() && originalUsage.getLong("partial_bytes") == 0L) { "original_usage_not_deduplicated" }
                NativeBridge.request(JSONObject().put("op", "stop_receiver"))
                check(!(NativeBridge.request(JSONObject().put("op", "receiver_status")) as JSONObject).getBoolean("running"))
                // The aborted listener releases its writer on the next runtime turn.
                var archiveReady = false
                repeat(40) {
                    if (!archiveReady) {
                        archiveReady = runCatching { NativeBridge.request(JSONObject().put("op", "archive_batch").put("root", receiverRoot)) }.isSuccess
                        if (!archiveReady) kotlinx.coroutines.delay(50)
                    }
                }
                check(archiveReady) { "offline_archive_unavailable" }
                val archiveFile = File(root, "originals.zip")
                val archive = OriginalArchive.export(targetContext, android.net.Uri.fromFile(archiveFile), receiverRoot)
                check(archive.ids.size == 5) { "archive_count" }
                archiveFile.writeBytes(byteArrayOf(0, 1, 2))
                check(runCatching { OriginalArchive.verify(targetContext, archive) }.isFailure) { "tampered_archive_accepted" }
                check((NativeBridge.request(JSONObject().put("op", "archive_batch").put("root", receiverRoot)) as JSONArray).length() == 5) { "premature_reclamation" }
                val verified = OriginalArchive.export(targetContext, android.net.Uri.fromFile(archiveFile), receiverRoot)
                val released = NativeBridge.request(JSONObject().put("op", "release_archived").put("root", verified.receiverRoot).put("ids", JSONArray(verified.ids)).put("verified", JSONArray(verified.resources.toList()))) as JSONObject
                check(released.getLong("bytes") > 0) { "reclamation_missing" }
                check((NativeBridge.request(JSONObject().put("op", "archive_batch").put("root", receiverRoot)) as JSONArray).length() == 0)
                val overview = NativeBridge.request(JSONObject().put("op", "receiver_settings").put("root", receiverRoot)) as JSONObject
                check(overview.getJSONObject("counts").getInt("received") == 5 && overview.getLong("used_bytes") == 0L) { "receipt_or_budget_lost" }
                val afterReclaim = NativeBridge.request(JSONObject().put("op", "receiver_storage_usage").put("root", receiverRoot)) as JSONObject
                check(afterReclaim.getLong("ready_bytes") == 0L && afterReclaim.getLong("partial_bytes") == 0L) { "reclaimed_usage_retained" }
                check(GalleryInventory.read(targetContext) == gallery) { "original_reclamation_changed_gallery" }
                val restarted = NativeBridge.request(JSONObject().put("op", "start_receiver").put("root", receiverRoot)
                    .put("listen", "127.0.0.1:38484").put("capacity", 32 * 1024 * 1024)) as JSONObject
                check(restarted.getString("receiver_id") == pairing.getString("receiver_id")) { "identity_changed_after_archive" }
                check(profile(receiverDirectory).getJSONObject("device").toString() == receiverProfile.toString()) { "profile_changed_after_restart" }
                check(profile(senderDirectory).getJSONArray("peers").getJSONObject(0).getJSONObject("profile").getString("name") == "Amber Otter") { "offline_peer_lost" }
                check((NativeBridge.request(JSONObject().put("op", "receiver_overview")) as JSONObject).getInt("received") == 5)
                check(NativeBridge.request(JSONObject().put("op", "run_sender").put("pairing", pairing)) == JSONObject.NULL) { "received_items_requeued" }
                checkRelayRetention(root, photo, movie, restarted, publishedIds)
                results.putString("result", "PASS: opt-in relay, verified derived copies, changed/missing copy preservation and durable receipts; original/gallery space separation, deduplicated originals, pending publication and reclamation; two grouped burst frames, one primary, original retention; durable names, bidirectional profile exchange, rename and invalid-name rejection; offline verified archive, tamper rejection, original reclamation, receipt retention; Kotlin JNI, paired TLS, photo/video/motion receipt, codec publication and dedupe")
            }
            resultCode = Activity.RESULT_OK
        } catch (error: Throwable) {
            results.putString("result", "FAIL: ${error.javaClass.simpleName}: ${error.message?.take(120)}")
        } finally {
            runCatching { NativeBridge.request(JSONObject().put("op", "stop_receiver")) }
            // Only remove this test's newly-created media, never user originals.
            for (collection in listOf(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, MediaStore.Video.Media.EXTERNAL_CONTENT_URI)) {
                for (id in publishedIds) targetContext.contentResolver.delete(collection, "${MediaStore.MediaColumns.DISPLAY_NAME} LIKE ?", arrayOf("PB_$id.%"))
            }
            root.deleteRecursively()
        }
        // finish() may terminate the instrumentation process before finally runs.
        // Report completion only after our fixture cleanup has finished.
        finish(resultCode, results)
    }
}
