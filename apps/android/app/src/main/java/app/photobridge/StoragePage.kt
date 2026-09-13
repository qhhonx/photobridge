package app.photobridge

import android.os.SystemClock
import android.text.format.Formatter
import android.view.View
import android.widget.*
import androidx.lifecycle.lifecycleScope
import com.google.android.material.dialog.MaterialAlertDialogBuilder
import kotlinx.coroutines.*
import org.json.JSONObject

/** Storage has its own presentation and bounded refresh, independent of reception. */
internal class StoragePage(
    private val activity: MainActivity,
    private val receiverRoot: String,
    private val export: () -> Unit,
    private val totals: (JSONObject) -> Unit,
) {
    private lateinit var free: TextView
    private lateinit var originalBytes: TextView
    private lateinit var partial: TextView
    private lateinit var reservation: TextView
    private lateinit var budgetProgress: ProgressBar
    private lateinit var galleryBytes: TextView
    private lateinit var galleryPending: TextView
    private lateinit var updated: TextView
    private lateinit var retention: RetentionControls
    private var work: Job? = null
    private var lastRefresh = 0L
    private var pendingDetails = false
    val view: View = with(activity) { scrollPage { panel ->
        card(panel) { body ->
            val heading = LinearLayout(activity).apply { gravity = android.view.Gravity.CENTER_VERTICAL }
            body.addView(heading, LinearLayout.LayoutParams(-1, -2))
            label(heading, getString(R.string.storage_device), 16, secondaryColor()).layoutParams = LinearLayout.LayoutParams(0, -2, 1f)
            heading.addView(com.google.android.material.button.MaterialButton(activity, null, com.google.android.material.R.attr.borderlessButtonStyle).apply {
                setText(R.string.storage_refresh); isAllCaps = false; minimumHeight = dp(48)
                setOnClickListener { refresh(force = true) }
            }, LinearLayout.LayoutParams(-2, -2))
            free = label(body, getString(R.string.history_loading), 32)
            updated = label(body, "", 13, secondaryColor())
        }
        section(panel, R.string.storage_originals_title)
        card(panel) { body ->
            originalBytes = label(body, getString(R.string.history_loading), 24)
            partial = label(body, "", 14, secondaryColor())
            reservation = label(body, "", 14, secondaryColor())
            budgetProgress = ProgressBar(activity, null, android.R.attr.progressBarStyleHorizontal).apply { max = 1000 }
            body.addView(budgetProgress, LinearLayout.LayoutParams(-1, dp(6)).apply { bottomMargin = dp(12) })
            label(body, getString(R.string.storage_reserved_note), 13, secondaryColor())
            action(body, R.string.storage_limits) { showStorageControls(StorageSection.LIMITS) { refresh(force = true) } }
            action(body, R.string.originals_export, action = export)
        }
        section(panel, R.string.storage_gallery_title)
        card(panel) { body ->
            galleryBytes = label(body, getString(R.string.history_loading), 24)
            galleryPending = label(body, "", 14, secondaryColor())
            label(body, getString(R.string.storage_gallery_note), 14, secondaryColor())
            action(body, R.string.receiver_open_photos, action = ::openPhotos)
        }
        section(panel, R.string.storage_retention_title)
        card(panel) { body ->
            retention = RetentionControls(activity, body)
            action(body, R.string.storage_how_to_free) {
                MaterialAlertDialogBuilder(activity).setTitle(R.string.storage_how_to_free)
                    .setMessage(R.string.storage_cleanup_help).setPositiveButton(R.string.receiver_close, null).show()
            }
        }
    } }
    fun refresh(force: Boolean = false, includeDetails: Boolean = true) {
        if (work?.isActive == true) {
            if (force && includeDetails) pendingDetails = true
            return
        }
        if (!force && SystemClock.elapsedRealtime() - lastRefresh < 10_000) return
        lastRefresh = SystemClock.elapsedRealtime()
        work = activity.lifecycleScope.launch {
            val settings = withContext(Dispatchers.IO) { runCatching {
                NativeBridge.request(JSONObject().put("op", "receiver_settings").put("root", receiverRoot)) as JSONObject
            } }
            settings.onSuccess { data ->
                totals(data.getJSONObject("counts"))
                free.text = size(data.getLong("free_bytes"))
                val config = data.getJSONObject("settings")
                retention.render(config.optBoolean("receiver_relay", false))
                val used = data.getLong("used_bytes"); val budget = config.getLong("receiver_budget_bytes")
                reservation.text = activity.getString(R.string.storage_reservation, size(used), size(budget))
                budgetProgress.progress = (used.toDouble() / budget.coerceAtLeast(1) * 1000).toInt().coerceIn(0, 1000)
                budgetProgress.contentDescription = reservation.text
            }.onFailure {
                free.setText(R.string.storage_unavailable); reservation.setText(R.string.storage_unavailable)
                budgetProgress.progress = 0
            }
            if (!includeDetails) return@launch
            val originals = withContext(Dispatchers.IO) { runCatching {
                NativeBridge.request(JSONObject().put("op", "receiver_storage_usage").put("root", receiverRoot)) as JSONObject
            } }
            originals.onSuccess { data ->
                originalBytes.text = size(data.getLong("ready_bytes"))
                partial.text = activity.getString(R.string.storage_partial, size(data.getLong("partial_bytes")))
            }.onFailure { originalBytes.setText(R.string.storage_unavailable); partial.text = "" }
            val gallery = withContext(Dispatchers.IO) { runCatching { GalleryInventory.read(activity) } }
            gallery.onSuccess { data ->
                galleryBytes.text = activity.getString(R.string.storage_gallery_amount, size(data.readyBytes), data.readyCount)
                galleryPending.text = activity.getString(R.string.storage_gallery_pending, size(data.pendingBytes), data.pendingCount) +
                    if (data.unknownSizes > 0) "\n" + activity.getString(R.string.storage_unknown_sizes, data.unknownSizes) else ""
            }.onFailure { galleryBytes.setText(R.string.storage_unavailable); galleryPending.text = "" }
            updated.setText(if (settings.isSuccess && originals.isSuccess && gallery.isSuccess) R.string.storage_snapshot_note else R.string.storage_snapshot_failed)
        }
        work?.invokeOnCompletion {
            if (pendingDetails && activity.lifecycle.currentState.isAtLeast(androidx.lifecycle.Lifecycle.State.STARTED)) {
                pendingDetails = false
                activity.lifecycleScope.launch { refresh(force = true) }
            }
        }
    }
    private fun size(bytes: Long) = Formatter.formatFileSize(activity, bytes)
}
