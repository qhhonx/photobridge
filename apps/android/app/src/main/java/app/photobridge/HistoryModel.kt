package app.photobridge

import androidx.lifecycle.ViewModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.json.JSONObject

internal data class HistoryItem(val cursor: Long, val id: String, val filename: String, val kind: String,
    val totalBytes: Long, val confirmedBytes: Long, val receipt: String, val processing: String, val originalsReleased: Boolean, val releaseReason: String? = null, val senderNames: String = "") {
    val statusLabel: Int get() = when {
        receipt != "received" -> R.string.receiver_item_receiving
        processing == "complete" -> R.string.receiver_item_published
        processing == "failed" -> R.string.receiver_item_failed
        else -> R.string.receiver_item_received
    }
}
internal data class HistoryState(val items: List<HistoryItem> = emptyList(), val total: Int = 0,
    val nextCursor: Long? = null, val loading: Boolean = false, val failed: Boolean = false,
    val filter: String = "all", val kind: String = "all", val sender: String? = null)

/** Rust applies filters and cursor pagination. UI keeps only requested pages. */
internal class HistoryModel : ViewModel() {
    private val mutable = MutableStateFlow(HistoryState())
    val state = mutable.asStateFlow()
    private val requests = Mutex()
    private var generation = 0
    suspend fun select(root: String, filter: String = mutable.value.filter, kind: String = mutable.value.kind, sender: String? = mutable.value.sender) {
        generation++
        mutable.value = HistoryState(filter = filter, kind = kind, sender = sender)
        load(root, null, replace = true)
    }
    suspend fun more(root: String) {
        val before = mutable.value.nextCursor ?: return
        if (!mutable.value.loading) load(root, before, replace = false)
    }
    // Poll just the visible page, not every historical item loaded so far.
    suspend fun refreshVisible(root: String, firstVisible: Int) {
        val snapshot = mutable.value
        if (snapshot.loading) return
        val start = (firstVisible - 4).coerceAtLeast(0).coerceAtMost(snapshot.items.size)
        val before = if (start == 0) null else snapshot.items[start - 1].cursor
        load(root, before, replace = false, refresh = true)
    }
    private suspend fun load(root: String, before: Long?, replace: Boolean, refresh: Boolean = false) {
        val expected = generation
        requests.withLock {
            if (expected != generation) return
            val snapshot = mutable.value
            if (!refresh) mutable.value = snapshot.copy(loading = true, failed = false)
            val result = withContext(Dispatchers.IO) { runCatching {
                val query = JSONObject().put("op", "receiver_history").put("root", root)
                    .put("state", snapshot.filter).put("kind", snapshot.kind)
                if (snapshot.sender != null) query.put("sender_id",snapshot.sender)
                if (before != null) query.put("before", before)
                val data = NativeBridge.request(query) as JSONObject
                val rows = data.getJSONArray("items")
                val items = (0 until rows.length()).map { i ->
                    val row = rows.getJSONObject(i)
                    val peers = row.optJSONArray("senders")
                    val names = if (peers == null) "" else (0 until peers.length()).joinToString(" · ") { peers.getJSONObject(it).getJSONObject("profile").getString("name") }
                    HistoryItem(row.getLong("cursor"), row.getString("id"), row.getString("filename"), row.getString("kind"),
                        row.getLong("total_bytes"), row.getLong("confirmed_bytes"), row.getString("receipt"),
                        row.getString("processing"), row.getBoolean("originals_released"), row.optString("release_reason").takeIf { it == "gallery" || it == "archive" }, names)
                }
                Triple(items, data.getInt("total"), if (data.isNull("next_cursor")) null else data.getLong("next_cursor"))
            } }
            if (expected != generation) return
            result.onSuccess { (page, total, cursor) ->
                val merged = if (replace) page else {
                    // Remove rows in this refreshed range that no longer match the filter.
                    val floor = page.lastOrNull()?.cursor ?: 0
                    val retained = snapshot.items.filterNot { item ->
                        refresh && (before == null || item.cursor < before) && (cursor == null || item.cursor >= floor)
                    }
                    (retained.associateBy { it.id } + page.associateBy { it.id }).values.sortedByDescending { it.cursor }
                }
                val next = if (!refresh || snapshot.items.isEmpty() || before == snapshot.nextCursor) cursor
                    else if (cursor == null && (page.isEmpty() || merged.lastOrNull()?.cursor == page.last().cursor)) null
                    else snapshot.nextCursor
                mutable.value = snapshot.copy(items = merged, total = total, nextCursor = next, loading = false, failed = false)
            }.onFailure {
                mutable.value = snapshot.copy(loading = false, failed = true)
            }
        }
    }
}
