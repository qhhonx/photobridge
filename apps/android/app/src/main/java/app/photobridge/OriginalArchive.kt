package app.photobridge

import android.app.AlertDialog
import android.content.Context
import android.net.Uri
import android.widget.Toast
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.withLock
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.InputStream
import java.security.MessageDigest
import java.util.zip.ZipEntry
import java.util.zip.ZipInputStream
import java.util.zip.ZipOutputStream

internal data class ArchiveEntry(val digest: String, val size: Long)
internal data class ArchiveTicket(val uri: Uri, val receiverRoot: String, val ids: List<String>, val entries: Map<String, ArchiveEntry>, val resources: Set<String>)
internal object OriginalArchive {
    private fun digest(bytes: ByteArray) = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
    private suspend fun copy(input: InputStream, limit: Long, write: (ByteArray, Int) -> Unit): ArchiveEntry {
        val hash = MessageDigest.getInstance("SHA-256"); val buffer = ByteArray(65536); var size = 0L
        while (true) {
            currentCoroutineContext().ensureActive()
            val count = input.read(buffer); if (count < 0) break
            check(count.toLong() <= limit - size) { "archive_integrity" }
            hash.update(buffer, 0, count); size += count; write(buffer, count)
        }
        return ArchiveEntry(hash.digest().joinToString("") { "%02x".format(it) }, size)
    }
    suspend fun export(context: Context, uri: Uri, receiverRoot: String = "${context.filesDir}/receiver"): ArchiveTicket {
        val items = NativeBridge.request(JSONObject().put("op", "archive_batch").put("root", receiverRoot)) as JSONArray
        check(items.length() > 0) { "archive_empty" }
        val entries = linkedMapOf<String, ArchiveEntry>(); val ids = mutableListOf<String>(); val digests = mutableSetOf<String>()
        checkNotNull(context.contentResolver.openOutputStream(uri, "wt")).use { output ->
            ZipOutputStream(output).use { zip ->
                for (index in 0 until items.length()) {
                    val item = items.getJSONObject(index); val id = item.getString("id"); ids += id
                    val asset = item.getJSONObject("asset"); val metadata = asset.toString().toByteArray(Charsets.UTF_8)
                    val manifest = "$id/manifest.json"; zip.putNextEntry(ZipEntry(manifest)); zip.write(metadata); zip.closeEntry()
                    entries[manifest] = ArchiveEntry(digest(metadata), metadata.size.toLong())
                    val resources = asset.getJSONArray("resources")
                    for (r in 0 until resources.length()) {
                        val resource = resources.getJSONObject(r); val hash = resource.getString("sha256")
                        val name = "$id/${resource.getString("role")}-${resource.getString("filename")}"
                        val expected = ArchiveEntry(hash, resource.getLong("size")); check(entries.put(name, expected) == null)
                        zip.putNextEntry(ZipEntry(name))
                        val actual = File(item.getJSONObject("resources").getString(hash)).inputStream().use { input -> copy(input, expected.size) { b, n -> zip.write(b, 0, n) } }
                        check(actual == expected) { "archive_integrity" }; zip.closeEntry(); digests += hash
                    }
                }
            }
        }
        val ticket = ArchiveTicket(uri, receiverRoot, ids, entries, digests)
        verify(context, ticket)
        NativeBridge.request(JSONObject().put("op", "record_event").put("receiver", true).put("root", receiverRoot).put("code", "original_archive_verified"))
        return ticket
    }
    suspend fun verify(context: Context, ticket: ArchiveTicket) {
        val remaining = ticket.entries.toMutableMap()
        checkNotNull(context.contentResolver.openInputStream(ticket.uri)).use { input ->
            ZipInputStream(input).use { zip ->
                while (true) {
                    val entry = zip.nextEntry ?: break
                    val expected = remaining.remove(entry.name) ?: error("archive_integrity")
                    check(copy(zip, expected.size) { _, _ -> } == expected) { "archive_integrity" }; zip.closeEntry()
                }
            }
        }
        check(remaining.isEmpty()) { "archive_integrity" }
    }
}

internal fun MainActivity.exportOriginals(uri: Uri) {
    lifecycleScope.launch {
        val dialog = AlertDialog.Builder(this@exportOriginals).setTitle(R.string.originals_exporting)
            .setMessage(R.string.originals_export_wait).setCancelable(false).setNegativeButton(R.string.receiver_close, null).create()
        val work = this.coroutineContext[Job]!!
        dialog.setOnShowListener { dialog.getButton(AlertDialog.BUTTON_NEGATIVE).setOnClickListener { work.cancel(); dialog.dismiss() } }
        dialog.show()
        val ticket = try {
            withContext(Dispatchers.IO) { ReceiverState.mediaOperations.withLock { OriginalArchive.export(this@exportOriginals, uri) } }
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (error: Exception) {
            val message = if (error.message in listOf("archive_empty", "not_found")) R.string.originals_export_empty else R.string.originals_export_failed
            Toast.makeText(this@exportOriginals, message, Toast.LENGTH_LONG).show(); null
        }
        finally { dialog.dismiss() }
        if (ticket != null) {
            AlertDialog.Builder(this@exportOriginals).setTitle(R.string.originals_verified)
                .setMessage(getString(R.string.originals_reclaim_confirmation, ticket.ids.size))
                .setNegativeButton(R.string.originals_keep, null)
                .setPositiveButton(R.string.originals_reclaim) { _, _ -> lifecycleScope.launch {
                    val result = withContext(Dispatchers.IO) { runCatching { ReceiverState.mediaOperations.withLock {
                        OriginalArchive.verify(this@exportOriginals, ticket)
                        NativeBridge.request(JSONObject().put("op", "release_archived").put("root", ticket.receiverRoot).put("ids", JSONArray(ticket.ids)).put("verified", JSONArray(ticket.resources.toList())))
                    } } }
                    Toast.makeText(this@exportOriginals, if (result.isSuccess) R.string.originals_reclaimed else R.string.originals_export_failed, Toast.LENGTH_LONG).show()
                } }.show()
        }
    }
}
