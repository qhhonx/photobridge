package app.photobridge

import android.content.ContentUris
import android.content.Intent
import android.graphics.Bitmap
import android.net.Uri
import android.os.CancellationSignal
import android.provider.MediaStore
import android.text.TextUtils
import android.text.format.Formatter
import android.util.LruCache
import android.util.Size
import android.view.View
import android.view.ViewGroup
import android.widget.*
import androidx.lifecycle.lifecycleScope
import androidx.recyclerview.widget.DiffUtil
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.ListAdapter
import androidx.recyclerview.widget.RecyclerView
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import org.json.JSONObject

internal class HistoryPage(private val activity: MainActivity, private val model: HistoryModel, private val root: String) : LinearLayout(activity) {
    private val count: TextView
    private val message: TextView
    private val more: Button
    private val retry: Button
    private val list: RecyclerView
    private val adapter = TransferAdapter(activity)
    private val layout = LinearLayoutManager(activity)
    private var retrying = false
    init {
        orientation = VERTICAL
        setPadding(activity.dp(20), activity.dp(4), activity.dp(20), 0)
        val filters = LinearLayout(activity)
        addView(filters)
        fun filter(title: Int, choices: List<Pair<String, Int>>, selected: String, changed: (String) -> Unit) {
            val group = LinearLayout(activity).apply { orientation = VERTICAL }
            filters.addView(group, LayoutParams(0, -2, 1f))
            activity.label(group, activity.getString(title), 13, activity.secondaryColor())
            group.addView(Spinner(activity).apply {
                contentDescription = activity.getString(title)
                adapter = ArrayAdapter(activity, android.R.layout.simple_spinner_dropdown_item, choices.map { activity.getString(it.second) })
                setSelection(choices.indexOfFirst { it.first == selected }.coerceAtLeast(0))
                onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
                    override fun onNothingSelected(parent: AdapterView<*>?) {}
                    override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) { changed(choices[position].first) }
                }
            }, LayoutParams(-1, activity.dp(48)))
        }
        filter(R.string.history_status, listOf("all" to R.string.filter_all, "receiving" to R.string.filter_receiving,
            "processing" to R.string.filter_processing, "published" to R.string.filter_published, "failed" to R.string.filter_failed), model.state.value.filter) {
            if (it != model.state.value.filter) activity.lifecycleScope.launch { model.select(root, filter = it) }
        }
        filter(R.string.history_kind, listOf("all" to R.string.filter_all, "photo" to R.string.filter_photo,
            "video" to R.string.filter_video, "motion" to R.string.filter_motion), model.state.value.kind) {
            if (it != model.state.value.kind) activity.lifecycleScope.launch { model.select(root, kind = it) }
        }
        val senderPicker = activity.action(this, R.string.history_sender_all) {
            activity.lifecycleScope.launch {
                val peers = withContext(Dispatchers.IO) { runCatching { DeviceProfiles.read(activity).getJSONArray("peers") }.getOrNull() }
                val ids = mutableListOf<String?>(null,"unknown")
                val labels = mutableListOf(activity.getString(R.string.history_sender_all),activity.getString(R.string.history_sender_unknown))
                if (peers != null) for (i in 0 until peers.length()) {
                    val profile = peers.getJSONObject(i).getJSONObject("profile")
                    ids += profile.getString("id"); labels += profile.getString("name")
                }
                com.google.android.material.dialog.MaterialAlertDialogBuilder(activity).setTitle(R.string.history_sender)
                    .setSingleChoiceItems(labels.toTypedArray(),ids.indexOf(model.state.value.sender)) { dialog, which ->
                        dialog.dismiss()
                        activity.lifecycleScope.launch { model.select(root,sender=ids[which]) }
                    }.setNegativeButton(R.string.receiver_close,null).show()
            }
        }
        activity.lifecycleScope.launch {
            model.state.collect { state ->
                senderPicker.text = if (state.sender == null) activity.getString(R.string.history_sender_all)
                    else if (state.sender == "unknown") activity.getString(R.string.history_sender_unknown)
                    else state.items.firstOrNull()?.senderNames?.takeIf { it.isNotBlank() } ?: activity.getString(R.string.history_sender_filtered)
            }
        }
        count = activity.label(this, "", 13, activity.secondaryColor())
        val content = FrameLayout(activity)
        addView(content, LayoutParams(-1, 0, 1f))
        list = RecyclerView(activity).apply {
            id = R.id.receiver_history_list
            layoutManager = layout; adapter = this@HistoryPage.adapter
            itemAnimator = null
            addOnScrollListener(object : RecyclerView.OnScrollListener() {
                override fun onScrolled(recyclerView: RecyclerView, dx: Int, dy: Int) {
                    if (dy > 0 && layout.findLastVisibleItemPosition() >= this@HistoryPage.adapter.itemCount - 8)
                        activity.lifecycleScope.launch { model.more(root) }
                }
            })
        }
        content.addView(list, FrameLayout.LayoutParams(-1, -1))
        message = TextView(activity).apply {
            textSize = 16f; setTextColor(activity.secondaryColor()); gravity = android.view.Gravity.CENTER
            setPadding(activity.dp(24), activity.dp(24), activity.dp(24), activity.dp(24))
        }
        content.addView(message, FrameLayout.LayoutParams(-1, -1))
        more = activity.action(this, R.string.history_more) {
            activity.lifecycleScope.launch { if (model.state.value.failed) model.select(root) else model.more(root) }
        }
        val footer = LinearLayout(activity)
        addView(footer)
        retry = activity.action(footer, R.string.receiver_retry_processing) {
            if (!retrying) activity.lifecycleScope.launch {
                retrying = true
                val result = withContext(Dispatchers.IO) { runCatching { NativeBridge.request(org.json.JSONObject().put("op", "retry_processing")) } }
                retrying = false
                if (result.isFailure) Toast.makeText(activity, R.string.settings_start_first, Toast.LENGTH_LONG).show()
                else model.refreshVisible(root, firstVisible())
            }
        }
    }
    fun firstVisible() = layout.findFirstVisibleItemPosition().coerceAtLeast(0)
    fun render(state: HistoryState) {
        val first = layout.findFirstVisibleItemPosition()
        val anchor = adapter.currentList.getOrNull(first)?.id
        val top = layout.findViewByPosition(first)?.top ?: 0
        adapter.submitList(state.items) {
            if (first > 0 && anchor != null) {
                val index = adapter.currentList.indexOfFirst { it.id == anchor }
                if (index >= 0 && !list.isComputingLayout && list.scrollState == RecyclerView.SCROLL_STATE_IDLE) layout.scrollToPositionWithOffset(index, top)
            }
        }
        count.text = activity.getString(R.string.history_count, state.items.size, state.total)
        message.visibility = if (state.items.isEmpty()) View.VISIBLE else View.GONE
        message.setText(when { state.loading -> R.string.history_loading; state.failed -> R.string.settings_failed; else -> R.string.history_empty })
        more.visibility = if (state.nextCursor != null || state.failed) View.VISIBLE else View.GONE
        more.isEnabled = !state.loading
        more.setText(if (state.failed) R.string.history_retry else R.string.history_more)
        retry.visibility = if (ReceiverState.snapshot.value.failed > 0 || state.items.any { it.processing == "failed" }) View.VISIBLE else View.GONE
    }
}

private data class Thumbnail(val bitmap: Bitmap, val uri: Uri)
private class TransferAdapter(private val activity: MainActivity) : ListAdapter<HistoryItem, TransferAdapter.Holder>(object : DiffUtil.ItemCallback<HistoryItem>() {
    override fun areItemsTheSame(old: HistoryItem, new: HistoryItem) = old.id == new.id
    override fun areContentsTheSame(old: HistoryItem, new: HistoryItem) = old == new
}) {
    private val permits = Semaphore(2)
    private val cache = object : LruCache<String, Thumbnail>(12 * 1024 * 1024) {
        override fun sizeOf(key: String, value: Thumbnail) = value.bitmap.allocationByteCount
    }
    init { stateRestorationPolicy = RecyclerView.Adapter.StateRestorationPolicy.PREVENT_WHEN_EMPTY }
    inner class Holder(val row: LinearLayout, val image: ImageView, val name: TextView, val status: TextView,
        val progress: ProgressBar, val size: TextView, val sender: TextView) : RecyclerView.ViewHolder(row) {
        var work: Job? = null
        var signal: CancellationSignal? = null
        var itemID: String? = null
        fun cancel() { signal?.cancel(); work?.cancel(); signal = null; itemID = null }
    }
    override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): Holder {
        val row = LinearLayout(activity).apply { gravity = android.view.Gravity.CENTER_VERTICAL; setPadding(0, activity.dp(14), 0, activity.dp(14)) }
        row.layoutParams = RecyclerView.LayoutParams(-1, -2)
        val image = ImageView(activity).apply { scaleType = ImageView.ScaleType.CENTER_CROP; importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO }
        row.addView(image, LinearLayout.LayoutParams(activity.dp(60), activity.dp(60)).apply { marginEnd = activity.dp(14) })
        val body = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL }
        row.addView(body, LinearLayout.LayoutParams(0, -2, 1f))
        val name = TextView(activity).apply { textSize = 16f; maxLines = 1; ellipsize = TextUtils.TruncateAt.END; includeFontPadding = false }
        body.addView(name)
        val sender = TextView(activity).apply { textSize = 12f; setTextColor(activity.secondaryColor()); maxLines=1; ellipsize=TextUtils.TruncateAt.END }
        body.addView(sender)
        val summary = LinearLayout(activity).apply { setPadding(0, activity.dp(7), 0, 0); gravity = android.view.Gravity.CENTER_VERTICAL }
        body.addView(summary)
        val status = TextView(activity).apply { textSize = 12f; setTextColor(activity.secondaryColor()); includeFontPadding = false }
        summary.addView(status, LinearLayout.LayoutParams(0, -2, 1f))
        val size = TextView(activity).apply { textSize = 12f; setTextColor(activity.secondaryColor()); includeFontPadding = false }
        summary.addView(size)
        val progress = ProgressBar(activity, null, android.R.attr.progressBarStyleHorizontal).apply { max = 1000 }
        body.addView(progress, LinearLayout.LayoutParams(-1, activity.dp(4)).apply { topMargin = activity.dp(8) })
        return Holder(row, image, name, status, progress, size, sender)
    }
    override fun onBindViewHolder(holder: Holder, position: Int) {
        val item = getItem(position)
        holder.cancel(); holder.itemID = item.id
        holder.name.text = item.filename
        holder.sender.text = activity.getString(R.string.history_from, item.senderNames.ifBlank { activity.getString(R.string.history_sender_unknown) })
        holder.status.text = activity.getString(if (item.originalsReleased && item.releaseReason == "gallery") R.string.history_relay_reclaimed else if (item.originalsReleased) R.string.history_archived else if (item.processing == "complete") R.string.filter_published else item.statusLabel)
        holder.progress.visibility = if (item.receipt == "received") View.GONE else View.VISIBLE
        holder.size.text = if (item.receipt == "received") Formatter.formatFileSize(activity, item.totalBytes)
            else "${Formatter.formatFileSize(activity, item.confirmedBytes)} / ${Formatter.formatFileSize(activity, item.totalBytes)}"
        holder.progress.progress = (item.confirmedBytes.toDouble() / item.totalBytes.coerceAtLeast(1) * 1000).toInt()
        holder.image.setPadding(activity.dp(14), activity.dp(14), activity.dp(14), activity.dp(14))
        holder.image.setBackgroundColor(activity.themeColor(com.google.android.material.R.attr.colorSurfaceVariant))
        holder.image.setImageResource(if (item.kind == "video") R.drawable.ic_video else if (item.kind == "motion") R.drawable.ic_motion else R.drawable.ic_photo)
        holder.row.setOnClickListener(null)
        if (item.processing != "complete") return
        val signal = CancellationSignal()
        holder.signal = signal
        holder.work = activity.lifecycleScope.launch {
            val thumbnail = cache.get(item.id) ?: withContext(Dispatchers.IO) {
                permits.withPermit {
                    currentCoroutineContext().ensureActive()
                    runCatching {
                        val resolver = activity.contentResolver
                        val evidence = NativeBridge.request(JSONObject().put("op", "gallery_evidence").put("id", item.id)) as JSONObject
                        val stored = evidence.optJSONObject("copy")?.let { Uri.parse(it.getString("locator")) }
                        val uri = if (stored?.scheme == "content" && stored.authority == "media") stored else {
                            // Pre-evidence receiver data can still use the legacy name.
                            val collection = if (item.kind == "video") MediaStore.Video.Media.EXTERNAL_CONTENT_URI else MediaStore.Images.Media.EXTERNAL_CONTENT_URI
                            resolver.query(collection, arrayOf(MediaStore.MediaColumns._ID),
                                "${MediaStore.MediaColumns.DISPLAY_NAME} LIKE ? AND ${MediaStore.MediaColumns.RELATIVE_PATH}=? AND ${MediaStore.MediaColumns.IS_PENDING}=0",
                                arrayOf("PB_${item.id}.%", "DCIM/PhotoBridge/"), null, signal)?.use { cursor ->
                                if (cursor.moveToFirst()) ContentUris.withAppendedId(collection, cursor.getLong(0)) else null
                            }
                        } ?: return@withPermit null
                        Thumbnail(resolver.loadThumbnail(uri, Size(160, 160), signal), uri).also { cache.put(item.id, it) }
                    }.getOrNull()
                }
            }
            if (holder.itemID == item.id && thumbnail != null) {
                holder.image.setPadding(0, 0, 0, 0)
                holder.image.setImageBitmap(thumbnail.bitmap)
                holder.row.setOnClickListener {
                    runCatching { activity.startActivity(Intent(Intent.ACTION_VIEW).setDataAndType(thumbnail.uri,
                        if (item.kind == "video") "video/*" else "image/*").addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)) }
                        .onFailure { Toast.makeText(activity, R.string.history_view_unavailable, Toast.LENGTH_SHORT).show() }
                }
            }
        }
    }
    override fun onViewRecycled(holder: Holder) { holder.cancel(); holder.image.setImageDrawable(null) }
}
